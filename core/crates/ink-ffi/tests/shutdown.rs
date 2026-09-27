//! Shutdown drops every engine first: ggml's Metal backend aborts the process at exit if a model
//! is still loaded, so `ink_shutdown` must unload every model and let go of every shell engine
//! before it returns. (The real-model version, in a child process, is `shutdown_qwen.rs`.)

mod common;

use std::ffi::{CString, c_char, c_void};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use ink_ffi::external::{ExternalOffline, InkEngineVTable, KIND_OFFLINE};

/// A shell engine written against the C table: it never answers, and records its release.
struct Silent {
    calls: AtomicUsize,
    cancels: AtomicUsize,
    released_at: Mutex<Option<u64>>,
}

unsafe extern "C" fn silent_transcribe(
    ctx: *mut c_void,
    _call: u64,
    _samples: *const f32,
    _len: usize,
    _options: *const c_char,
) {
    // SAFETY: ctx is the `Silent` the test registered and keeps alive.
    let me = unsafe { &*(ctx as *const Silent) };
    me.calls.fetch_add(1, Ordering::SeqCst);
}

unsafe extern "C" fn silent_cancel(ctx: *mut c_void, _call: u64) {
    // SAFETY: as above.
    let me = unsafe { &*(ctx as *const Silent) };
    me.cancels.fetch_add(1, Ordering::SeqCst);
}

unsafe extern "C" fn silent_release(ctx: *mut c_void) {
    // SAFETY: as above.
    let me = unsafe { &*(ctx as *const Silent) };
    *me.released_at.lock().unwrap() = Some(seq());
}

fn table(engine: &Arc<Silent>, info: &CString) -> InkEngineVTable {
    InkEngineVTable {
        size: std::mem::size_of::<InkEngineVTable>() as u32,
        kind: KIND_OFFLINE,
        info_json: info.as_ptr(),
        ctx: Arc::as_ptr(engine) as *mut c_void,
        transcribe: Some(silent_transcribe),
        cancel: Some(silent_cancel),
        release: Some(silent_release),
    }
}

#[test]
fn shutdown_unloads_every_model_and_releases_every_shell_engine_before_returning() {
    let dir = TempDir::new("shutdown");
    let loader = MockLoader::new(Behaviour::Say("x".into()));
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    let (core, events) = start(&dir, &[test_row(ROW_ID)], loader.clone(), installer);

    // A registry model, warm (residency holds it).
    core.command(r#"{"cmd":"model.warm","job":"dictation_final"}"#)
        .unwrap();
    events.wait_type("model.warmed", Duration::from_secs(5));
    assert_eq!(loader.journal.loads.lock().unwrap().len(), 1);

    // A shell engine for the meeting final, better than the model, stuck in a call that is never
    // answered: the meeting's final pass waits on it when shutdown starts.
    let silent = Arc::new(Silent {
        calls: AtomicUsize::new(0),
        cancels: AtomicUsize::new(0),
        released_at: Mutex::new(None),
    });
    let info = CString::new(
        r#"{"id":"silent","licence":"MIT","jobs":[{"job":"meeting_final","wer":1.0}]}"#,
    )
    .unwrap();
    let t = table(&silent, &info);
    // SAFETY: a valid table whose ctx outlives the core.
    let engine =
        unsafe { ExternalOffline::from_table(&t, core.shared().shutdown.clone()) }.unwrap();
    core.register(engine).unwrap();
    events.wait_type("engine.registered", Duration::from_secs(5));

    let mic = dir.path().join("mic.wav");
    speech_wav(&mic, 2.0, 3);
    core.command(&format!(
        r#"{{"cmd":"replay_meeting","mic":{:?},"pacing":"fast"}}"#,
        mic.to_str().unwrap()
    ))
    .unwrap();
    let until = Instant::now() + Duration::from_secs(30);
    while silent.calls.load(Ordering::SeqCst) == 0 {
        assert!(
            Instant::now() < until,
            "the final pass never called the shell engine"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    let started = Instant::now();
    let stopped = core.shutdown();
    let after = seq();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "shutdown waited on an engine that never answers: {:?}",
        started.elapsed()
    );
    assert_eq!(stopped.models_unloaded, 1);
    assert_eq!(stopped.engines_released, 1);

    // Both let go of before shutdown returned, and before the shell heard core.stopped.
    let dropped = loader.journal.drops.lock().unwrap().clone();
    assert_eq!(dropped.len(), 1, "the warm model was dropped once");
    let released = silent.released_at.lock().unwrap().expect("release ran");
    let stopped_at = events
        .seq_of("core.stopped")
        .expect("core.stopped delivered");
    assert!(dropped[0] < after && released < after);
    assert!(dropped[0] < stopped_at && released < stopped_at);
    assert_eq!(
        silent.cancels.load(Ordering::SeqCst),
        1,
        "the stuck call was cancelled"
    );
    assert_eq!(
        events.types().last().map(String::as_str),
        Some("core.stopped")
    );
    events.assert_valid();
}

#[test]
fn a_late_answer_after_shutdown_is_refused_harmlessly() {
    // The call ids of a finished core are gone: answering one is an error code, not a crash.
    static CALL: AtomicU64 = AtomicU64::new(0);
    unsafe extern "C" fn remember(
        _: *mut c_void,
        call: u64,
        _: *const f32,
        _: usize,
        _: *const c_char,
    ) {
        CALL.store(call, Ordering::SeqCst);
    }
    let dir = TempDir::new("late");
    let loader = MockLoader::new(Behaviour::Say("x".into()));
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    let (core, _events) = start(&dir, &[], loader, installer);
    let info =
        CString::new(r#"{"id":"late","licence":"MIT","jobs":[{"job":"meeting_final","wer":1.0}]}"#)
            .unwrap();
    let t = InkEngineVTable {
        size: std::mem::size_of::<InkEngineVTable>() as u32,
        kind: KIND_OFFLINE,
        info_json: info.as_ptr(),
        ctx: std::ptr::null_mut(),
        transcribe: Some(remember),
        cancel: None,
        release: None,
    };
    // SAFETY: a valid table; its functions never touch ctx.
    let engine =
        unsafe { ExternalOffline::from_table(&t, core.shared().shutdown.clone()) }.unwrap();
    core.register(engine).unwrap();
    let mic = dir.path().join("mic.wav");
    speech_wav(&mic, 1.0, 4);
    core.command(&format!(
        r#"{{"cmd":"replay_meeting","mic":{:?},"pacing":"fast"}}"#,
        mic.to_str().unwrap()
    ))
    .unwrap();
    let until = Instant::now() + Duration::from_secs(30);
    while CALL.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(5));
    }
    core.shutdown();
    let call = CALL.load(Ordering::SeqCst);
    assert_eq!(
        ink_ffi::external::complete(call, r#"{"segments":[]}"#),
        Err(ink_ffi::external::CompleteError::Unknown)
    );
}
