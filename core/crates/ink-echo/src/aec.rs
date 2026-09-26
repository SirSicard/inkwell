//! AEC3, driven one 10 ms frame at a time, with both of its outputs.
//!
//! The canceller is the `aec3` crate's port of WebRTC's AEC3, driven directly rather than through
//! that crate's processing graph: the same calls in the same order as its own AEC3 node (render:
//! load, split, analyse; capture: load, analyse, split, process with the linear output, merge,
//! store), without the graph's per-frame packets and queues. A test proves the two give the same
//! samples. Only the echo canceller runs: no high-pass, noise suppression, AGC or post filter, as
//! in S0.3's measurements.

use aec3::api::{EchoCanceller3Config, EchoControl};
use aec3::audio_processing::aec3::echo_canceller3::EchoCanceller3;
use aec3::audio_processing::audio_buffer::AudioBuffer;
use aec3::audio_processing::stream_config::StreamConfig;

use crate::{FRAME, RATE};

/// AEC3's main (and shadow) adaptive filter length, in 4 ms blocks: its default. S0.3 tried
/// 26, 40 and 64 on a real room, and every longer filter cancelled less after convergence
/// (24.3 dB at 13 against 23.6, 19.1 and 18.4 dB).
pub const FILTER_BLOCKS: usize = 13;

/// Samples by which the full (suppressed) output lags the capture input: AEC3's frame-to-block
/// framing. Measured once and pinned by a test (`the_output_latencies_are_the_pinned_constants`).
pub const FULL_LATENCY: usize = 128;
/// Samples by which the linear output lags the capture input. Pinned the same way.
pub const LINEAR_LATENCY: usize = 64;

const _: () = assert!(FULL_LATENCY <= FRAME && LINEAR_LATENCY <= FRAME);

/// AEC3 at 16 kHz mono with its linear output exported: the bare canceller, with no alignment.
/// [`EchoCanceller`](crate::EchoCanceller) wraps it with the path and the latencies; this is
/// public for measurement.
///
/// **Worker.** Every [`process`](Self::process) allocates inside AEC3.
pub struct Aec3 {
    echo: EchoCanceller3,
    stream: StreamConfig,
    render: AudioBuffer,
    capture: AudioBuffer,
    linear: AudioBuffer,
}

/// The configuration S0.3 measured: the crate's default, with the linear output exported.
/// With no configuration of its own, the crate's AEC3 node pairs the default with the default
/// multichannel one; mono audio never switches to it, but it is passed the same way.
fn configs() -> (EchoCanceller3Config, EchoCanceller3Config) {
    let mut config = EchoCanceller3Config::default();
    let mut multichannel = EchoCanceller3Config::create_default_multichannel_config();
    config.filter.export_linear_aec_output = true;
    multichannel.filter.export_linear_aec_output = true;
    (config, multichannel)
}

impl Default for Aec3 {
    fn default() -> Self {
        Self::new()
    }
}

impl Aec3 {
    /// A canceller in its initial state, configured as S0.3 measured it.
    pub fn new() -> Self {
        let (config, multichannel) = configs();
        let rate = RATE as usize;
        let buffer = || AudioBuffer::from_sample_rates(rate, 1, rate, 1, rate);
        Self {
            echo: EchoCanceller3::with_multichannel_config(
                config,
                Some(multichannel),
                RATE as i32,
                1,
                1,
            ),
            stream: StreamConfig::new(rate, 1, false),
            render: buffer(),
            capture: buffer(),
            linear: buffer(),
        }
    }

    /// One frame: `far` is the reference (what the speakers played), `mic` the capture. Writes
    /// the linear output (echo subtracted, no residual suppression) and the full output (after
    /// the suppressor). Both lag the input by their latency constants.
    pub fn process(
        &mut self,
        far: &[f32; FRAME],
        mic: &[f32; FRAME],
        linear: &mut [f32; FRAME],
        full: &mut [f32; FRAME],
    ) {
        self.render.copy_from(&[far.as_slice()], &self.stream);
        self.render.split_into_frequency_bands();
        self.echo.analyze_render(&mut self.render);

        self.capture.copy_from(&[mic.as_slice()], &self.stream);
        self.echo.analyze_capture(&mut self.capture);
        self.capture.split_into_frequency_bands();
        self.echo
            .process_capture_with_linear_output(&mut self.capture, &mut self.linear, false);
        self.linear.merge_frequency_bands();
        self.linear
            .copy_to_stream(&self.stream, &mut [linear.as_mut_slice()]);
        self.capture.merge_frequency_bands();
        self.capture
            .copy_to_stream(&self.stream, &mut [full.as_mut_slice()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aec3::nodes::audio::AudioFormat;
    use aec3::pipelines::linear;

    /// A far end of noise bursts, and a mic that hears it 30 ms late through a short, decaying
    /// path, plus a quiet talker of its own.
    fn scene(frames: usize) -> (Vec<f32>, Vec<f32>) {
        let mut s = 7u64;
        let mut rnd = || {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (s >> 33) as f64 / (1u64 << 31) as f64 - 0.5
        };
        let n = frames * FRAME;
        let far: Vec<f32> = (0..n)
            .map(|i| {
                let on = (i / 4_000) % 3 != 2;
                (if on { 0.2 * rnd() } else { 0.0 }) as f32
            })
            .collect();
        let mut mic = vec![0.0f32; n];
        for i in 530..n {
            mic[i] = 0.5 * far[i - 480] + 0.2 * far[i - 490] - 0.1 * far[i - 530]
                + (0.01 * (i as f64 * 0.05).sin()) as f32;
        }
        (far, mic)
    }

    #[test]
    fn the_filter_length_is_the_default_13_blocks() {
        let (config, multichannel) = configs();
        assert_eq!(config.filter.main.length_blocks, FILTER_BLOCKS);
        assert_eq!(config.filter.shadow.length_blocks, FILTER_BLOCKS);
        assert_eq!(multichannel.filter.main.length_blocks, FILTER_BLOCKS);
    }

    /// Driving AEC3 directly gives exactly what the crate's own pipeline gives, configured as the
    /// S0.3 configured it.
    #[test]
    fn the_direct_driver_matches_the_crates_pipeline_sample_for_sample() {
        let frames = 600;
        let (far, mic) = scene(frames);
        let fmt = AudioFormat::ten_ms(16_000, 1);
        let mut graph = linear::builder(fmt, fmt)
            .enable_high_pass_filter(false)
            .enable_noise_suppression(false)
            .enable_gain_controller2(false)
            .enable_post_filter(false)
            .export_linear_output(true)
            .build()
            .expect("pipeline");
        let mut direct = Aec3::new();
        let (mut g_out, mut g_lin) = ([0.0f32; FRAME], [0.0f32; FRAME]);
        let (mut d_out, mut d_lin) = ([0.0f32; FRAME], [0.0f32; FRAME]);
        let mut compared = 0;
        for f in 0..frames {
            let r = f * FRAME..(f + 1) * FRAME;
            graph.handle_render_frame(&far[r.clone()]).expect("render");
            assert!(
                graph
                    .process_capture_frame(&mic[r.clone()], &mut g_out)
                    .expect("capture")
            );
            let lin = graph
                .try_pull_linear_output()
                .expect("pull")
                .expect("linear");
            g_lin.copy_from_slice(lin.payload().samples());
            let far_f: &[f32; FRAME] = far[r.clone()].try_into().expect("frame");
            let mic_f: &[f32; FRAME] = mic[r].try_into().expect("frame");
            direct.process(far_f, mic_f, &mut d_lin, &mut d_out);
            assert_eq!(g_out, d_out, "full output, frame {f}");
            assert_eq!(g_lin, d_lin, "linear output, frame {f}");
            compared += 1;
        }
        assert_eq!(compared, frames);
        // And the scene really had echo to remove, so the comparison was not of silence.
        let tail = (frames - 100) * FRAME..frames * FRAME;
        let e = |x: &[f32]| x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>();
        let mut out = Vec::new();
        let mut direct = Aec3::new();
        for f in 0..frames {
            let r = f * FRAME..(f + 1) * FRAME;
            direct.process(
                far[r.clone()].try_into().expect("frame"),
                mic[r].try_into().expect("frame"),
                &mut d_lin,
                &mut d_out,
            );
            out.extend_from_slice(&d_out);
        }
        let erle = 10.0 * (e(&mic[tail.clone()]) / e(&out[tail])).log10();
        assert!(erle > 10.0, "ERLE {erle:.1} dB");
    }

    /// With no far end, AEC3 passes the capture through, delayed by its framing: a click comes out
    /// exactly `FULL_LATENCY` late on the full output and `LINEAR_LATENCY` late on the linear one.
    #[test]
    fn the_output_latencies_are_the_pinned_constants() {
        let mut aec = Aec3::new();
        let far = [0.0f32; FRAME];
        let mut mic_stream = vec![0.0f32; 20 * FRAME];
        let click = 10 * FRAME + 37;
        mic_stream[click] = 0.5;
        let (mut lin, mut full) = ([0.0f32; FRAME], [0.0f32; FRAME]);
        let (mut lin_all, mut full_all) = (Vec::new(), Vec::new());
        for f in 0..20 {
            let mic: &[f32; FRAME] = mic_stream[f * FRAME..(f + 1) * FRAME]
                .try_into()
                .expect("frame");
            aec.process(&far, mic, &mut lin, &mut full);
            lin_all.extend_from_slice(&lin);
            full_all.extend_from_slice(&full);
        }
        let peak = |x: &[f32]| {
            x.iter()
                .enumerate()
                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                .map(|(i, _)| i)
                .expect("samples")
        };
        assert_eq!(peak(&full_all), click + FULL_LATENCY);
        assert_eq!(peak(&lin_all), click + LINEAR_LATENCY);
        assert!((full_all[click + FULL_LATENCY] - 0.5).abs() < 0.01);
        assert!((lin_all[click + LINEAR_LATENCY] - 0.5).abs() < 0.01);
    }
}
