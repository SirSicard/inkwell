//! The order of an insertion, in pure Rust over a [`Backend`], so every branch is tested with a
//! mock pasteboard and a scripted target app.
//!
//! 1. Nothing to insert: touch nothing.
//! 2. Secure Input on: [`InsertOutcome::Blocked`], touch nothing.
//! 3. Neither synthetic keys nor Accessibility writes permitted: `PermissionDenied`, before the
//!    clipboard is touched.
//! 4. Paste: save the pasteboard, write the text as a promise, post Cmd+V, wait until the target
//!    has read the promise plus a quiet period, then put the saved items back, but only if the
//!    change count is still ours. A pasteboard that cannot be saved is never overwritten.
//! 5. If the paste was impossible, or nobody read the promise in time: an Accessibility write to
//!    the focused element, then Unicode key events.
#![cfg(target_os = "macos")]

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use ink_core::{InsertOutcome, Permission, PlatformError};

/// The waits around a paste.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PasteTiming {
    /// Between writing the pasteboard and posting Cmd+V. Inkwell 0.2 measured it load-bearing:
    /// a keystroke sent in the same tick can paste the previous contents.
    pub(crate) settle: Duration,
    /// The least time between Cmd+V and the restore, however early the first read came. 0.2's
    /// measured restore delay; it keeps an early reader (a clipboard manager) from making the
    /// restore beat the target's own read.
    pub(crate) min_hold: Duration,
    /// How long after the last read of the promise before restoring.
    pub(crate) quiet: Duration,
    /// How long to wait for the first read. None by then means the paste did not land.
    pub(crate) read_timeout: Duration,
}

impl PasteTiming {
    /// The shipped values.
    pub(crate) const DEFAULT: Self = Self {
        settle: Duration::from_millis(50),
        min_hold: Duration::from_millis(300),
        quiet: Duration::from_millis(200),
        read_timeout: Duration::from_secs(2),
    };
}

/// A promise on the pasteboard: the change count it produced, and a message for each time a
/// target read it.
pub(crate) struct Promise {
    pub(crate) change_count: i64,
    pub(crate) reads: Receiver<()>,
}

/// What a restore did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Restore {
    /// The saved items are back.
    Restored,
    /// Someone wrote the pasteboard after us (the user copied something): theirs stays.
    KeptNewerCopy,
}

/// What an Accessibility write did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AxInsert {
    /// The focused element took the text.
    Took,
    /// Refused, unsupported, nothing focused, or verifiably ignored: safe to try typing.
    Refused,
    /// The app did not answer in time, so the text may or may not be there. Typing on top could
    /// insert it twice, so the sequence stops.
    Unknown,
}

/// The OS operations an insertion needs. `MacBackend` implements them; the tests script them.
pub(crate) trait Backend {
    /// A saved copy of the pasteboard's items.
    type Saved;
    /// Whether Secure Input is on.
    fn secure_input(&self) -> bool;
    /// Whether this process may post synthetic events. Never prompts.
    fn can_post_events(&self) -> bool;
    /// Whether this process is trusted for Accessibility. Never prompts.
    fn ax_trusted(&self) -> bool;
    /// The pasteboard's change count.
    fn change_count(&self) -> i64;
    /// Copies every item and type on the pasteboard.
    fn save_pasteboard(&self) -> Result<Self::Saved, PlatformError>;
    /// Replaces the pasteboard with `text` as a promise, marked transient and concealed.
    fn write_promise(&self, text: &str) -> Result<Promise, PlatformError>;
    /// Puts `saved` back if the change count is still `ours`, as one step.
    fn restore_if_unchanged(&self, saved: Self::Saved, ours: i64)
    -> Result<Restore, PlatformError>;
    /// Posts Cmd+V.
    fn post_paste(&self) -> Result<(), PlatformError>;
    /// Replaces the focused element's selection with `text` through Accessibility.
    fn ax_insert(&self, text: &str) -> AxInsert;
    /// Types `text` as Unicode key events.
    fn type_text(&self, text: &str) -> Result<(), PlatformError>;
}

/// Whether a restore may write: only while the pasteboard still holds what we wrote.
pub(crate) fn should_restore(ours: i64, now: i64) -> bool {
    ours == now
}

/// Waits for the target to read the promise. Returns whether anyone read it by
/// `read_timeout` after `posted_at`; when someone did, it returns no earlier than `min_hold` after
/// `posted_at` and `quiet` after the last read.
pub(crate) fn wait_for_reads(
    reads: &Receiver<()>,
    posted_at: Instant,
    timing: PasteTiming,
) -> bool {
    let first_deadline = posted_at + timing.read_timeout;
    if reads
        .recv_timeout(first_deadline.saturating_duration_since(Instant::now()))
        .is_err()
    {
        // Timed out, or the provider is gone and no read can be known: not taken.
        return false;
    }
    let hold_until = posted_at + timing.min_hold;
    let mut last_read = Instant::now();
    loop {
        let deadline = hold_until.max(last_read + timing.quiet);
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        match reads.recv_timeout(deadline - now) {
            Ok(()) => last_read = Instant::now(),
            Err(RecvTimeoutError::Timeout) => return true,
            Err(RecvTimeoutError::Disconnected) => {
                thread::sleep(deadline.saturating_duration_since(Instant::now()));
                return true;
            }
        }
    }
}

/// Inserts `text` through `backend`, in the order the module docs give.
pub(crate) fn insert<B: Backend>(
    backend: &B,
    text: &str,
    timing: PasteTiming,
) -> Result<InsertOutcome, PlatformError> {
    if text.is_empty() {
        return Ok(InsertOutcome::Pasted);
    }
    if backend.secure_input() {
        return Ok(InsertOutcome::Blocked);
    }
    let can_post = backend.can_post_events();
    let ax_trusted = backend.ax_trusted();
    if !can_post && !ax_trusted {
        return Err(PlatformError::PermissionDenied(Permission::Accessibility));
    }
    let mut restore = Ok(());
    if can_post {
        match paste(backend, text, timing) {
            Paste::Taken(restored) => {
                // The text is in. A failed restore is reported, never swallowed, and nothing
                // falls back: typing it again would insert it twice.
                return restored
                    .map(|()| InsertOutcome::Pasted)
                    .map_err(|e| clipboard_lost("pasted", &e));
            }
            Paste::NotTaken(restored) => restore = restored,
        }
    }
    let outcome = fallback(backend, text, can_post, ax_trusted)?;
    restore
        .map(|()| outcome)
        .map_err(|e| clipboard_lost("typed", &e))
}

/// How the paste went, and how the restore after it went.
enum Paste {
    /// A target read the promise.
    Taken(Result<(), PlatformError>),
    /// Impossible, or nobody read it in time.
    NotTaken(Result<(), PlatformError>),
}

fn paste<B: Backend>(backend: &B, text: &str, timing: PasteTiming) -> Paste {
    let before = backend.change_count();
    // A pasteboard that cannot be saved is never overwritten: the user would lose it.
    let Ok(saved) = backend.save_pasteboard() else {
        return Paste::NotTaken(Ok(()));
    };
    let promise = match backend.write_promise(text) {
        Ok(promise) => promise,
        Err(_) => {
            // A write that failed after clearing has changed the pasteboard all the same.
            let now = backend.change_count();
            let restore = if now == before {
                Ok(())
            } else {
                backend.restore_if_unchanged(saved, now).map(drop)
            };
            return Paste::NotTaken(restore);
        }
    };
    thread::sleep(timing.settle);
    if backend.post_paste().is_err() {
        let restore = backend.restore_if_unchanged(saved, promise.change_count);
        return Paste::NotTaken(restore.map(drop));
    }
    let taken = wait_for_reads(&promise.reads, Instant::now(), timing);
    let restore = backend
        .restore_if_unchanged(saved, promise.change_count)
        .map(drop);
    if taken {
        Paste::Taken(restore)
    } else {
        Paste::NotTaken(restore)
    }
}

fn fallback<B: Backend>(
    backend: &B,
    text: &str,
    can_post: bool,
    ax_trusted: bool,
) -> Result<InsertOutcome, PlatformError> {
    if ax_trusted {
        match backend.ax_insert(text) {
            AxInsert::Took => return Ok(InsertOutcome::Typed),
            AxInsert::Unknown => {
                return Err(PlatformError::Failed(
                    "the focused app did not confirm the accessibility write in time; not typing over it"
                        .into(),
                ));
            }
            AxInsert::Refused => {}
        }
    }
    if can_post {
        backend.type_text(text)?;
        return Ok(InsertOutcome::Typed);
    }
    Err(PlatformError::Failed(
        "the focused app refused the accessibility write, and synthetic keys are not permitted"
            .into(),
    ))
}

fn clipboard_lost(done: &str, error: &PlatformError) -> PlatformError {
    PlatformError::Failed(format!(
        "{done}, but the previous clipboard could not be put back: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::sync::mpsc::{self, Sender};

    use super::*;

    const TEXT: &str = "Synthetic dictation, second draft.";
    const ORIGINAL: &str = "what the user had copied";

    const FAST: PasteTiming = PasteTiming {
        settle: Duration::ZERO,
        min_hold: Duration::from_millis(5),
        quiet: Duration::from_millis(5),
        read_timeout: Duration::from_millis(30),
    };

    /// What the focused app does when Cmd+V arrives.
    #[derive(Clone, Copy)]
    enum Target {
        /// Reads the promise.
        Reads,
        /// Reads it, then the user copies something else before the restore.
        ReadsThenUserCopies,
        /// Never reads it (no paste support, nothing editable focused).
        Ignores,
    }

    struct Mock {
        secure: bool,
        can_post: bool,
        ax_trusted: bool,
        target: Target,
        ax_result: AxInsert,
        fail_save: bool,
        fail_write: bool,
        fail_post: bool,
        fail_restore: bool,
        count: Cell<i64>,
        clipboard: RefCell<String>,
        reads: RefCell<Option<Sender<()>>>,
        log: RefCell<Vec<&'static str>>,
        typed: RefCell<Vec<String>>,
    }

    impl Default for Mock {
        fn default() -> Self {
            Self {
                secure: false,
                can_post: true,
                ax_trusted: true,
                target: Target::Reads,
                ax_result: AxInsert::Took,
                fail_save: false,
                fail_write: false,
                fail_post: false,
                fail_restore: false,
                count: Cell::new(40),
                clipboard: RefCell::new(ORIGINAL.into()),
                reads: RefCell::new(None),
                log: RefCell::new(Vec::new()),
                typed: RefCell::new(Vec::new()),
            }
        }
    }

    impl Mock {
        fn log(&self) -> Vec<&'static str> {
            self.log.borrow().clone()
        }

        fn clipboard(&self) -> String {
            self.clipboard.borrow().clone()
        }

        fn bump(&self, contents: &str) {
            self.count.set(self.count.get() + 1);
            *self.clipboard.borrow_mut() = contents.into();
        }

        fn read(&self) {
            if let Some(tx) = self.reads.borrow().as_ref() {
                tx.send(()).expect("the sequence holds the receiver");
            }
        }
    }

    fn failure() -> PlatformError {
        PlatformError::Failed("scripted failure".into())
    }

    impl Backend for Mock {
        type Saved = String;

        fn secure_input(&self) -> bool {
            self.secure
        }

        fn can_post_events(&self) -> bool {
            self.can_post
        }

        fn ax_trusted(&self) -> bool {
            self.ax_trusted
        }

        fn change_count(&self) -> i64 {
            self.count.get()
        }

        fn save_pasteboard(&self) -> Result<String, PlatformError> {
            self.log.borrow_mut().push("save");
            if self.fail_save {
                return Err(failure());
            }
            Ok(self.clipboard())
        }

        fn write_promise(&self, text: &str) -> Result<Promise, PlatformError> {
            self.log.borrow_mut().push("write");
            // Like the real one, the write clears first; a failure after that leaves it empty.
            self.bump("");
            if self.fail_write {
                return Err(failure());
            }
            *self.clipboard.borrow_mut() = format!("promise of {text}");
            let (tx, rx) = mpsc::channel();
            *self.reads.borrow_mut() = Some(tx);
            Ok(Promise {
                change_count: self.count.get(),
                reads: rx,
            })
        }

        fn restore_if_unchanged(&self, saved: String, ours: i64) -> Result<Restore, PlatformError> {
            self.log.borrow_mut().push("restore");
            if !should_restore(ours, self.count.get()) {
                return Ok(Restore::KeptNewerCopy);
            }
            if self.fail_restore {
                return Err(failure());
            }
            self.bump(&saved);
            Ok(Restore::Restored)
        }

        fn post_paste(&self) -> Result<(), PlatformError> {
            self.log.borrow_mut().push("paste");
            if self.fail_post {
                return Err(failure());
            }
            match self.target {
                Target::Reads => self.read(),
                Target::ReadsThenUserCopies => {
                    self.read();
                    self.bump("the user's newer copy");
                }
                Target::Ignores => {}
            }
            Ok(())
        }

        fn ax_insert(&self, _text: &str) -> AxInsert {
            self.log.borrow_mut().push("ax");
            self.ax_result
        }

        fn type_text(&self, text: &str) -> Result<(), PlatformError> {
            self.log.borrow_mut().push("type");
            self.typed.borrow_mut().push(text.into());
            Ok(())
        }
    }

    #[test]
    fn empty_text_touches_nothing() {
        let mock = Mock::default();
        assert_eq!(insert(&mock, "", FAST), Ok(InsertOutcome::Pasted));
        assert!(mock.log().is_empty());
    }

    #[test]
    fn secure_input_blocks_and_touches_nothing() {
        let mock = Mock {
            secure: true,
            ..Mock::default()
        };
        assert_eq!(insert(&mock, TEXT, FAST), Ok(InsertOutcome::Blocked));
        assert!(mock.log().is_empty());
        assert_eq!(mock.clipboard(), ORIGINAL);
    }

    #[test]
    fn no_permission_is_refused_before_the_clipboard_is_touched() {
        let mock = Mock {
            can_post: false,
            ax_trusted: false,
            ..Mock::default()
        };
        assert_eq!(
            insert(&mock, TEXT, FAST),
            Err(PlatformError::PermissionDenied(Permission::Accessibility))
        );
        assert!(mock.log().is_empty());
    }

    #[test]
    fn a_read_paste_restores_the_previous_clipboard() {
        let mock = Mock::default();
        assert_eq!(insert(&mock, TEXT, FAST), Ok(InsertOutcome::Pasted));
        assert_eq!(mock.log(), ["save", "write", "paste", "restore"]);
        assert_eq!(mock.clipboard(), ORIGINAL);
    }

    #[test]
    fn a_copy_made_during_the_paste_is_never_clobbered() {
        let mock = Mock {
            target: Target::ReadsThenUserCopies,
            ..Mock::default()
        };
        assert_eq!(insert(&mock, TEXT, FAST), Ok(InsertOutcome::Pasted));
        assert_eq!(mock.clipboard(), "the user's newer copy");
    }

    #[test]
    fn a_paste_nobody_reads_is_restored_then_written_through_accessibility() {
        let mock = Mock {
            target: Target::Ignores,
            ..Mock::default()
        };
        assert_eq!(insert(&mock, TEXT, FAST), Ok(InsertOutcome::Typed));
        assert_eq!(mock.log(), ["save", "write", "paste", "restore", "ax"]);
        assert_eq!(mock.clipboard(), ORIGINAL);
    }

    #[test]
    fn an_accessibility_refusal_falls_back_to_unicode_typing() {
        let mock = Mock {
            target: Target::Ignores,
            ax_result: AxInsert::Refused,
            ..Mock::default()
        };
        assert_eq!(insert(&mock, TEXT, FAST), Ok(InsertOutcome::Typed));
        assert_eq!(
            mock.log(),
            ["save", "write", "paste", "restore", "ax", "type"]
        );
        assert_eq!(*mock.typed.borrow(), [TEXT]);
    }

    #[test]
    fn an_unconfirmed_accessibility_write_is_not_typed_over() {
        let mock = Mock {
            target: Target::Ignores,
            ax_result: AxInsert::Unknown,
            ..Mock::default()
        };
        assert!(matches!(
            insert(&mock, TEXT, FAST),
            Err(PlatformError::Failed(_))
        ));
        assert!(!mock.log().contains(&"type"));
    }

    #[test]
    fn a_pasteboard_that_cannot_be_saved_is_never_overwritten() {
        let mock = Mock {
            fail_save: true,
            ..Mock::default()
        };
        assert_eq!(insert(&mock, TEXT, FAST), Ok(InsertOutcome::Typed));
        assert_eq!(mock.log(), ["save", "ax"]);
        assert_eq!(mock.clipboard(), ORIGINAL);
    }

    #[test]
    fn a_failed_write_puts_the_saved_clipboard_back() {
        let mock = Mock {
            fail_write: true,
            ..Mock::default()
        };
        assert_eq!(insert(&mock, TEXT, FAST), Ok(InsertOutcome::Typed));
        assert_eq!(mock.log(), ["save", "write", "restore", "ax"]);
        assert_eq!(mock.clipboard(), ORIGINAL);
    }

    #[test]
    fn a_failed_keystroke_restores_then_falls_back() {
        let mock = Mock {
            fail_post: true,
            ax_result: AxInsert::Refused,
            ..Mock::default()
        };
        assert_eq!(insert(&mock, TEXT, FAST), Ok(InsertOutcome::Typed));
        assert_eq!(
            mock.log(),
            ["save", "write", "paste", "restore", "ax", "type"]
        );
        assert_eq!(mock.clipboard(), ORIGINAL);
    }

    #[test]
    fn a_restore_failure_after_a_paste_is_reported_not_swallowed() {
        let mock = Mock {
            fail_restore: true,
            ..Mock::default()
        };
        let Err(PlatformError::Failed(message)) = insert(&mock, TEXT, FAST) else {
            panic!("a lost clipboard must not read as a clean paste");
        };
        assert!(message.contains("pasted"), "{message}");
        // No fallback: the text is in, typing it again would duplicate it.
        assert!(!mock.log().contains(&"ax"));
    }

    #[test]
    fn without_event_posting_only_accessibility_is_tried() {
        let mock = Mock {
            can_post: false,
            ..Mock::default()
        };
        assert_eq!(insert(&mock, TEXT, FAST), Ok(InsertOutcome::Typed));
        assert_eq!(mock.log(), ["ax"]);
    }

    #[test]
    fn without_event_posting_a_refused_write_is_an_error() {
        let mock = Mock {
            can_post: false,
            ax_result: AxInsert::Refused,
            ..Mock::default()
        };
        assert!(insert(&mock, TEXT, FAST).is_err());
        assert_eq!(mock.log(), ["ax"]);
    }

    #[test]
    fn without_accessibility_trust_a_missed_paste_is_typed() {
        let mock = Mock {
            ax_trusted: false,
            target: Target::Ignores,
            ..Mock::default()
        };
        assert_eq!(insert(&mock, TEXT, FAST), Ok(InsertOutcome::Typed));
        assert_eq!(mock.log(), ["save", "write", "paste", "restore", "type"]);
    }

    /// I5: errors end up in logs, so they never carry the text.
    #[test]
    fn errors_never_contain_the_text() {
        let failing = [
            Mock {
                can_post: false,
                ax_trusted: false,
                ..Mock::default()
            },
            Mock {
                fail_restore: true,
                ..Mock::default()
            },
            Mock {
                target: Target::Ignores,
                ax_result: AxInsert::Unknown,
                ..Mock::default()
            },
            Mock {
                can_post: false,
                ax_result: AxInsert::Refused,
                ..Mock::default()
            },
        ];
        for mock in failing {
            let error = insert(&mock, TEXT, FAST).expect_err("scripted to fail");
            assert!(!error.to_string().contains("dictation"), "{error}");
        }
    }

    #[test]
    fn restore_only_while_the_change_count_is_ours() {
        assert!(should_restore(7, 7));
        assert!(!should_restore(7, 8), "the user copied after us");
        assert!(
            !should_restore(7, 6),
            "a count that went backwards is not ours either"
        );
    }

    #[test]
    fn a_promise_nobody_reads_times_out() {
        let (_tx, rx) = mpsc::channel::<()>();
        let start = Instant::now();
        assert!(!wait_for_reads(&rx, start, FAST));
        assert!(start.elapsed() >= FAST.read_timeout);
    }

    #[test]
    fn an_early_read_still_waits_out_the_minimum_hold() {
        let timing = PasteTiming {
            min_hold: Duration::from_millis(40),
            ..FAST
        };
        let (tx, rx) = mpsc::channel();
        tx.send(()).expect("receiver alive");
        let start = Instant::now();
        assert!(wait_for_reads(&rx, start, timing));
        assert!(start.elapsed() >= timing.min_hold);
    }

    #[test]
    fn each_read_extends_the_quiet_period() {
        let timing = PasteTiming {
            min_hold: Duration::ZERO,
            quiet: Duration::from_millis(30),
            read_timeout: Duration::from_millis(500),
            ..FAST
        };
        let (tx, rx) = mpsc::channel();
        let start = Instant::now();
        let reader = thread::spawn(move || {
            tx.send(()).expect("receiver alive");
            thread::sleep(Duration::from_millis(20));
            tx.send(()).expect("receiver alive");
        });
        assert!(wait_for_reads(&rx, start, timing));
        // The second read came at about 20 ms, so the restore waits until about 50 ms.
        assert!(start.elapsed() >= Duration::from_millis(50));
        reader.join().expect("reader thread");
    }
}
