//! The echo fixture: the committed AMI excerpt's far end (`fixtures/ami/IS1009a-far.wav`, three
//! remote headsets) played through the gate's seeded room into the mic, late and on a drifting
//! clock (+75 ms, +50 ppm: laptop speakers), over the excerpt's own near-end talker
//! (`IS1009a-mic.wav`). The 30 s excerpt is played twice, so there is far-end speech well past
//! the first 10 s.
//!
//! Both sides go through capture, the pump and the chunks as a meeting's do; then the final
//! pass's echo stage runs over them ([`echo::replay`]: the path fitted over the whole recording,
//! AEC3 adapted on its start and run along it), and its output is measured against the truth:
//! the near end and the echo are known here, so the ERLE is the gate's (the mic's energy over the
//! full output's where only the far end plays), not the chain's own monitor. The chain's final
//! pass runs over the same recording too.
//!
//! The near end is a real headset recording, so it carries crosstalk: the other talkers, some
//! 20 dB down. That is echo AEC3 is not given a path for, and it stays in the output.

mod echo_rig;
mod meeting_rig;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use echo_rig::{Scene, Spec};
use ink_core::CancelToken;
use ink_echo::measure::{active, dilate, erle_db, frame_db, percentile};
use ink_echo::{EchoFrame, PathReport};
use ink_pipeline::meeting::{MeetingOutcome, echo};
use meeting_rig::*;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/ami")
}

/// A 16 kHz mono 16-bit fixture, as f64, played `times` times end to end.
fn read(file: &str, times: usize) -> Vec<f64> {
    let mut reader = hound::WavReader::open(fixtures().join(file)).expect("the AMI fixture");
    let spec = reader.spec();
    assert_eq!((spec.sample_rate, spec.channels), (16_000, 1));
    let once: Vec<f64> = reader
        .samples::<i16>()
        .map(|s| f64::from(s.expect("a sample")) / 32_768.0)
        .collect();
    once.repeat(times)
}

/// The canceller's streams, whole, on the mic's timeline.
#[derive(Default)]
struct Out {
    mic: Vec<f32>,
    reference: Vec<f32>,
    full: Vec<f32>,
}

impl Out {
    fn take(&mut self, f: &EchoFrame) {
        assert_eq!(
            f.index as usize * 160,
            self.mic.len(),
            "frames in order, none missing"
        );
        self.mic.extend_from_slice(&f.mic[..f.len]);
        self.reference.extend_from_slice(&f.reference[..f.len]);
        self.full.extend_from_slice(&f.full[..f.len]);
    }
}

struct Run {
    scene: Scene,
    report: PathReport,
    out: Out,
    outcome: MeetingOutcome,
}

fn run() -> &'static Run {
    static RUN: OnceLock<Run> = OnceLock::new();
    RUN.get_or_init(|| {
        let scene = echo_rig::mix(
            read("IS1009a-far.wav", 2),
            read("IS1009a-mic.wav", 2),
            Spec::speakers(),
        );
        let mut rig = RigBuilder {
            vad: vad_source(|_| Box::new(EnergyVad(-40.0))),
            no_live_engine: true,
            ..Default::default()
        }
        .build();
        rig.feed(&scene.mic, &scene.far);
        let ended = rig.stop();
        let mut out = Out::default();
        let report = echo::replay(&rig.chunks, T0_NS, &CancelToken::new(), |f| out.take(f))
            .expect("the replay")
            .expect("both sides were recorded");
        let outcome = ended
            .finalize(&rig.chunks, &CancelToken::new())
            .expect("the final pass");
        Run {
            scene,
            report,
            out,
            outcome,
        }
    })
}

#[test]
fn the_echo_path_is_found_within_2_ms_and_5_ppm() {
    let report = &run().report;
    let path = report.path.unwrap_or_else(|| panic!("{report}"));
    assert!((path.delay_ms() - 75.0).abs() <= 2.0, "{report}");
    assert!((path.drift_ppm() - 50.0).abs() <= 5.0, "{report}");
}

#[test]
fn erle_is_at_least_20_db_after_the_first_10_s_of_far_end_speech() {
    let Run { scene, out, .. } = run();
    let n = out.mic.len().min(scene.near.len());
    assert!(n >= 59 * 16_000, "the whole minute came out: {n}");
    // Where the far end plays (the reference, as the canceller lined it up), and where the near
    // end talks: 10 dB over its own floor, widened by 50 ms either side (the truth, known here).
    let far = active(&frame_db(&out.reference[..n]), -60.0);
    let near_db = frame_db(&scene.near[..n]);
    let floor = percentile(&near_db, 0.1);
    let near = dilate(&active(&near_db, floor + 10.0), 5, 5);
    let far_only: Vec<bool> = far.iter().zip(&near).map(|(f, t)| *f && !*t).collect();
    // The first 10 s of far-end speech, and the rest.
    let mut seen = 0usize;
    let first: Vec<bool> = far
        .iter()
        .map(|f| {
            seen += usize::from(*f);
            seen <= 1_000
        })
        .collect();
    let mask = |in_first: bool| -> Vec<bool> {
        far_only
            .iter()
            .zip(&first)
            .map(|(f, e)| *f && *e == in_first)
            .collect()
    };
    let (early, late) = (mask(true), mask(false));
    let count = |m: &[bool]| m.iter().filter(|x| **x).count();
    let before = erle_db(&out.mic[..n], &out.full[..n], &early);
    let after = erle_db(&out.mic[..n], &out.full[..n], &late);
    eprintln!(
        "AMI echo fixture: ERLE {before:.1} dB over the first 10 s of far-end speech ({} far-only \
         frames), {after:.1} dB after ({} frames)",
        count(&early),
        count(&late)
    );
    assert!(
        count(&late) >= 500,
        "too little far-end-only audio to judge"
    );
    assert!(after >= 20.0, "ERLE after the first 10 s: {after:.1} dB");
}

#[test]
fn the_final_pass_cancels_the_fixture_and_hears_the_near_end() {
    let outcome = &run().outcome;
    let echo = &outcome.echo;
    assert!(echo.cancelled, "{echo:?}");
    let path = echo.path.expect("a path");
    assert!((path.delay_ms - 75.0).abs() <= 2.0, "{echo:?}");
    assert!(
        outcome.mic.regions > 0 && outcome.mic.word_count > 0,
        "{:?}",
        outcome.mic
    );
    eprintln!(
        "AMI echo fixture, final pass: {echo:?}\n  mic {:?}",
        outcome.mic
    );
}
