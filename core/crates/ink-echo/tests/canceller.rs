//! The canceller as a stream: its frames line up with the mic, its reference follows the path,
//! and how the two streams are chunked changes nothing.

mod common;

use common::{Outputs, SceneSpec, cancel, find_path, scene};
use ink_core::Channel;
use ink_echo::{Alignment, CancellerConfig, EchoCanceller, EchoError};

fn run(mic: &[f32], far: &[f32], path: Alignment, chunk: (usize, usize)) -> Outputs {
    let mut c = EchoCanceller::new(path, CancellerConfig::default()).expect("canceller");
    let mut out = Outputs::default();
    // The two sizes swap every push, so neither stream runs away from the other.
    let (mut i, mut j, mut turn) = (0, 0, 0);
    while i < mic.len() || j < far.len() {
        let (a, b) = if turn % 2 == 0 {
            chunk
        } else {
            (chunk.1, chunk.0)
        };
        turn += 1;
        let m = &mic[i..(i + a).min(mic.len())];
        let f = &far[j..(j + b).min(far.len())];
        i += m.len();
        j += f.len();
        c.push(m, f, |fr| out.take(fr)).expect("in step");
    }
    c.finish(|fr| out.take(fr)).expect("finish");
    out
}

#[test]
fn with_no_far_end_the_outputs_are_the_mic_sample_for_sample() {
    // AEC3's own latencies are taken out: a click comes out where it went in, on both outputs.
    let mut mic = vec![0.0f32; 16_000];
    mic[5_037] = 0.5;
    mic[12_000] = -0.25;
    let far = vec![0.0f32; 16_000];
    let out = run(&mic, &far, Alignment::from_ms_ppm(20.0, 0.0), (333, 333));
    assert_eq!(out.mic, mic);
    assert_eq!(out.frames, (0..100).collect::<Vec<u64>>());
    for (name, y) in [("linear", &out.linear), ("full", &out.full)] {
        assert_eq!(y.len(), mic.len());
        for &at in &[5_037usize, 12_000] {
            assert!((y[at] - mic[at]).abs() < 0.01, "{name} at {at}: {}", y[at]);
        }
        let stray = y
            .iter()
            .enumerate()
            .filter(|(i, v)| ![5_037, 12_000].contains(i) && v.abs() > 0.01)
            .count();
        assert_eq!(stray, 0, "{name} has energy away from the clicks");
    }
}

#[test]
fn the_reference_leads_the_echo_by_the_margin() {
    // The mic hears the far end 30 ms late: sample k of the reference is far[k + 10 ms − 30 ms].
    let far: Vec<f32> = (0..32_000)
        .map(|i| (i as f32 * 0.013).sin() * 0.3 + (i as f32 * 0.071).sin() * 0.1)
        .collect();
    let mic = vec![0.0f32; far.len()];
    let out = run(&mic, &far, Alignment::from_ms_ppm(30.0, 0.0), (160, 160));
    let shift = 480 - 160;
    for k in [1_000usize, 7_777, 20_000, 31_000] {
        let want = far[k - shift];
        assert!(
            (out.reference[k] - want).abs() < 1e-3,
            "reference[{k}] = {}, far[{}] = {want}",
            out.reference[k],
            k - shift
        );
    }
}

#[test]
fn a_mic_that_leads_gets_a_reference_from_ahead_in_the_far_stream() {
    // The far-end stream arrives 50 ms late: the reference reads 60 ms ahead of the mic.
    let far: Vec<f32> = (0..32_000)
        .map(|i| (i as f32 * 0.021).sin() * 0.3)
        .collect();
    let mic = vec![0.0f32; far.len()];
    let out = run(&mic, &far, Alignment::from_ms_ppm(-50.0, 0.0), (160, 160));
    for k in [1_000usize, 15_000, 30_000] {
        assert!((out.reference[k] - far[k + 960]).abs() < 1e-3, "at {k}");
    }
}

#[test]
fn how_the_streams_are_chunked_changes_nothing() {
    let s = scene(SceneSpec::speakers());
    let path = find_path(&s).path.expect("a path");
    let (mic, far) = (&s.mic[..16_000 * 12], &s.far[..16_000 * 12]);
    let a = run(mic, far, path, (160, 160));
    let b = run(mic, far, path, (1_601, 97));
    let c = cancel(mic, far, path);
    for other in [&b, &c] {
        assert_eq!(a.frames, other.frames);
        assert_eq!(a.reference, other.reference);
        assert_eq!(a.linear, other.linear);
        assert_eq!(a.full, other.full);
    }
}

#[test]
fn the_last_partial_frame_comes_out_with_its_length() {
    let mic = vec![0.01f32; 1_000];
    let far = vec![0.0f32; 1_000];
    let mut c = EchoCanceller::new(Alignment::from_ms_ppm(0.0, 0.0), CancellerConfig::default())
        .expect("canceller");
    let mut lens = Vec::new();
    c.push(&mic, &far, |f| lens.push((f.index, f.len)))
        .expect("push");
    let total = c.finish(|f| lens.push((f.index, f.len))).expect("finish");
    assert_eq!(total, 7);
    assert_eq!(lens.last(), Some(&(6, 40)));
    assert_eq!(lens.len(), 7);
}

#[test]
fn a_stream_running_too_far_ahead_is_refused_whole() {
    let config = CancellerConfig {
        max_skew_s: 1.0,
        ..CancellerConfig::default()
    };
    let mut c = EchoCanceller::new(Alignment::from_ms_ppm(46.0, 2.0), config).expect("canceller");
    let second = vec![0.0f32; 16_000];
    c.push(&[], &second, |_| {}).expect("one second ahead fits");
    assert_eq!(
        c.push(&[], &second, |_| {}),
        Err(EchoError::Backlog {
            ahead: Channel::Far
        })
    );
    // The mic catching up drains it.
    let mut frames = 0;
    c.push(&second, &[], |_| frames += 1).expect("catch up");
    assert!(frames > 90, "{frames}");
    c.push(&[], &second, |_| {}).expect("room again");
}

#[test]
fn a_finished_canceller_refuses_more() {
    let mut c = EchoCanceller::new(Alignment::from_ms_ppm(0.0, 0.0), CancellerConfig::default())
        .expect("canceller");
    c.finish(|_| {}).expect("finish");
    assert_eq!(c.push(&[0.0; 10], &[], |_| {}), Err(EchoError::Ended));
    assert_eq!(c.finish(|_| {}), Err(EchoError::Ended));
}

#[test]
fn an_impossible_alignment_is_refused() {
    let ok = CancellerConfig::default();
    for bad in [
        Alignment::from_ms_ppm(f64::NAN, 0.0),
        Alignment::from_ms_ppm(20_000.0, 0.0),
        Alignment::from_ms_ppm(10.0, 50_000.0),
        Alignment::from_ms_ppm(10.0, f64::INFINITY),
    ] {
        assert!(matches!(
            EchoCanceller::new(bad, ok),
            Err(EchoError::BadAlignment)
        ));
    }
    let bad_margin = CancellerConfig {
        margin_ms: -1.0,
        ..ok
    };
    assert!(EchoCanceller::new(Alignment::from_ms_ppm(0.0, 0.0), bad_margin).is_err());
}
