//! Engines the shell registers over the C ABI (`InkEngineVTable`), as [`OfflineEngine`]s.
//!
//! A transcription is a call id and a wait. The worker opens a pending call, hands the id to the
//! engine's `transcribe`, and waits until [`complete`] answers it, from whatever thread the
//! engine answers on (before `transcribe` returns, or later). The wait ends early, and the call
//! is forgotten, when the job's cancel token or the core's shutdown token is set, so a shell
//! engine that never answers cannot hold up shutdown. A late answer then finds no call and is
//! refused with [`Unknown`], which is harmless.
//!
//! Call ids index a process-wide table rather than being pointers, so an engine that answers
//! twice, or after the core shut down, gets an error code instead of undefined behaviour.

use std::collections::HashMap;
use std::ffi::{CString, c_char, c_void};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, PoisonError};
use std::time::Duration;

use ink_core::{
    CancelToken, EngineError, EngineInfo, OfflineEngine, TimedText, TranscribeOptions, Transcript,
};
use ink_engines::JobScore;
use serde_json::{Value, json};

use crate::events;

/// `INK_ENGINE_OFFLINE`.
pub const KIND_OFFLINE: u32 = 1;

/// `InkEngineVTable`, field for field (see `inkwell.h`).
#[repr(C)]
pub struct InkEngineVTable {
    /// `sizeof(InkEngineVTable)` as the shell compiled it.
    pub size: u32,
    /// `INK_ENGINE_OFFLINE`.
    pub kind: u32,
    /// The engine's id, licence and jobs, as JSON.
    pub info_json: *const c_char,
    /// The engine's own pointer, passed back to every function.
    pub ctx: *mut c_void,
    /// Starts a transcription (required).
    pub transcribe:
        Option<unsafe extern "C" fn(*mut c_void, u64, *const f32, usize, *const c_char)>,
    /// Asks the engine to stop a call early (optional).
    pub cancel: Option<unsafe extern "C" fn(*mut c_void, u64)>,
    /// The core has let go of the engine (optional).
    pub release: Option<unsafe extern "C" fn(*mut c_void)>,
}

/// What an answer settles to.
type Answer = Result<Transcript, EngineError>;

#[derive(Default)]
struct Slot {
    answer: Mutex<Option<Answer>>,
    ready: Condvar,
    /// The engine's registered id, to name it in errors.
    engine: String,
}

#[derive(Default)]
struct Pending {
    calls: Mutex<HashMap<u64, Arc<Slot>>>,
    next: AtomicU64,
}

static PENDING: LazyLock<Pending> = LazyLock::new(Pending::default);

impl Pending {
    fn open(&self, engine: &str) -> (u64, Arc<Slot>) {
        // Starts at 1, so a zeroed id is never a real call.
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let slot = Arc::new(Slot {
            engine: engine.to_owned(),
            ..Slot::default()
        });
        self.lock().insert(id, slot.clone());
        (id, slot)
    }

    /// Takes call `id` out of the table. `false` when [`complete`] already took it: its answer
    /// is being written.
    fn forget(&self, id: u64) -> bool {
        self.lock().remove(&id).is_some()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Arc<Slot>>> {
        self.calls.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The call is not waiting: answered already, given up, or never issued.
#[derive(Debug, PartialEq, Eq)]
pub struct Unknown;

/// Why [`complete`] could not use an answer.
#[derive(Debug, PartialEq, Eq)]
pub enum CompleteError {
    /// See [`Unknown`].
    Unknown,
    /// The answer was not JSON of the expected shape. The call is answered with an error.
    Malformed,
}

/// Answers call `id` with the engine's `result_json` (see `inkwell.h`). **Any thread.**
pub fn complete(id: u64, result_json: &str) -> Result<(), CompleteError> {
    let Some(slot) = PENDING.lock().remove(&id) else {
        return Err(CompleteError::Unknown);
    };
    let parsed = parse_answer(&slot.engine, result_json);
    let malformed = parsed.is_none();
    let answer = parsed.unwrap_or_else(|| {
        Err(EngineError::Failed(format!(
            "shell engine {}: its answer could not be read",
            slot.engine
        )))
    });
    *slot.answer.lock().unwrap_or_else(PoisonError::into_inner) = Some(answer);
    slot.ready.notify_all();
    if malformed {
        Err(CompleteError::Malformed)
    } else {
        Ok(())
    }
}

/// An answer from engine `engine`: segments, or an error.
///
/// An error is read as its kind and an optional integer code, and nothing else: an engine's own
/// text could quote what it heard, nothing here could check it, and errors reach events and logs
/// (I5). Any other field of the error, a `message` included, is ignored.
fn parse_answer(engine: &str, json: &str) -> Option<Answer> {
    let v: Value = serde_json::from_str(json).ok()?;
    if let Some(error) = v.get("error") {
        let code = match error.get("code") {
            None => None,
            Some(c) => Some(c.as_i64()?),
        };
        return Some(Err(match error.get("kind").and_then(Value::as_str)? {
            "cancelled" => EngineError::Cancelled,
            "model_missing" => EngineError::ModelMissing(format!("shell engine {engine}")),
            "bad_request" => {
                EngineError::Failed(format!("shell engine {engine} could not read the request"))
            }
            // "failed", and any kind this core does not know, which is not echoed either.
            _ => EngineError::Failed(match code {
                Some(code) => format!("shell engine {engine} failed (code {code})"),
                None => format!("shell engine {engine} failed"),
            }),
        }));
    }
    let segments = v
        .get("segments")?
        .as_array()?
        .iter()
        .map(|s| {
            Some(TimedText {
                start_ms: s.get("start_ms")?.as_u64()?,
                end_ms: s.get("end_ms")?.as_u64()?,
                text: s.get("text")?.as_str()?.to_owned(),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Ok(Transcript { segments }))
}

/// Why a table could not be registered.
#[derive(Debug, PartialEq, Eq)]
pub struct BadTable(pub String);

/// A registered shell engine. Its `release` runs when the last reference drops.
pub struct ExternalOffline {
    ctx: *mut c_void,
    transcribe: unsafe extern "C" fn(*mut c_void, u64, *const f32, usize, *const c_char),
    cancel: Option<unsafe extern "C" fn(*mut c_void, u64)>,
    release: Option<unsafe extern "C" fn(*mut c_void)>,
    info: EngineInfo,
    scores: Vec<JobScore>,
    shutdown: CancelToken,
    /// Whether `release` runs on drop.
    armed: AtomicBool,
}

// SAFETY: the header's contract (THREADS 4 and 5) is that an engine's functions may be called
// from any core worker thread, several at once, and that `ctx` is valid until `release`. The
// shell promises the engine is thread-safe by registering it; the core only passes `ctx` back.
unsafe impl Send for ExternalOffline {}
// SAFETY: as for `Send`: every use of `ctx` is a call into the engine, which the contract allows
// concurrently from several threads.
unsafe impl Sync for ExternalOffline {}

impl ExternalOffline {
    /// Copies a table the shell passed. `shutdown` is the core's: a call waiting on this engine
    /// gives up when it is set.
    ///
    /// # Safety
    ///
    /// `table` points to a readable `InkEngineVTable` of at least the `size` it states, and its
    /// `info_json` is NULL or a NUL-terminated string, both valid for this call.
    pub unsafe fn from_table(
        table: *const InkEngineVTable,
        shutdown: CancelToken,
    ) -> Result<Self, BadTable> {
        let bad = |why: &str| BadTable(why.to_owned());
        if table.is_null() {
            return Err(bad("the table is NULL"));
        }
        // SAFETY: the caller guarantees a readable table; `size` is its first field, and a table
        // smaller than ours is refused before any other field is read.
        let size = unsafe { (*table).size } as usize;
        if size < std::mem::size_of::<InkEngineVTable>() {
            return Err(bad("the table is smaller than this core's InkEngineVTable"));
        }
        // SAFETY: at least our whole struct is readable, checked above.
        let t = unsafe { &*table };
        if t.kind != KIND_OFFLINE {
            return Err(bad("unknown engine kind"));
        }
        let transcribe = t.transcribe.ok_or_else(|| bad("transcribe is NULL"))?;
        if t.info_json.is_null() {
            return Err(bad("info_json is NULL"));
        }
        // SAFETY: non-null, and the caller guarantees a NUL-terminated string.
        let info = unsafe { crate::bounded_str(t.info_json, crate::INK_MAX_JSON) }
            .ok_or_else(|| bad("info_json is not UTF-8, or longer than INK_MAX_JSON"))?;
        let (info, scores) = parse_info(info).ok_or_else(|| {
            bad("info_json must be {\"id\",\"licence\",\"jobs\":[{\"job\",\"wer\"}]}")
        })?;
        Ok(Self {
            ctx: t.ctx,
            transcribe,
            cancel: t.cancel,
            release: t.release,
            info,
            scores,
            shutdown,
            armed: AtomicBool::new(true),
        })
    }

    /// The error rate per job the shell registered.
    pub fn scores(&self) -> &[JobScore] {
        &self.scores
    }

    /// Makes the drop skip `release`: for a table the router refused, which the header promises
    /// is never released.
    pub fn disarm(&self) {
        self.armed.store(false, Ordering::Release);
    }
}

fn parse_info(json: &str) -> Option<(EngineInfo, Vec<JobScore>)> {
    let v: Value = serde_json::from_str(json).ok()?;
    let id = v.get("id")?.as_str()?.to_owned();
    let licence = v.get("licence")?.as_str()?.to_owned();
    let scores = v
        .get("jobs")?
        .as_array()?
        .iter()
        .map(|j| {
            Some(JobScore {
                job: events::parse_job(j.get("job")?.as_str()?)?,
                wer: j.get("wer")?.as_f64()? as f32,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let info = EngineInfo {
        id,
        jobs: scores.iter().map(|s| s.job).collect(),
        licence,
    };
    Some((info, scores))
}

/// How often a waiting worker looks at its cancel tokens. They are flags with no wake-up, so a
/// worker blocked on an answer checks them this often, and only while a call is in flight.
const CANCEL_POLL: Duration = Duration::from_millis(20);

impl OfflineEngine for ExternalOffline {
    fn info(&self) -> EngineInfo {
        // The copy taken at registration: never a call into the shell.
        self.info.clone()
    }

    fn transcribe(
        &self,
        audio: &[f32],
        options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError> {
        let mut o = json!({"channel": events::channel(options.channel)});
        if let Some(context) = &options.context {
            o["context"] = Value::from(context.as_str());
        }
        let o = CString::new(o.to_string())
            .map_err(|_| EngineError::Failed("the options held a NUL byte".into()))?;
        let (id, slot) = PENDING.open(&self.info.id);
        // SAFETY: the function pointer came from the shell's table; `ctx` is the engine's own
        // and valid until `release`, which runs only after the last reference (this one
        // included) is dropped. The samples and options are valid until the call returns,
        // which is all the header promises.
        unsafe { (self.transcribe)(self.ctx, id, audio.as_ptr(), audio.len(), o.as_ptr()) };
        let cancelled = || options.cancel.is_cancelled() || self.shutdown.is_cancelled();
        let on_cancel = || {
            if let Some(cancel) = self.cancel {
                // SAFETY: as above; `cancel` is the shell's optional function.
                unsafe { cancel(self.ctx, id) };
            }
        };
        await_answer(id, &slot, cancelled, on_cancel)
    }
}

/// **Worker.** Waits for call `id`'s answer, or gives up once `cancelled` says so, calling
/// `on_cancel` then.
///
/// Giving up is committed by taking the call out of the table. If [`complete`] took it first, the
/// engine's answer is real and already on its way (complete writes it next, without waiting on
/// anything), so it is waited for and kept rather than lost to the cancel.
fn await_answer(
    id: u64,
    slot: &Slot,
    cancelled: impl Fn() -> bool,
    on_cancel: impl FnOnce(),
) -> Answer {
    let mut answer = slot.answer.lock().unwrap_or_else(PoisonError::into_inner);
    let mut answered = false;
    loop {
        if let Some(a) = answer.take() {
            return a;
        }
        if !answered && cancelled() {
            drop(answer);
            if PENDING.forget(id) {
                on_cancel();
                return Err(EngineError::Cancelled);
            }
            answered = true;
            answer = slot.answer.lock().unwrap_or_else(PoisonError::into_inner);
            continue;
        }
        answer = slot
            .ready
            .wait_timeout(answer, CANCEL_POLL)
            .unwrap_or_else(PoisonError::into_inner)
            .0;
    }
}

impl Drop for ExternalOffline {
    fn drop(&mut self) {
        if let Some(release) = self.release
            && self.armed.load(Ordering::Acquire)
        {
            // SAFETY: the last reference is going; no transcribe is in flight (each holds a
            // reference), so this is the "after the last call" the header promises.
            unsafe { release(self.ctx) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_parse_to_transcripts_or_errors() {
        let t = parse_answer(
            "e",
            r#"{"segments":[{"start_ms":0,"end_ms":5,"text":"a"}]}"#,
        );
        assert_eq!(t.unwrap().unwrap().text(), "a");
        assert_eq!(
            parse_answer("e", r#"{"error":{"kind":"cancelled"}}"#).unwrap(),
            Err(EngineError::Cancelled)
        );
        assert_eq!(
            parse_answer("e", r#"{"error":{"kind":"failed","code":7}}"#).unwrap(),
            Err(EngineError::Failed("shell engine e failed (code 7)".into()))
        );
        assert_eq!(
            parse_answer("e", r#"{"error":{"kind":"model_missing"}}"#).unwrap(),
            Err(EngineError::ModelMissing("shell engine e".into()))
        );
        assert_eq!(
            parse_answer("e", r#"{"error":{"kind":"bad_request"}}"#).unwrap(),
            Err(EngineError::Failed(
                "shell engine e could not read the request".into()
            ))
        );
        assert!(parse_answer("e", "{").is_none());
        assert!(parse_answer("e", r#"{"segments":[{"text":"no times"}]}"#).is_none());
        assert!(parse_answer("e", r#"{"error":{"kind":"failed","code":"7"}}"#).is_none());
    }

    /// I5: an engine's error text could quote what it heard, and nothing checks it. The core
    /// never reads it: errors reach events and logs as a kind and a code.
    #[test]
    fn an_engine_s_free_text_never_reaches_an_error() {
        for answer in [
            r#"{"error":{"kind":"failed","code":3,"message":"heard: zebrafish"}}"#,
            r#"{"error":{"kind":"model_missing","message":"zebrafish"}}"#,
            r#"{"error":{"kind":"zebrafish"}}"#,
        ] {
            let error = parse_answer("e", answer).unwrap().unwrap_err();
            assert!(!error.to_string().contains("zebrafish"), "{error}");
        }
    }

    #[test]
    fn a_call_is_answered_once_and_unknown_ids_are_refused() {
        let (id, slot) = PENDING.open("e");
        assert_eq!(complete(id, r#"{"segments":[]}"#), Ok(()));
        assert!(slot.answer.lock().unwrap().is_some());
        assert_eq!(
            complete(id, r#"{"segments":[]}"#),
            Err(CompleteError::Unknown)
        );
        assert_eq!(complete(0, "{}"), Err(CompleteError::Unknown));
        let (id, slot) = PENDING.open("e");
        assert_eq!(complete(id, "not json"), Err(CompleteError::Malformed));
        assert!(matches!(
            *slot.answer.lock().unwrap(),
            Some(Err(EngineError::Failed(_)))
        ));
    }

    /// The race: `complete` has taken the call from the table and not yet written its answer when
    /// the worker decides to cancel. The answer is real and on its way: it must be kept.
    #[test]
    fn an_answer_taken_before_the_cancel_is_kept() {
        let (id, slot) = PENDING.open("e");
        // complete()'s first half: the call leaves the table.
        let taken = PENDING.lock().remove(&id).expect("the call was pending");
        let waiter = std::thread::spawn(move || {
            let mut cancelled_by_us = false;
            let answer = await_answer(id, &taken, || true, || cancelled_by_us = true);
            (answer, cancelled_by_us)
        });
        std::thread::sleep(Duration::from_millis(50));
        // complete()'s second half: the answer is written.
        *slot.answer.lock().unwrap() = parse_answer(
            "e",
            r#"{"segments":[{"start_ms":0,"end_ms":1,"text":"kept"}]}"#,
        );
        slot.ready.notify_all();
        let (answer, cancelled) = waiter.join().unwrap();
        assert_eq!(answer.unwrap().text(), "kept");
        assert!(
            !cancelled,
            "the engine is not asked to cancel a call it answered"
        );
    }

    #[test]
    fn a_cancel_before_any_answer_forgets_the_call() {
        let (id, slot) = PENDING.open("e");
        let mut asked = false;
        assert_eq!(
            await_answer(id, &slot, || true, || asked = true),
            Err(EngineError::Cancelled)
        );
        assert!(asked);
        assert_eq!(
            complete(id, r#"{"segments":[]}"#),
            Err(CompleteError::Unknown)
        );
    }

    #[test]
    fn info_lists_jobs_with_their_rates() {
        let (info, scores) = parse_info(
            r#"{"id":"swift-mock","licence":"MIT","jobs":[{"job":"meeting_final","wer":9.5}]}"#,
        )
        .unwrap();
        assert_eq!(info.id, "swift-mock");
        assert_eq!(scores.len(), 1);
        assert!(
            parse_info(r#"{"id":"x","licence":"MIT","jobs":[{"job":"typing","wer":1}]}"#).is_none()
        );
    }
}
