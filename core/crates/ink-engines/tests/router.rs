//! The router: job → best installed engine for this OS; shell-registered engines compete too.

mod common;

use std::sync::{Arc, Barrier, Mutex};

use common::{Scratch, install, row, row_with};
use ink_core::mock::MockEngine;
use ink_core::{
    CancelToken, Channel, EngineError, EngineInfo, Job, OfflineEngine, StreamingEngine, TimedText,
    TranscribeOptions, Transcript,
};
use ink_engines::{
    EngineRow, ExternalEngine, JobScore, ModelDir, Os, Registry, Route, RouteError, Router, Runtime,
};

fn router(scratch: &Scratch, rows: Vec<EngineRow>, os: Os) -> (Router, ModelDir) {
    let dir = scratch.model_dir();
    let reg = Registry::new(rows).unwrap();
    (Router::new(&reg, dir.clone(), os), dir)
}

fn model_id(route: Route) -> String {
    match route {
        Route::Model(row) => row.id.clone(),
        other @ Route::External { .. } => panic!("expected a registry model, got {other:?}"),
    }
}

fn scores(pairs: &[(Job, f32)]) -> Vec<JobScore> {
    pairs
        .iter()
        .map(|&(job, wer)| JobScore { job, wer })
        .collect()
}

#[test]
fn the_lowest_error_rate_installed_row_wins() {
    let s = Scratch::new("best");
    let rows = vec![
        row("synthetic-a", &[(Job::DictationFinal, 9.0)]),
        row("synthetic-b", &[(Job::DictationFinal, 6.5)]),
        row("synthetic-c", &[(Job::DictationFinal, 7.0)]),
    ];
    let (r, dir) = router(&s, rows.clone(), Os::MacOs);
    for row in &rows {
        install(&dir, row);
    }
    assert_eq!(
        model_id(r.route(Job::DictationFinal).unwrap()),
        "synthetic-b"
    );
}

#[test]
fn rows_that_are_not_installed_are_not_candidates() {
    let s = Scratch::new("uninstalled");
    let best = row("synthetic-best", &[(Job::MeetingFinal, 3.0)]);
    let ok = row("synthetic-ok", &[(Job::MeetingFinal, 8.0)]);
    let (r, dir) = router(&s, vec![best.clone(), ok.clone()], Os::Windows);
    install(&dir, &ok);
    assert_eq!(
        model_id(r.route(Job::MeetingFinal).unwrap()),
        "synthetic-ok"
    );

    // Installing the better one changes the answer with no restart: routing reads the disk.
    install(&dir, &best);
    assert_eq!(
        model_id(r.route(Job::MeetingFinal).unwrap()),
        "synthetic-best"
    );

    // A truncated file is not an installed model.
    let path = dir.file_path(&best, &best.files[0]);
    std::fs::write(&path, b"x").unwrap();
    assert_eq!(
        model_id(r.route(Job::MeetingFinal).unwrap()),
        "synthetic-ok"
    );
}

#[test]
fn rows_for_another_os_are_not_candidates() {
    let s = Scratch::new("os");
    let mac_only = row_with(
        "synthetic-mac",
        &[(Job::LivePartials, 4.0)],
        &[Os::MacOs],
        b"mac",
    );
    let win_only = row_with(
        "synthetic-win",
        &[(Job::LivePartials, 5.0)],
        &[Os::Windows],
        b"win",
    );
    for (os, want) in [(Os::MacOs, "synthetic-mac"), (Os::Windows, "synthetic-win")] {
        let (r, dir) = router(&s, vec![mac_only.clone(), win_only.clone()], os);
        install(&dir, &mac_only);
        install(&dir, &win_only);
        assert_eq!(model_id(r.route(Job::LivePartials).unwrap()), want);
    }
}

#[test]
fn error_rates_are_compared_per_job() {
    let s = Scratch::new("per-job");
    let dictation_best = row(
        "synthetic-d",
        &[(Job::DictationFinal, 5.0), (Job::MeetingFinal, 20.0)],
    );
    let meeting_best = row(
        "synthetic-m",
        &[(Job::DictationFinal, 6.0), (Job::MeetingFinal, 15.0)],
    );
    let (r, dir) = router(
        &s,
        vec![dictation_best.clone(), meeting_best.clone()],
        Os::MacOs,
    );
    install(&dir, &dictation_best);
    install(&dir, &meeting_best);
    assert_eq!(
        model_id(r.route(Job::DictationFinal).unwrap()),
        "synthetic-d"
    );
    assert_eq!(model_id(r.route(Job::MeetingFinal).unwrap()), "synthetic-m");
}

#[test]
fn no_installed_engine_is_an_explicit_error() {
    let s = Scratch::new("none");
    let (r, _dir) = router(
        &s,
        vec![row("synthetic-a", &[(Job::DictationFinal, 5.0)])],
        Os::MacOs,
    );
    let err = r.route(Job::DictationFinal).unwrap_err();
    assert_eq!(
        err,
        RouteError::NoEngine {
            job: Job::DictationFinal
        }
    );
    assert_eq!(
        err.to_string(),
        "no engine installed for job DictationFinal"
    );
    assert!(matches!(
        EngineError::from(err),
        EngineError::ModelMissing(_)
    ));
    // Nothing does diarization at all: same explicit error, not some other engine.
    assert_eq!(
        r.route(Job::Diarization).unwrap_err(),
        RouteError::NoEngine {
            job: Job::Diarization
        }
    );
}

#[test]
fn ties_break_deterministically() {
    let s = Scratch::new("tie");
    let rows = vec![
        row("synthetic-z", &[(Job::DictationFinal, 5.0)]),
        row("synthetic-a", &[(Job::DictationFinal, 5.0)]),
        row("synthetic-m", &[(Job::DictationFinal, 5.0)]),
    ];
    // Registry order must not matter: try it forwards and backwards.
    for order in [rows.clone(), rows.iter().rev().cloned().collect()] {
        let (r, dir) = router(&s, order.clone(), Os::MacOs);
        for row in &order {
            install(&dir, row);
        }
        for _ in 0..3 {
            assert_eq!(
                model_id(r.route(Job::DictationFinal).unwrap()),
                "synthetic-a"
            );
        }
    }

    // At an equal error rate a shell-registered engine wins: the shell registers the engines that
    // run on the platform's accelerators.
    let (r, dir) = router(&s, rows.clone(), Os::MacOs);
    install(&dir, &rows[1]);
    let ext = Arc::new(MockEngine::new("zz-shell-engine", &[Job::DictationFinal]));
    r.register_offline(ext, &scores(&[(Job::DictationFinal, 5.0)]))
        .unwrap();
    assert_eq!(
        r.route(Job::DictationFinal).unwrap().id(),
        "zz-shell-engine"
    );
}

#[test]
fn shell_engines_route_like_builtin_ones_and_unregister_cleanly() {
    let s = Scratch::new("external");
    let builtin = row("synthetic-builtin", &[(Job::LivePartials, 6.0)]);
    let (r, dir) = router(&s, vec![builtin.clone()], Os::MacOs);
    install(&dir, &builtin);

    // Worse than the installed row: not chosen.
    let worse = Arc::new(MockEngine::new("shell-worse", &[Job::LivePartials]));
    r.register_streaming(worse, &scores(&[(Job::LivePartials, 9.0)]))
        .unwrap();
    assert_eq!(
        r.route(Job::LivePartials).unwrap().id(),
        "synthetic-builtin"
    );

    // Better: chosen, as the streaming engine that was registered.
    let better = Arc::new(MockEngine::new("shell-better", &[Job::LivePartials]));
    r.register_streaming(better, &scores(&[(Job::LivePartials, 4.0)]))
        .unwrap();
    match r.route(Job::LivePartials).unwrap() {
        Route::External {
            engine: ExternalEngine::Streaming(e),
            ..
        } => {
            assert_eq!(StreamingEngine::info(&*e).id, "shell-better")
        }
        other => panic!("expected the shell's streaming engine, got {other:?}"),
    }

    assert!(r.unregister("shell-better"));
    assert!(!r.unregister("shell-better"));
    assert_eq!(
        r.route(Job::LivePartials).unwrap().id(),
        "synthetic-builtin"
    );

    // With the files gone and only the worse shell engine left, it is the answer.
    std::fs::remove_file(dir.file_path(&builtin, &builtin.files[0])).unwrap();
    assert_eq!(r.route(Job::LivePartials).unwrap().id(), "shell-worse");
    assert!(r.unregister("shell-worse"));
    assert_eq!(
        r.route(Job::LivePartials).unwrap_err(),
        RouteError::NoEngine {
            job: Job::LivePartials
        }
    );
}

#[test]
fn a_routed_shell_engine_is_the_one_that_answers() {
    let s = Scratch::new("answers");
    let (r, _dir) = router(&s, vec![], Os::MacOs);
    let audio = [0.25f32, -0.25, 0.5, -0.5];
    let transcript = Transcript {
        segments: vec![TimedText {
            start_ms: 0,
            end_ms: 1_000,
            text: "synthetic words".into(),
        }],
    };
    let engine = MockEngine::new("shell-offline", &[Job::DictationFinal])
        .with_fixture(&audio, transcript.clone());
    r.register_offline(
        Arc::new(engine.clone()),
        &scores(&[(Job::DictationFinal, 3.0)]),
    )
    .unwrap();

    let Route::External {
        engine: ExternalEngine::Offline(chosen),
        ..
    } = r.route(Job::DictationFinal).unwrap()
    else {
        panic!("expected the shell's offline engine");
    };
    let options = TranscribeOptions {
        channel: Channel::Mic,
        context: None,
        cancel: CancelToken::new(),
        live: false,
    };
    assert_eq!(chosen.transcribe(&audio, &options).unwrap(), transcript);
    assert_eq!(engine.calls().len(), 1);
}

#[test]
fn registrations_are_validated() {
    let s = Scratch::new("validate");
    let taken = row("synthetic-taken", &[(Job::DictationFinal, 5.0)]);
    let (r, _dir) = router(&s, vec![taken], Os::MacOs);
    let offline =
        |id: &str, jobs: &[Job]| -> Arc<dyn OfflineEngine> { Arc::new(MockEngine::new(id, jobs)) };

    // An id a registry row already uses.
    assert_eq!(
        r.register_offline(
            offline("synthetic-taken", &[Job::DictationFinal]),
            &scores(&[(Job::DictationFinal, 1.0)])
        ),
        Err(RouteError::AlreadyRegistered {
            id: "synthetic-taken".into()
        })
    );

    // A second registration under one id.
    r.register_offline(
        offline("shell-a", &[Job::DictationFinal]),
        &scores(&[(Job::DictationFinal, 1.0)]),
    )
    .unwrap();
    assert_eq!(
        r.register_offline(
            offline("shell-a", &[Job::DictationFinal]),
            &scores(&[(Job::DictationFinal, 1.0)])
        ),
        Err(RouteError::AlreadyRegistered {
            id: "shell-a".into()
        })
    );

    let invalid = |result: Result<(), RouteError>| {
        assert!(
            matches!(result, Err(RouteError::Invalid { .. })),
            "{result:?}"
        )
    };
    // A job with no score, and a score for a job the engine does not list.
    invalid(r.register_offline(
        offline("shell-b", &[Job::DictationFinal, Job::MeetingFinal]),
        &scores(&[(Job::DictationFinal, 1.0)]),
    ));
    invalid(r.register_offline(
        offline("shell-c", &[Job::DictationFinal]),
        &scores(&[(Job::DictationFinal, 1.0), (Job::MeetingFinal, 1.0)]),
    ));
    // An offline engine claiming live partials, a streaming one claiming a final pass, anyone
    // claiming diarization (the shell registers no diarizer).
    invalid(r.register_offline(
        offline("shell-d", &[Job::LivePartials]),
        &scores(&[(Job::LivePartials, 1.0)]),
    ));
    invalid(r.register_streaming(
        Arc::new(MockEngine::new("shell-e", &[Job::MeetingFinal])),
        &scores(&[(Job::MeetingFinal, 1.0)]),
    ));
    invalid(r.register_offline(
        offline("shell-f", &[Job::Diarization]),
        &scores(&[(Job::Diarization, 1.0)]),
    ));
    // No jobs, an empty id, a non-finite error rate.
    invalid(r.register_offline(offline("shell-g", &[]), &[]));
    invalid(r.register_offline(
        offline("", &[Job::DictationFinal]),
        &scores(&[(Job::DictationFinal, 1.0)]),
    ));
    invalid(r.register_offline(
        offline("shell-h", &[Job::DictationFinal]),
        &scores(&[(Job::DictationFinal, f32::NAN)]),
    ));

    // None of the refused ones got in.
    assert_eq!(r.route(Job::DictationFinal).unwrap().id(), "shell-a");
    assert_eq!(
        r.route(Job::LivePartials).unwrap_err(),
        RouteError::NoEngine {
            job: Job::LivePartials
        }
    );
}

#[test]
fn the_router_is_send_and_sync_and_usable_from_many_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Router>();

    let s = Scratch::new("threads");
    let builtin = row("synthetic-builtin", &[(Job::DictationFinal, 6.0)]);
    let (r, dir) = router(&s, vec![builtin.clone()], Os::Windows);
    install(&dir, &builtin);
    let r = Arc::new(r);
    let handles: Vec<_> = (0..8)
        .map(|t| {
            let r = Arc::clone(&r);
            std::thread::spawn(move || {
                let id = format!("shell-{t}");
                for _ in 0..50 {
                    r.register_offline(
                        Arc::new(MockEngine::new(&id, &[Job::DictationFinal])),
                        &scores(&[(Job::DictationFinal, 7.0 + t as f32)]),
                    )
                    .unwrap();
                    // Every shell engine is worse than the installed row.
                    assert_eq!(
                        r.route(Job::DictationFinal).unwrap().id(),
                        "synthetic-builtin"
                    );
                    assert!(r.unregister(&id));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
}

/// An engine whose reported id can change after it is registered, as a shell engine's could.
struct RenamingEngine {
    id: Mutex<String>,
}

impl OfflineEngine for RenamingEngine {
    fn info(&self) -> EngineInfo {
        EngineInfo {
            id: self.id.lock().unwrap().clone(),
            jobs: vec![Job::DictationFinal],
            licence: "MIT".into(),
        }
    }

    fn transcribe(
        &self,
        _audio: &[f32],
        _options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError> {
        Err(EngineError::Unsupported("synthetic engine"))
    }
}

#[test]
fn a_route_carries_the_id_the_engine_was_registered_under() {
    let s = Scratch::new("renamed");
    let (r, _dir) = router(&s, vec![], Os::MacOs);
    let engine = Arc::new(RenamingEngine {
        id: Mutex::new("shell-v1".into()),
    });
    r.register_offline(engine.clone(), &scores(&[(Job::DictationFinal, 3.0)]))
        .unwrap();
    *engine.id.lock().unwrap() = "shell-v2".into();

    let route = r.route(Job::DictationFinal).unwrap();
    assert_eq!(route.id(), "shell-v1");
    // So unregistering by the route's id removes what was routed.
    assert!(r.unregister(route.id()));
    assert_eq!(
        r.route(Job::DictationFinal).unwrap_err(),
        RouteError::NoEngine {
            job: Job::DictationFinal
        }
    );
}

#[test]
fn racing_registrations_of_one_id_admit_exactly_one() {
    const THREADS: usize = 16;
    let s = Scratch::new("race");
    let (r, _dir) = router(&s, vec![], Os::Windows);
    let r = Arc::new(r);
    let start = Arc::new(Barrier::new(THREADS));
    let handles: Vec<_> = (0..THREADS)
        .map(|t| {
            let (r, start) = (Arc::clone(&r), Arc::clone(&start));
            std::thread::spawn(move || {
                start.wait();
                r.register_offline(
                    Arc::new(MockEngine::new("shell-contested", &[Job::MeetingFinal])),
                    &scores(&[(Job::MeetingFinal, t as f32)]),
                )
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "{results:?}"
    );
    assert!(results.iter().all(|r| r.is_ok()
        || *r
            == Err(RouteError::AlreadyRegistered {
                id: "shell-contested".into()
            })));
    assert_eq!(r.route(Job::MeetingFinal).unwrap().id(), "shell-contested");
    assert!(r.unregister("shell-contested"));
    assert!(!r.unregister("shell-contested"));
}

/// A model that dictates quickly on the CPU (sherpa-onnx), competing with a more accurate one that
/// takes seconds there (llama.cpp), both installed.
fn quick_and_slow(s: &Scratch) -> (Registry, ModelDir) {
    let slow = row(
        "synthetic-llama",
        &[(Job::DictationFinal, 4.6), (Job::MeetingFinal, 16.1)],
    );
    let quick = EngineRow {
        runtime: Runtime::SherpaOnnx,
        kind: ink_engines::RowKind::Speech,
        ..row(
            "synthetic-sherpa",
            &[(Job::DictationFinal, 16.4), (Job::LivePartials, 27.9)],
        )
    };
    let dir = s.model_dir();
    install(&dir, &slow);
    install(&dir, &quick);
    (Registry::new(vec![slow, quick]).unwrap(), dir)
}

#[test]
fn with_a_gpu_the_lowest_error_rate_dictates() {
    let s = Scratch::new("gpu");
    let (reg, dir) = quick_and_slow(&s);
    for r in [
        Router::new(&reg, dir.clone(), Os::Windows),
        Router::new(&reg, dir.clone(), Os::Windows).with_gpu_probe(|| true),
    ] {
        assert_eq!(
            model_id(r.route(Job::DictationFinal).unwrap()),
            "synthetic-llama"
        );
    }
}

#[test]
fn without_a_gpu_dictation_goes_to_the_engine_quick_on_the_cpu() {
    let s = Scratch::new("no-gpu");
    let (reg, dir) = quick_and_slow(&s);
    let asked = Arc::new(Mutex::new(0));
    let probe = {
        let asked = asked.clone();
        move || {
            *asked.lock().unwrap() += 1;
            false
        }
    };
    let r = Router::new(&reg, dir.clone(), Os::Windows).with_gpu_probe(probe);
    assert_eq!(*asked.lock().unwrap(), 0, "not asked until it matters");
    // Other jobs are not asked about: the meeting's final pass keeps the most accurate model.
    assert_eq!(
        model_id(r.route(Job::MeetingFinal).unwrap()),
        "synthetic-llama"
    );
    assert_eq!(
        model_id(r.route(Job::LivePartials).unwrap()),
        "synthetic-sherpa"
    );
    assert_eq!(*asked.lock().unwrap(), 0);
    for _ in 0..3 {
        assert_eq!(
            model_id(r.route(Job::DictationFinal).unwrap()),
            "synthetic-sherpa"
        );
    }
    assert_eq!(*asked.lock().unwrap(), 1, "asked once");
}

#[test]
fn without_a_gpu_the_slow_engine_still_dictates_when_it_is_the_only_one() {
    let s = Scratch::new("no-gpu-only");
    let (reg, dir) = quick_and_slow(&s);
    let quick = reg.get("synthetic-sherpa").unwrap().clone();
    std::fs::remove_file(dir.marker_path(&quick)).unwrap();
    let r = Router::new(&reg, dir, Os::Windows).with_gpu_probe(|| false);
    assert_eq!(
        model_id(r.route(Job::DictationFinal).unwrap()),
        "synthetic-llama",
        "slow is better than nothing"
    );
}

/// The probe may start a runtime (llama.cpp's backend and its devices), so a row that is not
/// installed never makes it run: not when a quick engine dictates, nor when nothing is installed.
#[test]
fn a_slow_row_that_is_not_installed_never_asks_the_gpu_probe() {
    let s = Scratch::new("no-gpu-uninstalled");
    let (reg, dir) = quick_and_slow(&s);
    let slow = reg.get("synthetic-llama").unwrap().clone();
    std::fs::remove_file(dir.marker_path(&slow)).unwrap();
    let asked = Arc::new(Mutex::new(0));
    let probe = {
        let asked = asked.clone();
        move || {
            *asked.lock().unwrap() += 1;
            false
        }
    };
    let r = Router::new(&reg, dir.clone(), Os::Windows).with_gpu_probe(probe);
    assert_eq!(
        model_id(r.route(Job::DictationFinal).unwrap()),
        "synthetic-sherpa"
    );
    let quick = reg.get("synthetic-sherpa").unwrap().clone();
    std::fs::remove_file(dir.marker_path(&quick)).unwrap();
    assert_eq!(
        r.route(Job::DictationFinal).unwrap_err(),
        RouteError::NoEngine {
            job: Job::DictationFinal
        }
    );
    assert_eq!(*asked.lock().unwrap(), 0, "never asked");
}

#[test]
fn without_a_gpu_a_shell_engine_still_competes_on_its_error_rate() {
    let s = Scratch::new("no-gpu-shell");
    let (reg, dir) = quick_and_slow(&s);
    let r = Router::new(&reg, dir, Os::Windows).with_gpu_probe(|| false);
    let shell = Arc::new(MockEngine::new("shell-dictation", &[Job::DictationFinal]));
    r.register_offline(shell, &scores(&[(Job::DictationFinal, 10.0)]))
        .unwrap();
    assert_eq!(
        r.route(Job::DictationFinal).unwrap().id(),
        "shell-dictation"
    );
}

#[test]
fn an_installed_row_the_shell_runs_is_never_routed() {
    // A Core ML row fills no job: installed, it is still no candidate for any, and a shell engine
    // serves as before.
    let s = Scratch::new("core-ml");
    let mut core_ml = row("synthetic-core-ml", &[]);
    core_ml.runtime = Runtime::CoreMl;
    let (r, dir) = router(&s, vec![core_ml.clone()], Os::MacOs);
    install(&dir, &core_ml);
    assert!(dir.is_installed(&core_ml));
    for job in [
        Job::DictationFinal,
        Job::MeetingFinal,
        Job::LivePartials,
        Job::Diarization,
        Job::VoiceActivity,
    ] {
        assert_eq!(r.route(job).unwrap_err(), RouteError::NoEngine { job });
    }
}

#[test]
fn a_language_row_installed_never_serves_a_speech_job() {
    let s = Scratch::new("language");
    let mut chat = row("synthetic-chat", &[]);
    chat.files[0].name = "chat.gguf".into();
    chat.files[0].url = chat.files[0].url.replace("weights.bin", "chat.gguf");
    chat.kind = ink_engines::RowKind::Language(ink_engines::LanguageRow {
        name: "Synthetic Chat".into(),
        chat: ink_engines::ChatQuirks::default(),
    });
    let speech = row("synthetic-asr", &[(Job::DictationFinal, 9.0)]);
    let (r, dir) = router(&s, vec![chat.clone(), speech.clone()], Os::Windows);
    install(&dir, &chat);
    for job in [
        Job::DictationFinal,
        Job::MeetingFinal,
        Job::LivePartials,
        Job::Diarization,
        Job::VoiceActivity,
    ] {
        assert!(
            r.route(job).is_err(),
            "{job:?} routed with only the chat model installed"
        );
    }
    install(&dir, &speech);
    assert_eq!(
        model_id(r.route(Job::DictationFinal).unwrap()),
        "synthetic-asr"
    );
}
