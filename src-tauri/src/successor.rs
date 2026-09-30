//! The one-time notice in the last 0.2 release: where Inkwell goes next.
//!
//! Inkwell 1.0 is a new app for Apple silicon Macs and Windows. The updater
//! built into 0.2 can only install another 0.2 build, so without this notice a
//! 0.2 user would never hear that 1.0 exists. Linux and Intel Macs get no 1.0,
//! so there the notice says this version is the last one and keeps working.
//!
//! Which notice a machine gets is decided here rather than in the webview,
//! because the webview cannot tell an Apple silicon Mac from an Intel one:
//! WebKit reports "Intel Mac OS X" on both.

use serde::Serialize;

/// The page that offers Inkwell 1.0. Fixed here, and the command that opens it
/// takes no argument, so the webview can open this page and nothing else.
/// Must match SITE_URL in homepage/lib/constants.ts and in src/constants.ts,
/// which the notice prints (tests below check).
///
/// The notice is only true once the live page offers 1.0, which nothing here
/// can check: docs/RELEASING.md holds the release until it does.
pub const SITE_URL: &str = "https://getinkwell.vercel.app";

/// Who is reading the notice, which decides what it says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    /// 1.0 runs here (on macOS 26 or later).
    AppleSilicon,
    /// 1.0 runs here.
    Windows,
    /// No 1.0: this is the last version.
    IntelMac,
    /// No 1.0: this is the last version.
    Linux,
}

/// Decide the audience from the build's OS and architecture, and whether an
/// Intel build is running under Rosetta.
///
/// Rosetta matters because the 0.2 downloads offered an Intel build too, and
/// its updater keeps serving Intel builds to whoever installed one. Someone who
/// picked it on an Apple silicon Mac can run 1.0, and telling them their Mac
/// has reached its last version would be wrong.
pub fn audience(os: &str, arch: &str, translated: bool) -> Audience {
    match (os, arch) {
        ("windows", _) => Audience::Windows,
        ("macos", "aarch64") => Audience::AppleSilicon,
        ("macos", _) if translated => Audience::AppleSilicon,
        ("macos", _) => Audience::IntelMac,
        // 0.2 builds for macOS, Windows and Linux only.
        _ => Audience::Linux,
    }
}

/// Whether this process is an Intel build translated by Rosetta.
#[cfg(target_os = "macos")]
fn translated() -> bool {
    proc_translated() == Some(1)
}

#[cfg(not(target_os = "macos"))]
fn translated() -> bool {
    false
}

/// The check Apple documents: `sysctl.proc_translated` is 1 under Rosetta, 0
/// for a native process, and absent (the call fails, `None`) on an Intel Mac.
#[cfg(target_os = "macos")]
fn proc_translated() -> Option<std::ffi::c_int> {
    use std::ffi::{c_char, c_int, c_void};

    extern "C" {
        fn sysctlbyname(
            name: *const c_char,
            oldp: *mut c_void,
            oldlenp: *mut usize,
            newp: *mut c_void,
            newlen: usize,
        ) -> c_int;
    }

    let mut value: c_int = 0;
    let mut len = std::mem::size_of::<c_int>();
    // SAFETY: the name is a NUL-terminated literal, `value` and `len` describe
    // a writable buffer of exactly the size passed, and nothing is written
    // (newp is null, newlen 0).
    let rc = unsafe {
        sysctlbyname(
            c"sysctl.proc_translated".as_ptr(),
            (&mut value as *mut c_int).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0).then_some(value)
}

#[tauri::command]
pub fn successor_audience() -> Audience {
    audience(std::env::consts::OS, std::env::consts::ARCH, translated())
}

/// Open [`SITE_URL`] in the default browser.
///
/// A link in the webview cannot do this: the app registers no handler for
/// new-window requests, and without one wry (0.55) drops a `target="_blank"`
/// click on macOS and on Windows.
#[tauri::command]
pub fn open_successor_site() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(target_os = "windows")]
    let opener = "explorer";
    #[cfg(target_os = "linux")]
    let opener = "xdg-open";

    std::process::Command::new(opener)
        .arg(SITE_URL)
        .spawn()
        .map_err(|e| {
            format!(
                "Could not open the browser ({}). The address is {}",
                e, SITE_URL
            )
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_silicon_and_windows_are_told_about_1_0() {
        assert_eq!(audience("macos", "aarch64", false), Audience::AppleSilicon);
        assert_eq!(audience("windows", "x86_64", false), Audience::Windows);
    }

    #[test]
    fn intel_macs_and_linux_are_told_this_is_the_last_version() {
        assert_eq!(audience("macos", "x86_64", false), Audience::IntelMac);
        assert_eq!(audience("linux", "x86_64", false), Audience::Linux);
    }

    /// The Intel build on an Apple silicon Mac is an Apple silicon user.
    #[test]
    fn an_intel_build_under_rosetta_is_told_about_1_0() {
        assert_eq!(audience("macos", "x86_64", true), Audience::AppleSilicon);
    }

    /// The frontend switches on these exact strings.
    #[test]
    fn audiences_reach_the_webview_as_these_names() {
        let names: Vec<String> = [
            Audience::AppleSilicon,
            Audience::Windows,
            Audience::IntelMac,
            Audience::Linux,
        ]
        .iter()
        .map(|a| serde_json::to_string(a).unwrap())
        .collect();
        assert_eq!(
            names,
            [
                "\"apple_silicon\"",
                "\"windows\"",
                "\"intel_mac\"",
                "\"linux\""
            ]
        );
    }

    /// Runs the real sysctl call. On Apple silicon the key exists, so the call
    /// must succeed and read 0 for a native build; the Rosetta answer needs an
    /// Intel build to observe.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn a_native_apple_silicon_build_reads_not_translated() {
        assert_eq!(proc_translated(), Some(0));
        assert!(!translated());
    }

    /// The notice sends people to the same site the homepage calls canonical.
    #[test]
    fn the_site_matches_the_homepage() {
        let constants = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../homepage/lib/constants.ts"
        ))
        .unwrap();
        assert!(
            constants.contains(&format!("export const SITE_URL = \"{}\";", SITE_URL)),
            "SITE_URL differs from homepage/lib/constants.ts"
        );
    }

    /// The address the notice prints is the one this command opens.
    #[test]
    fn the_notice_prints_the_address_it_opens() {
        let constants =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../src/constants.ts"))
                .unwrap();
        assert!(
            constants.contains(&format!("export const SITE_URL = \"{}\"", SITE_URL)),
            "SITE_URL differs from src/constants.ts"
        );
    }
}
