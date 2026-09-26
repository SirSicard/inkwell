//! Text insertion: a promised paste first, then an Accessibility write, then Unicode key events.
//!
//! The order and every decision are in `sequence`, tested against a mock pasteboard. This module
//! wires that sequence to the real pasteboard (`pasteboard`), synthetic keys (`keys`) and
//! Accessibility (`crate::ax`).
//!
//! **Needs the main run loop.** Everything here runs on the calling worker, but AppKit serves the
//! promise on the main run loop, which the shell keeps running. If it is not running, nobody can
//! read the promise, the paste times out, and the fallbacks run: slower, never a hang. Calling
//! `insert` from the main thread would do the same (the main thread would be waiting instead of
//! serving), which is one more reason the trait says worker.
#![cfg(target_os = "macos")]

mod keys;
mod pasteboard;
pub(crate) mod sequence;

use std::cell::RefCell;
use std::sync::mpsc;

use ink_core::{InsertOutcome, PlatformError, TextInserter};
use objc2::rc::Retained;
use objc2_app_kit::NSPasteboard;

use crate::{ax, focus};
use pasteboard::{PasteProvider, Saved};
use sequence::{AxInsert, Backend, PasteTiming, Promise, Restore};

/// [`TextInserter`] for macOS.
#[derive(Debug)]
pub struct MacTextInserter {
    timing: PasteTiming,
}

impl MacTextInserter {
    /// An inserter with the shipped timings. It holds no OS resources.
    pub fn new() -> Self {
        Self {
            timing: PasteTiming::DEFAULT,
        }
    }
}

impl Default for MacTextInserter {
    fn default() -> Self {
        Self::new()
    }
}

impl TextInserter for MacTextInserter {
    /// Blocks for the length of the paste: about 0.35 s when the target reads it, up to about
    /// 2 s before falling back when nothing does. Never prompts for a permission.
    fn insert(&self, text: &str) -> Result<InsertOutcome, PlatformError> {
        sequence::insert(&MacBackend::default(), text, self.timing)
    }
}

/// One insertion's view of the OS: the general pasteboard, and the promise's provider, held until
/// the insertion is over.
struct MacBackend {
    pasteboard: Retained<NSPasteboard>,
    provider: RefCell<Option<Retained<PasteProvider>>>,
}

impl Default for MacBackend {
    fn default() -> Self {
        Self {
            pasteboard: pasteboard::general(),
            provider: RefCell::new(None),
        }
    }
}

impl Backend for MacBackend {
    type Saved = Saved;

    fn secure_input(&self) -> bool {
        focus::secure_input_enabled()
    }

    fn can_post_events(&self) -> bool {
        keys::can_post_events()
    }

    fn ax_trusted(&self) -> bool {
        ax::is_process_trusted()
    }

    fn change_count(&self) -> i64 {
        pasteboard::change_count(&self.pasteboard)
    }

    fn save_pasteboard(&self) -> Result<Saved, PlatformError> {
        pasteboard::save(&self.pasteboard)
    }

    fn write_promise(&self, text: &str) -> Result<Promise, PlatformError> {
        let (tx, rx) = mpsc::channel();
        let (change_count, provider) = pasteboard::write_promise(&self.pasteboard, text, tx)?;
        *self.provider.borrow_mut() = Some(provider);
        Ok(Promise {
            change_count,
            reads: rx,
        })
    }

    fn restore_if_unchanged(&self, saved: Saved, ours: i64) -> Result<Restore, PlatformError> {
        pasteboard::restore_if_unchanged(&self.pasteboard, saved, ours)
    }

    fn post_paste(&self) -> Result<(), PlatformError> {
        keys::post_paste()
    }

    fn ax_insert(&self, text: &str) -> AxInsert {
        ax::insert_text(text)
    }

    fn type_text(&self, text: &str) -> Result<(), PlatformError> {
        keys::type_text(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs Accessibility and a main run loop, and types into whatever has focus, so it only
    /// runs by hand. `examples/input_check.rs` is the supported way to exercise this.
    #[test]
    #[ignore = "needs Accessibility (TCC), a main run loop and a focused text field"]
    fn inserts_into_the_focused_app() {
        let outcome = MacTextInserter::new().insert("Inkwell test insertion.");
        assert!(outcome.is_ok(), "{outcome:?}");
    }
}
