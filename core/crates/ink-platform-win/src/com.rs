//! COM on the calling thread, and small shared helpers for Win32 errors and strings.
//!
//! Every public method of this crate runs on a core worker thread it does not own. COM needs an
//! apartment on the thread that calls it, so each call that uses COM opens a [`ComScope`]: it
//! joins the multithreaded apartment, and leaves again only if it was the one that joined. A
//! thread that is already in an apartment (a shell thread in an STA, say) keeps it; the objects
//! used here (MMDevice, WASAPI, the session manager, UI Automation) work from either.
//!
//! Threads this crate creates (capture, detection, the hotkey hook, the clipboard owner) open one
//! scope for their whole life.
#![cfg(windows)]

use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;

use ink_core::PlatformError;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::core::PWSTR;

/// This thread's membership of a COM apartment, for as long as the scope lives. Not `Send`: it
/// must end on the thread that opened it.
pub(crate) struct ComScope {
    /// Whether `CoInitializeEx` succeeded here, so `CoUninitialize` is owed.
    joined: bool,
    _not_send: PhantomData<*const ()>,
}

impl ComScope {
    /// Joins the MTA, or keeps the apartment the thread is already in.
    pub(crate) fn enter() -> Result<Self, PlatformError> {
        // SAFETY: no reserved argument; any thread may call it.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr == RPC_E_CHANGED_MODE {
            // Already an STA: usable as it is, and not ours to leave.
            return Ok(Self {
                joined: false,
                _not_send: PhantomData,
            });
        }
        hr.ok()
            .map_err(|e| PlatformError::Failed(format!("COM would not initialise: {e}")))?;
        // S_OK or S_FALSE: both are balanced by one CoUninitialize.
        Ok(Self {
            joined: true,
            _not_send: PhantomData,
        })
    }
}

impl Drop for ComScope {
    fn drop(&mut self) {
        if self.joined {
            // SAFETY: balances the successful CoInitializeEx on this same thread (not `Send`).
            unsafe { CoUninitialize() };
        }
    }
}

/// After a thread's start timed out: tells it to stand down (`cancelled`), then takes its ready
/// message if it arrived in between. `Some` means the thread got going anyway and the caller must
/// stop it now, so nothing it holds (a hook, a clipboard promise) outlives the failure it reports.
pub(crate) fn abandon_start<T, E>(
    cancelled: &AtomicBool,
    ready: &Receiver<Result<T, E>>,
) -> Option<T> {
    cancelled.store(true, Ordering::Release);
    match ready.try_recv() {
        Ok(Ok(started)) => Some(started),
        _ => None,
    }
}

/// A windows-rs error as a device error, with what was being done.
pub(crate) fn device_error(what: &str, error: &windows::core::Error) -> PlatformError {
    PlatformError::Device(format!("{what}: {error}"))
}

/// A windows-rs error as a general failure, with what was being done.
pub(crate) fn failed(what: &str, error: &windows::core::Error) -> PlatformError {
    PlatformError::Failed(format!("{what}: {error}"))
}

/// Copies a COM-allocated wide string and frees it. `None` for a null pointer or invalid UTF-16.
///
/// # Safety
///
/// `text` must be null or a NUL-terminated wide string allocated with `CoTaskMemAlloc`, owned by
/// the caller; it is freed here and must not be used again.
pub(crate) unsafe fn take_co_string(text: PWSTR) -> Option<String> {
    if text.is_null() {
        return None;
    }
    // SAFETY: per the contract, a live NUL-terminated wide string.
    let copied = unsafe { text.to_string() }.ok();
    // SAFETY: per the contract, allocated with CoTaskMemAlloc and owned here.
    unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(text.0.cast())) };
    copied
}

/// The part of a path after the last `\` or `/`: `C:\Program Files\App\app.exe` → `app.exe`.
pub(crate) fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// An executable name without `.exe`, for showing: `Zoom.exe` → `Zoom`.
pub(crate) fn display_stem(exe: &str) -> String {
    let stem = exe
        .len()
        .checked_sub(4)
        .filter(|&cut| exe.is_char_boundary(cut) && exe[cut..].eq_ignore_ascii_case(".exe"))
        .map_or(exe, |cut| &exe[..cut]);
    if stem.is_empty() {
        exe.to_owned()
    } else {
        stem.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scope_can_be_entered_twice_on_one_thread() {
        let outer = ComScope::enter().expect("MTA");
        let inner = ComScope::enter().expect("nested");
        drop(inner);
        drop(outer);
    }

    #[test]
    fn an_abandoned_start_hands_back_a_thread_that_got_going_meanwhile() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<u32, ()>>(1);
        let cancelled = AtomicBool::new(false);
        assert_eq!(
            abandon_start(&cancelled, &rx),
            None,
            "not ready: nothing to stop"
        );
        assert!(cancelled.load(Ordering::Acquire), "told to stand down");
        // The race: the thread reported ready just after the wait timed out.
        tx.send(Ok(7)).unwrap();
        assert_eq!(
            abandon_start(&cancelled, &rx),
            Some(7),
            "the caller stops it"
        );
        tx.send(Err(())).unwrap();
        assert_eq!(
            abandon_start(&cancelled, &rx),
            None,
            "a failed start holds nothing"
        );
    }

    #[test]
    fn file_names_and_stems() {
        assert_eq!(file_name(r"C:\Program Files\App\App.exe"), "App.exe");
        assert_eq!(file_name("C:/a/b.exe"), "b.exe");
        assert_eq!(file_name("plain.exe"), "plain.exe");
        assert_eq!(display_stem("Zoom.exe"), "Zoom");
        assert_eq!(display_stem("chrome.EXE"), "chrome");
        assert_eq!(display_stem("tool"), "tool");
        assert_eq!(display_stem(".exe"), ".exe");
        assert_eq!(display_stem("é.exe"), "é");
    }
}
