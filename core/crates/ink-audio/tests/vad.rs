//! VAD: trim the ends only, never the pauses inside the speech.
//!
//! The model binding arrives later; here a scripted source stands in for it and returns one
//! probability per 512-sample window, as Silero does.

use ink_audio::vad::{
    Segment, SpeechProbability, SpeechSegmenter, VAD_WINDOW, VadConfig, trim_ends,
};
use ink_core::EngineError;

/// Returns the scripted probabilities in order (0.0 once they run out) and records what it saw.
struct Scripted {
    probs: Vec<f32>,
    next: usize,
    resets: usize,
    first_samples: Vec<f32>,
}

impl Scripted {
    fn new(probs: Vec<f32>) -> Self {
        Self {
            probs,
            next: 0,
            resets: 0,
            first_samples: Vec::new(),
        }
    }
}

impl SpeechProbability for Scripted {
    fn reset(&mut self) {
        self.next = 0;
        self.resets += 1;
    }

    fn probability(&mut self, window: &[f32; VAD_WINDOW]) -> Result<f32, EngineError> {
        self.first_samples.push(window[0]);
        let p = self.probs.get(self.next).copied().unwrap_or(0.0);
        self.next += 1;
        Ok(p)
    }
}

/// Probabilities from runs of (windows, probability).
fn script(runs: &[(usize, f32)]) -> Vec<f32> {
    runs.iter()
        .flat_map(|&(n, p)| std::iter::repeat_n(p, n))
        .collect()
}

fn audio_for(windows: usize) -> Vec<f32> {
    vec![0.0; windows * VAD_WINDOW]
}

const PAD: usize = 4_000; // the default edge pad, 250 ms

#[test]
fn vad_trims_the_ends_and_keeps_pauses_inside_the_speech() {
    // Silence, speech, a 2 s pause, speech, silence. Only the outer silence goes: cutting the
    // pause out would splice unrelated phonemes together, and leave the long-audio chunker no
    // quiet place to cut.
    let probs = script(&[(40, 0.0), (30, 0.9), (62, 0.02), (30, 0.9), (40, 0.0)]);
    let windows = probs.len();
    let audio = audio_for(windows);
    let mut source = Scripted::new(probs);
    let range = trim_ends(&audio, &mut source, &VadConfig::default())
        .unwrap()
        .expect("speech");
    assert_eq!(range.start, 40 * VAD_WINDOW - PAD);
    assert_eq!(range.end, (40 + 30 + 62 + 30) * VAD_WINDOW + PAD);
    // The pause is inside the range, whole.
    assert!(range.start < 70 * VAD_WINDOW && range.end > 132 * VAD_WINDOW);
}

#[test]
fn no_speech_at_all_is_none() {
    let audio = audio_for(50);
    let mut source = Scripted::new(vec![0.1; 50]);
    assert_eq!(
        trim_ends(&audio, &mut source, &VadConfig::default()).unwrap(),
        None
    );
    assert_eq!(
        trim_ends(&[], &mut source, &VadConfig::default()).unwrap(),
        None
    );
}

#[test]
fn the_threshold_decides_the_onset() {
    let cfg = VadConfig::default();
    let below = script(&[(10, 0.0), (20, cfg.threshold - 0.01), (10, 0.0)]);
    let mut source = Scripted::new(below);
    assert_eq!(trim_ends(&audio_for(40), &mut source, &cfg).unwrap(), None);

    let at = script(&[(10, 0.0), (20, cfg.threshold), (10, 0.0)]);
    let mut source = Scripted::new(at);
    let range = trim_ends(&audio_for(40), &mut source, &cfg)
        .unwrap()
        .unwrap();
    assert_eq!(range, 10 * VAD_WINDOW - PAD..30 * VAD_WINDOW + PAD);
}

#[test]
fn hysteresis_keeps_speech_going_between_the_two_thresholds() {
    // Once speech has started, a probability between the lower and upper threshold (a soft
    // consonant, a trailing vowel) keeps it going; only one under the lower threshold ends it.
    let cfg = VadConfig::default();
    let between = (cfg.threshold + cfg.neg_threshold) / 2.0;
    let mut seg = SpeechSegmenter::new(cfg);
    let probs = script(&[(5, 0.0), (10, 0.9), (30, between), (20, 0.0)]);
    let segments = segments_of(&mut seg, &probs);
    assert_eq!(segments, vec![Segment { start: 5, end: 45 }]);
}

/// Every segment the segmenter emits for `probs`, then at the end.
fn segments_of(seg: &mut SpeechSegmenter, probs: &[f32]) -> Vec<Segment> {
    let mut out: Vec<Segment> = probs.iter().filter_map(|&p| seg.push(p)).collect();
    out.extend(seg.finish());
    out
}

#[test]
fn the_hangover_bridges_a_short_dip() {
    // A dip shorter than the hangover (160 ms against 256 ms) does not split the speech.
    let cfg = VadConfig::default();
    assert!(5 < cfg.hangover_windows);
    let mut seg = SpeechSegmenter::new(cfg);
    let probs = script(&[(3, 0.0), (10, 0.9), (5, 0.1), (10, 0.9), (20, 0.0)]);
    assert_eq!(
        segments_of(&mut seg, &probs),
        vec![Segment { start: 3, end: 28 }]
    );
}

#[test]
fn speech_ends_where_the_silence_began_not_where_the_hangover_ran_out() {
    let cfg = VadConfig::default();
    let mut seg = SpeechSegmenter::new(cfg);
    let probs = script(&[(3, 0.0), (10, 0.9), (30, 0.0)]);
    let mut emitted_at = None;
    for (i, &p) in probs.iter().enumerate() {
        if let Some(s) = seg.push(p) {
            assert_eq!(s, Segment { start: 3, end: 13 });
            emitted_at = Some(i);
        }
    }
    // Known only once the hangover has run out.
    assert_eq!(emitted_at, Some(13 + cfg.hangover_windows - 1));
    assert_eq!(seg.finish(), None);
}

#[test]
fn a_single_window_spike_is_not_speech() {
    // A key click can score one window as speech. It must not pull the start a second early.
    let probs = script(&[(5, 0.0), (1, 0.95), (40, 0.0), (20, 0.9), (10, 0.0)]);
    let windows = probs.len();
    let mut source = Scripted::new(probs);
    let range = trim_ends(&audio_for(windows), &mut source, &VadConfig::default())
        .unwrap()
        .unwrap();
    assert_eq!(range.start, 46 * VAD_WINDOW - PAD);
}

#[test]
fn a_short_word_before_a_pause_is_kept() {
    // "So ... [pause] ... the rest." Three windows (96 ms) of speech, a second of silence, then the
    // sentence. A 250 ms minimum-speech rule would drop "so" and trim it off.
    let probs = script(&[(10, 0.0), (3, 0.9), (31, 0.0), (40, 0.9), (10, 0.0)]);
    let windows = probs.len();
    let mut source = Scripted::new(probs);
    let range = trim_ends(&audio_for(windows), &mut source, &VadConfig::default())
        .unwrap()
        .unwrap();
    assert_eq!(range.start, 10 * VAD_WINDOW - PAD);
}

#[test]
fn the_pad_is_clamped_and_the_last_partial_window_is_zero_padded() {
    // Speech from the first window to the last, in a buffer that is not a whole number of
    // windows: the range is the whole buffer, never past it.
    let len = 20 * VAD_WINDOW + 100;
    let audio: Vec<f32> = (0..len).map(|i| i as f32).collect();
    let mut source = Scripted::new(vec![0.9; 21]);
    let range = trim_ends(&audio, &mut source, &VadConfig::default())
        .unwrap()
        .unwrap();
    assert_eq!(range, 0..len);
    // The source saw 21 windows of 512, each starting where it should.
    assert_eq!(source.first_samples.len(), 21);
    for (k, &first) in source.first_samples.iter().enumerate() {
        assert_eq!(first, (k * VAD_WINDOW) as f32);
    }
}

#[test]
fn the_source_is_reset_first_and_its_errors_propagate() {
    let mut source = Scripted::new(script(&[(10, 0.9)]));
    trim_ends(&audio_for(10), &mut source, &VadConfig::default()).unwrap();
    trim_ends(&audio_for(10), &mut source, &VadConfig::default()).unwrap();
    assert_eq!(
        source.resets, 2,
        "each trim starts the model's state afresh"
    );

    struct Failing;
    impl SpeechProbability for Failing {
        fn reset(&mut self) {}
        fn probability(&mut self, _: &[f32; VAD_WINDOW]) -> Result<f32, EngineError> {
            Err(EngineError::ModelMissing("vad".into()))
        }
    }
    assert_eq!(
        trim_ends(&audio_for(3), &mut Failing, &VadConfig::default()),
        Err(EngineError::ModelMissing("vad".into()))
    );
}

#[test]
fn a_probability_outside_0_to_1_is_an_error_not_a_guess() {
    for bad in [f32::NAN, -0.1, 1.5] {
        let mut source = Scripted::new(vec![0.9, bad, 0.9]);
        let result = trim_ends(&audio_for(3), &mut source, &VadConfig::default());
        assert!(
            matches!(result, Err(EngineError::Failed(_))),
            "{bad}: {result:?}"
        );
    }
}

#[test]
fn the_segmenter_emits_each_segment_as_it_closes() {
    // For the live path: segments come out as soon as their hangover has run.
    let mut seg = SpeechSegmenter::new(VadConfig::default());
    let probs = script(&[(2, 0.0), (6, 0.9), (20, 0.0), (6, 0.9), (3, 0.0)]);
    assert_eq!(
        segments_of(&mut seg, &probs),
        vec![Segment { start: 2, end: 8 }, Segment { start: 28, end: 34 }]
    );
    assert_eq!(
        Segment { start: 2, end: 8 }.samples(),
        2 * VAD_WINDOW..8 * VAD_WINDOW
    );
}

#[test]
fn windows_are_silero_sized() {
    assert_eq!(VAD_WINDOW, 512, "32 ms at 16 kHz, Silero's window");
}
