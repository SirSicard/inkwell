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
use std::ffi::{CStr, CString, c_char, c_void};
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
}

#[derive(Default)]
struct Pending {
    calls: Mutex<HashMap<u64, Arc<Slot>>>,
    next: AtomicU64,
}

static PENDING: LazyLock<Pending> = LazyLock::new(Pending::default);

impl Pending {
    fn open(&self) -> (u64, Arc<Slot>) {
        // Starts at 1, so a zeroed id is never a real call.
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let slot = Arc::new(Slot::default());
        self.lock().insert(id, slot.clone());
        (id, slot)
    }

    fn forget(&self, id: u64) {
        self.lock().remove(&id);
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
    let parsed = parse_answer(result_json);
    let malformed = parsed.is_none();
    let answer = parsed.unwrap_or_else(|| {
        Err(EngineError::Failed(
            "the shell engine's answer could not be read".into(),
        ))
    });
    *slot.answer.lock().unwrap_or_else(PoisonError::into_inner) = Some(answer);
    slot.ready.notify_all();
    if malformed {
        Err(CompleteError::Malformed)
    } else {
        Ok(())
    }
}

fn parse_answer(json: &str) -> Option<Answer> {
    let v: Value = serde_json::from_str(json).ok()?;
    if let Some(error) = v.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        return Some(Err(match error.get("kind").and_then(Value::as_str)? {
            "cancelled" => EngineError::Cancelled,
            "model_missing" => EngineError::ModelMissing(message),
            _ => EngineError::Failed(format!("shell engine: {message}")),
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
        let info = unsafe { CStr::from_ptr(t.info_json) }
            .to_str()
            .map_err(|_| bad("info_json is not UTF-8"))?;
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
        let (id, slot) = PENDING.open();
        // SAFETY: the function pointer came from the shell's table; `ctx` is the engine's own
        // and valid until `release`, which runs only after the last reference (this one
        // included) is dropped. The samples and options are valid until the call returns,
        // which is all the header promises.
        unsafe { (self.transcribe)(self.ctx, id, audio.as_ptr(), audio.len(), o.as_ptr()) };
        let mut answer = slot.answer.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(a) = answer.take() {
                return a;
            }
            if options.cancel.is_cancelled() || self.shutdown.is_cancelled() {
                drop(answer);
                PENDING.forget(id);
                if let Some(cancel) = self.cancel {
                    // SAFETY: as above; `cancel` is the shell's optional function.
                    unsafe { cancel(self.ctx, id) };
                }
                return Err(EngineError::Cancelled);
            }
            answer = slot
                .ready
                .wait_timeout(answer, CANCEL_POLL)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
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
        let t = parse_answer(r#"{"segments":[{"start_ms":0,"end_ms":5,"text":"a"}]}"#);
        assert_eq!(t.unwrap().unwrap().text(), "a");
        assert_eq!(
            parse_answer(r#"{"error":{"kind":"cancelled","message":""}}"#).unwrap(),
            Err(EngineError::Cancelled)
        );
        assert!(matches!(
            parse_answer(r#"{"error":{"kind":"failed","message":"gpu"}}"#).unwrap(),
            Err(EngineError::Failed(m)) if m.contains("gpu")
        ));
        assert!(parse_answer("{").is_none());
        assert!(parse_answer(r#"{"segments":[{"text":"no times"}]}"#).is_none());
    }

    #[test]
    fn a_call_is_answered_once_and_unknown_ids_are_refused() {
        let (id, slot) = PENDING.open();
        assert_eq!(complete(id, r#"{"segments":[]}"#), Ok(()));
        assert!(slot.answer.lock().unwrap().is_some());
        assert_eq!(
            complete(id, r#"{"segments":[]}"#),
            Err(CompleteError::Unknown)
        );
        assert_eq!(complete(0, "{}"), Err(CompleteError::Unknown));
        let (id, slot) = PENDING.open();
        assert_eq!(complete(id, "not json"), Err(CompleteError::Malformed));
        assert!(matches!(
            *slot.answer.lock().unwrap(),
            Some(Err(EngineError::Failed(_)))
        ));
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
