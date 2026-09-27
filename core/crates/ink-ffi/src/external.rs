//! Engines the shell registers over the C ABI (`InkEngineVTable`): offline engines
//! ([`ExternalOffline`]), live streams ([`streaming`]) and language models ([`llm`]).
//!
//! Every call into a shell engine is a call id and a wait. The worker opens a pending call, hands
//! the id to the engine's function, and waits until [`complete`] answers it, from whatever thread
//! the engine answers on (before its function returns, or later). The wait ends early, and the
//! call is forgotten, when the job's cancel token or the core's shutdown token is set, or when the
//! call's deadline passes (live streams and language models have one), so a shell engine that
//! never answers cannot hold up a meeting or shutdown. A late answer then finds no call and is
//! refused with [`CompleteError::Unknown`], which is harmless.
//!
//! Call ids index a process-wide table rather than being pointers, so an engine that answers
//! twice, or after the core shut down, gets an error code instead of undefined behaviour.
//!
//! # Tables
//!
//! A table is copied up to the size it states, into a zeroed table of this core's layout, so a
//! shell built against ABI 1 (a table ending at `release`) still registers an offline engine, and
//! the ABI 2 fields read as NULL for it. A streaming engine or a language model needs the ABI 2
//! fields, so its table must be at least this core's size. Each kind's functions are required and
//! every other kind's must be NULL: a table that fills another kind's function almost certainly
//! states the wrong kind, and is refused rather than half used.

pub mod llm;
pub mod streaming;

use std::collections::HashMap;
use std::ffi::{CString, c_char, c_void};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ink_core::{
    CancelToken, EngineError, EngineInfo, LlmError, OfflineEngine, TimedText, TranscribeOptions,
    Transcript,
};
use ink_engines::JobScore;
use serde_json::{Value, json};

use crate::events;

pub use self::llm::ExternalLlm;
pub use self::streaming::{ExternalStreaming, StreamEventError, stream_event};

/// `INK_ENGINE_OFFLINE`.
pub const KIND_OFFLINE: u32 = 1;
/// `INK_ENGINE_STREAMING`.
pub const KIND_STREAMING: u32 = 2;
/// `INK_ENGINE_LLM`.
pub const KIND_LLM: u32 = 3;

/// `transcribe`.
pub type TranscribeFn = unsafe extern "C" fn(*mut c_void, u64, *const f32, usize, *const c_char);
/// `cancel`.
pub type CancelFn = unsafe extern "C" fn(*mut c_void, u64);
/// `release`.
pub type ReleaseFn = unsafe extern "C" fn(*mut c_void);
/// `stream_open`.
pub type StreamOpenFn = unsafe extern "C" fn(*mut c_void, u64, u64, *const c_char);
/// `stream_push`.
pub type StreamPushFn = unsafe extern "C" fn(*mut c_void, u64, u64, *const f32, usize);
/// `stream_finish`.
pub type StreamFinishFn = unsafe extern "C" fn(*mut c_void, u64, u64);
/// `stream_close`.
pub type StreamCloseFn = unsafe extern "C" fn(*mut c_void, u64);
/// `generate`.
pub type GenerateFn = unsafe extern "C" fn(*mut c_void, u64, *const c_char);

/// `InkEngineVTable`, field for field (see `inkwell.h`).
#[repr(C)]
pub struct InkEngineVTable {
    /// `sizeof(InkEngineVTable)` as the shell compiled it.
    pub size: u32,
    /// `INK_ENGINE_*`.
    pub kind: u32,
    /// The engine's id, licence and jobs (or model), as JSON.
    pub info_json: *const c_char,
    /// The engine's own pointer, passed back to every function.
    pub ctx: *mut c_void,
    /// Transcribes a buffer (offline engines).
    pub transcribe: Option<TranscribeFn>,
    /// Asks the engine to stop a call early (optional, any kind).
    pub cancel: Option<CancelFn>,
    /// The core has let go of the engine (optional, any kind).
    pub release: Option<ReleaseFn>,
    /// Opens a live stream (streaming engines; ABI 2).
    pub stream_open: Option<StreamOpenFn>,
    /// Feeds a live stream (streaming engines; ABI 2).
    pub stream_push: Option<StreamPushFn>,
    /// Flushes a live stream (streaming engines; ABI 2).
    pub stream_finish: Option<StreamFinishFn>,
    /// Frees a live stream (streaming engines; ABI 2).
    pub stream_close: Option<StreamCloseFn>,
    /// Generates text (language models; ABI 2).
    pub generate: Option<GenerateFn>,
}

impl Default for InkEngineVTable {
    /// This core's size, kind 0 (none), and every pointer NULL.
    fn default() -> Self {
        Self {
            size: std::mem::size_of::<Self>() as u32,
            kind: 0,
            info_json: std::ptr::null(),
            ctx: std::ptr::null_mut(),
            transcribe: None,
            cancel: None,
            release: None,
            stream_open: None,
            stream_push: None,
            stream_finish: None,
            stream_close: None,
            generate: None,
        }
    }
}

/// The size of an ABI 1 table: every field before `stream_open`.
pub const VTABLE_V1_SIZE: usize = std::mem::offset_of!(InkEngineVTable, stream_open);

// ------------------------------------------------------------------------------------------------
// Pending calls

/// What a call's answer must be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Expect {
    /// `{"segments":[...]}`: a transcription.
    Segments,
    /// `{"ok":true}`: a live-stream step.
    Ack,
    /// `{"text":"..."}`: a generation.
    Text,
}

/// A call's answer, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// A transcription.
    Segments(Transcript),
    /// A live-stream step went through.
    Ack,
    /// Generated text.
    Text(String),
}

/// What an engine's error answer may say: a kind, nothing else (I5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ErrorKind {
    /// `failed`, and any kind this core does not know.
    Failed,
    /// `cancelled`.
    Cancelled,
    /// `model_missing`.
    ModelMissing,
    /// `bad_request`.
    BadRequest,
    /// `unavailable`: the engine cannot run on this Mac now.
    Unavailable,
    /// The answer itself could not be read.
    Unreadable,
}

/// An engine's error answer: its kind and its own optional code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ShellError {
    pub(crate) kind: ErrorKind,
    pub(crate) code: Option<i64>,
}

impl ShellError {
    const UNREADABLE: Self = Self {
        kind: ErrorKind::Unreadable,
        code: None,
    };

    /// What went wrong, naming engine `engine` and never quoting it.
    fn describe(self, engine: &str) -> String {
        let code = self
            .code
            .map(|c| format!(" (code {c})"))
            .unwrap_or_default();
        match self.kind {
            ErrorKind::Failed => format!("shell engine {engine} failed{code}"),
            ErrorKind::Cancelled => format!("shell engine {engine} cancelled the call{code}"),
            ErrorKind::ModelMissing => format!("shell engine {engine}{code}"),
            ErrorKind::BadRequest => {
                format!("shell engine {engine} could not read the request{code}")
            }
            ErrorKind::Unavailable => {
                format!("shell engine {engine} is unavailable on this Mac now{code}")
            }
            ErrorKind::Unreadable => format!("shell engine {engine}: its answer could not be read"),
        }
    }

    /// As an engine's error.
    pub(crate) fn engine_error(self, engine: &str) -> EngineError {
        match self.kind {
            ErrorKind::Cancelled => EngineError::Cancelled,
            ErrorKind::ModelMissing => EngineError::ModelMissing(self.describe(engine)),
            _ => EngineError::Failed(self.describe(engine)),
        }
    }

    /// As a language model's error.
    pub(crate) fn llm_error(self, engine: &str) -> LlmError {
        match self.kind {
            ErrorKind::Cancelled => LlmError::Cancelled,
            ErrorKind::ModelMissing => {
                LlmError::Engine(format!("model not installed: {}", self.describe(engine)))
            }
            _ => LlmError::Engine(self.describe(engine)),
        }
    }
}

/// What an answer settles to.
pub(crate) type Settled = Result<Answer, ShellError>;

struct Slot {
    answer: Mutex<Option<Settled>>,
    ready: Condvar,
    expect: Expect,
}

#[derive(Default)]
struct Pending {
    calls: Mutex<HashMap<u64, Arc<Slot>>>,
    next: AtomicU64,
}

static PENDING: LazyLock<Pending> = LazyLock::new(Pending::default);

impl Pending {
    /// Takes call `id` out of the table. `false` when [`complete`] already took it: its answer
    /// is being written.
    fn forget(&self, id: u64) -> bool {
        self.lock().remove(&id).is_some()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Arc<Slot>>> {
        self.calls.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Why a wait ended without an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GaveUp {
    /// The job's cancel token or the core's shutdown token was set.
    Cancelled,
    /// The call's deadline passed.
    TimedOut,
}

/// One call into a shell engine, from before the engine is called until its answer is read.
/// Dropping it without waiting forgets the call.
pub(crate) struct Call {
    id: u64,
    slot: Arc<Slot>,
}

impl Call {
    /// Opens a call whose answer must be `expect`.
    pub(crate) fn open(expect: Expect) -> Self {
        // Starts at 1, so a zeroed id is never a real call.
        let id = PENDING.next.fetch_add(1, Ordering::Relaxed) + 1;
        let slot = Arc::new(Slot {
            answer: Mutex::new(None),
            ready: Condvar::new(),
            expect,
        });
        PENDING.lock().insert(id, slot.clone());
        Self { id, slot }
    }

    /// The id the engine answers.
    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    /// **Worker.** Waits for the answer, or gives up once `cancelled` says so or `timeout` passes,
    /// calling `on_give_up` then (to tell the engine).
    ///
    /// Giving up is committed by taking the call out of the table. If [`complete`] took it first,
    /// the engine's answer is real and already on its way (complete writes it next, without
    /// waiting on anything), so it is waited for and kept rather than lost.
    pub(crate) fn wait(
        self,
        cancelled: impl Fn() -> bool,
        timeout: Option<Duration>,
        on_give_up: impl FnOnce(),
    ) -> Result<Settled, GaveUp> {
        let deadline = timeout.map(|t| Instant::now() + t);
        let slot = self.slot.clone();
        let mut answer = slot.answer.lock().unwrap_or_else(PoisonError::into_inner);
        let mut answered = false;
        loop {
            if let Some(a) = answer.take() {
                return Ok(a);
            }
            if !answered {
                let why = if cancelled() {
                    Some(GaveUp::Cancelled)
                } else if deadline.is_some_and(|d| Instant::now() >= d) {
                    Some(GaveUp::TimedOut)
                } else {
                    None
                };
                if let Some(why) = why {
                    drop(answer);
                    if PENDING.forget(self.id) {
                        on_give_up();
                        return Err(why);
                    }
                    answered = true;
                    answer = slot.answer.lock().unwrap_or_else(PoisonError::into_inner);
                    continue;
                }
            }
            let mut poll = CANCEL_POLL;
            if let Some(d) = deadline {
                poll = poll.min(d.saturating_duration_since(Instant::now()));
            }
            answer = slot
                .ready
                .wait_timeout(answer, poll)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        // Harmless when the call was answered or given up already: it is no longer in the table.
        PENDING.forget(self.id);
    }
}

/// Why [`complete`] could not use an answer.
#[derive(Debug, PartialEq, Eq)]
pub enum CompleteError {
    /// The call is not waiting: answered already, given up, or never issued.
    Unknown,
    /// The answer was not JSON of the expected shape. The call is answered with an error.
    Malformed,
}

/// Answers call `id` with the engine's `result_json` (see `inkwell.h`). **Any thread.**
pub fn complete(id: u64, result_json: &str) -> Result<(), CompleteError> {
    let Some(slot) = PENDING.lock().remove(&id) else {
        return Err(CompleteError::Unknown);
    };
    let parsed = parse_answer(slot.expect, result_json);
    let malformed = parsed.is_none();
    *slot.answer.lock().unwrap_or_else(PoisonError::into_inner) =
        Some(parsed.unwrap_or(Err(ShellError::UNREADABLE)));
    slot.ready.notify_all();
    if malformed {
        Err(CompleteError::Malformed)
    } else {
        Ok(())
    }
}

/// An answer of the kind `expect`, or an error.
///
/// An error is read as its kind and an optional integer code, and nothing else: an engine's own
/// text could quote what it heard, nothing here could check it, and errors reach events and logs
/// (I5). Any other field of the error, a `message` included, is ignored.
fn parse_answer(expect: Expect, json: &str) -> Option<Settled> {
    let v: Value = serde_json::from_str(json).ok()?;
    if let Some(error) = v.get("error") {
        return Some(Err(parse_error(error)?));
    }
    Some(Ok(match expect {
        Expect::Segments => Answer::Segments(Transcript {
            segments: v
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
                .collect::<Option<Vec<_>>>()?,
        }),
        Expect::Ack => {
            if v.get("ok")?.as_bool()? {
                Answer::Ack
            } else {
                // `{"ok":false}` says nothing about why: an answer, but not one of ours.
                return None;
            }
        }
        Expect::Text => Answer::Text(v.get("text")?.as_str()?.to_owned()),
    }))
}

fn parse_error(error: &Value) -> Option<ShellError> {
    let code = match error.get("code") {
        None => None,
        Some(c) => Some(c.as_i64()?),
    };
    let kind = match error.get("kind").and_then(Value::as_str)? {
        "cancelled" => ErrorKind::Cancelled,
        "model_missing" => ErrorKind::ModelMissing,
        "bad_request" => ErrorKind::BadRequest,
        "unavailable" => ErrorKind::Unavailable,
        // "failed", and any kind this core does not know, which is not echoed either.
        _ => ErrorKind::Failed,
    };
    Some(ShellError { kind, code })
}

/// How often a waiting worker looks at its cancel tokens. They are flags with no wake-up, so a
/// worker blocked on an answer checks them this often, and only while a call is in flight.
const CANCEL_POLL: Duration = Duration::from_millis(20);

// ------------------------------------------------------------------------------------------------
// Tables

/// Why a table could not be registered.
#[derive(Debug, PartialEq, Eq)]
pub struct BadTable(pub String);

fn bad(why: &str) -> BadTable {
    BadTable(why.to_owned())
}

/// What every kind of registered engine keeps from its table: the context, the optional
/// functions, and the release that runs when the last reference drops.
pub(crate) struct Shell {
    pub(crate) ctx: *mut c_void,
    cancel: Option<CancelFn>,
    release: Option<ReleaseFn>,
    /// The core's shutdown token: a call waiting on this engine gives up when it is set.
    pub(crate) shutdown: CancelToken,
    /// Whether `release` runs on drop.
    armed: AtomicBool,
    /// The id the engine registered under, to name it in errors.
    pub(crate) id: String,
}

// SAFETY: the header's contract (THREADS 4 and 5) is that an engine's functions may be called
// from any core worker thread, several at once, and that `ctx` is valid until `release`. The
// shell promises the engine is thread-safe by registering it; the core only passes `ctx` back.
unsafe impl Send for Shell {}
// SAFETY: as for `Send`: every use of `ctx` is a call into the engine, which the contract allows
// concurrently from several threads.
unsafe impl Sync for Shell {}

impl Shell {
    /// Tells the engine the core no longer wants call `call`'s answer.
    pub(crate) fn cancel_call(&self, call: u64) {
        if let Some(cancel) = self.cancel {
            // SAFETY: the shell's optional function, with its own `ctx`, valid until `release`,
            // which cannot have run while `self` is alive.
            unsafe { cancel(self.ctx, call) };
        }
    }

    /// Whether the core is shutting down.
    pub(crate) fn shutting_down(&self) -> bool {
        self.shutdown.is_cancelled()
    }

    /// Makes the drop skip `release`: for a table the core refused, which the header promises is
    /// never released.
    pub(crate) fn disarm(&self) {
        self.armed.store(false, Ordering::Release);
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        if let Some(release) = self.release
            && self.armed.load(Ordering::Acquire)
        {
            // SAFETY: the last reference is going; no call is in flight and no stream is open
            // (each holds a reference), so this is the "after the last call" the header promises.
            unsafe { release(self.ctx) };
        }
    }
}

/// A table, copied into this core's layout (see the module docs).
///
/// # Safety
///
/// `table` is NULL or points to a readable `InkEngineVTable` of at least the `size` it states.
unsafe fn copy_table(table: *const InkEngineVTable) -> Result<(InkEngineVTable, usize), BadTable> {
    if table.is_null() {
        return Err(bad("the table is NULL"));
    }
    // SAFETY: the caller guarantees a readable table; `size` is its first field, and every table
    // has at least that.
    let size = unsafe { std::ptr::addr_of!((*table).size).read_unaligned() } as usize;
    if size < VTABLE_V1_SIZE {
        return Err(bad("the table is smaller than an ABI 1 InkEngineVTable"));
    }
    let mut copy = InkEngineVTable::default();
    let n = size.min(std::mem::size_of::<InkEngineVTable>());
    // SAFETY: the caller guarantees `size` readable bytes, of which the first `n` are copied into
    // a table of this layout. Every field is a plain integer, a raw pointer or an optional
    // function pointer, for which the shell's bytes (a C struct of the same layout) are valid,
    // and the fields past `n` keep their NULL defaults.
    unsafe {
        std::ptr::copy_nonoverlapping(table.cast::<u8>(), (&raw mut copy).cast::<u8>(), n);
    }
    Ok((copy, size))
}

/// A registered engine of one kind, ready for the router or the language models.
pub enum Registration {
    /// `INK_ENGINE_OFFLINE`.
    Offline(ExternalOffline),
    /// `INK_ENGINE_STREAMING`.
    Streaming(ExternalStreaming),
    /// `INK_ENGINE_LLM`.
    Llm(ExternalLlm),
}

impl Registration {
    /// Copies a table the shell passed. `shutdown` is the core's: a call waiting on this engine
    /// gives up when it is set.
    ///
    /// # Safety
    ///
    /// `table` is NULL or points to a readable `InkEngineVTable` of at least the `size` it states,
    /// and its `info_json` is NULL or a NUL-terminated string, both valid for this call.
    pub unsafe fn from_table(
        table: *const InkEngineVTable,
        shutdown: CancelToken,
    ) -> Result<Self, BadTable> {
        // SAFETY: forwarded from this function's own contract.
        let (t, size) = unsafe { copy_table(table)? };
        let full = size >= std::mem::size_of::<InkEngineVTable>();
        let offline = t.transcribe.is_some();
        let streaming = [
            t.stream_open.is_some(),
            t.stream_push.is_some(),
            t.stream_finish.is_some(),
            t.stream_close.is_some(),
        ];
        let llm = t.generate.is_some();
        let info = || -> Result<&str, BadTable> {
            if t.info_json.is_null() {
                return Err(bad("info_json is NULL"));
            }
            // SAFETY: non-null, and the caller guarantees a NUL-terminated string.
            unsafe { crate::bounded_str(t.info_json, crate::INK_MAX_JSON) }
                .ok_or_else(|| bad("info_json is not UTF-8, or longer than INK_MAX_JSON"))
        };
        match t.kind {
            KIND_OFFLINE => {
                if streaming.iter().any(|f| *f) || llm {
                    return Err(bad(
                        "an offline table fills another kind's function: is the kind right?",
                    ));
                }
                let transcribe = t.transcribe.ok_or_else(|| bad("transcribe is NULL"))?;
                let (info, scores) = parse_info(info()?).ok_or_else(|| {
                    bad("info_json must be {\"id\",\"licence\",\"jobs\":[{\"job\",\"wer\"}]}")
                })?;
                Ok(Self::Offline(ExternalOffline {
                    shell: shell(&t, &info.id, shutdown),
                    transcribe,
                    info,
                    scores,
                }))
            }
            KIND_STREAMING => {
                if !full {
                    return Err(bad(
                        "a streaming table must be this core's InkEngineVTable size",
                    ));
                }
                if offline || llm {
                    return Err(bad(
                        "a streaming table fills another kind's function: is the kind right?",
                    ));
                }
                let (Some(open), Some(push), Some(finish), Some(close)) = (
                    t.stream_open,
                    t.stream_push,
                    t.stream_finish,
                    t.stream_close,
                ) else {
                    return Err(bad(
                        "stream_open, stream_push, stream_finish and stream_close are required",
                    ));
                };
                let (info, scores) = parse_info(info()?).ok_or_else(|| {
                    bad("info_json must be {\"id\",\"licence\",\"jobs\":[{\"job\",\"wer\"}]}")
                })?;
                Ok(Self::Streaming(ExternalStreaming::new(
                    shell(&t, &info.id, shutdown),
                    streaming::Functions {
                        open,
                        push,
                        finish,
                        close,
                    },
                    info,
                    scores,
                )))
            }
            KIND_LLM => {
                if !full {
                    return Err(bad(
                        "a language model's table must be this core's InkEngineVTable size",
                    ));
                }
                if offline || streaming.iter().any(|f| *f) {
                    return Err(bad(
                        "a language model's table fills another kind's function: is the kind right?",
                    ));
                }
                let generate = t.generate.ok_or_else(|| bad("generate is NULL"))?;
                let info = llm::parse_info(info()?).ok_or_else(|| {
                    bad("info_json must be {\"id\",\"licence\",\"model\",\"local\":true|false}")
                })?;
                Ok(Self::Llm(ExternalLlm::new(
                    shell(&t, &info.id, shutdown),
                    generate,
                    info,
                )))
            }
            _ => Err(bad("unknown engine kind")),
        }
    }

    /// The id it registers under.
    pub fn id(&self) -> &str {
        match self {
            Self::Offline(e) => &e.info.id,
            Self::Streaming(e) => e.id(),
            Self::Llm(e) => e.id(),
        }
    }

    /// Makes the drop skip `release`: for a table the core refused.
    pub fn disarm(&self) {
        match self {
            Self::Offline(e) => e.shell.disarm(),
            Self::Streaming(e) => e.disarm(),
            Self::Llm(e) => e.disarm(),
        }
    }
}

impl From<ExternalOffline> for Registration {
    fn from(e: ExternalOffline) -> Self {
        Self::Offline(e)
    }
}

fn shell(t: &InkEngineVTable, id: &str, shutdown: CancelToken) -> Shell {
    Shell {
        ctx: t.ctx,
        cancel: t.cancel,
        release: t.release,
        shutdown,
        armed: AtomicBool::new(true),
        id: id.to_owned(),
    }
}

/// `info_json` of an offline or streaming engine.
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

// ------------------------------------------------------------------------------------------------
// Offline engines

/// A registered offline engine. Its `release` runs when the last reference drops.
pub struct ExternalOffline {
    shell: Shell,
    transcribe: TranscribeFn,
    info: EngineInfo,
    scores: Vec<JobScore>,
}

impl ExternalOffline {
    /// Copies an offline engine's table the shell passed (see [`Registration::from_table`]).
    ///
    /// # Safety
    ///
    /// As [`Registration::from_table`].
    pub unsafe fn from_table(
        table: *const InkEngineVTable,
        shutdown: CancelToken,
    ) -> Result<Self, BadTable> {
        // SAFETY: forwarded from this function's own contract.
        match unsafe { Registration::from_table(table, shutdown)? } {
            Registration::Offline(e) => Ok(e),
            other => {
                other.disarm();
                Err(bad("not an offline engine's table"))
            }
        }
    }

    /// The error rate per job the shell registered.
    pub fn scores(&self) -> &[JobScore] {
        &self.scores
    }

    /// Makes the drop skip `release`: for a table the router refused, which the header promises
    /// is never released.
    pub fn disarm(&self) {
        self.shell.disarm();
    }
}

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
        let call = Call::open(Expect::Segments);
        let id = call.id();
        // SAFETY: the function pointer came from the shell's table; `ctx` is the engine's own
        // and valid until `release`, which runs only after the last reference (this one
        // included) is dropped. The samples and options are valid until the call returns,
        // which is all the header promises.
        unsafe { (self.transcribe)(self.shell.ctx, id, audio.as_ptr(), audio.len(), o.as_ptr()) };
        let cancelled = || options.cancel.is_cancelled() || self.shell.shutting_down();
        // No deadline: a final pass over a long region takes as long as it takes, and the job's
        // cancel token and shutdown both end the wait.
        match call.wait(cancelled, None, || self.shell.cancel_call(id)) {
            Ok(Ok(Answer::Segments(t))) => Ok(t),
            Ok(Ok(_)) => Err(ShellError::UNREADABLE.engine_error(&self.info.id)),
            Ok(Err(e)) => Err(e.engine_error(&self.info.id)),
            Err(_) => Err(EngineError::Cancelled),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segments(json: &str) -> Option<Settled> {
        parse_answer(Expect::Segments, json)
    }

    fn engine_error(json: &str) -> EngineError {
        segments(json).unwrap().unwrap_err().engine_error("e")
    }

    #[test]
    fn answers_parse_to_transcripts_or_errors() {
        let t = segments(r#"{"segments":[{"start_ms":0,"end_ms":5,"text":"a"}]}"#);
        assert_eq!(
            t,
            Some(Ok(Answer::Segments(Transcript {
                segments: vec![TimedText {
                    start_ms: 0,
                    end_ms: 5,
                    text: "a".into()
                }]
            })))
        );
        assert_eq!(
            engine_error(r#"{"error":{"kind":"cancelled"}}"#),
            EngineError::Cancelled
        );
        assert_eq!(
            engine_error(r#"{"error":{"kind":"failed","code":7}}"#),
            EngineError::Failed("shell engine e failed (code 7)".into())
        );
        assert_eq!(
            engine_error(r#"{"error":{"kind":"model_missing"}}"#),
            EngineError::ModelMissing("shell engine e".into())
        );
        assert_eq!(
            engine_error(r#"{"error":{"kind":"bad_request"}}"#),
            EngineError::Failed("shell engine e could not read the request".into())
        );
        assert_eq!(
            engine_error(r#"{"error":{"kind":"unavailable","code":2}}"#),
            EngineError::Failed("shell engine e is unavailable on this Mac now (code 2)".into())
        );
        assert!(segments("{").is_none());
        assert!(segments(r#"{"segments":[{"text":"no times"}]}"#).is_none());
        assert!(segments(r#"{"error":{"kind":"failed","code":"7"}}"#).is_none());
    }

    #[test]
    fn each_call_reads_only_its_own_kind_of_answer() {
        assert_eq!(
            parse_answer(Expect::Ack, r#"{"ok":true}"#),
            Some(Ok(Answer::Ack))
        );
        assert_eq!(parse_answer(Expect::Ack, r#"{"ok":false}"#), None);
        assert_eq!(parse_answer(Expect::Ack, r#"{"segments":[]}"#), None);
        assert_eq!(
            parse_answer(Expect::Text, r#"{"text":"polished"}"#),
            Some(Ok(Answer::Text("polished".into())))
        );
        assert_eq!(parse_answer(Expect::Text, r#"{"ok":true}"#), None);
        assert_eq!(parse_answer(Expect::Segments, r#"{"text":"x"}"#), None);
    }

    /// I5: an engine's error text could quote what it heard, and nothing checks it. The core
    /// never reads it: errors reach events and logs as a kind and a code.
    #[test]
    fn an_engine_s_free_text_never_reaches_an_error() {
        for answer in [
            r#"{"error":{"kind":"failed","code":3,"message":"heard: zebrafish"}}"#,
            r#"{"error":{"kind":"model_missing","message":"zebrafish"}}"#,
            r#"{"error":{"kind":"unavailable","message":"zebrafish"}}"#,
            r#"{"error":{"kind":"zebrafish"}}"#,
        ] {
            for expect in [Expect::Segments, Expect::Ack, Expect::Text] {
                let error = parse_answer(expect, answer).unwrap().unwrap_err();
                assert!(
                    !error.engine_error("e").to_string().contains("zebrafish"),
                    "{answer}"
                );
                assert!(
                    !error.llm_error("e").to_string().contains("zebrafish"),
                    "{answer}"
                );
            }
        }
    }

    #[test]
    fn a_call_is_answered_once_and_unknown_ids_are_refused() {
        let call = Call::open(Expect::Segments);
        let id = call.id();
        assert_eq!(complete(id, r#"{"segments":[]}"#), Ok(()));
        assert!(call.slot.answer.lock().unwrap().is_some());
        assert_eq!(
            complete(id, r#"{"segments":[]}"#),
            Err(CompleteError::Unknown)
        );
        assert_eq!(complete(0, "{}"), Err(CompleteError::Unknown));
        let call = Call::open(Expect::Segments);
        assert_eq!(
            complete(call.id(), "not json"),
            Err(CompleteError::Malformed)
        );
        assert_eq!(
            *call.slot.answer.lock().unwrap(),
            Some(Err(ShellError::UNREADABLE))
        );
    }

    #[test]
    fn a_dropped_call_is_forgotten() {
        let call = Call::open(Expect::Ack);
        let id = call.id();
        drop(call);
        assert_eq!(complete(id, r#"{"ok":true}"#), Err(CompleteError::Unknown));
    }

    /// The race: `complete` has taken the call from the table and not yet written its answer when
    /// the worker decides to cancel. The answer is real and on its way: it must be kept.
    #[test]
    fn an_answer_taken_before_the_cancel_is_kept() {
        let call = Call::open(Expect::Segments);
        let (id, slot) = (call.id(), call.slot.clone());
        // complete()'s first half: the call leaves the table.
        let taken = PENDING.lock().remove(&id).expect("the call was pending");
        let waiter = std::thread::spawn(move || {
            let mut cancelled_by_us = false;
            let answer = call.wait(|| true, None, || cancelled_by_us = true);
            (answer, cancelled_by_us)
        });
        std::thread::sleep(Duration::from_millis(50));
        // complete()'s second half: the answer is written.
        *taken.answer.lock().unwrap() = parse_answer(
            Expect::Segments,
            r#"{"segments":[{"start_ms":0,"end_ms":1,"text":"kept"}]}"#,
        );
        taken.ready.notify_all();
        drop(slot);
        let (answer, cancelled) = waiter.join().unwrap();
        let Ok(Ok(Answer::Segments(t))) = answer else {
            panic!("{answer:?}")
        };
        assert_eq!(t.text(), "kept");
        assert!(
            !cancelled,
            "the engine is not asked to cancel a call it answered"
        );
    }

    #[test]
    fn a_cancel_before_any_answer_forgets_the_call() {
        let call = Call::open(Expect::Segments);
        let id = call.id();
        let mut asked = false;
        assert_eq!(
            call.wait(|| true, None, || asked = true),
            Err(GaveUp::Cancelled)
        );
        assert!(asked);
        assert_eq!(
            complete(id, r#"{"segments":[]}"#),
            Err(CompleteError::Unknown)
        );
    }

    #[test]
    fn a_deadline_gives_up_and_a_late_answer_is_refused() {
        let call = Call::open(Expect::Ack);
        let id = call.id();
        let started = Instant::now();
        let mut asked = false;
        assert_eq!(
            call.wait(|| false, Some(Duration::from_millis(60)), || asked = true),
            Err(GaveUp::TimedOut)
        );
        assert!(started.elapsed() >= Duration::from_millis(60));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(asked, "the engine is told the core gave up");
        assert_eq!(complete(id, r#"{"ok":true}"#), Err(CompleteError::Unknown));
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
