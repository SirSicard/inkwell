//! Every function in `inkwell.h`, called as a shell calls it: through the C ABI, with a C event
//! callback and an asynchronous engine answering from its own thread. The Swift smoke test does
//! the same from Swift (mac/Tests); this one runs on both CI runners.
//!
//! One test function: the core is process-wide.

mod common;

use std::ffi::{CString, c_char, c_void};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{Recorder, TempDir, speech_wav};
use ink_ffi::external::{InkEngineVTable, KIND_OFFLINE};
use ink_ffi::*;

unsafe extern "C" fn on_event(ctx: *mut c_void, json: *const c_char, len: usize) {
    // SAFETY: ctx is the test's recorder, alive until after ink_shutdown; the core passes `len`
    // bytes plus a NUL.
    let (recorder, bytes) = unsafe {
        (
            &*(ctx as *const Recorder),
            std::slice::from_raw_parts(json.cast::<u8>(), len + 1),
        )
    };
    assert_eq!(bytes[len], 0, "NUL-terminated");
    recorder.push(std::str::from_utf8(&bytes[..len]).unwrap());
}

/// An engine that answers from a thread of its own, as a Swift engine answering from a Task.
#[derive(Default)]
struct AsyncEngine {
    calls: AtomicUsize,
    threads: Mutex<Vec<Option<String>>>,
    released: AtomicUsize,
}

unsafe extern "C" fn async_transcribe(
    ctx: *mut c_void,
    call: u64,
    samples: *const f32,
    len: usize,
    options: *const c_char,
) {
    // SAFETY: ctx is the test's engine; samples and options are valid for this call.
    let (me, audio, options) = unsafe {
        (
            &*(ctx as *const AsyncEngine),
            std::slice::from_raw_parts(samples, len).to_vec(),
            std::ffi::CStr::from_ptr(options)
                .to_str()
                .unwrap()
                .to_owned(),
        )
    };
    me.calls.fetch_add(1, Ordering::SeqCst);
    me.threads
        .lock()
        .unwrap()
        .push(std::thread::current().name().map(str::to_owned));
    assert!(options.contains("\"channel\""), "{options}");
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(2));
        let end_ms = audio.len() / 16;
        let answer = CString::new(format!(
            r#"{{"segments":[{{"start_ms":0,"end_ms":{end_ms},"text":"answered async"}}]}}"#
        ))
        .unwrap();
        // SAFETY: a NUL-terminated string valid for the call.
        assert_eq!(
            unsafe { ink_engine_complete(call, answer.as_ptr()) },
            INK_OK
        );
    });
}

unsafe extern "C" fn async_release(ctx: *mut c_void) {
    // SAFETY: as above.
    let me = unsafe { &*(ctx as *const AsyncEngine) };
    me.released.fetch_add(1, Ordering::SeqCst);
}

fn table(engine: &AsyncEngine, info: &CString) -> InkEngineVTable {
    InkEngineVTable {
        size: std::mem::size_of::<InkEngineVTable>() as u32,
        kind: KIND_OFFLINE,
        info_json: info.as_ptr(),
        ctx: engine as *const AsyncEngine as *mut c_void,
        transcribe: Some(async_transcribe),
        cancel: None,
        release: Some(async_release),
    }
}

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

#[test]
fn every_function_in_the_header_works_through_the_c_abi() {
    let dir = TempDir::new("abi");
    let recorder = Recorder::new();
    let ctx = Arc::as_ptr(&recorder) as *mut c_void;
    let config = c(&serde_json::json!({
        "data_dir": dir.path().join("data"),
        "log_level": "debug",
        "log_stderr": false
    })
    .to_string());

    // SAFETY (every unsafe call below): arguments are NULL or valid NUL-terminated strings and
    // tables, and the recorder and engines outlive the core.
    unsafe {
        let mut bands = InkBands::default();
        assert_eq!(ink_command(c("{}").as_ptr()), INK_ERR_NOT_INITIALIZED);
        assert_eq!(ink_bands_read(&mut bands), INK_OK, "zeros before init");
        assert_eq!(
            ink_bands_read(std::ptr::null_mut()),
            INK_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            ink_init(std::ptr::null(), Some(on_event), ctx),
            INK_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            ink_init(config.as_ptr(), None, ctx),
            INK_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            ink_init(
                c(r#"{"data_dir":"relative"}"#).as_ptr(),
                Some(on_event),
                ctx
            ),
            INK_ERR_INVALID_ARGUMENT
        );
        assert_eq!(ink_init(config.as_ptr(), Some(on_event), ctx), INK_OK);
        assert_eq!(
            ink_init(config.as_ptr(), Some(on_event), ctx),
            INK_ERR_ALREADY_INITIALIZED
        );
        let ready = recorder.wait_type("core.ready", Duration::from_secs(5));
        assert_eq!(ready["abi"], 1, "INK_ABI_VERSION");

        // ink_init installed the core's logger: nobody else can.
        struct Other;
        impl log::Log for Other {
            fn enabled(&self, _: &log::Metadata<'_>) -> bool {
                true
            }
            fn log(&self, _: &log::Record<'_>) {}
            fn flush(&self) {}
        }
        assert!(log::set_logger(&Other).is_err());

        // Engines: malformed tables are refused and never released; a taken id is refused.
        let engine = AsyncEngine::default();
        let refused = AsyncEngine::default();
        let info =
            c(r#"{"id":"swiftish","licence":"MIT","jobs":[{"job":"meeting_final","wer":2.5}]}"#);
        assert_eq!(
            ink_register_engine(std::ptr::null()),
            INK_ERR_INVALID_ARGUMENT
        );
        let bad_info = c(r#"{"id":"x","jobs":[]}"#);
        assert_eq!(
            ink_register_engine(&table(&refused, &bad_info)),
            INK_ERR_INVALID_ARGUMENT
        );
        let mut small = table(&refused, &info);
        small.size = 8;
        assert_eq!(ink_register_engine(&small), INK_ERR_INVALID_ARGUMENT);
        let live_partials =
            c(r#"{"id":"odd","licence":"MIT","jobs":[{"job":"live_partials","wer":1}]}"#);
        assert_eq!(
            ink_register_engine(&table(&refused, &live_partials)),
            INK_ERR_FAILED
        );
        assert_eq!(ink_register_engine(&table(&engine, &info)), INK_OK);
        assert_eq!(
            ink_register_engine(&table(&refused, &info)),
            INK_ERR_FAILED,
            "id taken"
        );
        let registered = recorder.wait_type("engine.registered", Duration::from_secs(5));
        assert_eq!(registered["id"], "swiftish");
        assert_eq!(registered["jobs"][0]["job"], "meeting_final");

        // Commands: unreadable ones are refused at once; a replay becomes a record.
        assert_eq!(ink_command(std::ptr::null()), INK_ERR_INVALID_ARGUMENT);
        assert_eq!(
            ink_command(c("not json").as_ptr()),
            INK_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            ink_command(c(r#"{"cmd":"fly"}"#).as_ptr()),
            INK_ERR_INVALID_ARGUMENT
        );
        let (mic, far) = (dir.path().join("mic.wav"), dir.path().join("far.wav"));
        speech_wav(&mic, 2.5, 8);
        speech_wav(&far, 2.0, 9);
        let replay = c(&serde_json::json!({
            "cmd": "replay_meeting", "mic": mic, "far": far, "pacing": "fast", "id": "r1"
        })
        .to_string());
        assert_eq!(ink_command(replay.as_ptr()), INK_OK);
        let finished = recorder.wait_type("meeting.finished", Duration::from_secs(60));
        assert_eq!(finished["revision"], 2);
        assert!(
            engine.calls.load(Ordering::SeqCst) >= 2,
            "both sides went to the engine"
        );
        let threads = engine.threads.lock().unwrap().clone();
        assert!(
            threads.iter().all(|t| t.as_deref() == Some("ink-meeting")),
            "engine called off the worker: {threads:?}"
        );
        let finals: Vec<_> = recorder
            .all()
            .into_iter()
            .filter(|v| v["type"] == "meeting.transcribed")
            .collect();
        assert!(
            finals
                .iter()
                .all(|p| p["pass"]["word_count"].as_u64() > Some(0))
        );

        // The ink's bands were published while the meeting ran, and are copied out.
        assert_eq!(ink_bands_read(&mut bands), INK_OK);
        assert!(bands.published > 0, "{bands:?}");
        assert_eq!(
            (bands.low, bands.mid, bands.high),
            (0.0, 0.0, 0.0),
            "idle is still"
        );

        assert_eq!(
            ink_engine_complete(u64::MAX, c("{}").as_ptr()),
            INK_ERR_UNKNOWN_CALL
        );

        // Unregistering releases the engine once no call is in flight.
        let unregister = c(r#"{"cmd":"engine.unregister","engine":"swiftish"}"#);
        assert_eq!(ink_command(unregister.as_ptr()), INK_OK);
        recorder.wait_type("engine.unregistered", Duration::from_secs(5));
        assert_eq!(engine.released.load(Ordering::SeqCst), 1);

        // Shutdown: core.stopped is the last event, and nothing arrives after it returns.
        let again = AsyncEngine::default();
        let info2 =
            c(r#"{"id":"second","licence":"MIT","jobs":[{"job":"dictation_final","wer":3}]}"#);
        assert_eq!(ink_register_engine(&table(&again, &info2)), INK_OK);
        assert_eq!(ink_shutdown(), INK_OK);
        assert_eq!(
            again.released.load(Ordering::SeqCst),
            1,
            "released before shutdown returned"
        );
        assert_eq!(
            refused.released.load(Ordering::SeqCst),
            0,
            "refused tables are never released"
        );
        let count = recorder.all().len();
        assert_eq!(
            recorder.types().last().map(String::as_str),
            Some("core.stopped")
        );
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            recorder.all().len(),
            count,
            "an event after ink_shutdown returned"
        );
        assert_eq!(ink_shutdown(), INK_ERR_NOT_INITIALIZED);
        assert_eq!(ink_command(unregister.as_ptr()), INK_ERR_NOT_INITIALIZED);
        assert_eq!(
            ink_register_engine(&table(&again, &info2)),
            INK_ERR_NOT_INITIALIZED
        );

        // The core can start again in the same process, with the same bands reader.
        let started = Instant::now();
        assert_eq!(ink_init(config.as_ptr(), Some(on_event), ctx), INK_OK);
        assert!(recorder.wait_count("core.ready", 2, Duration::from_secs(5)));
        assert_eq!(ink_bands_read(&mut bands), INK_OK);
        assert!(bands.published > 0, "the count survives a restart");
        assert_eq!(ink_shutdown(), INK_OK);
        assert!(started.elapsed() < Duration::from_secs(10));
    }
    recorder.assert_valid();
}
