//! The router: job → best installed engine for this OS; shell-registered engines compete too.

mod common;

use std::sync::Arc;

use common::{Scratch, install, row, row_with};
use ink_core::mock::MockEngine;
use ink_core::{
    CancelToken, Channel, EngineError, Job, OfflineEngine, StreamingEngine, TimedText,
    TranscribeOptions, Transcript,
};
use ink_engines::{
    EngineRow, ExternalEngine, JobScore, ModelDir, Os, Registry, Route, RouteError, Router,
};

fn router(scratch: &Scratch, rows: Vec<EngineRow>, os: Os) -> (Router, ModelDir) {
    let dir = scratch.model_dir();
    let reg = Registry::new(rows).unwrap();
    (Router::new(&reg, dir.clone(), os), dir)
}

fn model_id(route: Route) -> String {
    match route {
        Route::Model(row) => row.id.clone(),
        Route::External(e) => panic!("expected a registry model, got {e:?}"),
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
        Route::External(ExternalEngine::Streaming(e)) => {
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

    let Route::External(ExternalEngine::Offline(chosen)) = r.route(Job::DictationFinal).unwrap()
    else {
        panic!("expected the shell's offline engine");
    };
    let options = TranscribeOptions {
        channel: Channel::Mic,
        context: None,
        cancel: CancelToken::new(),
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
