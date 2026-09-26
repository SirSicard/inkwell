//! Resampling to 16 kHz: the offline tail pad, and the streaming resampler's state across pushes.
//!
//! Tolerances are stated per test. The sinc kernel runs SIMD code chosen at runtime (NEON, AVX,
//! SSE), so numbers can differ in the last bits between CPUs; nothing here compares bit for bit
//! across machines.

use ink_audio::gain::{rms, to_dbfs};
use ink_audio::resample::{ResampleError, StreamResampler, resample, resampled_len};
use ink_audio::synth::tone;

const RATES: [u32; 8] = [
    8_000, 11_025, 22_050, 24_000, 32_000, 44_100, 48_000, 96_000,
];

#[test]
fn offline_output_length_matches_the_ratio() {
    for rate in RATES {
        // An odd length, so rounding is exercised.
        let n = rate as usize + 1_237;
        let input = tone(n as f64 / f64::from(rate), 440.0, 0.5, rate);
        assert_eq!(input.len(), n);
        let out = resample(&input, rate).unwrap();
        let expected = ((n as u64 * 16_000) as f64 / f64::from(rate)).round() as usize;
        assert_eq!(out.len(), expected, "{rate} Hz");
        assert_eq!(resampled_len(n as u64, rate), expected as u64, "{rate} Hz");
    }
}

#[test]
fn offline_resampler_keeps_the_last_samples() {
    // The sinc filter delays its output by half its kernel and holds that much input back. Without
    // a flush pad the last few milliseconds of every take (the end of the last word) were lost.
    // One second of silence, then a tone that runs to the very last input sample.
    for rate in [44_100u32, 48_000] {
        let mut input = vec![0.0f32; rate as usize];
        input.extend(tone(0.06, 1_000.0, 0.5, rate));
        let out = resample(&input, rate).unwrap();
        assert_eq!(out.len() as u64, resampled_len(input.len() as u64, rate));
        // The final 5 ms of output carry the tone at its level (0.5 peak = −9 dBFS RMS). Only the
        // kernel's edge at the very end softens it, by well under the 1.5 dB allowed.
        let tail = &out[out.len() - 80..];
        let level = to_dbfs(rms(tail));
        let want = to_dbfs(0.5 / 2f32.sqrt());
        assert!(
            (level - want).abs() < 1.5,
            "{rate} Hz: the end of the signal came out at {level:.2} dBFS, want {want:.2}"
        );
        // And the silence before it stays silent: the tail is not smeared backwards.
        assert!(rms(&out[..15_000]) < 1e-4, "{rate} Hz");
    }
}

#[test]
fn a_tone_comes_through_aligned_and_at_level() {
    // 1 kHz resampled from 48 kHz and 44.1 kHz must match a 1 kHz tone generated at 16 kHz,
    // sample for sample: the filter delay is trimmed exactly, not to the nearest output sample (a
    // third of a sample late is a 0.4 rad phase error at 1 kHz, a 20 % miss). Allowed: 1 % of full
    // scale away from the ends.
    for rate in [44_100u32, 48_000, 96_000, 22_050] {
        let input = tone(1.0, 1_000.0, 0.5, rate);
        let out = resample(&input, rate).unwrap();
        let ideal = tone(1.0, 1_000.0, 0.5, 16_000);
        let worst = out[800..15_200]
            .iter()
            .zip(&ideal[800..15_200])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 0.01, "{rate} Hz: worst error {worst}");
    }
}

#[test]
fn a_click_lands_at_the_same_time_after_resampling() {
    // Nothing is trimmed from the head: rubato's sinc resampler holds its lookahead back at the
    // end rather than delaying the start, so trimming its reported delay from the head (as an
    // earlier implementation did) threw away the first ~2.7 ms and moved every event early.
    for rate in [44_100u32, 48_000, 8_000, 96_000] {
        let mut input = vec![0.0f32; rate as usize];
        let click_at = rate as usize / 2; // 0.5 s
        input[click_at] = 1.0;
        let out = resample(&input, rate).unwrap();
        let peak = out
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .map(|(i, _)| i)
            .unwrap();
        assert!(
            (peak as i64 - 8_000).abs() <= 1,
            "{rate} Hz: a click at 0.5 s came out at sample {peak}, want 8000"
        );
    }
}

#[test]
fn a_tone_above_8_khz_does_not_alias_into_the_output() {
    // 10 kHz at 48 kHz would fold to 6 kHz without the anti-alias filter.
    let input = tone(0.5, 10_000.0, 0.5, 48_000);
    let out = resample(&input, 48_000).unwrap();
    let level = to_dbfs(rms(&out[400..7_600]));
    assert!(level < -60.0, "aliased energy at {level:.1} dBFS");
}

#[test]
fn sixteen_khz_passes_through_untouched() {
    let input = tone(0.3, 440.0, 0.5, 16_000);
    assert_eq!(resample(&input, 16_000).unwrap(), input);
    let mut stream = StreamResampler::new(16_000).unwrap();
    let mut out = Vec::new();
    stream.push(&input[..1_001], &mut out).unwrap();
    assert_eq!(out, input[..1_001], "no delay at the canonical rate");
    stream.push(&input[1_001..], &mut out).unwrap();
    stream.finish(&mut out).unwrap();
    assert_eq!(out, input);
}

/// Feeds `input` in pushes whose sizes cycle through `sizes`, then finishes.
fn stream_in_pushes(input: &[f32], rate: u32, sizes: &[usize]) -> Vec<f32> {
    let mut stream = StreamResampler::new(rate).unwrap();
    let mut out = Vec::new();
    let mut at = 0;
    let mut k = 0;
    while at < input.len() {
        let n = sizes[k % sizes.len()].min(input.len() - at);
        stream.push(&input[at..at + n], &mut out).unwrap();
        at += n;
        k += 1;
    }
    stream.finish(&mut out).unwrap();
    out
}

#[test]
fn streaming_in_odd_pushes_equals_feeding_it_whole() {
    // The resampler is built once per device stream and carries its filter history, its fractional
    // position and any partial input chunk across pushes, so how the pump happens to slice the
    // audio cannot change the signal. Allowed: 1e-6 absolute (in practice the chunking is
    // identical and so are the samples).
    for rate in [44_100u32, 48_000, 8_000] {
        let input: Vec<f32> = tone(1.3, 440.0, 0.4, rate)
            .iter()
            .zip(tone(1.3, 3_100.0, 0.3, rate))
            .map(|(a, b)| a + b)
            .collect();
        let whole = stream_in_pushes(&input, rate, &[input.len()]);
        let odd = stream_in_pushes(&input, rate, &[1, 7, 480, 1_023, 3, 4_096, 13]);
        assert_eq!(whole.len(), odd.len(), "{rate} Hz");
        let worst = whole
            .iter()
            .zip(&odd)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            worst <= 1e-6,
            "{rate} Hz: pushes changed the output by {worst}"
        );
        // And it is the offline resampler's output.
        let offline = resample(&input, rate).unwrap();
        assert_eq!(offline.len(), whole.len());
        let worst = offline
            .iter()
            .zip(&whole)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            worst <= 1e-6,
            "{rate} Hz: streaming and offline differ by {worst}"
        );
    }
}

#[test]
fn streaming_output_keeps_up_with_its_input() {
    // Live partials need the audio promptly: after any push, what is still held back is bounded by
    // one input chunk (20 ms) plus the kernel's lookahead, whatever the push sizes, and the bound
    // the resampler reports is one it keeps.
    for rate in [48_000u32, 44_100, 11_025, 8_000] {
        let input = tone(2.0, 440.0, 0.4, rate);
        let mut stream = StreamResampler::new(rate).unwrap();
        let bound = stream.max_held_back_frames();
        assert!(
            bound > 0 && bound <= 16_000 / 25,
            "{rate} Hz: held back up to {bound} frames (40 ms)"
        );
        if rate == 48_000 {
            assert!(bound <= 23 * 16, "48 kHz: {bound} frames, over 23 ms");
        }
        let mut out = Vec::new();
        let mut fed = 0u64;
        let mut worst = 0;
        for chunk in input.chunks(333) {
            stream.push(chunk, &mut out).unwrap();
            fed += chunk.len() as u64;
            let due = resampled_len(fed, rate);
            assert!(out.len() as u64 <= due, "{rate} Hz: ran ahead of its input");
            worst = worst.max(due - out.len() as u64);
        }
        assert!(
            worst <= bound as u64,
            "{rate} Hz: held back {worst} frames, bound {bound}"
        );
    }
}

#[test]
fn invalid_rates_are_refused() {
    assert!(matches!(
        StreamResampler::new(0),
        Err(ResampleError::InvalidRate(0))
    ));
    assert!(matches!(
        resample(&[0.0; 10], 0),
        Err(ResampleError::InvalidRate(0))
    ));
    assert!(StreamResampler::new(1_000_000).is_err(), "above 384 kHz");
}

#[test]
fn an_empty_stream_finishes_empty() {
    let mut stream = StreamResampler::new(48_000).unwrap();
    let mut out = Vec::new();
    stream.finish(&mut out).unwrap();
    assert!(out.is_empty());
    assert!(resample(&[], 44_100).unwrap().is_empty());
}

#[test]
fn finish_leaves_the_resampler_ready_for_the_same_devices_next_stream() {
    // Built once per device: after a stream ends, the next one through the same resampler comes
    // out exactly as through a fresh one.
    for rate in [44_100u32, 48_000, 11_025] {
        let first = tone(0.7, 300.0, 0.4, rate);
        let second = tone(0.9, 2_000.0, 0.3, rate);
        let mut reused = StreamResampler::new(rate).unwrap();
        let mut out = Vec::new();
        reused.push(&first, &mut out).unwrap();
        reused.finish(&mut out).unwrap();
        assert_eq!(out, resample(&first, rate).unwrap(), "{rate} Hz first");
        out.clear();
        reused.push(&second, &mut out).unwrap();
        reused.finish(&mut out).unwrap();
        assert_eq!(out, resample(&second, rate).unwrap(), "{rate} Hz second");
    }
}

#[test]
fn resampled_len_rounds_an_exact_half_up() {
    // Exact ties: 3 frames at 96 kHz last exactly half a 16 kHz sample, and a half rounds up.
    assert_eq!(resampled_len(3, 96_000), 1);
    assert_eq!(resampled_len(9, 96_000), 2); // 1.5
    assert_eq!(resampled_len(1, 32_000), 1); // 0.5
    assert_eq!(resampled_len(3, 32_000), 2); // 1.5
    // Just either side of a tie.
    assert_eq!(resampled_len(2, 96_000), 0); // 0.33
    assert_eq!(resampled_len(4, 96_000), 1); // 0.67
    // And the resampler's output has exactly that length.
    assert_eq!(resample(&[0.1; 3], 96_000).unwrap().len(), 1);
    assert_eq!(resample(&[0.1; 3], 32_000).unwrap().len(), 2);
}
