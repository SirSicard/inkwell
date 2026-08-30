//! Live partial text while the hotkey is held.
//!
//! Between releasing the key and seeing the paste there is a silent gap, and
//! nothing on screen proves the app heard a word of it. This fills that gap.
//!
//! It is feedback and only feedback. The text it produces is never pasted and
//! never stored; the offline pipeline still produces everything the user keeps.
//! That split is not a simplification, it is the finding of the spike in
//! `docs/streaming-spike-2026-07-31.md`: a streaming decoder emits no casing and
//! no punctuation, and it drops the final word of an utterance because it never
//! gets the right-context the offline pass reads from the whole waveform. Good
//! enough to watch, not good enough to keep.
//!
//! The recognizer is confined to one thread and never shared, the same shape as
//! `EngineService` and for the same reason: `OnlineRecognizer` is not `Sync`,
//! and no `unsafe impl` is needed if it is moved in once and spoken to by
//! message.

use crate::overlay::OVERLAY_LABEL;
use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

/// Event carrying partial text to the overlay window.
pub const PARTIAL_EVENT: &str = "partial-text";

/// Longest partial the overlay can show before it starts scrolling off. The
/// tail is what survives a trim, because the words you just said are the ones
/// you are checking.
const MAX_PARTIAL_CHARS: usize = 96;

/// Resamples a *continuous* stream to 16 kHz by linear interpolation, carrying
/// its fractional position and one sample across chunk boundaries so a chunked
/// feed produces the same signal as one long call.
///
/// The offline path's `recording::resample_to_16k` is deliberately not reused.
/// It is a windowed-sinc resampler whose `SincFixedIn` wants the whole input
/// length up front, which a live feed does not have, and its quality is bought
/// for audio the user keeps. This audio is thrown away a second after it is
/// drawn, so linear interpolation is the honest trade rather than a shortcut.
pub struct StreamResampler {
    /// Source samples per output sample.
    ratio: f64,
    /// Absolute source position of the next output sample.
    next_src: f64,
    /// Absolute count of source samples handed in so far.
    consumed: usize,
    /// Source sample at index `consumed - 1`, so an output sample landing on a
    /// chunk boundary can still interpolate across it.
    prev: f32,
}

impl StreamResampler {
    pub fn new(source_rate: usize) -> Self {
        Self {
            ratio: source_rate as f64 / 16_000.0,
            next_src: 0.0,
            consumed: 0,
            prev: 0.0,
        }
    }

    pub fn push(&mut self, chunk: &[f32]) -> Vec<f32> {
        if chunk.is_empty() {
            return Vec::new();
        }
        let end = self.consumed + chunk.len();
        let consumed = self.consumed;
        let prev = self.prev;
        // `j` is an absolute source index. Anything below `consumed` can only
        // ever be `consumed - 1`, which is what `prev` holds: the loop below
        // never leaves `next_src` further back than that.
        let at = |j: usize| -> f32 {
            match j.checked_sub(consumed) {
                Some(k) => chunk.get(k).copied().unwrap_or(0.0),
                None => prev,
            }
        };

        let mut out = Vec::with_capacity(chunk.len() * 16_000 / 44_100 + 2);
        loop {
            let i = self.next_src.floor() as usize;
            let frac = self.next_src - i as f64;
            if i >= end {
                break;
            }
            // Only an output that falls *between* two samples needs the second
            // one. Requiring it unconditionally would hold back every output
            // by a sample, which at a 16 kHz capture rate turns an exact
            // passthrough into a permanent one-sample lag.
            if frac > 0.0 && i + 1 >= end {
                break;
            }
            let s0 = at(i);
            let s1 = if frac > 0.0 { at(i + 1) } else { s0 };
            out.push(s0 + (s1 - s0) * frac as f32);
            self.next_src += self.ratio;
        }

        self.prev = chunk[chunk.len() - 1];
        self.consumed = end;
        out
    }
}

/// Turn a raw streaming hypothesis into what the overlay shows.
///
/// Lowercased on purpose. Streaming models emit `HELLO THERE` and the offline
/// pass emits `Hello there.` a moment later; all-caps replaced by properly-cased
/// text reads as the app correcting a mistake, where lowercase replaced by
/// cased text reads as it settling on an answer. Same two frames, opposite
/// story.
pub fn display_partial(raw: &str, max_chars: usize) -> String {
    let mut text = raw.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    if text.chars().count() <= max_chars {
        return text;
    }
    // Keep the tail: the words just spoken are the ones being checked. Trim to
    // a word boundary so the line does not start mid-syllable.
    let skip = text.chars().count() - max_chars;
    let cut: usize = text
        .char_indices()
        .nth(skip)
        .map(|(i, _)| i)
        .unwrap_or(0);
    text = text[cut..].to_string();
    if let Some(space) = text.find(' ') {
        text = text[space + 1..].to_string();
    }
    format!("… {}", text)
}

enum Request {
    /// Hand the worker the handle it emits partials through. `AppState` is
    /// constructed before the Tauri app exists, so the service starts deaf and
    /// `setup` gives it a voice. Nothing can record before that runs.
    Attach(AppHandle),
    Load {
        dir: PathBuf,
        reply: Sender<Result<(), String>>,
    },
    Unload,
    /// Start a new utterance. `source_rate` is the capture device's rate.
    Begin {
        source_rate: usize,
    },
    Feed(Vec<f32>),
    End,
}

pub struct StreamingService {
    tx: Sender<Request>,
    /// Whether a model is loaded and partials can actually run. Read by the
    /// hotkey path, which must not block on the worker to decide whether to
    /// spawn a feeder.
    ready: Arc<AtomicBool>,
}

/// Locate one of the transducer's three .onnx files in a model directory,
/// preferring int8 when both precisions are present. Filenames carry the epoch
/// and chunk config, so they cannot be hardcoded across model revisions.
fn find_onnx(dir: &Path, needle: &str) -> Option<PathBuf> {
    let mut hits: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let n = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            n.ends_with(".onnx") && n.contains(needle)
        })
        .collect();
    hits.sort();
    hits.iter()
        .find(|p| p.to_string_lossy().contains("int8"))
        .cloned()
        .or_else(|| hits.first().cloned())
}

/// Construct the recognizer from a model directory.
///
/// Public so `examples/streaming_check.rs` can drive the configuration this
/// ships with rather than a copy of it. A harness that rebuilds the config
/// itself proves the model works and says nothing about the settings the app
/// actually runs, which is where the decisions are (`enable_endpoint`, thread
/// count, int8 preference).
pub fn build(dir: &Path) -> Result<OnlineRecognizer, String> {
    let encoder = find_onnx(dir, "encoder").ok_or("No encoder .onnx in the streaming model dir")?;
    let decoder = find_onnx(dir, "decoder").ok_or("No decoder .onnx in the streaming model dir")?;
    let joiner = find_onnx(dir, "joiner").ok_or("No joiner .onnx in the streaming model dir")?;
    let tokens = dir.join("tokens.txt");
    if !tokens.exists() {
        return Err("No tokens.txt in the streaming model dir".to_string());
    }

    let mut config = OnlineRecognizerConfig::default();
    config.model_config.transducer.encoder = Some(encoder.to_string_lossy().into_owned());
    config.model_config.transducer.decoder = Some(decoder.to_string_lossy().into_owned());
    config.model_config.transducer.joiner = Some(joiner.to_string_lossy().into_owned());
    config.model_config.tokens = Some(tokens.to_string_lossy().into_owned());
    // Two threads, so the offline pass that produces the real text still has
    // cores when it starts. The spike measured 40x real time at this setting.
    config.model_config.num_threads = 2;
    config.model_config.provider = Some("cpu".into());
    config.decoding_method = Some("greedy_search".into());
    // Endpointing is wrong here, which the spike had no reason to discover: it
    // exists to cut a continuous feed into utterances, and it resets the stream
    // when it fires. One held hotkey is one utterance by definition, so the
    // only thing endpointing could do is blank the line mid-sentence when the
    // user pauses for breath.
    config.enable_endpoint = false;

    OnlineRecognizer::create(&config).ok_or_else(|| {
        "Could not create the streaming recognizer (model files may be incomplete)".to_string()
    })
}

/// Emit a partial to the overlay, or do nothing if `setup` has not attached a
/// handle yet.
fn emit(app: Option<&AppHandle>, text: &str) {
    if let Some(app) = app {
        let _ = app.emit_to(OVERLAY_LABEL, PARTIAL_EVENT, text);
    }
}

impl StreamingService {
    pub fn start() -> Self {
        let (tx, rx) = channel::<Request>();
        let ready = Arc::new(AtomicBool::new(false));
        let ready_worker = ready.clone();

        std::thread::Builder::new()
            .name("streaming-asr".into())
            .spawn(move || {
                let mut app: Option<AppHandle> = None;
                let mut recognizer: Option<OnlineRecognizer> = None;
                let mut stream: Option<OnlineStream> = None;
                let mut resampler: Option<StreamResampler> = None;
                let mut last = String::new();

                while let Ok(req) = rx.recv() {
                    match req {
                        Request::Attach(handle) => app = Some(handle),
                        Request::Load { dir, reply } => {
                            let result = build(&dir).map(|r| {
                                recognizer = Some(r);
                                ready_worker.store(true, Ordering::Relaxed);
                                log::info!("Streaming model loaded from {}", dir.display());
                            });
                            if let Err(e) = &result {
                                log::warn!("Streaming model load failed: {}", e);
                            }
                            let _ = reply.send(result);
                        }
                        Request::Unload => {
                            ready_worker.store(false, Ordering::Relaxed);
                            stream = None;
                            resampler = None;
                            recognizer = None;
                            log::info!("Streaming model unloaded");
                        }
                        Request::Begin { source_rate } => {
                            if let Some(r) = recognizer.as_ref() {
                                stream = Some(r.create_stream());
                                resampler = Some(StreamResampler::new(source_rate));
                                last.clear();
                                emit(app.as_ref(), "");
                            }
                        }
                        Request::Feed(samples) => {
                            let (Some(r), Some(s), Some(rs)) =
                                (recognizer.as_ref(), stream.as_ref(), resampler.as_mut())
                            else {
                                continue;
                            };
                            let pcm = rs.push(&samples);
                            if pcm.is_empty() {
                                continue;
                            }
                            s.accept_waveform(16_000, &pcm);
                            while r.is_ready(s) {
                                r.decode(s);
                            }
                            if let Some(res) = r.get_result(s) {
                                let text = display_partial(&res.text, MAX_PARTIAL_CHARS);
                                if !text.is_empty() && text != last {
                                    emit(app.as_ref(), &text);
                                    last = text;
                                }
                            }
                        }
                        Request::End => {
                            if let (Some(r), Some(s)) = (recognizer.as_ref(), stream.as_ref()) {
                                s.input_finished();
                                while r.is_ready(s) {
                                    r.decode(s);
                                }
                            }
                            // Dropped rather than reset, so the next utterance
                            // cannot inherit a hypothesis from this one.
                            stream = None;
                            resampler = None;
                            last.clear();
                            // The text deliberately stays on screen. The
                            // overlay lives on for the length of the offline
                            // decode, and that decode is the silent gap this
                            // whole feature exists to fill: blanking the line
                            // the moment the key comes up would put the gap
                            // back. The overlay clears itself when the next
                            // recording starts.
                        }
                    }
                }
                // If the worker is gone, nothing can produce partials, and
                // `is_ready()` must stop claiming otherwise. Without this the
                // flag stays true for the rest of the session and the settings
                // UI reports a working feature that has no worker behind it,
                // which is worse than reporting it broken.
                ready_worker.store(false, Ordering::Relaxed);
                log::info!("Streaming thread shutting down");
            })
            .expect("failed to spawn streaming thread");

        Self { tx, ready }
    }

    /// Is a model loaded? False means every other method here is a no-op, which
    /// is the normal state: the feature is off by default.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    /// Load the streaming model. Blocking and slow, like `EngineService::load`,
    /// so main-thread callers must go through `spawn_blocking`.
    pub fn load(&self, dir: PathBuf) -> Result<(), String> {
        let (reply, wait) = channel();
        self.tx
            .send(Request::Load { dir, reply })
            .map_err(|_| "Streaming thread is gone".to_string())?;
        wait.recv()
            .map_err(|_| "Streaming thread stopped responding".to_string())?
    }

    /// Give the worker the handle it emits partials through. Called once, from
    /// `setup`.
    pub fn attach(&self, app: AppHandle) {
        let _ = self.tx.send(Request::Attach(app));
    }

    pub fn unload(&self) {
        let _ = self.tx.send(Request::Unload);
    }

    pub fn begin(&self, source_rate: usize) {
        let _ = self.tx.send(Request::Begin { source_rate });
    }

    pub fn feed(&self, samples: Vec<f32>) {
        let _ = self.tx.send(Request::Feed(samples));
    }

    pub fn end(&self) {
        let _ = self.tx.send(Request::End);
    }
}

#[cfg(test)]
mod resampler_tests {
    use super::StreamResampler;

    #[test]
    fn a_matching_rate_passes_samples_through() {
        // Exactly, with nothing held back. A 16 kHz capture device is the one
        // case where this struct should be doing no work at all.
        let mut r = StreamResampler::new(16_000);
        let out = r.push(&[0.0, 1.0, 2.0, 3.0, 4.0]);
        assert_eq!(out, vec![0.0, 1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn halving_the_rate_takes_every_other_sample() {
        let mut r = StreamResampler::new(32_000);
        let out = r.push(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(out, vec![0.0, 2.0, 4.0]);
    }

    #[test]
    fn chunking_produces_the_same_signal_as_one_call() {
        // This is the whole reason the struct carries state. A resampler that
        // restarted per chunk would emit a tiny discontinuity every 100ms.
        let input: Vec<f32> = (0..300).map(|i| i as f32).collect();

        let mut whole = StreamResampler::new(48_000);
        let expected = whole.push(&input);

        let mut chunked = StreamResampler::new(48_000);
        let mut got = Vec::new();
        for chunk in input.chunks(17) {
            got.extend(chunked.push(chunk));
        }

        assert_eq!(got.len(), expected.len());
        for (a, b) in got.iter().zip(expected.iter()) {
            assert!((a - b).abs() < 1e-4, "chunked {} vs whole {}", a, b);
        }
    }

    #[test]
    fn interpolates_across_a_chunk_boundary_rather_than_restarting() {
        // 24 kHz to 16 kHz is 1.5 source samples per output sample, so the
        // second output lands halfway between two samples that a naive
        // implementation would have split across two chunks.
        let mut r = StreamResampler::new(24_000);
        let mut out = r.push(&[0.0, 10.0]);
        out.extend(r.push(&[20.0, 30.0]));
        // positions 0.0, 1.5, 3.0 -> 0, 15, 30
        assert_eq!(out.len(), 3);
        assert!((out[0] - 0.0).abs() < 1e-4);
        assert!((out[1] - 15.0).abs() < 1e-4, "got {}", out[1]);
        assert!((out[2] - 30.0).abs() < 1e-4, "got {}", out[2]);
    }

    #[test]
    fn an_empty_chunk_is_harmless() {
        let mut r = StreamResampler::new(48_000);
        assert!(r.push(&[]).is_empty());
        // and does not disturb the position
        assert_eq!(r.push(&[0.0, 1.0, 2.0, 3.0]).len(), 2);
    }

    #[test]
    fn upsampling_from_8k_produces_more_samples_than_it_consumed() {
        let mut r = StreamResampler::new(8_000);
        let out = r.push(&[0.0, 2.0, 4.0, 6.0]);
        assert!(out.len() > 4, "expected upsampling, got {} samples", out.len());
    }
}

#[cfg(test)]
mod display_tests {
    use super::display_partial;

    #[test]
    fn lowercases_so_the_final_text_reads_as_refinement() {
        assert_eq!(
            display_partial("HELLO THERE", 96),
            "hello there"
        );
    }

    #[test]
    fn collapses_the_whitespace_a_decoder_leaves_behind() {
        assert_eq!(display_partial("  hello   there \n", 96), "hello there");
    }

    #[test]
    fn keeps_the_tail_not_the_head_when_it_overflows() {
        let long = "one two three four five six seven eight";
        let out = display_partial(long, 12);
        assert!(out.starts_with("… "), "got {:?}", out);
        assert!(out.ends_with("eight"), "kept the wrong end: {:?}", out);
        assert!(!out.contains("one"), "should have dropped the head: {:?}", out);
    }

    #[test]
    fn a_trim_lands_on_a_word_boundary() {
        let out = display_partial("alpha bravo charlie delta", 10);
        // Whatever survives, it must not begin mid-word.
        let body = out.trim_start_matches("… ");
        assert!(
            "alpha bravo charlie delta".split_whitespace().any(|w| body.starts_with(w)),
            "started mid-word: {:?}",
            out
        );
    }

    #[test]
    fn empty_stays_empty_so_the_overlay_can_tell_nothing_from_silence() {
        assert_eq!(display_partial("", 96), "");
        assert_eq!(display_partial("   ", 96), "");
    }

    #[test]
    fn multibyte_text_is_trimmed_by_character_not_byte() {
        // Slicing a UTF-8 string by byte offset panics mid-codepoint, and this
        // model's tokens.txt is not the only one this path will ever see.
        let out = display_partial("ααα βββ γγγ δδδ εεε", 8);
        assert!(out.chars().count() > 0);
    }
}
