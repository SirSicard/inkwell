//! What allocates per block, measured.
//!
//! Nothing in this crate runs on the realtime thread or the pump (see the crate docs), so none of
//! this is I4. It pins two facts the threading table states: GCC-PHAT's per-window work and the
//! canceller's own per-frame work (the resample, the buffers, the latency line) allocate nothing
//! after construction, and AEC3 itself allocates on every frame, which is why the canceller
//! stays on a worker.

use std::hint::black_box;

use assert_no_alloc::{AllocDisabler, assert_no_alloc, violation_count};
use ink_echo::aec::Aec3;
use ink_echo::gcc::{GccParams, GccPhat};
use ink_echo::{Alignment, CancellerConfig, EchoCanceller, FRAME};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn allocations_in(work: impl FnOnce()) -> u32 {
    let before = violation_count();
    assert_no_alloc(work);
    violation_count() - before
}

fn signal(n: usize, seed: u32) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let t = i as f32 + seed as f32 * 101.0;
            ((t * 0.013).sin() + 0.5 * (t * 0.171).sin() + 0.2 * (t * 0.77).sin()) * 0.1
        })
        .collect()
}

#[test]
fn control_the_guard_fires_in_this_binary() {
    assert_eq!(allocations_in(|| {}), 0);
    assert_eq!(
        allocations_in(|| {
            black_box(Vec::<f32>::with_capacity(8));
        }),
        2
    );
}

#[test]
fn a_gcc_phat_window_allocates_nothing() {
    let mut g = GccPhat::new(GccParams::default());
    let w = g.window_len();
    let far = signal(w, 1);
    let mut mic = vec![0.0f32; w];
    mic[500..].copy_from_slice(&far[..w - 500]);
    let _ = g.analyze(&mic, &far, 1.0);
    let mut est = None;
    assert_eq!(allocations_in(|| est = Some(g.analyze(&mic, &far, 2.0))), 0);
    assert!(est.is_some_and(|e| e.candidate && (e.lag - 500.0).abs() < 0.5));
}

/// AEC3 allocates inside every frame; the canceller adds nothing of its own on top.
#[test]
fn the_canceller_allocates_only_what_aec3_does() {
    let frames = 400;
    let far = signal(frames * FRAME, 2);
    let mic: Vec<f32> = (0..far.len())
        .map(|i| if i >= 736 { 0.5 * far[i - 736] } else { 0.0 })
        .collect();

    // The bare AEC3, past its start-up, frame by frame.
    let mut aec = Aec3::new();
    let (mut lin, mut full) = ([0.0f32; FRAME], [0.0f32; FRAME]);
    let frame = |x: &[f32], f: usize| -> [f32; FRAME] {
        x[f * FRAME..(f + 1) * FRAME].try_into().expect("frame")
    };
    for f in 0..200 {
        aec.process(&frame(&far, f), &frame(&mic, f), &mut lin, &mut full);
    }
    let mut bare = 0;
    for f in 200..frames {
        let (fa, mi) = (frame(&far, f), frame(&mic, f));
        bare += allocations_in(|| aec.process(&fa, &mi, &mut lin, &mut full));
    }
    assert!(
        bare > 0,
        "AEC3 no longer allocates per frame: move the canceller's docs on"
    );

    // The canceller over the same stretch, pushed a frame at a time. Its frames lag one behind
    // (AEC3's latency), so the pushes of frames 200..400 run AEC3 on those same 200 frames.
    let mut c = EchoCanceller::new(
        Alignment::from_ms_ppm(46.0, 0.0),
        CancellerConfig::default(),
    )
    .expect("canceller");
    // On a +46 ms path the reference reads behind the mic, so with the far end pushed a little
    // ahead, every mic push completes exactly one frame.
    let lead = 2 * FRAME;
    c.push(&[], &far[..lead], |_| {}).expect("push");
    let mut emitted = 0usize;
    for f in 0..200 {
        let (m, fa) = (
            &mic[f * FRAME..(f + 1) * FRAME],
            &far[lead + f * FRAME..lead + (f + 1) * FRAME],
        );
        c.push(m, fa, |_| emitted += 1).expect("push");
    }
    let mut wrapped = 0;
    for f in 200..frames - 2 {
        let m = &mic[f * FRAME..(f + 1) * FRAME];
        let fa = &far[lead + f * FRAME..lead + (f + 1) * FRAME];
        wrapped += allocations_in(|| {
            c.push(m, fa, |fr| {
                black_box(fr);
            })
            .expect("push");
        });
    }
    let per_frame_bare = f64::from(bare) / 200.0;
    let per_frame_wrapped = f64::from(wrapped) / f64::from((frames - 202) as u32);
    eprintln!("allocations per frame: AEC3 {per_frame_bare:.2}, canceller {per_frame_wrapped:.2}");
    assert!(emitted > 190, "{emitted}");
    assert!(
        (per_frame_wrapped - per_frame_bare).abs() < 0.05,
        "the canceller adds allocations of its own: {per_frame_wrapped:.2} per frame against AEC3's \
         {per_frame_bare:.2}"
    );
}
