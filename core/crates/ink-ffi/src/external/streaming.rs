//! Live streams from a shell engine (`INK_ENGINE_STREAMING`), as a [`StreamingEngine`].
//!
//! A stream's life, as the header gives it: `stream_open`, `stream_push` any number of times,
//! `stream_finish` unless the core abandons the stream, then `stream_close`, exactly once per open
//! whatever happened before. [`ExternalStream`]'s drop is what closes, so an error, a timeout, a
//! chain dropping its stream and shutdown all close it the same way.
//!
//! The engine's words come back through `ink_stream_event` ([`stream_event`]), from any thread,
//! into the sink the chain handed to [`StreamingEngine::open_stream`]. Streams are numbered by the
//! core and looked up in a process-wide table, so an event for a stream that is closed, or was
//! never opened, gets an error code instead of reaching a chain that has moved on. Each stream's
//! sink sits behind its own lock, which closing takes too: once a stream is closed no event of its
//! reaches the chain, even one racing the close.
//!
//! Steps have deadlines ([`Timeouts`]). A push must be answered well within real time (the
//! recognition itself runs elsewhere); a shell engine that stops answering costs its stream (the
//! chain reports the live engine failed and goes on) rather than stalling the meeting's worker.

use std::collections::HashMap;
use std::ffi::CString;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use ink_core::{
    AsrEvent, Channel, EngineError, EngineInfo, EngineStream, EventSink, StreamingEngine, TimedText,
};
use ink_engines::JobScore;
use serde_json::{Value, json};

use super::{
    Answer, Call, Expect, GaveUp, Shell, ShellError, StreamCloseFn, StreamFinishFn, StreamOpenFn,
    StreamPushFn,
};
use crate::events;

/// The four functions of a streaming engine's table.
pub(crate) struct Functions {
    pub(crate) open: StreamOpenFn,
    pub(crate) push: StreamPushFn,
    pub(crate) finish: StreamFinishFn,
    pub(crate) close: StreamCloseFn,
}

/// How long each step of a stream may take to be answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeouts {
    /// `stream_open`.
    pub open: Duration,
    /// `stream_push`: the engine copies the samples and answers; recognition runs elsewhere.
    pub push: Duration,
    /// `stream_finish`: the last recognition and the trailing events.
    pub finish: Duration,
}

impl Default for Timeouts {
    /// The header's: 10 s to open, 2 s per push, 30 s to finish.
    fn default() -> Self {
        Self {
            open: Duration::from_secs(10),
            push: Duration::from_secs(2),
            finish: Duration::from_secs(30),
        }
    }
}

struct Inner {
    shell: Shell,
    functions: Functions,
    timeouts: Timeouts,
}

/// A registered streaming engine. Its `release` runs once the router has let go of it and every
/// stream it opened is closed.
pub struct ExternalStreaming {
    inner: Arc<Inner>,
    info: EngineInfo,
    scores: Vec<JobScore>,
}

impl ExternalStreaming {
    pub(crate) fn new(
        shell: Shell,
        functions: Functions,
        info: EngineInfo,
        scores: Vec<JobScore>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                shell,
                functions,
                timeouts: Timeouts::default(),
            }),
            info,
            scores,
        }
    }

    /// The id it registered under.
    pub fn id(&self) -> &str {
        &self.info.id
    }

    /// The error rate per job the shell registered.
    pub fn scores(&self) -> &[JobScore] {
        &self.scores
    }

    /// Makes the drop skip `release`: for a table the router refused.
    pub fn disarm(&self) {
        self.inner.shell.disarm();
    }

    /// The same engine with other deadlines, before it is shared (tests shorten them).
    pub fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.timeouts = timeouts;
        }
        self
    }
}

impl StreamingEngine for ExternalStreaming {
    fn info(&self) -> EngineInfo {
        // The copy taken at registration: never a call into the shell.
        self.info.clone()
    }

    fn open_stream(
        &self,
        channel: Channel,
        events: EventSink<AsrEvent>,
    ) -> Result<Box<dyn EngineStream>, EngineError> {
        let options = CString::new(json!({"channel": events::channel(channel)}).to_string())
            .map_err(|_| EngineError::Failed("the stream options held a NUL byte".into()))?;
        // Registered before the engine hears of it, so an event the engine sends while it answers
        // the open already has somewhere to go. From here the stream's drop closes it, whatever
        // the open answers: the header promises a close for every open.
        let stream = ExternalStream::register(self.inner.clone(), events);
        let call = Call::open(Expect::Ack);
        let id = call.id();
        let f = self.inner.functions.open;
        // SAFETY: the shell's function with its own `ctx`, valid until `release`, which cannot
        // run while `self.inner` is alive. `options` is valid until the call returns.
        unsafe { f(self.inner.shell.ctx, id, stream.id, options.as_ptr()) };
        stream.settle(call, self.inner.timeouts.open, "stream_open")?;
        Ok(Box::new(stream))
    }
}

/// Where a stream's events go, until it is closed.
struct Route {
    sink: Mutex<Option<EventSink<AsrEvent>>>,
    /// The engine's id, to name it in the events the core writes itself.
    engine: String,
}

impl Route {
    fn sink(&self) -> MutexGuard<'_, Option<EventSink<AsrEvent>>> {
        // A take or a call of the sink: consistent at every step.
        self.sink.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Open streams, by number.
static STREAMS: LazyLock<Mutex<HashMap<u64, Arc<Route>>>> = LazyLock::new(Mutex::default);
static NEXT_STREAM: AtomicU64 = AtomicU64::new(0);

fn streams() -> MutexGuard<'static, HashMap<u64, Arc<Route>>> {
    // Every critical section is one insert, lookup or removal.
    STREAMS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One open stream of a shell engine. Dropping it closes it (see the module docs).
pub struct ExternalStream {
    inner: Arc<Inner>,
    id: u64,
    route: Arc<Route>,
}

impl ExternalStream {
    fn register(inner: Arc<Inner>, events: EventSink<AsrEvent>) -> Self {
        // Starts at 1, so a zeroed number is never a stream.
        let id = NEXT_STREAM.fetch_add(1, Ordering::Relaxed) + 1;
        let route = Arc::new(Route {
            sink: Mutex::new(Some(events)),
            engine: inner.shell.id.clone(),
        });
        streams().insert(id, route.clone());
        Self { inner, id, route }
    }

    /// **Worker.** Waits for a step's answer: `Ok` for `{"ok":true}`, the engine's error, or a
    /// failure naming the step when the core gave up.
    fn settle(&self, call: Call, timeout: Duration, step: &str) -> Result<(), EngineError> {
        let shell = &self.inner.shell;
        let id = call.id();
        match call.wait(
            || shell.shutting_down(),
            Some(timeout),
            || shell.cancel_call(id),
        ) {
            Ok(Ok(Answer::Ack)) => Ok(()),
            Ok(Ok(_)) => Err(ShellError::UNREADABLE.engine_error(&shell.id)),
            Ok(Err(e)) => Err(e.engine_error(&shell.id)),
            Err(GaveUp::Cancelled) => Err(EngineError::Cancelled),
            Err(GaveUp::TimedOut) => {
                log::warn!(
                    "shell engine {}: {step} was not answered within {timeout:?}; the stream is \
                     closed",
                    shell.id
                );
                Err(EngineError::Failed(format!(
                    "shell engine {} did not answer {step} within {timeout:?}",
                    shell.id
                )))
            }
        }
    }
}

impl EngineStream for ExternalStream {
    fn push(&mut self, audio: &[f32]) -> Result<(), EngineError> {
        if audio.is_empty() {
            return Ok(());
        }
        let call = Call::open(Expect::Ack);
        let f = self.inner.functions.push;
        // SAFETY: as in `open_stream`; the samples are valid until the call returns, which is all
        // the header promises.
        unsafe {
            f(
                self.inner.shell.ctx,
                call.id(),
                self.id,
                audio.as_ptr(),
                audio.len(),
            )
        };
        self.settle(call, self.inner.timeouts.push, "stream_push")
    }

    fn finish(self: Box<Self>) -> Result<(), EngineError> {
        let call = Call::open(Expect::Ack);
        let f = self.inner.functions.finish;
        // SAFETY: as in `open_stream`.
        unsafe { f(self.inner.shell.ctx, call.id(), self.id) };
        // The engine sent every trailing event before it answered; the drop then closes.
        self.settle(call, self.inner.timeouts.finish, "stream_finish")
    }
}

impl Drop for ExternalStream {
    fn drop(&mut self) {
        // No event of this stream reaches the chain from here, even one racing this close.
        self.route.sink().take();
        streams().remove(&self.id);
        let f = self.inner.functions.close;
        // SAFETY: as in `open_stream`; called once, for a stream the engine was asked to open.
        unsafe { f(self.inner.shell.ctx, self.id) };
    }
}

/// Why [`stream_event`] refused an event.
#[derive(Debug, PartialEq, Eq)]
pub enum StreamEventError {
    /// The stream is not open: never opened, or closed already. The event is dropped.
    Unknown,
    /// The event is not one the header describes. It is dropped.
    Malformed,
}

/// An event of stream `stream` (`ink_stream_event`). **Any thread**; it only queues.
pub fn stream_event(stream: u64, json: &str) -> Result<(), StreamEventError> {
    let Some(route) = streams().get(&stream).cloned() else {
        return Err(StreamEventError::Unknown);
    };
    let event = parse_event(&route.engine, json).ok_or(StreamEventError::Malformed)?;
    let sink = route.sink();
    let Some(sink) = sink.as_ref() else {
        return Err(StreamEventError::Unknown);
    };
    // The chain's sink only queues (ink-core's EventSink contract), so calling it under the
    // stream's lock never waits on anything.
    sink(event);
    Ok(())
}

/// One of the header's events, or `None`. An object with exactly one of `partial`, `final` or
/// `stalled`.
fn parse_event(engine: &str, json: &str) -> Option<AsrEvent> {
    let v: Value = serde_json::from_str(json).ok()?;
    let obj = v.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    let (key, value) = obj.iter().next()?;
    Some(match key.as_str() {
        "partial" => AsrEvent::Partial {
            text: value.as_str()?.to_owned(),
        },
        "final" => {
            let start_ms = value.get("start_ms")?.as_u64()?;
            let end_ms = value.get("end_ms")?.as_u64()?;
            if end_ms < start_ms {
                return None;
            }
            AsrEvent::Final(TimedText {
                start_ms,
                end_ms,
                text: value.get("text")?.as_str()?.to_owned(),
            })
        }
        "stalled" => {
            let code = match value.get("code") {
                None => None,
                Some(c) => Some(c.as_i64()?),
            };
            // The core writes the reason: the engine's own text is never read (I5).
            AsrEvent::Stalled {
                reason: match code {
                    Some(c) => format!("shell engine {engine} fell behind real time (code {c})"),
                    None => format!("shell engine {engine} fell behind real time"),
                },
            }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::{c_char, c_void};
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    use ink_core::CancelToken;

    use super::*;

    #[test]
    fn events_parse_or_are_refused() {
        assert_eq!(
            parse_event("e", r#"{"partial":"so the"}"#),
            Some(AsrEvent::Partial {
                text: "so the".into()
            })
        );
        assert_eq!(
            parse_event(
                "e",
                r#"{"final":{"start_ms":10,"end_ms":900,"text":"so the plan"}}"#
            ),
            Some(AsrEvent::Final(TimedText {
                start_ms: 10,
                end_ms: 900,
                text: "so the plan".into()
            }))
        );
        assert_eq!(
            parse_event("e", r#"{"stalled":{"code":4}}"#),
            Some(AsrEvent::Stalled {
                reason: "shell engine e fell behind real time (code 4)".into()
            })
        );
        for bad in [
            "{",
            "{}",
            r#"{"partial":3}"#,
            r#"{"partial":"a","final":{"start_ms":0,"end_ms":1,"text":"b"}}"#,
            r#"{"final":{"start_ms":900,"end_ms":10,"text":"backwards"}}"#,
            r#"{"final":{"text":"no times"}}"#,
            r#"{"stalled":{"code":"slow"}}"#,
            r#"{"typing":"x"}"#,
        ] {
            assert_eq!(parse_event("e", bad), None, "{bad}");
        }
    }

    /// I5: a stall's reason is the core's, never the engine's words.
    #[test]
    fn a_stall_never_carries_the_engine_s_text() {
        let e = parse_event("e", r#"{"stalled":{"reason":"heard zebrafish"}}"#).unwrap();
        assert!(!format!("{e:?}").contains("zebrafish"));
    }

    /// A fake engine for the timeout path: it never answers a push.
    #[derive(Default)]
    struct Mute {
        closed: AtomicU64,
        cancels: AtomicU64,
        answer_open: AtomicBool,
        last_stream: AtomicU64,
    }

    unsafe extern "C" fn mute_open(ctx: *mut c_void, call: u64, stream: u64, _: *const c_char) {
        // SAFETY: ctx is the test's `Mute`, alive for the test.
        let me = unsafe { &*(ctx as *const Mute) };
        me.last_stream.store(stream, Ordering::SeqCst);
        if me.answer_open.load(Ordering::SeqCst) {
            let _ = super::super::complete(call, r#"{"ok":true}"#);
        }
    }
    unsafe extern "C" fn mute_push(_: *mut c_void, _: u64, _: u64, _: *const f32, _: usize) {}
    unsafe extern "C" fn mute_finish(_: *mut c_void, _: u64, _: u64) {}
    unsafe extern "C" fn mute_close(ctx: *mut c_void, _: u64) {
        // SAFETY: as above.
        let me = unsafe { &*(ctx as *const Mute) };
        me.closed.fetch_add(1, Ordering::SeqCst);
    }
    unsafe extern "C" fn mute_cancel(ctx: *mut c_void, _: u64) {
        // SAFETY: as above.
        let me = unsafe { &*(ctx as *const Mute) };
        me.cancels.fetch_add(1, Ordering::SeqCst);
    }

    fn mute_engine(me: &Mute, timeouts: Timeouts) -> ExternalStreaming {
        let table = super::super::InkEngineVTable {
            kind: super::super::KIND_STREAMING,
            info_json: c"{\"id\":\"mute\",\"licence\":\"MIT\",\"jobs\":[{\"job\":\"live_partials\",\"wer\":1}]}".as_ptr(),
            ctx: me as *const Mute as *mut c_void,
            cancel: Some(mute_cancel),
            stream_open: Some(mute_open),
            stream_push: Some(mute_push),
            stream_finish: Some(mute_finish),
            stream_close: Some(mute_close),
            ..Default::default()
        };
        // SAFETY: a valid table whose ctx outlives the engine.
        match unsafe { super::super::Registration::from_table(&table, CancelToken::new()) } {
            Ok(super::super::Registration::Streaming(e)) => e.with_timeouts(timeouts),
            _ => panic!("a streaming table"),
        }
    }

    fn quick() -> Timeouts {
        Timeouts {
            open: Duration::from_millis(80),
            push: Duration::from_millis(80),
            finish: Duration::from_millis(80),
        }
    }

    #[test]
    fn a_push_that_is_never_answered_fails_in_time_and_the_stream_is_closed() {
        let me = Mute::default();
        me.answer_open.store(true, Ordering::SeqCst);
        let engine = mute_engine(&me, quick());
        let mut stream = engine
            .open_stream(Channel::Mic, Arc::new(|_| {}))
            .expect("opened");
        let started = Instant::now();
        let error = stream.push(&[0.1; 160]).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(
            matches!(&error, EngineError::Failed(m) if m.contains("stream_push")),
            "{error}"
        );
        assert_eq!(
            me.cancels.load(Ordering::SeqCst),
            1,
            "told the core gave up"
        );
        assert_eq!(me.closed.load(Ordering::SeqCst), 0);
        drop(stream);
        assert_eq!(me.closed.load(Ordering::SeqCst), 1, "closed on drop");
    }

    #[test]
    fn a_stream_whose_open_fails_is_closed_all_the_same() {
        let me = Mute::default();
        let engine = mute_engine(&me, quick());
        let error = engine
            .open_stream(Channel::Far, Arc::new(|_| {}))
            .err()
            .expect("the open was never answered");
        assert!(matches!(error, EngineError::Failed(_)), "{error}");
        assert_eq!(
            me.closed.load(Ordering::SeqCst),
            1,
            "a close for every open"
        );
    }

    #[test]
    fn events_reach_the_sink_until_the_stream_closes() {
        let me = Mute::default();
        me.answer_open.store(true, Ordering::SeqCst);
        let engine = mute_engine(&me, quick());
        let got = Arc::new(Mutex::new(Vec::new()));
        let sink = got.clone();
        let stream = engine
            .open_stream(
                Channel::Mic,
                Arc::new(move |e| sink.lock().unwrap().push(e)),
            )
            .unwrap();
        let id = me.last_stream.load(Ordering::SeqCst);
        assert_eq!(stream_event(id, r#"{"partial":"hel"}"#), Ok(()));
        assert_eq!(
            stream_event(id, r#"{"partial":7}"#),
            Err(StreamEventError::Malformed)
        );
        drop(stream);
        assert_eq!(
            stream_event(id, r#"{"partial":"late"}"#),
            Err(StreamEventError::Unknown)
        );
        assert_eq!(
            *got.lock().unwrap(),
            vec![AsrEvent::Partial { text: "hel".into() }]
        );
        assert_eq!(
            stream_event(0, r#"{"partial":"x"}"#),
            Err(StreamEventError::Unknown)
        );
    }
}
