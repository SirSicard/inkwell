//! The order of an insertion, in pure Rust over a [`Backend`], so every branch is tested with a
//! scripted clipboard and target app.
//!
//! 1. Nothing to insert: touch nothing.
//! 2. The target runs above our integrity level (an app run as administrator): Windows would drop
//!    the keystrokes without a word, so [`InsertOutcome::Blocked`], and nothing is touched.
//! 3. Paste: save the clipboard, put the text on it **rendered on demand** (delayed rendering, so
//!    the moment the target reads it is known) and excluded from clipboard history and cloud
//!    sync, press Ctrl+V, wait until the target has read it plus a quiet period, then put the saved
//!    clipboard back, but only if nobody wrote it since. A clipboard that cannot be saved is never
//!    overwritten.
//! 4. If nothing read the clipboard in time, type the text as Unicode key events.
#![cfg(windows)]

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use ink_core::{InsertOutcome, PlatformError};

/// The waits around a paste: the Mac's values (Inkwell 0.2's measured ones), until the Windows
/// checklist says otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PasteTiming {
    /// Between writing the clipboard and pressing Ctrl+V.
    pub(crate) settle: Duration,
    /// The least time between Ctrl+V and the restore, however early the read came.
    pub(crate) min_hold: Duration,
    /// How long after the last read before restoring.
    pub(crate) quiet: Duration,
    /// How long to wait for the first read. None by then means the paste did not land.
    pub(crate) read_timeout: Duration,
}

impl PasteTiming {
    pub(crate) const DEFAULT: Self = Self {
        settle: Duration::from_millis(50),
        min_hold: Duration::from_millis(300),
        quiet: Duration::from_millis(200),
        read_timeout: Duration::from_secs(2),
    };
}

/// The text on the clipboard, rendered on demand: a message for each time a target read it.
pub(crate) struct Promise {
    pub(crate) reads: Receiver<()>,
}

/// A write that did not happen, and whether the clipboard is as it was.
#[derive(Debug)]
pub(crate) struct WriteFailed {
    pub(crate) clipboard_back: bool,
}

/// What a restore did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Restore {
    /// The saved clipboard is back, every format.
    Restored,
    /// Back, minus `lost` formats that could not be copied (GDI handles, private formats) or
    /// put back.
    RestoredPartly { lost: usize },
    /// Someone wrote the clipboard after us (the user copied something): theirs stays.
    KeptNewerCopy,
}

/// The OS operations an insertion needs. `WinBackend` implements them; the tests script them.
pub(crate) trait Backend {
    /// A saved copy of the clipboard.
    type Saved;
    /// Whether the focused window drops our synthetic input (UIPI).
    fn target_blocked(&self) -> bool;
    /// Copies every format on the clipboard.
    fn save_clipboard(&self) -> Result<Self::Saved, PlatformError>;
    /// Replaces the clipboard with `text`, rendered on demand. The backend keeps `saved` for
    /// [`restore`](Self::restore); on failure it has already tried to put it back.
    fn write_delayed(&self, text: &str, saved: Self::Saved) -> Result<Promise, WriteFailed>;
    /// Presses Ctrl+V.
    fn post_paste(&self) -> Result<(), PlatformError>;
    /// Puts the saved clipboard back if the clipboard is still ours, as one step.
    fn restore(&self) -> Result<Restore, PlatformError>;
    /// Types `text` as Unicode key events.
    fn type_text(&self, text: &str) -> Result<(), PlatformError>;
}

/// Waits for the target to read the clipboard (see the Mac's twin in ink-platform-mac). Returns
/// whether anyone read it by `read_timeout` after `posted_at`; when someone did, it returns no
/// earlier than `min_hold` after `posted_at` and `quiet` after the last read.
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
    if backend.target_blocked() {
        return Ok(InsertOutcome::Blocked);
    }
    let clipboard_back = match paste(backend, text, timing) {
        Paste::Taken { clipboard_back } => {
            // The text is in. A clipboard that did not come back is part of the outcome, never an
            // error (an error invites a retry, which would insert the text twice).
            return Ok(if clipboard_back {
                InsertOutcome::Pasted
            } else {
                InsertOutcome::InsertedClipboardNotRestored
            });
        }
        Paste::NotTaken { clipboard_back } => clipboard_back,
    };
    match backend.type_text(text) {
        Ok(()) if clipboard_back => Ok(InsertOutcome::Typed),
        Ok(()) => Ok(InsertOutcome::InsertedClipboardNotRestored),
        Err(error) if clipboard_back => Err(error),
        Err(error) => Err(PlatformError::Failed(format!(
            "{}; the previous clipboard could not be put back either",
            reason(&error)
        ))),
    }
}

enum Paste {
    Taken { clipboard_back: bool },
    NotTaken { clipboard_back: bool },
}

/// Whether a restore left the user's clipboard as it should be: back in full, or replaced by a
/// newer copy of their own.
fn clipboard_back(restore: &Result<Restore, PlatformError>) -> bool {
    matches!(restore, Ok(Restore::Restored | Restore::KeptNewerCopy))
}

fn paste<B: Backend>(backend: &B, text: &str, timing: PasteTiming) -> Paste {
    // A clipboard that cannot be saved is never overwritten: the user would lose it.
    let Ok(saved) = backend.save_clipboard() else {
        return Paste::NotTaken {
            clipboard_back: true,
        };
    };
    let promise = match backend.write_delayed(text, saved) {
        Ok(promise) => promise,
        Err(failed) => {
            return Paste::NotTaken {
                clipboard_back: failed.clipboard_back,
            };
        }
    };
    thread::sleep(timing.settle);
    if backend.post_paste().is_err() {
        return Paste::NotTaken {
            clipboard_back: clipboard_back(&backend.restore()),
        };
    }
    let taken = wait_for_reads(&promise.reads, Instant::now(), timing);
    let back = clipboard_back(&backend.restore());
    if taken {
        Paste::Taken {
            clipboard_back: back,
        }
    } else {
        Paste::NotTaken {
            clipboard_back: back,
        }
    }
}

fn reason(error: &PlatformError) -> String {
    match error {
        PlatformError::Failed(message) => message.clone(),
        other => other.to_string(),
    }
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
        read_timeout: Duration::from_millis(40),
    };

    #[derive(Clone, Copy)]
    enum Target {
        /// Reads the clipboard on Ctrl+V.
        Reads,
        /// Reads it, then the user copies something before the restore.
        ReadsThenUserCopies,
        /// Never reads it.
        Ignores,
    }

    struct Mock {
        blocked: bool,
        target: Target,
        fail_save: bool,
        fail_write: Option<bool>,
        fail_post: bool,
        fail_type: bool,
        lossy_restore: bool,
        clipboard: RefCell<String>,
        saved: RefCell<Option<String>>,
        reader: RefCell<Option<Sender<()>>>,
        typed: RefCell<Option<String>>,
        user_copied: Cell<bool>,
        calls: RefCell<Vec<&'static str>>,
    }

    impl Mock {
        fn new(target: Target) -> Self {
            Self {
                blocked: false,
                target,
                fail_save: false,
                fail_write: None,
                fail_post: false,
                fail_type: false,
                lossy_restore: false,
                clipboard: RefCell::new(ORIGINAL.into()),
                saved: RefCell::new(None),
                reader: RefCell::new(None),
                typed: RefCell::new(None),
                user_copied: Cell::new(false),
                calls: RefCell::new(Vec::new()),
            }
        }

        fn call(&self, name: &'static str) {
            self.calls.borrow_mut().push(name);
        }
    }

    impl Backend for Mock {
        type Saved = String;

        fn target_blocked(&self) -> bool {
            self.blocked
        }

        fn save_clipboard(&self) -> Result<String, PlatformError> {
            self.call("save");
            if self.fail_save {
                return Err(PlatformError::Failed("save".into()));
            }
            Ok(self.clipboard.borrow().clone())
        }

        fn write_delayed(&self, text: &str, saved: String) -> Result<Promise, WriteFailed> {
            self.call("write");
            if let Some(clipboard_back) = self.fail_write {
                return Err(WriteFailed { clipboard_back });
            }
            *self.saved.borrow_mut() = Some(saved);
            *self.clipboard.borrow_mut() = text.into();
            let (tx, rx) = mpsc::channel();
            *self.reader.borrow_mut() = Some(tx);
            Ok(Promise { reads: rx })
        }

        fn post_paste(&self) -> Result<(), PlatformError> {
            self.call("paste");
            if self.fail_post {
                return Err(PlatformError::Failed("SendInput".into()));
            }
            let reader = self.reader.borrow();
            let reader = reader.as_ref().expect("written first");
            match self.target {
                Target::Reads => reader.send(()).unwrap(),
                Target::ReadsThenUserCopies => {
                    reader.send(()).unwrap();
                    self.user_copied.set(true);
                    *self.clipboard.borrow_mut() = "the user's new copy".into();
                }
                Target::Ignores => {}
            }
            Ok(())
        }

        fn restore(&self) -> Result<Restore, PlatformError> {
            self.call("restore");
            if self.user_copied.get() {
                return Ok(Restore::KeptNewerCopy);
            }
            let saved = self.saved.borrow_mut().take().expect("saved at write");
            *self.clipboard.borrow_mut() = saved;
            Ok(if self.lossy_restore {
                Restore::RestoredPartly { lost: 1 }
            } else {
                Restore::Restored
            })
        }

        fn type_text(&self, text: &str) -> Result<(), PlatformError> {
            self.call("type");
            if self.fail_type {
                return Err(PlatformError::Failed("SendInput sent 0 of 68".into()));
            }
            *self.typed.borrow_mut() = Some(text.into());
            Ok(())
        }
    }

    #[test]
    fn nothing_to_insert_touches_nothing() {
        let mock = Mock::new(Target::Reads);
        assert_eq!(insert(&mock, "", FAST).unwrap(), InsertOutcome::Pasted);
        assert!(mock.calls.borrow().is_empty());
    }

    #[test]
    fn an_elevated_target_is_blocked_before_anything_is_touched() {
        let mut mock = Mock::new(Target::Reads);
        mock.blocked = true;
        assert_eq!(insert(&mock, TEXT, FAST).unwrap(), InsertOutcome::Blocked);
        assert!(mock.calls.borrow().is_empty());
        assert_eq!(*mock.clipboard.borrow(), ORIGINAL);
    }

    #[test]
    fn a_read_paste_restores_the_clipboard() {
        let mock = Mock::new(Target::Reads);
        assert_eq!(insert(&mock, TEXT, FAST).unwrap(), InsertOutcome::Pasted);
        assert_eq!(*mock.calls.borrow(), ["save", "write", "paste", "restore"]);
        assert_eq!(*mock.clipboard.borrow(), ORIGINAL);
        assert!(mock.typed.borrow().is_none(), "never typed on top");
    }

    #[test]
    fn a_newer_copy_by_the_user_is_kept() {
        let mock = Mock::new(Target::ReadsThenUserCopies);
        assert_eq!(insert(&mock, TEXT, FAST).unwrap(), InsertOutcome::Pasted);
        assert_eq!(*mock.clipboard.borrow(), "the user's new copy");
    }

    #[test]
    fn a_lossy_restore_is_reported_as_inserted_not_restored() {
        let mut mock = Mock::new(Target::Reads);
        mock.lossy_restore = true;
        assert_eq!(
            insert(&mock, TEXT, FAST).unwrap(),
            InsertOutcome::InsertedClipboardNotRestored
        );
    }

    #[test]
    fn an_unread_paste_restores_then_types() {
        let mock = Mock::new(Target::Ignores);
        assert_eq!(insert(&mock, TEXT, FAST).unwrap(), InsertOutcome::Typed);
        assert_eq!(
            *mock.calls.borrow(),
            ["save", "write", "paste", "restore", "type"]
        );
        assert_eq!(mock.typed.borrow().as_deref(), Some(TEXT));
        assert_eq!(*mock.clipboard.borrow(), ORIGINAL);
    }

    #[test]
    fn a_clipboard_that_cannot_be_saved_is_never_overwritten() {
        let mut mock = Mock::new(Target::Reads);
        mock.fail_save = true;
        assert_eq!(insert(&mock, TEXT, FAST).unwrap(), InsertOutcome::Typed);
        assert_eq!(*mock.calls.borrow(), ["save", "type"]);
        assert_eq!(*mock.clipboard.borrow(), ORIGINAL);
    }

    #[test]
    fn a_failed_write_types_and_says_whether_the_clipboard_is_back() {
        let mut mock = Mock::new(Target::Reads);
        mock.fail_write = Some(true);
        assert_eq!(insert(&mock, TEXT, FAST).unwrap(), InsertOutcome::Typed);
        let mut lost = Mock::new(Target::Reads);
        lost.fail_write = Some(false);
        assert_eq!(
            insert(&lost, TEXT, FAST).unwrap(),
            InsertOutcome::InsertedClipboardNotRestored
        );
    }

    #[test]
    fn a_failed_ctrl_v_restores_then_types() {
        let mut mock = Mock::new(Target::Reads);
        mock.fail_post = true;
        assert_eq!(insert(&mock, TEXT, FAST).unwrap(), InsertOutcome::Typed);
        assert_eq!(
            *mock.calls.borrow(),
            ["save", "write", "paste", "restore", "type"]
        );
    }

    #[test]
    fn typing_that_fails_is_an_error_naming_the_clipboard_too() {
        let mut mock = Mock::new(Target::Ignores);
        mock.fail_type = true;
        assert!(matches!(
            insert(&mock, TEXT, FAST),
            Err(PlatformError::Failed(_))
        ));
        let mut worse = Mock::new(Target::Ignores);
        worse.fail_type = true;
        worse.lossy_restore = true;
        match insert(&worse, TEXT, FAST) {
            Err(PlatformError::Failed(m)) => assert!(m.contains("clipboard"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_wait_holds_for_the_minimum_and_the_quiet_period() {
        let timing = PasteTiming {
            settle: Duration::ZERO,
            min_hold: Duration::from_millis(60),
            quiet: Duration::from_millis(20),
            read_timeout: Duration::from_millis(500),
        };
        let (tx, rx) = mpsc::channel();
        let start = Instant::now();
        tx.send(()).unwrap();
        assert!(wait_for_reads(&rx, start, timing));
        assert!(start.elapsed() >= Duration::from_millis(60));
        let (_tx, silent) = mpsc::channel::<()>();
        let timing = PasteTiming {
            read_timeout: Duration::from_millis(20),
            ..timing
        };
        assert!(!wait_for_reads(&silent, Instant::now(), timing));
    }
}
