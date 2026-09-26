//! The CI echo fixture: seeded synthetic speech through a seeded synthetic room, on a drifting
//! clock (see `common`). Required: the path is found within ±2 ms and ±5 ppm, ERLE is at least
//! 20 dB after the first 10 s of far-end speech (the first 10 s reported), and with earbuds there
//! is no path and no cancellation.

mod common;

use common::{Scene, SceneSpec, find_path, scene, speakers_run};
use ink_echo::measure::{active, dilate, erle_db, frame_db, projection_gain_db};
use ink_echo::{CancellerConfig, EchoCanceller};

/// Frames where the far end plays and the near end is silent, from the truth.
fn far_only(s: &Scene, reference: &[f32]) -> Vec<bool> {
    let far_act = active(&frame_db(reference), -70.0);
    let near_act = dilate(&active(&frame_db(&s.near), -70.0), 5, 5);
    far_act
        .iter()
        .zip(&near_act)
        .map(|(f, n)| *f && !*n)
        .collect()
}

fn check_alignment(spec: SceneSpec, report: &ink_echo::PathReport) {
    let path = report.path.unwrap_or_else(|| panic!("no path: {report}"));
    let expect_ms = spec.delay_ms * (1.0 + spec.drift_ppm * 1e-6);
    let d_err = path.delay_ms() - expect_ms;
    let p_err = path.drift_ppm() - spec.drift_ppm;
    eprintln!("{report}; error {d_err:+.3} ms, {p_err:+.2} ppm");
    assert!(d_err.abs() <= 2.0, "delay error {d_err:+.3} ms");
    assert!(p_err.abs() <= 5.0, "drift error {p_err:+.2} ppm");
}

#[test]
fn the_echo_path_is_found_within_2_ms_and_5_ppm() {
    check_alignment(SceneSpec::speakers(), &speakers_run().report);
}

#[test]
fn a_mic_that_leads_the_far_end_is_aligned_too() {
    let spec = SceneSpec::mic_leads();
    check_alignment(spec, &find_path(&scene(spec)));
}

#[test]
fn erle_is_at_least_20_db_after_the_first_10_s_of_far_end_speech() {
    let run = speakers_run();
    let (s, out) = (&run.scene, &run.out);
    assert_eq!(out.mic, s.mic, "the mic passes through untouched");
    let fo = far_only(s, &out.reference);
    let first_far = active(&frame_db(&out.reference), -70.0)
        .iter()
        .position(|a| *a)
        .expect("far end plays");
    let cut = first_far + 1_000; // 10 s of 10 ms frames
    let early: Vec<bool> = fo.iter().enumerate().map(|(f, o)| *o && f < cut).collect();
    let late: Vec<bool> = fo.iter().enumerate().map(|(f, o)| *o && f >= cut).collect();
    let (e_early, e_late) = (
        erle_db(&s.mic, &out.full, &early),
        erle_db(&s.mic, &out.full, &late),
    );
    let (whole, linear) = (
        erle_db(&s.mic, &out.full, &fo),
        erle_db(&s.mic, &out.linear, &late),
    );
    eprintln!(
        "ERLE far-only: first 10 s {e_early:.1} dB, after {e_late:.1} dB, whole {whole:.1} dB; \
         linear stage after 10 s {linear:.1} dB ({} + {} frames)",
        early.iter().filter(|x| **x).count(),
        late.iter().filter(|x| **x).count()
    );
    assert!(
        late.iter().filter(|x| **x).count() > 500,
        "too few frames to judge"
    );
    assert!(e_late >= 20.0, "ERLE after 10 s {e_late:.1} dB");
}

#[test]
fn the_linear_output_keeps_the_near_end_in_double_talk() {
    let run = speakers_run();
    let (s, out) = (&run.scene, &run.out);
    let far_act = active(&frame_db(&out.reference), -70.0);
    let near_act = active(&frame_db(&s.near), -70.0);
    let dt: Vec<bool> = far_act
        .iter()
        .zip(&near_act)
        .map(|(f, n)| *f && *n)
        .collect();
    let lin = projection_gain_db(&out.linear, &s.near, &dt);
    let full = projection_gain_db(&out.full, &s.near, &dt);
    eprintln!("near end in double talk: linear {lin:+.2} dB, full {full:+.2} dB");
    assert!(
        lin > -1.0,
        "the linear output lost {lin:.2} dB of the near end"
    );
}

#[test]
fn earbuds_give_no_echo_path_and_no_cancellation() {
    let s = scene(SceneSpec::earbuds());
    let report = find_path(&s);
    eprintln!("{report}");
    assert!(report.path.is_none(), "{report}");
    let canceller = EchoCanceller::when_found(&report, CancellerConfig::default()).expect("valid");
    assert!(canceller.is_none(), "no path, so no AEC");
    // The same rule with echo present does build one.
    assert!(
        EchoCanceller::when_found(&speakers_run().report, CancellerConfig::default())
            .expect("valid")
            .is_some()
    );
}
