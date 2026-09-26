//! The echo-only word gate on the CI echo fixture, fed by the canceller's own frames.
//!
//! No model runs in CI, so the voice activity of the full output comes from a stand-in: an oracle
//! that reports speech in the 32 ms windows where the near-end talker is (the fixture knows). That
//! is what a working VAD on the full output reports; how well Silero does it on real audio is for
//! the local replay with the engines. This test checks everything around it: the far end's
//! activity read from the canceller's reference, and the word, frame and window timelines lining
//! up.
//!
//! Words are stand-ins too, 300 ms spans: the "leaked" ones where the far end talks alone and the
//! linear output still carries residual echo (what its transcript would attribute to "you"), and
//! the user's own where the near end talks, alone or over the far end.

mod common;

use common::speakers_run;
use ink_core::TimedText;
use ink_echo::measure::frame_db;
use ink_echo::{EchoGate, FRAME, GateConfig, Verdict};

const VAD_WINDOW: usize = 512;

fn word(start_ms: u64) -> TimedText {
    TimedText {
        start_ms,
        end_ms: start_ms + 300,
        text: String::from("word"),
    }
}

#[test]
fn leaked_echo_words_are_dropped_and_the_users_words_kept() {
    let run = speakers_run();
    let (s, out) = (&run.scene, &run.out);
    let mut gate = EchoGate::new(GateConfig::default(), VAD_WINDOW);

    // Replay the frames into the gate in order, as the pipeline would while cancelling.
    for (i, chunk) in out.reference.chunks(FRAME).enumerate() {
        let mut f = ink_echo::EchoFrame {
            index: i as u64,
            len: chunk.len(),
            mic: [0.0; FRAME],
            reference: [0.0; FRAME],
            linear: [0.0; FRAME],
            full: [0.0; FRAME],
        };
        f.reference[..chunk.len()].copy_from_slice(chunk);
        gate.push_frame(&f);
    }
    // The oracle VAD over the full output's windows: speech where the near end talks.
    for (i, w) in s.near.chunks(VAD_WINDOW).enumerate() {
        let level = frame_db(w).into_iter().fold(f64::MIN, f64::max);
        gate.push_speech(i as u64, if level > -60.0 { 0.95 } else { 0.02 })
            .expect("a probability");
    }

    let near_db = frame_db(&s.near);
    let lin_db = frame_db(&out.linear);
    let ref_db = frame_db(&out.reference);
    let frames_of = |w: &TimedText| (w.start_ms / 10) as usize..(w.end_ms / 10) as usize;
    let (mut leaked, mut own, mut quiet) = (Vec::new(), Vec::new(), Vec::new());
    let mut t = 0;
    while t + 300 < (common::TOTAL_S * 1000.0) as u64 {
        let w = word(t);
        let r = frames_of(&w);
        // The near end is silent from 500 ms before to 500 ms after (so no pad reaches it).
        let near_quiet = near_db[r.start.saturating_sub(50)..(r.end + 50).min(near_db.len())]
            .iter()
            .all(|d| *d < -60.0);
        let near_talks = near_db[r.clone()].iter().filter(|d| **d > -50.0).count() >= 10;
        // The far end plays through most of it (a word straddling the far end's first onset is
        // mostly not echo, and the gate keeps it: rule 1).
        let far_talks = ref_db[r.clone()].iter().filter(|d| **d > -50.0).count() >= 20;
        let residue = lin_db[r.clone()].iter().any(|d| *d > -60.0);
        if near_quiet && far_talks && residue {
            leaked.push(w);
        } else if near_talks {
            own.push(w);
        } else if near_quiet
            && ref_db[r.start.saturating_sub(40)..r.end]
                .iter()
                .all(|d| *d < -90.0)
        {
            quiet.push(w);
        }
        t += 300;
    }
    assert!(
        leaked.len() > 30 && own.len() > 30 && quiet.len() >= 3,
        "too few words to judge: {} leaked, {} own, {} quiet",
        leaked.len(),
        own.len(),
        quiet.len()
    );

    let dropped = |ws: &[TimedText]| gate.verdicts(ws).iter().filter(|v| !v.keep()).count();
    let (d_leaked, d_own) = (dropped(&leaked), dropped(&own));
    eprintln!(
        "leaked echo words dropped {d_leaked}/{}; the user's words dropped {d_own}/{}",
        leaked.len(),
        own.len()
    );
    assert_eq!(d_leaked, leaked.len(), "a leaked echo word survived");
    assert_eq!(d_own, 0, "one of the user's words was dropped");
    // Where the far end is silent nothing is echo, whatever the VAD says.
    assert!(gate.verdicts(&quiet).iter().all(|v| *v == Verdict::NoEcho));
}
