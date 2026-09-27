//! Ask: `meeting.ask {question}` answered about the live meeting, on its own thread, `ink-ask`.
//!
//! A language model call takes seconds (up to the shell model's deadline), so it gets a thread of
//! its own: the meetings thread must stay free to stop a meeting, and the screens' thread to save
//! a note. Questions are answered one at a time, in order; a few may wait
//! ([`MAX_WAITING`]), more are refused as busy rather than queued without end.
//!
//! The answer is `meeting.answered`, echoing the command's id as `ref`, with the model's text: the
//! shell renders it as words only (no links: model text can say anything). A failure is
//! `command.failed` with that id: no meeting, no language model, or the model's error, named
//! without the question or the transcript (I5).

use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use ink_core::{RecordId, Store};
use ink_llm::tasks::RecordContext;
use ink_llm::tasks::ask::answer;
use ink_llm::tasks::due::RecordTime;

use crate::events::{self, event};
use crate::runtime::{Runs, Shared, lock};

/// Questions that may wait behind the one being answered.
pub const MAX_WAITING: usize = 4;

struct Question {
    id: Option<String>,
    question: String,
}

/// The Ask thread.
pub struct Asking {
    tx: Sender<Question>,
    waiting: Arc<AtomicUsize>,
    thread: JoinHandle<()>,
}

impl Asking {
    /// Starts `ink-ask`.
    pub fn start(shared: Arc<Shared>, runs: Arc<Mutex<Runs>>) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Question>();
        let waiting = Arc::new(AtomicUsize::new(0));
        let thread = {
            let waiting = waiting.clone();
            thread::Builder::new()
                .name("ink-ask".into())
                .spawn(move || run(&shared, &runs, &rx, &waiting))?
        };
        Ok(Self {
            tx,
            waiting,
            thread,
        })
    }

    /// Queues a question, or refuses it (as `command.failed`) when too many wait.
    pub fn ask(&self, shared: &Shared, id: Option<String>, question: String) -> Result<(), String> {
        if self.waiting.fetch_add(1, Ordering::AcqRel) > MAX_WAITING {
            self.waiting.fetch_sub(1, Ordering::AcqRel);
            shared.events.emit(events::command_failed(
                "meeting.ask",
                id.as_deref(),
                "too many questions are waiting; ask again when one is answered",
            ));
            return Ok(());
        }
        self.tx
            .send(Question { id, question })
            .map_err(|_| "the Ask thread has stopped".to_owned())
    }

    /// Ends the thread once the question being answered is done (the shutdown's cancel stops
    /// its model call); what waits is dropped.
    pub fn stop(self) {
        drop(self.tx);
        if self.thread.join().is_err() {
            log::error!("the Ask thread panicked outside its per-question boundary");
        }
    }
}

fn run(shared: &Shared, runs: &Mutex<Runs>, rx: &Receiver<Question>, waiting: &AtomicUsize) {
    while let Ok(q) = rx.recv() {
        let fail = |message: &str| {
            log::warn!("command meeting.ask failed: {message}");
            shared.events.emit(events::command_failed(
                "meeting.ask",
                q.id.as_deref(),
                message,
            ));
        };
        if shared.shutdown.is_cancelled() {
            waiting.fetch_sub(1, Ordering::AcqRel);
            continue;
        }
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let record = lock(runs)
                .meeting
                .as_ref()
                .and_then(|m| m.record().cloned());
            match record {
                None => fail("no meeting is being recorded"),
                Some(record) => match answer_about(shared, &record, &q.question) {
                    Ok(text) => shared.events.emit(event(
                        "meeting.answered",
                        &[
                            ("record", Some(record.0.as_str().into())),
                            ("ref", q.id.clone().map(Into::into)),
                            ("text", Some(text.into())),
                        ],
                    )),
                    Err(message) => fail(&message),
                },
            }
        }));
        if ran.is_err() {
            // The payload is not logged: it could hold the question or the answer (I5).
            log::error!("meeting.ask panicked; the next question still runs");
            fail("a bug in the core stopped this question");
        }
        waiting.fetch_sub(1, Ordering::AcqRel);
    }
}

/// **Worker.** Answers `question` about `record` so far, with the registered language model.
fn answer_about(shared: &Shared, record: &RecordId, question: &str) -> Result<String, String> {
    let Some(llm) = crate::engines::llm(shared) else {
        return Err("no language model is available to answer on this Mac".into());
    };
    let store: &dyn Store = shared.store.as_ref();
    let stored = store
        .record(record)
        .map_err(|e| format!("the meeting could not be read: {e}"))?
        .ok_or("the meeting's record is gone")?;
    let segments = store
        .segments(record)
        .map_err(|e| format!("the transcript could not be read: {e}"))?;
    let names = store.speaker_names(record).unwrap_or_default();
    let ctx = RecordContext {
        title: stored.title.as_deref(),
        time: RecordTime {
            started_at_unix_ms: stored.started_at_unix_ms,
            utc_offset_minutes: 0,
        },
        speaker_names: &names,
    };
    answer(
        question,
        &segments,
        &ctx,
        &crate::engines::ask_options(shared),
        llm.as_ref(),
        &shared.shutdown,
    )
    .map_err(|e| format!("the model could not answer: {e}"))
}
