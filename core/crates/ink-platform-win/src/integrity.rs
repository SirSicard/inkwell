//! The foreground window's process, and whether synthetic input can reach it.
//!
//! **UIPI.** Windows drops keystrokes and clipboard-paste keystrokes that a process sends to a
//! window of a higher integrity level (an app run as administrator), and says nothing: `SendInput`
//! reports success. So insertion checks the target first and reports `Blocked`, the Windows twin
//! of the Mac's Secure Input.
//!
//! The target's level comes from its token. A process that runs elevated usually refuses a
//! non-elevated caller its token: that refusal is itself the answer.
#![cfg(windows)]

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_MANDATORY_LABEL,
    TOKEN_QUERY, TokenIntegrityLevel,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::core::PWSTR;

/// A process's integrity level, as its mandatory-label RID (`SECURITY_MANDATORY_MEDIUM_RID` is
/// 0x2000, high is 0x3000, system 0x4000).
pub(crate) type Integrity = u32;

/// What reading a target's level found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TargetLevel {
    /// Its level.
    Known(Integrity),
    /// Its token was refused to us: it runs at a level we cannot read, so above ours.
    Refused,
    /// Nothing could be read at all (no foreground window, or the process is gone).
    Unknown,
}

/// Whether input from a process at `own` is dropped by `target`. A level that could not be read
/// counts as dropping it: sending blind into an elevated window fails silently. Pure.
pub(crate) fn blocks_input(own: Integrity, target: TargetLevel) -> bool {
    match target {
        TargetLevel::Known(level) => level > own,
        TargetLevel::Refused | TargetLevel::Unknown => true,
    }
}

/// A process handle, closed on drop.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: ours, closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// The level of a process whose handle allows `PROCESS_QUERY_LIMITED_INFORMATION`.
fn level_of(process: HANDLE) -> TargetLevel {
    let mut token = HANDLE::default();
    // SAFETY: a live process handle and a live out-parameter.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.is_err() {
        return TargetLevel::Refused;
    }
    let token = Owned(token);
    // A label is a SID plus attributes; 64 bytes of u64s hold it with room, aligned.
    let mut buffer = [0u64; 8];
    let mut needed = 0u32;
    // SAFETY: a live token; the buffer and its size in bytes are live locals.
    let read = unsafe {
        GetTokenInformation(
            token.0,
            TokenIntegrityLevel,
            Some(buffer.as_mut_ptr().cast()),
            size_of_val(&buffer) as u32,
            &mut needed,
        )
    };
    if read.is_err() {
        return TargetLevel::Unknown;
    }
    // SAFETY: GetTokenInformation filled the buffer with a TOKEN_MANDATORY_LABEL whose SID points
    // inside the same buffer; the buffer is 8-byte aligned.
    unsafe {
        let label = &*buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>();
        let sid = label.Label.Sid;
        let count = *GetSidSubAuthorityCount(sid);
        if count == 0 {
            return TargetLevel::Unknown;
        }
        TargetLevel::Known(*GetSidSubAuthority(sid, u32::from(count) - 1))
    }
}

/// This process's integrity level; medium when it cannot be read.
pub(crate) fn own_level() -> Integrity {
    // SAFETY: the pseudo-handle of this process needs no closing.
    match level_of(unsafe { GetCurrentProcess() }) {
        TargetLevel::Known(level) => level,
        _ => 0x2000,
    }
}

/// The foreground window, if any (none while the secure desktop is up).
pub(crate) fn foreground_window() -> Option<HWND> {
    // SAFETY: no arguments.
    let window = unsafe { GetForegroundWindow() };
    (!window.is_invalid()).then_some(window)
}

/// The process that owns `window`.
pub(crate) fn window_pid(window: HWND) -> Option<u32> {
    let mut pid = 0u32;
    // SAFETY: a window handle (a stale one fails safely) and a live out-parameter.
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    (pid != 0).then_some(pid)
}

/// The foreground window (as a number) and the process that owns it; `None` without one (the
/// secure desktop).
pub(crate) fn foreground_target() -> Option<(usize, u32)> {
    let window = foreground_window()?;
    Some((window.0 as usize, window_pid(window)?))
}

/// The level of the process owning the foreground window; `Unknown` without one.
pub(crate) fn foreground_level() -> TargetLevel {
    match foreground_target() {
        Some((_, pid)) => level_of_pid(pid),
        None => TargetLevel::Unknown,
    }
}

/// The level of process `pid`.
pub(crate) fn level_of_pid(pid: u32) -> TargetLevel {
    // SAFETY: plain arguments; the handle is closed by `Owned`.
    match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(process) => {
            let process = Owned(process);
            level_of(process.0)
        }
        Err(_) => TargetLevel::Refused,
    }
}

/// Whether the foreground window would drop our synthetic input. No foreground window, or one
/// whose level cannot be read, counts as dropping it.
pub(crate) fn foreground_blocks_input() -> bool {
    blocks_input(own_level(), foreground_level())
}

/// The full path of `pid`'s executable, if it can be read.
pub(crate) fn image_path(pid: u32) -> Option<String> {
    // SAFETY: plain arguments; the handle is closed by `Owned`.
    let process =
        Owned(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?);
    let mut buffer = [0u16; 1024];
    let mut len = buffer.len() as u32;
    // SAFETY: a live handle; the buffer and its length in characters are live locals.
    unsafe {
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
    }
    .ok()?;
    String::from_utf16(&buffer[..(len as usize).min(buffer.len())]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEDIUM: Integrity = 0x2000;
    const HIGH: Integrity = 0x3000;

    #[test]
    fn a_higher_or_unreadable_target_blocks() {
        assert!(blocks_input(MEDIUM, TargetLevel::Known(HIGH)));
        assert!(blocks_input(MEDIUM, TargetLevel::Refused));
        assert!(!blocks_input(MEDIUM, TargetLevel::Known(MEDIUM)));
        assert!(
            !blocks_input(HIGH, TargetLevel::Known(HIGH)),
            "both elevated"
        );
        assert!(!blocks_input(MEDIUM, TargetLevel::Known(0x1000)), "low");
        assert!(
            blocks_input(MEDIUM, TargetLevel::Unknown),
            "an unreadable level is never taken as reachable"
        );
    }

    /// Runs on CI: reading this process's own token needs nothing.
    #[test]
    fn this_process_has_a_real_integrity_level() {
        let level = own_level();
        assert!((0x1000..=0x4000).contains(&level), "{level:#x}");
        let path = image_path(std::process::id()).expect("own image path");
        assert!(path.to_ascii_lowercase().ends_with(".exe"), "{path}");
    }
}
