use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::lock;
use crate::audio::Channel;
use crate::engine::{
    AsrEvent, Diarizer, EngineInfo, EngineStream, Job, OfflineEngine, SpeakerTurn, StreamingEngine,
    TranscribeOptions, Transcript,
};
use crate::error::{EngineError, LlmError};
use crate::llm::{Endpoint, Llm, LlmInfo, LlmRequest, LlmResponse};
use crate::threading::{CancelToken, EventSink};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(FNV_OFFSET, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(FNV_PRIME)
    })
}

/// The key a [`MockEngine`] fixture is stored under: 64-bit FNV-1a over each sample's IEEE-754
/// bits, little-endian.
///
/// Stable across platforms and runs, so it can be written into a test. It is not a security hash,
/// and `0.0` and `-0.0` hash differently on purpose: the key is the exact audio.
pub fn fixture_hash(audio: &[f32]) -> u64 {
    fnv1a(audio.iter().flat_map(|s| s.to_bits().to_le_bytes()))
}

/// One input a [`MockEngine`] received.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MockCall {
    /// [`fixture_hash`] of the input.
    pub hash: u64,
    /// Samples received.
    pub frames: usize,
    /// RMS level in dBFS, floored at −120 so silence is a number.
    pub rms_dbfs: f32,
    /// Absolute peak.
    pub peak: f32,
    /// The channel the call named.
    pub channel: Channel,
}

fn level(audio: &[f32]) -> (f32, f32) {
    if audio.is_empty() {
        return (-120.0, 0.0);
    }
    let mean_square = audio
        .iter()
        .map(|&s| f64::from(s) * f64::from(s))
        .sum::<f64>()
        / audio.len() as f64;
    let rms_dbfs = if mean_square > 0.0 {
        (10.0 * mean_square.log10()).max(-120.0)
    } else {
        -120.0
    };
    let peak = audio.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
    (rms_dbfs as f32, peak)
}

struct EngineState {
    info: EngineInfo,
    fixtures: Mutex<HashMap<u64, Transcript>>,
    calls: Mutex<Vec<MockCall>>,
}

impl EngineState {
    fn answer(&self, audio: &[f32], channel: Channel) -> Result<Transcript, EngineError> {
        let hash = fixture_hash(audio);
        let (rms_dbfs, peak) = level(audio);
        lock(&self.calls).push(MockCall {
            hash,
            frames: audio.len(),
            rms_dbfs,
            peak,
            channel,
        });
        lock(&self.fixtures).get(&hash).cloned().ok_or_else(|| {
            EngineError::Failed(format!(
                "mock engine {}: no fixture for hash {hash:016x} ({} samples)",
                self.info.id,
                audio.len()
            ))
        })
    }
}

/// An offline and streaming engine that answers from fixtures.
///
/// Unknown audio is an error that names its hash, never an empty transcript: a silent empty answer
/// is exactly the failure the supersede guard exists for, and a mock must not hide it.
#[derive(Clone)]
pub struct MockEngine {
    state: Arc<EngineState>,
}

impl MockEngine {
    /// An engine with no fixtures.
    pub fn new(id: &str, jobs: &[Job]) -> Self {
        Self {
            state: Arc::new(EngineState {
                info: EngineInfo {
                    id: id.into(),
                    jobs: jobs.to_vec(),
                    licence: "MIT".into(),
                },
                fixtures: Mutex::default(),
                calls: Mutex::default(),
            }),
        }
    }

    /// Adds a fixture and returns the engine, for building one in an expression.
    pub fn with_fixture(self, audio: &[f32], transcript: Transcript) -> Self {
        self.add_fixture(audio, transcript);
        self
    }

    /// Answers `transcript` whenever the exact `audio` arrives.
    pub fn add_fixture(&self, audio: &[f32], transcript: Transcript) {
        lock(&self.state.fixtures).insert(fixture_hash(audio), transcript);
    }

    /// Every input received so far, in order.
    pub fn calls(&self) -> Vec<MockCall> {
        lock(&self.state.calls).clone()
    }
}

impl OfflineEngine for MockEngine {
    fn info(&self) -> EngineInfo {
        self.state.info.clone()
    }

    fn transcribe(
        &self,
        audio: &[f32],
        options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError> {
        if options.cancel.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        self.state.answer(audio, options.channel)
    }
}

impl StreamingEngine for MockEngine {
    fn info(&self) -> EngineInfo {
        self.state.info.clone()
    }

    fn open_stream(
        &self,
        channel: Channel,
        events: EventSink<AsrEvent>,
    ) -> Result<Box<dyn EngineStream>, EngineError> {
        Ok(Box::new(MockStream {
            state: Arc::clone(&self.state),
            channel,
            events,
            audio: Vec::new(),
        }))
    }
}

/// Buffers the stream. Each push reports a partial; `finish` answers the whole stream's audio from
/// the fixtures and delivers every segment as a final.
struct MockStream {
    state: Arc<EngineState>,
    channel: Channel,
    events: EventSink<AsrEvent>,
    audio: Vec<f32>,
}

impl EngineStream for MockStream {
    fn push(&mut self, audio: &[f32]) -> Result<(), EngineError> {
        self.audio.extend_from_slice(audio);
        (self.events)(AsrEvent::Partial {
            text: format!("partial after {} samples", self.audio.len()),
        });
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<(), EngineError> {
        let transcript = self.state.answer(&self.audio, self.channel)?;
        for segment in transcript.segments {
            (self.events)(AsrEvent::Final(segment));
        }
        Ok(())
    }
}

/// A diarizer that returns the same turns every time.
pub struct MockDiarizer {
    turns: Vec<SpeakerTurn>,
    calls: AtomicUsize,
}

impl MockDiarizer {
    /// Answers `turns` to every call.
    pub fn new(turns: Vec<SpeakerTurn>) -> Self {
        Self {
            turns,
            calls: AtomicUsize::new(0),
        }
    }

    /// Offline calls so far.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

impl Diarizer for MockDiarizer {
    fn info(&self) -> EngineInfo {
        EngineInfo {
            id: "mock-diarizer".into(),
            jobs: vec![Job::Diarization],
            licence: "MIT".into(),
        }
    }

    fn diarize(
        &self,
        _audio: &[f32],
        cancel: &CancelToken,
    ) -> Result<Vec<SpeakerTurn>, EngineError> {
        if cancel.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(self.turns.clone())
    }

    fn open_stream(
        &self,
        turns: EventSink<SpeakerTurn>,
    ) -> Result<Box<dyn EngineStream>, EngineError> {
        Ok(Box::new(MockDiarizerStream {
            turns: self.turns.clone(),
            sink: turns,
        }))
    }
}

struct MockDiarizerStream {
    turns: Vec<SpeakerTurn>,
    sink: EventSink<SpeakerTurn>,
}

impl EngineStream for MockDiarizerStream {
    fn push(&mut self, _audio: &[f32]) -> Result<(), EngineError> {
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<(), EngineError> {
        for turn in self.turns {
            (self.sink)(turn);
        }
        Ok(())
    }
}

/// A language model with one canned reply. It does not apply the local-only guard; that is
/// `ink-llm`'s job, and a mock that did it would test the wrong crate.
pub struct MockLlm {
    info: LlmInfo,
    reply: String,
    calls: AtomicUsize,
}

impl MockLlm {
    /// A model at `endpoint` that always answers `reply`.
    pub fn new(endpoint: Endpoint, reply: &str) -> Self {
        Self {
            info: LlmInfo {
                provider: "mock".into(),
                model: "mock".into(),
                endpoint,
            },
            reply: reply.into(),
            calls: AtomicUsize::new(0),
        }
    }

    /// Completed calls so far.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

impl Llm for MockLlm {
    fn info(&self) -> LlmInfo {
        self.info.clone()
    }

    fn complete(
        &self,
        _request: &LlmRequest,
        cancel: &CancelToken,
    ) -> Result<LlmResponse, LlmError> {
        if cancel.is_cancelled() {
            return Err(LlmError::Cancelled);
        }
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(LlmResponse {
            text: self.reply.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_matches_the_published_vectors() {
        assert_eq!(fnv1a(*b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(*b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(*b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn level_of_a_full_scale_square_is_zero_dbfs() {
        let (rms, peak) = level(&[1.0, -1.0, 1.0, -1.0]);
        assert!(rms.abs() < 1e-6, "{rms}");
        assert_eq!(peak, 1.0);
        assert_eq!(level(&[]), (-120.0, 0.0));
        assert_eq!(level(&[0.0; 8]).0, -120.0);
    }
}
