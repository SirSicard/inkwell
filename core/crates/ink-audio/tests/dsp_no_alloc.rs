//! The DSP that runs per block allocates its buffers once, up front.
//!
//! None of this runs on the realtime thread (I4 covers that path); it runs on the pump, where an
//! allocation per block would still be a steady drip of heap traffic for the length of a meeting.
//! Each stage is built outside the guard, then driven inside it, from its very first call. The
//! control test proves the guard fires in this binary.

use std::hint::black_box;

use assert_no_alloc::{AllocDisabler, assert_no_alloc, violation_count};
use ink_audio::bands::{BandAnalyzer, bands_channel};
use ink_audio::resample::StreamResampler;
use ink_audio::synth::speech_like;
use ink_audio::take::TakeRecorder;
use ink_audio::vad::{SpeechProbability, VAD_WINDOW, VadConfig, trim_ends};
use ink_audio::{Agc, Downmix};
use ink_core::EngineError;

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

/// Runs `work` under the no-alloc guard; returns the (de)allocations it made.
fn allocations_in(work: impl FnOnce()) -> u32 {
    let before = violation_count();
    assert_no_alloc(work);
    violation_count() - before
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
fn streaming_resampler_pushes_and_finish_allocate_nothing() {
    for rate in [48_000u32, 44_100, 8_000] {
        let input = speech_like(2.0, -30.0, 1);
        let mut stream = StreamResampler::new(rate).unwrap();
        let mut out = Vec::with_capacity(64_000);
        for block in input.chunks(441) {
            assert_eq!(
                allocations_in(|| stream.push(block, &mut out).unwrap()),
                0,
                "{rate} Hz push"
            );
        }
        assert_eq!(
            allocations_in(|| stream.finish(&mut out).unwrap()),
            0,
            "{rate} Hz finish"
        );
    }
}

#[test]
fn agc_process_and_flush_allocate_nothing() {
    let mut audio = speech_like(3.0, -60.0, 2);
    let mut agc = Agc::without_vad();
    for block in audio.chunks_mut(160) {
        assert_eq!(allocations_in(|| agc.process(block)), 0);
    }
    let mut tail = Vec::with_capacity(Agc::LATENCY);
    assert_eq!(allocations_in(|| agc.flush(&mut tail)), 0);
}

#[test]
fn agc_with_a_vad_allocates_nothing_but_what_its_vad_does() {
    // The VAD-gated AGC with a VAD that allocates nothing itself: the gate, its verdicts and the
    // frames waiting for them are all allocated up front.
    struct Speech;
    impl SpeechProbability for Speech {
        fn reset(&mut self) {}
        fn probability(&mut self, _: &[f32; VAD_WINDOW]) -> Result<f32, EngineError> {
            Ok(0.9)
        }
    }
    let mut audio = speech_like(3.0, -60.0, 7);
    let mut agc = Agc::with_vad(Box::new(Speech), VadConfig::default());
    for block in audio.chunks_mut(160) {
        assert_eq!(allocations_in(|| agc.process(block)), 0);
    }
    let mut tail = Vec::with_capacity(Agc::LATENCY);
    assert_eq!(allocations_in(|| agc.flush(&mut tail)), 0);
    assert!(agc.gain() > 1.0, "it learned");
}

#[test]
fn bands_analysis_publish_and_read_allocate_nothing() {
    let audio = speech_like(1.0, -20.0, 3);
    let mut analyzer = BandAnalyzer::new();
    let (mut writer, reader) = bands_channel();
    for block in audio.chunks(160) {
        assert_eq!(
            allocations_in(|| {
                if let Some(b) = analyzer.process(block) {
                    writer.publish(b);
                }
            }),
            0
        );
        assert_eq!(
            allocations_in(|| {
                black_box(reader.read());
            }),
            0
        );
    }
    assert!(reader.read().published > 0);
}

#[test]
fn downmix_allocates_nothing_with_room() {
    let stereo = speech_like(0.5, -20.0, 4);
    let mut out = Vec::with_capacity(stereo.len());
    for how in [Downmix::Primary, Downmix::Average] {
        out.clear();
        assert_eq!(allocations_in(|| how.apply(&stereo, 2, &mut out)), 0);
    }
}

#[test]
fn take_recorder_pushes_allocate_nothing() {
    let audio = speech_like(2.0, -30.0, 5);
    let mut rec = TakeRecorder::new();
    for block in audio[..16_000].chunks(160) {
        assert_eq!(allocations_in(|| drop(rec.push(block))), 0, "idle");
    }
    // Opening a take allocates its buffer, once.
    assert!(rec.press(16_000));
    for block in audio[16_000..].chunks(160) {
        assert_eq!(allocations_in(|| drop(rec.push(block))), 0, "recording");
    }
}

#[test]
fn vad_trim_allocates_nothing() {
    struct Energy;
    impl SpeechProbability for Energy {
        fn reset(&mut self) {}
        fn probability(&mut self, w: &[f32; VAD_WINDOW]) -> Result<f32, EngineError> {
            Ok(if w.iter().any(|s| s.abs() > 0.01) {
                0.9
            } else {
                0.0
            })
        }
    }
    let audio = speech_like(2.0, -20.0, 6);
    let mut source = Energy;
    let cfg = VadConfig::default();
    assert_eq!(
        allocations_in(|| {
            black_box(trim_ends(&audio, &mut source, &cfg).unwrap());
        }),
        0
    );
}
