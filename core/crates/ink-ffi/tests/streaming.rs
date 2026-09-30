//! A live-partials engine the shell registers (`INK_ENGINE_STREAMING`), driven through the C ABI
//! the way the Mac shell's FluidAudio engine is: streams opened, fed and finished on the meeting's
//! worker, events sent back through `ink_stream_event` from the engine's own thread, and every
//! stream closed once. Plus the table checks every kind gets.
//!
//! The first test owns the process-wide core (`ink_init`); the others use no global core.

mod common;

use std::collections::HashMap;
use std::ffi::{CString, c_char, c_void};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use common::{Recorder, TempDir, speech_wav};
use ink_core::CancelToken;
use ink_ffi::external::{
    InkEngineVTable, KIND_LLM, KIND_OFFLINE, KIND_STREAMING, Registration, VTABLE_V1_SIZE,
};
use ink_ffi::*;

unsafe extern "C" fn on_event(ctx: *mut c_void, json: *const c_char, len: usize) {
    // SAFETY: ctx is the test's recorder, alive until after ink_shutdown.
    let (recorder, bytes) = unsafe {
        (
            &*(ctx as *const Recorder),
            std::slice::from_raw_parts(json.cast::<u8>(), len),
        )
    };
    recorder.push(std::str::from_utf8(bytes).unwrap());
}

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

/// What the engine's own thread does, in order: a stream's events, then the answer that follows
/// them.
enum Work {
    Event(u64, String),
    Answer(u64),
    Stop,
}

/// A streaming engine with a thread of its own, as a Swift engine answering from a Task: pushes
/// are answered at once (the samples copied), words arrive later from that thread.
struct Streamy {
    samples: Mutex<HashMap<u64, usize>>,
    channels: Mutex<HashMap<u64, String>>,
    opened: AtomicUsize,
    closed: AtomicUsize,
    released: AtomicUsize,
    calls: Mutex<Vec<Option<String>>>,
    work: Mutex<Sender<Work>>,
}

impl Streamy {
    fn start() -> (Arc<Self>, JoinHandle<Vec<i32>>) {
        let (tx, rx) = mpsc::channel::<Work>();
        let thread = std::thread::Builder::new()
            .name("engine-queue".into())
            .spawn(move || {
                let mut statuses = Vec::new();
                while let Ok(work) = rx.recv() {
                    // SAFETY (both calls): NUL-terminated strings valid for the call.
                    match work {
                        Work::Event(stream, json) => {
                            statuses.push(unsafe { ink_stream_event(stream, c(&json).as_ptr()) })
                        }
                        Work::Answer(call) => statuses.push(unsafe {
                            ink_engine_complete(call, c(r#"{"ok":true}"#).as_ptr())
                        }),
                        Work::Stop => break,
                    }
                }
                statuses
            })
            .unwrap();
        let me = Arc::new(Self {
            samples: Mutex::default(),
            channels: Mutex::default(),
            opened: AtomicUsize::new(0),
            closed: AtomicUsize::new(0),
            released: AtomicUsize::new(0),
            calls: Mutex::default(),
            work: Mutex::new(tx),
        });
        (me, thread)
    }

    fn send(&self, work: Work) {
        self.work.lock().unwrap().send(work).unwrap();
    }

    fn note_call(&self) {
        self.calls
            .lock()
            .unwrap()
            .push(std::thread::current().name().map(str::to_owned));
    }
}

fn me(ctx: *mut c_void) -> &'static Streamy {
    // SAFETY: ctx is the test's `Streamy`, kept alive past the core's release.
    unsafe { &*(ctx as *const Streamy) }
}

unsafe extern "C" fn s_open(ctx: *mut c_void, call: u64, stream: u64, options: *const c_char) {
    let me = me(ctx);
    me.note_call();
    // SAFETY: valid for the call.
    let options = unsafe { std::ffi::CStr::from_ptr(options) }
        .to_str()
        .unwrap()
        .to_owned();
    let v: serde_json::Value = serde_json::from_str(&options).unwrap();
    me.channels
        .lock()
        .unwrap()
        .insert(stream, v["channel"].as_str().unwrap().to_owned());
    me.samples.lock().unwrap().insert(stream, 0);
    me.opened.fetch_add(1, Ordering::SeqCst);
    // Answered later, from the engine's thread.
    me.send(Work::Answer(call));
}

unsafe extern "C" fn s_push(ctx: *mut c_void, call: u64, stream: u64, _: *const f32, len: usize) {
    let me = me(ctx);
    me.note_call();
    let (before, after) = {
        let mut samples = me.samples.lock().unwrap();
        let n = samples.get_mut(&stream).unwrap();
        let before = *n;
        *n += len;
        (before, *n)
    };
    // A partial every half second of audio; the far side also reports a stall once.
    if before / 8_000 != after / 8_000 {
        me.send(Work::Event(
            stream,
            format!(r#"{{"partial":"heard {} half seconds"}}"#, after / 8_000),
        ));
        let far = me.channels.lock().unwrap()[&stream] == "far";
        if far && after / 8_000 == 1 {
            me.send(Work::Event(stream, r#"{"stalled":{"code":3}}"#.into()));
        }
    }
    // SAFETY: answered synchronously, before this function returns: allowed.
    assert_eq!(
        unsafe { ink_engine_complete(call, c(r#"{"ok":true}"#).as_ptr()) },
        INK_OK
    );
}

unsafe extern "C" fn s_finish(ctx: *mut c_void, call: u64, stream: u64) {
    let me = me(ctx);
    me.note_call();
    let end_ms = me.samples.lock().unwrap()[&stream] / 16;
    me.send(Work::Event(
        stream,
        format!(
            r#"{{"final":{{"start_ms":0,"end_ms":{end_ms},"text":"synthetic settled words"}}}}"#
        ),
    ));
    me.send(Work::Event(stream, r#"{"partial":""}"#.into()));
    // After the events: the answer comes last, from the same ordered thread.
    me.send(Work::Answer(call));
}

unsafe extern "C" fn s_close(ctx: *mut c_void, stream: u64) {
    let me = me(ctx);
    me.closed.fetch_add(1, Ordering::SeqCst);
    me.samples.lock().unwrap().remove(&stream);
}

unsafe extern "C" fn s_release(ctx: *mut c_void) {
    me(ctx).released.fetch_add(1, Ordering::SeqCst);
}

fn streaming_table(engine: &Streamy, info: &CString) -> InkEngineVTable {
    InkEngineVTable {
        kind: KIND_STREAMING,
        info_json: info.as_ptr(),
        ctx: engine as *const Streamy as *mut c_void,
        release: Some(s_release),
        stream_open: Some(s_open),
        stream_push: Some(s_push),
        stream_finish: Some(s_finish),
        stream_close: Some(s_close),
        ..Default::default()
    }
}

#[test]
fn a_shell_streaming_engine_gives_a_meeting_its_live_partials() {
    let dir = TempDir::new("streaming");
    let recorder = Recorder::new();
    let ctx = Arc::as_ptr(&recorder) as *mut c_void;
    let config = c(&serde_json::json!({
        "data_dir": dir.path().join("data"),
        "log_level": "warn",
        "log_stderr": false
    })
    .to_string());
    let (engine, engine_thread) = Streamy::start();
    let info = c(
        r#"{"id":"parakeet-live","licence":"CC-BY-4.0","jobs":[{"job":"live_partials","wer":23.4}]}"#,
    );

    // SAFETY (every unsafe call below): arguments are valid NUL-terminated strings and tables,
    // and the recorder and engine outlive the core.
    unsafe {
        assert_eq!(ink_init(config.as_ptr(), Some(on_event), ctx), INK_OK);
        recorder.wait_type("core.ready", Duration::from_secs(5));
        assert_eq!(
            ink_register_engine(&streaming_table(&engine, &info)),
            INK_OK
        );
        let registered = recorder.wait_type("engine.registered", Duration::from_secs(5));
        assert_eq!(registered["kind"], "streaming");
        assert_eq!(registered["jobs"][0]["job"], "live_partials");

        let (mic, far) = (dir.path().join("mic.wav"), dir.path().join("far.wav"));
        speech_wav(&mic, 2.5, 21);
        speech_wav(&far, 2.0, 22);
        let replay = c(&serde_json::json!({
            "cmd": "replay_meeting", "mic": mic, "far": far, "pacing": "fast"
        })
        .to_string());
        assert_eq!(ink_command(replay.as_ptr()), INK_OK);
        let ended = recorder
            .wait_for(Duration::from_secs(60), |v| {
                v["type"] == "meeting.finished" || v["type"] == "meeting.failed"
            })
            .expect("the meeting ended");
        let _ = ended;

        // Live partials and a settled final from each side, through the stream events.
        let all = recorder.all();
        let partials: Vec<_> = all
            .iter()
            .filter(|v| v["type"] == "meeting.partial")
            .collect();
        for side in ["mic", "far"] {
            assert!(
                partials
                    .iter()
                    .any(|p| p["channel"] == side
                        && p["text"].as_str().unwrap().starts_with("heard")),
                "no {side} partial: {partials:?}"
            );
            let finals: Vec<_> = all
                .iter()
                .filter(|v| v["type"] == "meeting.final" && v["channel"] == side)
                .collect();
            assert_eq!(finals.len(), 1, "{side}: {finals:?}");
            assert_eq!(finals[0]["text"], "synthetic settled words");
            assert!(
                finals[0]["end_ms"].as_u64() > Some(1_500),
                "{:?}",
                finals[0]
            );
        }
        assert!(
            all.iter().any(|v| v["type"] == "meeting.warning"
                && v["kind"] == "live_engine_stalled"
                && v["channel"] == "far"),
            "the far stream's stall is reported"
        );

        // Each side got its own stream: opened, fed and finished on the meeting's worker, and
        // closed exactly once.
        assert_eq!(engine.opened.load(Ordering::SeqCst), 2);
        assert_eq!(engine.closed.load(Ordering::SeqCst), 2);
        let calls = engine.calls.lock().unwrap().clone();
        assert!(!calls.is_empty());
        assert!(
            calls.iter().all(|t| t.as_deref() == Some("ink-meeting")),
            "{calls:?}"
        );
        // A closed stream refuses events: the chain has moved on.
        assert_eq!(
            ink_stream_event(1, c(r#"{"partial":"late"}"#).as_ptr()),
            INK_ERR_UNKNOWN_CALL
        );
        assert_eq!(
            ink_stream_event(1, std::ptr::null()),
            INK_ERR_INVALID_ARGUMENT
        );

        // Shutdown lets go of the engine: its release runs before it returns.
        assert_eq!(ink_shutdown(), INK_OK);
        assert_eq!(engine.released.load(Ordering::SeqCst), 1);
    }
    engine.send(Work::Stop);
    let statuses = engine_thread.join().unwrap();
    assert!(
        statuses.iter().all(|s| *s == INK_OK),
        "every event and answer was taken while its stream was open: {statuses:?}"
    );
    recorder.assert_valid();
}

// ------------------------------------------------------------------------------------------------
// Table checks, per kind

unsafe extern "C" fn nop_transcribe(
    _: *mut c_void,
    _: u64,
    _: *const f32,
    _: usize,
    _: *const c_char,
) {
}
unsafe extern "C" fn nop_open(_: *mut c_void, _: u64, _: u64, _: *const c_char) {}
unsafe extern "C" fn nop_push(_: *mut c_void, _: u64, _: u64, _: *const f32, _: usize) {}
unsafe extern "C" fn nop_finish(_: *mut c_void, _: u64, _: u64) {}
unsafe extern "C" fn nop_close(_: *mut c_void, _: u64) {}
unsafe extern "C" fn nop_generate(_: *mut c_void, _: u64, _: *const c_char) {}

fn offline_info() -> CString {
    c(r#"{"id":"o","licence":"MIT","jobs":[{"job":"dictation_final","wer":3}]}"#)
}
fn live_info() -> CString {
    c(r#"{"id":"s","licence":"MIT","jobs":[{"job":"live_partials","wer":3}]}"#)
}
fn llm_info() -> CString {
    c(r#"{"id":"l","licence":"Apple","model":"system","local":true}"#)
}

fn register(table: &InkEngineVTable) -> Result<Registration, String> {
    // SAFETY: a valid table for the call; every function is a no-op and ctx is never read.
    unsafe { Registration::from_table(table, CancelToken::new()) }.map_err(|e| e.0)
}

fn streaming(info: &CString) -> InkEngineVTable {
    InkEngineVTable {
        kind: KIND_STREAMING,
        info_json: info.as_ptr(),
        stream_open: Some(nop_open),
        stream_push: Some(nop_push),
        stream_finish: Some(nop_finish),
        stream_close: Some(nop_close),
        ..Default::default()
    }
}

#[test]
fn each_kind_needs_its_own_functions_and_no_other_kind_s() {
    let (oi, si, li) = (offline_info(), live_info(), llm_info());
    let offline = InkEngineVTable {
        kind: KIND_OFFLINE,
        info_json: oi.as_ptr(),
        transcribe: Some(nop_transcribe),
        ..Default::default()
    };
    assert!(matches!(register(&offline), Ok(Registration::Offline(_))));
    assert!(matches!(
        register(&streaming(&si)),
        Ok(Registration::Streaming(_))
    ));
    let llm = InkEngineVTable {
        kind: KIND_LLM,
        info_json: li.as_ptr(),
        generate: Some(nop_generate),
        ..Default::default()
    };
    let Ok(Registration::Llm(model)) = register(&llm) else {
        panic!("a language model's table")
    };
    assert_eq!(model.id(), "l");

    // Another kind's function: the kind is probably wrong, so the table is refused.
    for (what, table) in [
        (
            "offline + generate",
            InkEngineVTable {
                generate: Some(nop_generate),
                ..offline_copy(&offline)
            },
        ),
        (
            "offline + open",
            InkEngineVTable {
                stream_open: Some(nop_open),
                ..offline_copy(&offline)
            },
        ),
        (
            "streaming + transcribe",
            InkEngineVTable {
                transcribe: Some(nop_transcribe),
                ..streaming(&si)
            },
        ),
        (
            "llm + close",
            InkEngineVTable {
                stream_close: Some(nop_close),
                ..offline_copy(&llm)
            },
        ),
    ] {
        assert!(register(&table).is_err(), "{what}");
    }
    // Missing functions of its own kind.
    for missing in 0..4 {
        let mut t = streaming(&si);
        match missing {
            0 => t.stream_open = None,
            1 => t.stream_push = None,
            2 => t.stream_finish = None,
            _ => t.stream_close = None,
        }
        assert!(
            register(&t).is_err(),
            "streaming without function {missing}"
        );
    }
    assert!(
        register(&InkEngineVTable {
            generate: None,
            ..offline_copy(&llm)
        })
        .is_err()
    );
    assert!(
        register(&InkEngineVTable {
            transcribe: None,
            ..offline_copy(&offline)
        })
        .is_err()
    );
    // Each kind's info shape.
    assert!(
        register(&streaming(&li)).is_err(),
        "a model's info for a stream"
    );
    assert!(
        register(&InkEngineVTable {
            info_json: si.as_ptr(),
            ..offline_copy(&llm)
        })
        .is_err()
    );
    assert!(
        register(&InkEngineVTable {
            kind: 9,
            ..offline_copy(&offline)
        })
        .is_err()
    );
}

/// A copy of a table (the struct holds raw pointers, so it is not `Clone`).
fn offline_copy(t: &InkEngineVTable) -> InkEngineVTable {
    InkEngineVTable {
        size: t.size,
        kind: t.kind,
        info_json: t.info_json,
        ctx: t.ctx,
        transcribe: t.transcribe,
        cancel: t.cancel,
        release: t.release,
        stream_open: t.stream_open,
        stream_push: t.stream_push,
        stream_finish: t.stream_finish,
        stream_close: t.stream_close,
        generate: t.generate,
    }
}

#[test]
fn an_abi_1_table_still_registers_an_offline_engine_and_nothing_newer() {
    let (oi, si, li) = (offline_info(), live_info(), llm_info());
    // The shell's table ends at `release`; whatever lies after it in memory is not read.
    #[repr(C)]
    struct V1 {
        size: u32,
        kind: u32,
        info_json: *const c_char,
        ctx: *mut c_void,
        transcribe:
            Option<unsafe extern "C" fn(*mut c_void, u64, *const f32, usize, *const c_char)>,
        cancel: Option<unsafe extern "C" fn(*mut c_void, u64)>,
        release: Option<unsafe extern "C" fn(*mut c_void)>,
    }
    assert_eq!(std::mem::size_of::<V1>(), VTABLE_V1_SIZE);
    let v1 = V1 {
        size: VTABLE_V1_SIZE as u32,
        kind: KIND_OFFLINE,
        info_json: oi.as_ptr(),
        ctx: std::ptr::null_mut(),
        transcribe: Some(nop_transcribe),
        cancel: None,
        release: None,
    };
    let table = (&v1 as *const V1).cast::<InkEngineVTable>();
    // SAFETY: `size` says exactly what is readable, which is all the core reads.
    let offline = unsafe { Registration::from_table(table, CancelToken::new()) };
    assert!(matches!(offline, Ok(Registration::Offline(_))));
    for (kind, info) in [(KIND_STREAMING, &si), (KIND_LLM, &li)] {
        let v1 = V1 {
            kind,
            info_json: info.as_ptr(),
            transcribe: None,
            ..v1
        };
        let table = (&v1 as *const V1).cast::<InkEngineVTable>();
        // SAFETY: as above.
        let r = unsafe { Registration::from_table(table, CancelToken::new()) };
        assert!(r.is_err(), "kind {kind} needs the ABI 2 fields");
    }
    let tiny = InkEngineVTable {
        size: 8,
        ..streaming(&si)
    };
    assert!(register(&tiny).is_err());

    // A newer shell's longer table: the fields this core does not know are ignored.
    #[repr(C)]
    struct Newer {
        table: InkEngineVTable,
        later: [u64; 2],
    }
    let newer = Newer {
        table: InkEngineVTable {
            size: std::mem::size_of::<Newer>() as u32,
            ..streaming(&si)
        },
        later: [7, 7],
    };
    assert!(matches!(
        register(&newer.table),
        Ok(Registration::Streaming(_))
    ));
}

/// Windows' live partials: a registry model the core runs itself (Parakeet on sherpa-onnx there),
/// through the trailing-window scheme, loaded once through residency for both sides.
#[test]
fn a_registry_model_gives_a_meeting_its_live_partials() {
    use common::{Behaviour, MockInstaller, MockLoader, test_row};
    use ink_core::Job;
    use ink_engines::{JobScore, Runtime};

    let dir = TempDir::new("live-model");
    let row = ink_engines::EngineRow {
        scores: vec![JobScore {
            job: Job::LivePartials,
            wer: 27.9,
        }],
        runtime: Runtime::SherpaOnnx,
        ..test_row("test-live")
    };
    // One segment per decode, over the whole window: settled when the stream finishes.
    let loader = MockLoader::new(Behaviour::Say("live words".into()));
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    let (core, recorder) = common::start(&dir, &[row], loader.clone(), installer);
    let (mic, far) = (dir.path().join("mic.wav"), dir.path().join("far.wav"));
    speech_wav(&mic, 2.5, 21);
    speech_wav(&far, 2.0, 22);
    core.command(
        &serde_json::json!({"cmd": "replay_meeting", "mic": mic, "far": far, "pacing": "fast"})
            .to_string(),
    )
    .unwrap();
    recorder
        .wait_for(Duration::from_secs(60), |v| {
            v["type"] == "meeting.finished" || v["type"] == "meeting.failed"
        })
        .expect("the meeting ended");
    let all = recorder.all();
    for side in ["mic", "far"] {
        let finals: Vec<_> = all
            .iter()
            .filter(|v| v["type"] == "meeting.final" && v["channel"] == side)
            .collect();
        assert_eq!(finals.len(), 1, "{side}: {finals:?}");
        assert_eq!(finals[0]["text"], "live words");
    }
    assert!(
        !all.iter()
            .any(|v| v["type"] == "meeting.warning" && v["kind"] == "live_engine_failed"),
        "{all:?}"
    );
    assert_eq!(
        loader.journal.loads.lock().unwrap().len(),
        1,
        "one copy of the model, for both sides"
    );
    recorder.assert_valid();
    core.shutdown();
}
