//! Permissions: what Windows gates, checked without prompting.
//!
//! | Permission | `check` | `request` |
//! |---|---|---|
//! | Microphone | the privacy consent store in the registry (below) | Settings > Privacy > Microphone |
//! | System Audio | `Granted`: loopback needs no permission on Windows | `Unsupported` |
//! | Accessibility | `Granted`: `SendInput` and UI Automation need none (an elevated target is a different matter: insertion reports `Blocked`) | `Unsupported` |
//! | Input Monitoring | `Granted`: the low-level keyboard hook needs none | `Unsupported` |
//!
//! **The microphone** is three switches, all of which must allow it for a desktop app: the
//! device-wide one (`HKLM\...\ConsentStore\microphone`), the user's "Microphone access"
//! (`HKCU\...\ConsentStore\microphone`) and "Let desktop apps access your microphone"
//! (`...\microphone\NonPackaged`). Each holds `Value` = `Allow` or `Deny`. A desktop app that is
//! denied is not refused a stream: Windows fills it with silence. So the mic's `start` checks
//! here first and refuses instead.
#![cfg(windows)]

use ink_core::{Permission, PermissionProbe, PermissionState, PlatformError};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, w};

const CONSENT_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

/// What one consent switch holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Switch {
    Allow,
    Deny,
    /// Absent, or a value this crate does not know.
    Unreadable,
}

impl Switch {
    fn parse(value: Option<&str>) -> Self {
        match value {
            Some(v) if v.eq_ignore_ascii_case("Allow") => Self::Allow,
            Some(v) if v.eq_ignore_ascii_case("Deny") => Self::Deny,
            _ => Self::Unreadable,
        }
    }
}

/// The microphone's state from its three switches (device, user, desktop apps). Any `Deny` is
/// `Denied`; all `Allow` is `Granted`; otherwise Windows gave no reliable answer.
pub(crate) fn microphone_state(switches: [Switch; 3]) -> PermissionState {
    if switches.contains(&Switch::Deny) {
        PermissionState::Denied
    } else if switches.iter().all(|&s| s == Switch::Allow) {
        PermissionState::Granted
    } else {
        PermissionState::Unknown
    }
}

/// A `REG_SZ` value, or `None`.
fn read_string(root: HKEY, key: &str, value: PCWSTR) -> Option<String> {
    let key = HSTRING::from(key);
    let mut buffer = [0u16; 64];
    let mut bytes = (buffer.len() * 2) as u32;
    // SAFETY: a live key path and value name; the buffer and its size in bytes are live locals.
    let status = unsafe {
        RegGetValueW(
            root,
            &key,
            value,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let len = (bytes as usize / 2).min(buffer.len());
    let text = &buffer[..len];
    let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
    String::from_utf16(&text[..end]).ok()
}

/// The microphone permission for this (desktop) app. Never prompts; reads the registry only.
pub(crate) fn microphone() -> PermissionState {
    let non_packaged = format!(r"{CONSENT_KEY}\NonPackaged");
    microphone_state([
        Switch::parse(read_string(HKEY_LOCAL_MACHINE, CONSENT_KEY, w!("Value")).as_deref()),
        Switch::parse(read_string(HKEY_CURRENT_USER, CONSENT_KEY, w!("Value")).as_deref()),
        Switch::parse(read_string(HKEY_CURRENT_USER, &non_packaged, w!("Value")).as_deref()),
    ])
}

/// [`PermissionProbe`] for Windows.
#[derive(Debug, Default)]
pub struct WinPermissionProbe {
    _private: (),
}

impl WinPermissionProbe {
    /// A probe. It holds no OS resources.
    pub fn new() -> Self {
        Self::default()
    }
}

impl PermissionProbe for WinPermissionProbe {
    fn check(&self, permission: Permission) -> PermissionState {
        match permission {
            Permission::Microphone => microphone(),
            // Nothing on Windows gates these for a desktop app.
            _ => PermissionState::Granted,
        }
    }

    /// Opens the microphone page of Settings, where all three switches live. Windows shows no
    /// prompt to a desktop app; there is nothing to request for the others.
    fn request(&self, permission: Permission) -> Result<(), PlatformError> {
        if permission != Permission::Microphone {
            return Err(PlatformError::Unsupported(
                "Windows has no such permission for a desktop app",
            ));
        }
        // SAFETY: static wide strings; no window owner.
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                w!("ms-settings:privacy-microphone"),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
        // ShellExecute reports success as a value above 32.
        if result.0 as usize > 32 {
            Ok(())
        } else {
            Err(PlatformError::Failed(format!(
                "could not open Settings (ShellExecute returned {})",
                result.0 as usize
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Switch::{Allow, Deny, Unreadable};

    #[test]
    fn any_deny_denies_and_all_allow_grants() {
        assert_eq!(microphone_state([Allow; 3]), PermissionState::Granted);
        assert_eq!(
            microphone_state([Allow, Allow, Deny]),
            PermissionState::Denied
        );
        assert_eq!(
            microphone_state([Deny, Allow, Allow]),
            PermissionState::Denied
        );
        assert_eq!(
            microphone_state([Deny, Unreadable, Allow]),
            PermissionState::Denied
        );
        assert_eq!(
            microphone_state([Allow, Unreadable, Allow]),
            PermissionState::Unknown
        );
    }

    #[test]
    fn switch_values_parse_case_blind() {
        assert_eq!(Switch::parse(Some("Allow")), Allow);
        assert_eq!(Switch::parse(Some("deny")), Deny);
        assert_eq!(Switch::parse(Some("Prompt")), Unreadable);
        assert_eq!(Switch::parse(None), Unreadable);
    }

    /// Runs on CI: reading the registry needs no permission and never prompts. A runner's answer
    /// may be anything but a crash.
    #[test]
    fn the_real_check_answers_without_prompting() {
        let probe = WinPermissionProbe::new();
        let _ = probe.check(Permission::Microphone);
        assert_eq!(
            probe.check(Permission::SystemAudio),
            PermissionState::Granted
        );
        assert!(matches!(
            probe.request(Permission::Accessibility),
            Err(PlatformError::Unsupported(_))
        ));
    }
}
