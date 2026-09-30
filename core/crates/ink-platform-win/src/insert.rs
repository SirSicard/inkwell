//! Text insertion: a delayed-render paste, or Unicode key events when no paste could go out.
//!
//! The order and every decision are in `sequence`, tested against a scripted clipboard. This
//! module wires it to the real clipboard (`clipboard`: an owner thread per insertion), synthetic
//! keys (`keys`) and the integrity check (`crate::integrity`).
//!
//! Unlike the Mac, nothing here needs the shell's main thread: the clipboard's owner window runs
//! on a thread of its own for the length of the insertion.
#![cfg(windows)]

mod clipboard;
mod keys;

pub(crate) use keys::{send_heartbeat, send_mask_key};
pub(crate) mod sequence;

use std::cell::RefCell;

use ink_core::{InsertOutcome, PlatformError, TextInserter};

use crate::integrity;
use clipboard::{Owner, Saved};
use sequence::{
    Backend, Focus, FocusTarget, PasteTiming, PostFailed, Promise, Restore, WriteFailed,
};

/// [`TextInserter`] for Windows.
#[derive(Debug)]
pub struct WinTextInserter {
    timing: PasteTiming,
}

impl WinTextInserter {
    /// An inserter with the shipped timings. It holds no OS resources.
    pub fn new() -> Self {
        Self {
            timing: PasteTiming::DEFAULT,
        }
    }
}

impl Default for WinTextInserter {
    fn default() -> Self {
        Self::new()
    }
}

impl TextInserter for WinTextInserter {
    /// Blocks for the length of the paste: about 0.35 s when the target reads it, about 2 s when
    /// nothing does (then nothing more is inserted, and the error says so). Never prompts:
    /// Windows has nothing to prompt for.
    fn insert(&self, text: &str) -> Result<InsertOutcome, PlatformError> {
        sequence::insert(&WinBackend::default(), text, self.timing)
    }
}

/// One insertion's view of the OS: the clipboard's owner thread, held until the restore.
#[derive(Default)]
struct WinBackend {
    owner: RefCell<Option<Owner>>,
}

impl Backend for WinBackend {
    type Saved = Saved;

    fn focus(&self) -> Focus {
        match integrity::foreground_target() {
            Some((window, pid))
                if !integrity::blocks_input(
                    integrity::own_level(),
                    integrity::level_of_pid(pid),
                ) =>
            {
                Focus::Ready(FocusTarget { window, pid })
            }
            _ => Focus::Blocked,
        }
    }

    fn wait_for_release(&self) -> bool {
        keys::wait_for_release()
    }

    fn save_clipboard(&self) -> Result<Saved, PlatformError> {
        clipboard::save()
    }

    fn write_delayed(&self, text: &str, saved: Saved) -> Result<Promise, WriteFailed> {
        let (owner, reads) = clipboard::write_delayed(text, saved)?;
        *self.owner.borrow_mut() = Some(owner);
        Ok(Promise { reads })
    }

    fn post_paste(&self) -> Result<(), PostFailed> {
        keys::post_paste().map_err(|sent_any| PostFailed { sent_any })
    }

    fn restore(&self) -> Result<Restore, PlatformError> {
        // The owner thread ends when its handle drops, after the restore.
        let owner = self.owner.borrow_mut().take().ok_or_else(|| {
            PlatformError::Failed("nothing to restore: the clipboard was not written".into())
        })?;
        owner.restore()
    }

    fn type_text(&self, text: &str, target: FocusTarget) -> Result<(), PlatformError> {
        keys::type_text(text, || self.still_focused(target))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Types into whatever has focus and uses the real clipboard, so it only runs by hand, from a
    /// terminal on the desktop with a text field focused. `examples/win_check.rs --insert` is the
    /// supported way to exercise this.
    #[test]
    #[ignore = "types into the focused app and uses the clipboard"]
    fn inserts_into_the_focused_app() {
        let outcome = WinTextInserter::new().insert("Inkwell test insertion.");
        assert!(outcome.is_ok(), "{outcome:?}");
    }
}
