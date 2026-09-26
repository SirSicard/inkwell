//! Focus: the frontmost app, Secure Input, and the selected text.
//!
//! **Threads.** `NSWorkspace.frontmostApplication` and `NSRunningApplication` are safe to read
//! off the main thread (Apple documents `NSRunningApplication` as thread-safe, with values that
//! update as the main run loop runs), so `focus` does not hop. In a process whose main run loop
//! never runs, the frontmost app can be stale; the shell's is always running. Secure Input is one
//! call into HIToolbox. The selection goes through Accessibility (`crate::ax`).
#![cfg(target_os = "macos")]

use ink_core::{AppRef, FocusInfo, FocusReader, PlatformError};
use objc2_app_kit::NSWorkspace;

use crate::ax;

// SAFETY: the signature is HIToolbox's (`CarbonEvents.h` in older SDKs): no arguments, a
// `Boolean` (unsigned char) result.
#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    /// `Boolean IsSecureEventInputEnabled(void)` from HIToolbox. The header no longer ships in
    /// the SDK; the symbol is still exported (`Carbon.tbd`). No arguments, reads global state.
    safe fn IsSecureEventInputEnabled() -> u8;
}

/// Whether any process has Secure Input on (a password field, a terminal's secure keyboard
/// entry). While it is, synthetic input is refused, so insertion reports `Blocked`.
pub(crate) fn secure_input_enabled() -> bool {
    IsSecureEventInputEnabled() != 0
}

/// [`FocusReader`] for macOS.
#[derive(Debug, Default)]
pub struct MacFocusReader {
    _private: (),
}

impl MacFocusReader {
    /// A reader. It holds no OS resources.
    pub fn new() -> Self {
        Self::default()
    }
}

impl FocusReader for MacFocusReader {
    /// Needs no permission.
    fn focus(&self) -> Result<FocusInfo, PlatformError> {
        Ok(FocusInfo {
            app: frontmost_app(),
            secure_input: secure_input_enabled(),
        })
    }

    /// Needs Accessibility; without it, `None`. Never prompts.
    fn selected_text(&self) -> Result<Option<String>, PlatformError> {
        ax::selected_text()
    }
}

fn frontmost_app() -> Option<AppRef> {
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    app_ref(
        app.bundleIdentifier().map(|s| s.to_string()),
        app.executableURL()
            .and_then(|url| url.lastPathComponent())
            .map(|s| s.to_string()),
        app.localizedName().map(|s| s.to_string()),
        app.processIdentifier(),
    )
}

/// Builds an [`AppRef`]: the bundle identifier as the id, or the executable's file name for a
/// process without a bundle; `None` when there is neither. A pid of -1 (an app without a process)
/// is no pid, and the display name falls back to the id.
pub(crate) fn app_ref(
    bundle_id: Option<String>,
    executable: Option<String>,
    name: Option<String>,
    pid: i32,
) -> Option<AppRef> {
    let id = bundle_id
        .filter(|s| !s.is_empty())
        .or(executable.filter(|s| !s.is_empty()))?;
    let name = name.filter(|s| !s.is_empty()).unwrap_or_else(|| id.clone());
    Some(AppRef {
        id,
        pid: u32::try_from(pid).ok(),
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_is_known_by_its_bundle_id() {
        let app = app_ref(
            Some("com.example.Editor".into()),
            Some("Editor".into()),
            Some("Example Editor".into()),
            4242,
        );
        assert_eq!(
            app,
            Some(AppRef {
                id: "com.example.Editor".into(),
                pid: Some(4242),
                name: "Example Editor".into(),
            })
        );
    }

    #[test]
    fn a_process_without_a_bundle_is_known_by_its_executable() {
        let app = app_ref(None, Some("tool".into()), None, 7).expect("an app");
        assert_eq!(app.id, "tool");
        assert_eq!(app.name, "tool");
    }

    #[test]
    fn no_pid_and_no_identity_are_handled() {
        let app = app_ref(Some("com.example.Editor".into()), None, None, -1).expect("an app");
        assert_eq!(app.pid, None);
        assert_eq!(app_ref(None, None, Some("Name".into()), 1), None);
        assert_eq!(app_ref(Some(String::new()), None, None, 1), None);
    }

    /// Runs on CI: the frontmost app and Secure Input need no permission.
    #[test]
    fn focus_reads_without_any_permission() {
        let focus = MacFocusReader::new().focus();
        assert!(focus.is_ok(), "{focus:?}");
    }

    /// Needs Accessibility and a selection in the frontmost app.
    #[test]
    #[ignore = "needs Accessibility (TCC) and a selection in the frontmost app"]
    fn reads_the_selection_in_the_frontmost_app() {
        let selection = MacFocusReader::new().selected_text();
        assert!(matches!(selection, Ok(Some(_))), "{selection:?}");
    }
}
