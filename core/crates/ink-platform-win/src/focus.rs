//! Focus: the foreground app, whether it would drop our input, and the selected text.
//!
//! The foreground app is the process owning the foreground window, named by its executable.
//! `secure_input` is the Windows equivalent of the Mac's Secure Input: the target runs above our
//! integrity level, so synthetic input would be dropped (`crate::integrity`).
//!
//! The selection comes from UI Automation: the focused element's Text pattern. Apps that expose no
//! Text pattern (many games, some custom editors) answer `None`, as an app without an
//! accessibility selection does on the Mac. UI Automation needs no permission. The selected text
//! is returned to the caller only; it is never logged. A password field's selection is never read
//! (nor one whose password flag cannot be read), and the calls into the target app have short
//! timeouts (1 s to connect, 1.5 s per answer), so a hung app does not hold voice editing for UI
//! Automation's default 20 s transaction timeout.
#![cfg(windows)]

use ink_core::{AppRef, FocusInfo, FocusReader, PlatformError};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation, IUIAutomation2, IUIAutomationTextPattern, UIA_TextPatternId,
};
use windows::core::{BOOL, Interface};

use crate::com::{ComScope, display_stem, failed, file_name};
use crate::integrity;

/// The longest selection read, in characters: voice editing works on a paragraph or a page, not a
/// whole document.
const MAX_SELECTION: i32 = 100_000;

/// How long UI Automation waits to reach the focused app, and for each answer from it.
const CONNECTION_TIMEOUT_MS: u32 = 1_000;
const TRANSACTION_TIMEOUT_MS: u32 = 1_500;

/// Whether an element's password flag forbids reading its selection: set, or unreadable.
fn is_password(flag: windows::core::Result<BOOL>) -> bool {
    flag.map_or(true, BOOL::as_bool)
}

/// [`FocusReader`] for Windows.
#[derive(Debug, Default)]
pub struct WinFocusReader {
    _private: (),
}

impl WinFocusReader {
    /// A reader. It holds no OS resources.
    pub fn new() -> Self {
        Self::default()
    }
}

/// The app owning the foreground window.
pub(crate) fn foreground_app() -> Option<AppRef> {
    let pid = integrity::foreground_window().and_then(integrity::window_pid)?;
    let path = integrity::image_path(pid)?;
    let exe = file_name(&path).to_owned();
    Some(AppRef {
        name: display_stem(&exe),
        id: exe,
        pid: Some(pid),
    })
}

impl FocusReader for WinFocusReader {
    /// Needs no permission.
    fn focus(&self) -> Result<FocusInfo, PlatformError> {
        Ok(FocusInfo {
            app: foreground_app(),
            secure_input: integrity::foreground_blocks_input(),
        })
    }

    /// Through UI Automation; `None` when nothing is selected or the app exposes no selection.
    fn selected_text(&self) -> Result<Option<String>, PlatformError> {
        let _com = ComScope::enter()?;
        // SAFETY: an in-process COM class, in this thread's apartment.
        let automation: IUIAutomation =
            unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) }
                .map_err(|e| failed("UI Automation is not available", &e))?;
        if let Ok(timeouts) = automation.cast::<IUIAutomation2>() {
            // SAFETY: a live automation object; plain millisecond values.
            unsafe {
                let _ = timeouts.SetConnectionTimeout(CONNECTION_TIMEOUT_MS);
                let _ = timeouts.SetTransactionTimeout(TRANSACTION_TIMEOUT_MS);
            }
        }
        // SAFETY: a live automation object. No focused element is an ordinary answer.
        let Ok(element) = (unsafe { automation.GetFocusedElement() }) else {
            return Ok(None);
        };
        // SAFETY: a live element.
        if is_password(unsafe { element.CurrentIsPassword() }) {
            return Ok(None);
        }
        // SAFETY: a live element; an element without the pattern fails, which means no selection.
        let Ok(pattern) =
            (unsafe { element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) })
        else {
            return Ok(None);
        };
        // SAFETY: a live pattern.
        let ranges =
            unsafe { pattern.GetSelection() }.map_err(|e| failed("reading the selection", &e))?;
        // SAFETY: a live range array.
        let count = unsafe { ranges.Length() }.map_err(|e| failed("reading the selection", &e))?;
        let mut text = String::new();
        for index in 0..count {
            // SAFETY: `index < count`.
            let range = unsafe { ranges.GetElement(index) }
                .map_err(|e| failed("reading the selection", &e))?;
            // SAFETY: a live range.
            let part = unsafe { range.GetText(MAX_SELECTION) }
                .map_err(|e| failed("reading the selection", &e))?;
            text.push_str(&part.to_string());
        }
        Ok((!text.is_empty()).then_some(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_field_or_an_unreadable_flag_is_never_read() {
        assert!(is_password(Ok(BOOL::from(true))));
        assert!(is_password(Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_FAIL
        ))));
        assert!(!is_password(Ok(BOOL::from(false))));
    }

    /// Runs anywhere: with no foreground window (a service session, a runner), the answer is
    /// empty, never an error.
    #[test]
    fn focus_answers_without_error() {
        let info = WinFocusReader::new().focus().expect("focus");
        if let Some(app) = info.app {
            assert!(app.id.to_ascii_lowercase().ends_with(".exe"), "{}", app.id);
        }
    }

    /// Reads the selection of whatever has focus, so it runs by hand from the desktop:
    /// `examples/win_check.rs --focus`.
    #[test]
    #[ignore = "reads the focused app's selection"]
    fn the_selection_reads_without_error() {
        let _ = WinFocusReader::new().selected_text().expect("selection");
    }
}
