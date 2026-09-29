//! The order of an insertion, in pure Rust over a [`Backend`], so every branch is tested with a
//! scripted clipboard and target app.
//!
//! 1. Nothing to insert: touch nothing.
//! 2. The target runs above our integrity level (an app run as administrator): Windows would drop
//!    the keystrokes without a word, so [`InsertOutcome::Blocked`], and nothing is touched. So is
//!    a target whose level cannot be read, and no foreground window at all (the secure desktop).
//!    The target, window and process, is recorded here, and checked again right before Ctrl+V and
//!    before each batch of typed keys: if focus moved, nothing more is sent, because it would land
//!    in whatever has focus now ("the focused window changed").
//! 3. A modifier key still held (after a chord hotkey) is waited for, once and briefly; if it stays
//!    down, nothing is touched and the error says **nothing was inserted**, so the core keeps the
//!    dictation (it is already in the record) rather than lose it.
//! 4. Paste: save the clipboard, put the text on it **rendered on demand** (delayed rendering, so
//!    the moment the target reads it is known) and excluded from clipboard history and cloud
//!    sync, press Ctrl+V, wait until the target has read it plus a quiet period, then put the saved
//!    clipboard back, but only if nobody wrote it since. A clipboard that cannot be saved is never
//!    overwritten.
//!
//!    Only reads **after** Ctrl+V count as the target's. A read before it is another program
//!    (a clipboard manager, remote-desktop clipboard sync, which read on every change); once it
//!    has made Windows render the text, the target's own read is no longer seen, so such a paste
//!    cannot be confirmed and says so.
//! 5. Type the text as Unicode key events **only when no Ctrl+V went out** (the clipboard could
//!    not be saved or written, or Windows took none of the keystroke). A Ctrl+V that went out and
//!    was not read in time may still land later, in an app that was only slow: typing as well
//!    could insert the text twice, or the late paste could insert the restored clipboard. So that
//!    case inserts nothing more and says "nothing was inserted".
//!
//! Every error that leaves the text out says so in words that begin "nothing was inserted"
//! ([`not_inserted`]).
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

/// The window an insertion is for, recorded when it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FocusTarget {
    /// The foreground window's handle, as a number.
    pub(crate) window: usize,
    /// The process that owns it.
    pub(crate) pid: u32,
}

/// What has focus when the insertion starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Focus {
    /// A window our input reaches.
    Ready(FocusTarget),
    /// No foreground window, or one above our integrity level (or of a level we cannot read).
    Blocked,
}

/// The error for input that stopped because focus moved.
pub(crate) fn focus_changed(what: &str) -> PlatformError {
    not_inserted(&format!(
        "the focused window changed {what}, and it would have gone to whatever has focus now"
    ))
}

/// A Ctrl+V that did not go out cleanly.
#[derive(Debug)]
pub(crate) struct PostFailed {
    /// Whether any of its keystrokes went in: then a paste may still be pending.
    pub(crate) sent_any: bool,
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
    /// What has focus now, and whether our synthetic input reaches it (UIPI).
    fn focus(&self) -> Focus;
    /// Whether `target` still has focus and still takes our input.
    fn still_focused(&self, target: FocusTarget) -> bool {
        self.focus() == Focus::Ready(target)
    }
    /// Waits briefly for held modifier keys to be released. `false` if one is still down.
    fn wait_for_release(&self) -> bool;
    /// Copies every format on the clipboard.
    fn save_clipboard(&self) -> Result<Self::Saved, PlatformError>;
    /// Replaces the clipboard with `text`, rendered on demand. The backend keeps `saved` for
    /// [`restore`](Self::restore); on failure it has already tried to put it back.
    fn write_delayed(&self, text: &str, saved: Self::Saved) -> Result<Promise, WriteFailed>;
    /// Presses Ctrl+V.
    fn post_paste(&self) -> Result<(), PostFailed>;
    /// Puts the saved clipboard back if the clipboard is still ours, as one step.
    fn restore(&self) -> Result<Restore, PlatformError>;
    /// Types `text` as Unicode key events into `target`, checking before each batch that it still
    /// has focus.
    fn type_text(&self, text: &str, target: FocusTarget) -> Result<(), PlatformError>;
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
    let Focus::Ready(target) = backend.focus() else {
        return Ok(InsertOutcome::Blocked);
    };
    if !backend.wait_for_release() {
        return Err(not_inserted(
            "a modifier key is still held, and sending into it would change the keys",
        ));
    }
    let clipboard_back = match paste(backend, text, target, timing) {
        Paste::Taken { clipboard_back } => {
            // The text is in. A clipboard that did not come back is part of the outcome, never an
            // error (an error invites a retry, which would insert the text twice).
            return Ok(if clipboard_back {
                InsertOutcome::Pasted
            } else {
                InsertOutcome::InsertedClipboardNotRestored
            });
        }
        Paste::NotPosted { clipboard_back } => clipboard_back,
        Paste::FocusMoved { clipboard_back } => {
            return Err(PlatformError::Failed(format!(
                "{}{}",
                reason(&focus_changed("before the paste")),
                clipboard_note(clipboard_back)
            )));
        }
        Paste::Unread {
            clipboard_back,
            read_early: false,
        } => {
            return Err(not_inserted(&format!(
                "the app did not take the paste within {} s, and typing it as well could insert \
                 it twice{}",
                timing.read_timeout.as_secs_f32(),
                clipboard_note(clipboard_back)
            )));
        }
        Paste::Unread {
            clipboard_back,
            read_early: true,
        } => {
            // Not "nothing was inserted": the app may well have pasted what the other program
            // made Windows render. Nothing is typed on top.
            return Err(PlatformError::Failed(format!(
                "the paste could not be confirmed: another program (a clipboard manager or \
                 remote-desktop clipboard sync) read the clipboard first; check the app before \
                 inserting again{}",
                clipboard_note(clipboard_back)
            )));
        }
    };
    match backend.type_text(text, target) {
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
    /// The target read it.
    Taken { clipboard_back: bool },
    /// No Ctrl+V went out: typing is safe.
    NotPosted { clipboard_back: bool },
    /// Focus moved before Ctrl+V: nothing was sent.
    FocusMoved { clipboard_back: bool },
    /// Ctrl+V went out and nothing read the clipboard after it in time: it may still land.
    Unread {
        clipboard_back: bool,
        /// Something read it before Ctrl+V, so the target's read could not be seen.
        read_early: bool,
    },
}

/// The clause an error adds when the user's clipboard did not come back.
fn clipboard_note(clipboard_back: bool) -> &'static str {
    if clipboard_back {
        ""
    } else {
        "; the previous clipboard could not be put back either"
    }
}

/// Whether a restore left the user's clipboard as it should be: back in full, or replaced by a
/// newer copy of their own.
fn clipboard_back(restore: &Result<Restore, PlatformError>) -> bool {
    matches!(restore, Ok(Restore::Restored | Restore::KeptNewerCopy))
}

fn paste<B: Backend>(backend: &B, text: &str, target: FocusTarget, timing: PasteTiming) -> Paste {
    // A clipboard that cannot be saved is never overwritten: the user would lose it.
    let Ok(saved) = backend.save_clipboard() else {
        return Paste::NotPosted {
            clipboard_back: true,
        };
    };
    let promise = match backend.write_delayed(text, saved) {
        Ok(promise) => promise,
        Err(failed) => {
            return Paste::NotPosted {
                clipboard_back: failed.clipboard_back,
            };
        }
    };
    thread::sleep(timing.settle);
    // Reads so far are not the target's: it has not been asked yet.
    let read_early = promise.reads.try_iter().count() > 0;
    if !backend.still_focused(target) {
        return Paste::FocusMoved {
            clipboard_back: clipboard_back(&backend.restore()),
        };
    }
    if let Err(PostFailed { sent_any: false }) = backend.post_paste() {
        return Paste::NotPosted {
            clipboard_back: clipboard_back(&backend.restore()),
        };
    }
    // Posted, or partly: from here a paste may land, so the text is never typed as well.
    let taken = wait_for_reads(&promise.reads, Instant::now(), timing);
    let back = clipboard_back(&backend.restore());
    if taken {
        Paste::Taken {
            clipboard_back: back,
        }
    } else {
        Paste::Unread {
            clipboard_back: back,
            read_early,
        }
    }
}

/// The error for an insertion that left the text out: the core keeps the dictation.
pub(crate) fn not_inserted(why: &str) -> PlatformError {
    PlatformError::Failed(format!("nothing was inserted: {why}"))
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
        /// A clipboard manager reads it as soon as it is written; the target then pastes the
        /// rendered text without a read we can see.
        ManagerReadsFirst,
    }

    const WINDOW: FocusTarget = FocusTarget {
        window: 0x1234,
        pid: 42,
    };

    struct Mock {
        blocked: bool,
        /// Focus moves away once the text is on the clipboard (after `write`).
        focus_moves: bool,
        target: Target,
        fail_save: bool,
        fail_write: Option<bool>,
        /// `Some(sent_any)`: Ctrl+V fails, with or without some keystrokes in.
        fail_post: Option<bool>,
        fail_type: bool,
        held: bool,
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
                focus_moves: false,
                target,
                fail_save: false,
                fail_write: None,
                fail_post: None,
                fail_type: false,
                held: false,
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

        fn focus(&self) -> Focus {
            if self.blocked {
                return Focus::Blocked;
            }
            let written = self.calls.borrow().contains(&"write");
            if self.focus_moves && written {
                Focus::Ready(FocusTarget {
                    window: 0x9999,
                    pid: 7,
                })
            } else {
                Focus::Ready(WINDOW)
            }
        }

        fn wait_for_release(&self) -> bool {
            self.call("wait");
            !self.held
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
            if matches!(self.target, Target::ManagerReadsFirst) {
                tx.send(()).unwrap();
            }
            *self.reader.borrow_mut() = Some(tx);
            Ok(Promise { reads: rx })
        }

        fn post_paste(&self) -> Result<(), PostFailed> {
            self.call("paste");
            if let Some(sent_any) = self.fail_post {
                return Err(PostFailed { sent_any });
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
                Target::Ignores | Target::ManagerReadsFirst => {}
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

        fn type_text(&self, text: &str, target: FocusTarget) -> Result<(), PlatformError> {
            self.call("type");
            assert_eq!(
                target, WINDOW,
                "typing goes to the window recorded at the start"
            );
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
    fn focus_moving_before_ctrl_v_sends_nothing_and_restores() {
        let mut mock = Mock::new(Target::Reads);
        mock.focus_moves = true;
        match insert(&mock, TEXT, FAST) {
            Err(PlatformError::Failed(m)) => {
                assert!(m.starts_with("nothing was inserted"), "{m}");
                assert!(m.contains("focused window changed"), "{m}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            *mock.calls.borrow(),
            ["wait", "save", "write", "restore"],
            "no Ctrl+V, no typing"
        );
        assert_eq!(*mock.clipboard.borrow(), ORIGINAL);
    }

    #[test]
    fn a_modifier_held_too_long_leaves_everything_untouched_and_says_nothing_was_inserted() {
        let mut mock = Mock::new(Target::Reads);
        mock.held = true;
        match insert(&mock, TEXT, FAST) {
            Err(PlatformError::Failed(m)) => {
                assert!(m.starts_with("nothing was inserted"), "{m}");
                assert!(!m.contains(TEXT), "the text never reaches an error");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            *mock.calls.borrow(),
            ["wait"],
            "waited once, touched nothing"
        );
        assert_eq!(*mock.clipboard.borrow(), ORIGINAL);
    }

    #[test]
    fn a_read_paste_restores_the_clipboard() {
        let mock = Mock::new(Target::Reads);
        assert_eq!(insert(&mock, TEXT, FAST).unwrap(), InsertOutcome::Pasted);
        assert_eq!(
            *mock.calls.borrow(),
            ["wait", "save", "write", "paste", "restore"]
        );
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
    fn an_unread_paste_restores_and_is_never_typed_on_top() {
        let mock = Mock::new(Target::Ignores);
        match insert(&mock, TEXT, FAST) {
            Err(PlatformError::Failed(m)) => {
                assert!(m.starts_with("nothing was inserted"), "{m}");
                assert!(m.contains("twice"), "{m}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            *mock.calls.borrow(),
            ["wait", "save", "write", "paste", "restore"]
        );
        assert!(
            mock.typed.borrow().is_none(),
            "a late paste could still land"
        );
        assert_eq!(*mock.clipboard.borrow(), ORIGINAL);
    }

    #[test]
    fn a_read_before_ctrl_v_is_not_the_targets_and_the_paste_is_not_confirmed() {
        let mock = Mock::new(Target::ManagerReadsFirst);
        match insert(&mock, TEXT, FAST) {
            Err(PlatformError::Failed(m)) => {
                assert!(m.contains("could not be confirmed"), "{m}");
                assert!(
                    !m.starts_with("nothing was inserted"),
                    "it may have landed: {m}"
                );
            }
            other => panic!("the early read counted as the paste: {other:?}"),
        }
        assert!(mock.typed.borrow().is_none());
        assert_eq!(*mock.clipboard.borrow(), ORIGINAL);
    }

    #[test]
    fn a_clipboard_that_cannot_be_saved_is_never_overwritten() {
        let mut mock = Mock::new(Target::Reads);
        mock.fail_save = true;
        assert_eq!(insert(&mock, TEXT, FAST).unwrap(), InsertOutcome::Typed);
        assert_eq!(*mock.calls.borrow(), ["wait", "save", "type"]);
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
    fn a_ctrl_v_that_never_went_out_restores_then_types() {
        let mut mock = Mock::new(Target::Reads);
        mock.fail_post = Some(false);
        assert_eq!(insert(&mock, TEXT, FAST).unwrap(), InsertOutcome::Typed);
        assert_eq!(
            *mock.calls.borrow(),
            ["wait", "save", "write", "paste", "restore", "type"]
        );
    }

    #[test]
    fn a_ctrl_v_that_partly_went_out_is_waited_for_never_typed_over() {
        // Some of its keystrokes went in: the paste may land, and here the target reads it.
        let mut mock = Mock::new(Target::Ignores);
        mock.fail_post = Some(true);
        assert!(insert(&mock, TEXT, FAST).is_err());
        assert!(mock.typed.borrow().is_none());
        assert_eq!(
            *mock.calls.borrow(),
            ["wait", "save", "write", "paste", "restore"]
        );
    }

    #[test]
    fn typing_that_fails_is_an_error_naming_the_clipboard_too() {
        let mut mock = Mock::new(Target::Ignores);
        mock.fail_post = Some(false);
        mock.fail_type = true;
        assert!(matches!(
            insert(&mock, TEXT, FAST),
            Err(PlatformError::Failed(_))
        ));
        let mut worse = Mock::new(Target::Ignores);
        worse.fail_post = Some(false);
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
