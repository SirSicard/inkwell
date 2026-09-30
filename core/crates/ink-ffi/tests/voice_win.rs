//! Windows dictation through the C ABI, as the WinUI shell starts it: `ink_init` gives dictation
//! Windows' platform (the keyboard hook, WASAPI, insertion and focus), so `dictation.enable` binds
//! a key instead of answering `unsupported`. Runs without a desktop: the key asked for here is
//! refused by the hook's token check before any hook is installed. Holding the default key (right
//! Ctrl), speaking and pasting are windows/S3.5a-CHECKLIST.md.
//!
//! One test function: the core is process-wide.
#![cfg(windows)]

mod common;

use std::ffi::{CString, c_char, c_void};
use std::sync::Arc;
use std::time::Duration;

use common::{Recorder, TempDir};
use ink_ffi::*;

const WAIT: Duration = Duration::from_secs(10);

unsafe extern "C" fn on_event(ctx: *mut c_void, json: *const c_char, len: usize) {
    // SAFETY: ctx is the test's recorder, alive until after ink_shutdown; the core passes `len`
    // bytes plus a NUL.
    let (recorder, bytes) = unsafe {
        (
            &*(ctx as *const Recorder),
            std::slice::from_raw_parts(json.cast::<u8>(), len),
        )
    };
    recorder.push(std::str::from_utf8(bytes).unwrap());
}

fn command(json: &str) {
    let json = CString::new(json).unwrap();
    // SAFETY: a NUL-terminated string valid for the call.
    assert_eq!(unsafe { ink_command(json.as_ptr()) }, INK_OK);
}

#[test]
fn dictation_runs_on_windows_platform_and_says_why_a_key_cannot_be_held() {
    let dir = TempDir::new("voice-win");
    let recorder = Recorder::new();
    let ctx = Arc::as_ptr(&recorder) as *mut c_void;
    let config = CString::new(
        serde_json::json!({"data_dir": dir.path().join("data"), "log_stderr": false}).to_string(),
    )
    .unwrap();
    // SAFETY: a valid config string; the recorder outlives the core (shut down below).
    assert_eq!(
        unsafe { ink_init(config.as_ptr(), Some(on_event), ctx) },
        INK_OK
    );
    recorder.wait_type("core.ready", WAIT);

    // Fn is a key the settings know (the Mac's default) but Windows never sees: the Windows
    // platform refuses it, by name, and holds nothing.
    command(r#"{"cmd":"setting.set","key":"dictation.key","value":"fn","id":"key"}"#);
    recorder
        .wait_for(WAIT, |v| v["type"] == "setting.value" && v["value"] == "fn")
        .expect("the key stored");
    command(r#"{"cmd":"dictation.enable","utc_offset_minutes":0,"id":"on"}"#);
    let off = recorder
        .wait_for(WAIT, |v| v["ref"] == "on")
        .expect("dictation.enable answered");
    assert_eq!(off["type"], "dictation.off", "{off}");
    assert_eq!(off["reason"], "key_refused", "not unsupported: {off}");
    assert!(
        off["message"].as_str().unwrap_or_default().contains("Fn"),
        "{off}"
    );

    command(r#"{"cmd":"dictation.disable","id":"off"}"#);
    let disabled = recorder
        .wait_for(WAIT, |v| v["ref"] == "off")
        .expect("dictation.disable answered");
    assert_eq!(disabled["reason"], "disabled", "{disabled}");

    assert_eq!(ink_shutdown(), INK_OK);
    recorder.assert_valid();
}

/// Holds the default key through the real hook: needs a desktop session, so it runs by hand
/// (windows/S3.5a-CHECKLIST.md): `cargo test -p ink-ffi --test voice_win -- --ignored`.
#[test]
#[ignore = "installs a global keyboard hook"]
fn dictation_holds_right_ctrl_by_default() {
    let dir = TempDir::new("voice-win-hook");
    let recorder = Recorder::new();
    let ctx = Arc::as_ptr(&recorder) as *mut c_void;
    let config = CString::new(
        serde_json::json!({"data_dir": dir.path().join("data"), "log_stderr": false}).to_string(),
    )
    .unwrap();
    // SAFETY: as above.
    assert_eq!(
        unsafe { ink_init(config.as_ptr(), Some(on_event), ctx) },
        INK_OK
    );
    recorder.wait_type("core.ready", WAIT);
    command(r#"{"cmd":"dictation.enable","utc_offset_minutes":0,"id":"on"}"#);
    let ready = recorder
        .wait_for(WAIT, |v| v["ref"] == "on")
        .expect("dictation.enable answered");
    assert_eq!(ready["type"], "dictation.ready", "{ready}");
    assert_eq!(ready["key"], "right_control");
    assert_eq!(ink_shutdown(), INK_OK);
}
