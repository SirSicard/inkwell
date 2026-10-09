//! A shell engine as the finals' fallback (on the Mac, FluidAudio's Parakeet): registered with
//! higher error rates than the registry's model (Qwen3-ASR), it serves the dictation and meeting
//! finals only while that model is not installed (its download not finished), and the router hands
//! the jobs back to the model once it is. The shell sees the choice through `engine.route`.

mod common;

use std::ffi::{CString, c_char, c_void};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::*;
use ink_core::{CancelToken, Channel, Job, OfflineEngine, TranscribeOptions};
use ink_engines::ModelDir;
use ink_ffi::external::{ExternalOffline, InkEngineVTable, KIND_OFFLINE};
use ink_ffi::gate::Routed;

/// A shell engine answering "from the fallback", counting its calls.
static CALLS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn fallback(
    _: *mut c_void,
    call: u64,
    _: *const f32,
    len: usize,
    _: *const c_char,
) {
    CALLS.fetch_add(1, Ordering::SeqCst);
    let answer = CString::new(format!(
        r#"{{"segments":[{{"start_ms":0,"end_ms":{},"text":"from the fallback"}}]}}"#,
        len / 16
    ))
    .unwrap();
    // SAFETY: a NUL-terminated string valid for the call.
    unsafe { ink_ffi::ink_engine_complete(call, answer.as_ptr()) };
}

fn options() -> TranscribeOptions {
    TranscribeOptions {
        channel: Channel::Mic,
        context: None,
        cancel: CancelToken::new(),
        live: false,
    }
}

#[test]
fn a_shell_fallback_serves_the_finals_until_the_registry_model_is_installed() {
    let dir = TempDir::new("fallback");
    let loader = MockLoader::new(Behaviour::Say("from the model".into()));
    let installer = std::sync::Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    let row = test_row(ROW_ID); // dictation 5.0, meeting 8.0
    let (core, events) = start(&dir, std::slice::from_ref(&row), loader.clone(), installer);
    // The model is still downloading: its files may be there, its marker is not.
    let models = ModelDir::new(dir.path().join("models"));
    std::fs::remove_file(models.marker_path(&row)).unwrap();
    assert!(!models.is_installed(&row));

    let route = |job: &str| {
        let before = events.count("engine.routed");
        core.command(&format!(r#"{{"cmd":"engine.route","job":"{job}"}}"#))
            .unwrap();
        assert!(events.wait_count("engine.routed", before + 1, Duration::from_secs(5)));
        events
            .all()
            .into_iter()
            .rev()
            .find(|v| v["type"] == "engine.routed")
            .unwrap()
    };
    // Nothing installed, nothing registered: the shell is told so.
    let none = route("meeting_final");
    assert_eq!(none["job"], "meeting_final");
    assert!(none.get("id").is_none(), "{none}");

    // The fallback, with the rates measured for it: worse than the model on both jobs.
    let info = CString::new(
        r#"{"id":"parakeet-offline","licence":"CC-BY-4.0","jobs":[{"job":"dictation_final","wer":6.7},{"job":"meeting_final","wer":23.4}]}"#,
    )
    .unwrap();
    let table = InkEngineVTable {
        kind: KIND_OFFLINE,
        info_json: info.as_ptr(),
        transcribe: Some(fallback),
        ..Default::default()
    };
    // SAFETY: a valid table; its function never touches ctx.
    let engine =
        unsafe { ExternalOffline::from_table(&table, core.shared().shutdown.clone()) }.unwrap();
    core.register(engine).unwrap();

    // While the model downloads, the fallback serves both finals.
    for (job, name) in [
        (Job::DictationFinal, "dictation_final"),
        (Job::MeetingFinal, "meeting_final"),
    ] {
        let routed = route(name);
        assert_eq!(routed["id"], "parakeet-offline", "{routed}");
        assert_eq!(routed["source"], "shell");
        let t = Routed::new(core.shared().clone(), job)
            .transcribe(&[0.0; 16_000], &options())
            .unwrap();
        assert_eq!(t.text(), "from the fallback");
    }
    assert_eq!(CALLS.load(Ordering::SeqCst), 2);
    assert_eq!(
        loader.journal.loads.lock().unwrap().len(),
        0,
        "the model was never loaded"
    );

    // The download finishes: the next call routes to the model, and the fallback is not called.
    install(&models, &row);
    for (job, name) in [
        (Job::DictationFinal, "dictation_final"),
        (Job::MeetingFinal, "meeting_final"),
    ] {
        let routed = route(name);
        assert_eq!(routed["id"], ROW_ID, "{routed}");
        assert_eq!(routed["source"], "registry");
        let t = Routed::new(core.shared().clone(), job)
            .transcribe(&[0.0; 16_000], &options())
            .unwrap();
        assert_eq!(t.text(), "from the model");
    }
    assert_eq!(
        CALLS.load(Ordering::SeqCst),
        2,
        "the fallback is no longer called"
    );

    // An unknown job is refused when read.
    assert!(
        core.command(r#"{"cmd":"engine.route","job":"typing"}"#)
            .is_err()
    );
    core.shutdown();
    events.assert_valid();
}
