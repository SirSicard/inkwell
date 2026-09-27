//! `ink_shutdown` unloads the real model before the process exits. ggml's Metal backend aborts
//! the process at exit if a model is still loaded, so a shell that quits through `ink_shutdown`
//! must exit cleanly, and one that does not must not (the control: it shows this test can fail).
//!
//! Local only (CI has no model): the model is read from
//! `$INK_BENCH_DIR/models/qwen3-asr-1.7b-gguf/`, linked into a model directory laid out as the
//! downloader leaves it.
//!
//! ```text
//! INK_BENCH_DIR=<bench data> cargo test -p ink-ffi --features engine-llama \
//!     --test shutdown_qwen -- --ignored --nocapture
//! ```

#![cfg(all(feature = "engine-llama", target_os = "macos"))]

use std::ffi::{CString, c_char, c_void};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ink_engines::{ModelDir, Registry};
use ink_ffi::{INK_OK, ink_command, ink_init, ink_shutdown};

const ID: &str = "qwen3-asr-1.7b-q8";
/// Turns the child on: "shutdown" or "exit".
const CHILD: &str = "INK_FFI_QWEN_CHILD";
/// The directory the parent made for the child.
const DIR: &str = "INK_FFI_QWEN_DIR";

static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

unsafe extern "C" fn on_event(_: *mut c_void, json: *const c_char, len: usize) {
    // SAFETY: the core passes `len` valid bytes for the call.
    let bytes = unsafe { std::slice::from_raw_parts(json.cast::<u8>(), len) };
    EVENTS
        .lock()
        .unwrap()
        .push(String::from_utf8_lossy(bytes).into_owned());
}

fn bench_model() -> PathBuf {
    let bench = std::env::var_os("INK_BENCH_DIR")
        .expect("set INK_BENCH_DIR to the bench data (a real-model check never passes without it)");
    Path::new(&bench).join("models/qwen3-asr-1.7b-gguf")
}

/// Links the bench model into `root` as an installed registry model.
fn install(root: &Path) {
    let dir = ModelDir::new(root);
    let row = Registry::builtin().unwrap().get(ID).unwrap().clone();
    for f in &row.files {
        let from = bench_model().join(&f.name);
        assert!(
            from.is_file(),
            "the bench model is missing {}",
            from.display()
        );
        let to = dir.file_path(&row, f);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&from, &to).unwrap();
    }
    std::fs::write(dir.marker_path(&row), &row.revision).unwrap();
    assert!(dir.is_installed(&row));
}

#[test]
#[ignore = "needs the Qwen3-ASR model under $INK_BENCH_DIR/models/; run locally"]
fn loading_qwen_then_ink_shutdown_exits_cleanly() {
    let dir = std::env::temp_dir().join(format!("ink-ffi-qwen-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    install(&dir.join("models"));
    let child = |mode: &str| {
        Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "child_warms_qwen_through_the_abi"])
            .env(CHILD, mode)
            .env(DIR, &dir)
            .status()
            .unwrap()
    };
    let clean = child("shutdown");
    let leaked = child("exit");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        clean.success(),
        "loaded, then ink_shutdown, then exit: {clean:?}"
    );
    assert_eq!(
        leaked.signal(),
        Some(6),
        "the control: exiting with the model loaded should abort: {leaked:?}"
    );
}

#[test]
#[ignore = "a helper for the test above; does nothing unless INK_FFI_QWEN_CHILD is set"]
fn child_warms_qwen_through_the_abi() {
    let Ok(mode) = std::env::var(CHILD) else {
        return;
    };
    let dir = PathBuf::from(std::env::var_os(DIR).unwrap());
    let config = CString::new(
        serde_json::json!({
            "data_dir": dir.join(format!("data-{mode}")),
            "models_dir": dir.join("models"),
            "log_level": "warn"
        })
        .to_string(),
    )
    .unwrap();
    let warm = CString::new(r#"{"cmd":"model.warm","job":"dictation_final"}"#).unwrap();
    // SAFETY: valid strings, and a callback that needs no context.
    unsafe {
        assert_eq!(
            ink_init(config.as_ptr(), Some(on_event), std::ptr::null_mut()),
            INK_OK
        );
        assert_eq!(ink_command(warm.as_ptr()), INK_OK);
    }
    let until = Instant::now() + Duration::from_secs(120);
    loop {
        let events = EVENTS.lock().unwrap().clone();
        if events.iter().any(|e| e.contains("\"model.warmed\"")) {
            assert!(events.iter().any(|e| e.contains(ID)), "{events:?}");
            break;
        }
        assert!(
            !events
                .iter()
                .any(|e| e.contains("warm_failed") || e.contains("refused")),
            "{events:?}"
        );
        assert!(Instant::now() < until, "the model did not load: {events:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
    if mode == "shutdown" {
        assert_eq!(ink_shutdown(), INK_OK);
    }
    // Returning ends the process: with the model still loaded in "exit" mode.
}
