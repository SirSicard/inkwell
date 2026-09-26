//! File import: an audio file becomes a record.
//!
//! ```text
//! file ─► blocks ─► Downmix (average) ─► StreamResampler ─► SpeechPass ─► one engine call per speech region ─► record
//! ```
//!
//! It goes through exactly the gain stage and the speech regions of a meeting's final pass
//! ([`speech`](crate::speech)): Inkwell 0.2's import path fed the engine the file's raw level, and a
//! quiet recording came back empty (fixed in 0.2.x by levelling the whole file; here the gain is
//! learned from speech, per window). Only speech reaches the engine, so a long silence in a
//! recording cannot come back as invented text.
//!
//! - **One stream.** A file has no stream identity: it is stored on the mic channel (shown as
//!   "you"), undiarized. Channels are averaged, since a recording is a mix and a voice panned to
//!   one side must not be lost.
//! - **Nothing half-written.** The file is transcribed first; the record is created only once there
//!   are words, and a record whose segments could not be saved is deleted again. A region the
//!   engine fails on is left out and reported in the outcome: unlike a meeting's final pass, an
//!   import has no earlier transcript to keep instead.
//! - **Bounded memory.** The file is read in one-second blocks; the pass holds about a window and a
//!   region.
//!
//! WAV files are read here ([`import_wav`]). Other formats are decoded by the shell, which hands the
//! samples over ([`import_samples`]).
//!
//! **Worker.** It runs for as long as the engine takes and checks `cancel` between regions.

use std::cell::RefCell;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use ink_audio::{Downmix, StreamResampler, VadConfig, WindowError};
use ink_core::{
    CancelToken, Channel, Clock, NewRecord, OfflineEngine, RecordId, RecordKind, Segment, Store,
    StoreError, StreamFormat,
};

use crate::events::VoiceDetection;
use crate::meeting::events::{ChannelPass, MeetingEvent, MeetingWarning, Phase};
use crate::meeting::offline::{self, Pass, Stop};
use crate::speech::{Region, RegionConfig, SpeechPass, VadSource, little_speech_heard};

/// What an import calls.
#[derive(Clone)]
pub struct ImportServices {
    /// The engine (the router's choice for the meeting final: long audio).
    pub engine: Arc<dyn OfflineEngine>,
    /// The library.
    pub store: Arc<dyn Store>,
    /// For the record's times.
    pub clock: Arc<dyn Clock>,
}

/// How an import behaves.
#[derive(Clone, Debug, Default)]
pub struct ImportSettings {
    /// The VAD's thresholds.
    pub vad: VadConfig,
    /// How speech regions are formed.
    pub regions: RegionConfig,
    /// Words the engine should favour.
    pub context: Option<String>,
}

/// A finished import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportOutcome {
    /// The new record.
    pub record: RecordId,
    /// What the pass did.
    pub pass: ChannelPass,
    /// Whether the file was judged with voice detection to the end. Without it, the fallback sent
    /// every stretch that was not silence or room tone, and the shell says so.
    pub detection: VoiceDetection,
    /// What went wrong on the way (regions that came back empty or failed, the VAD failing).
    pub warnings: Vec<MeetingWarning>,
}

/// Why an import made no record.
#[derive(Debug)]
#[non_exhaustive]
pub enum ImportError {
    /// The file could not be opened or decoded. The message names the file, never its content.
    Read(String),
    /// A format with no channels or no rate.
    Format(StreamFormat),
    /// The region sizes in the settings cannot work.
    Regions(WindowError),
    /// The cancel token was set.
    Cancelled,
    /// No words came back: the VAD found no speech, or every region came back empty or failed.
    NothingHeard {
        /// What the pass did.
        pass: ChannelPass,
        /// Why, region by region.
        warnings: Vec<MeetingWarning>,
    },
    /// The record could not be saved.
    Store(StoreError),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(msg) => write!(f, "import: {msg}"),
            Self::Format(format) => write!(
                f,
                "import: unusable format ({} Hz, {} channels)",
                format.sample_rate, format.channels
            ),
            Self::Regions(e) => write!(f, "import: {e}"),
            Self::Cancelled => f.write_str("import cancelled"),
            Self::NothingHeard { pass, .. } => write!(
                f,
                "import: no words in {} speech regions ({} empty, {} failed)",
                pass.regions, pass.empty_regions, pass.failed_regions
            ),
            Self::Store(e) => write!(f, "import: {e}"),
        }
    }
}

impl std::error::Error for ImportError {}

/// Frames read from a file per block: one second at 48 kHz.
const BLOCK_FRAMES: usize = 48_000;

/// **Worker.** Imports a WAV file (8- to 32-bit PCM, or 32-bit float). Without a `title`, the file's
/// name (less its extension) is the title.
pub fn import_wav(
    path: &Path,
    title: Option<String>,
    services: &ImportServices,
    settings: &ImportSettings,
    vad: &VadSource,
    cancel: &CancelToken,
) -> Result<ImportOutcome, ImportError> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let read_error = |e: hound::Error| ImportError::Read(format!("{name}: {e}"));
    let mut reader = hound::WavReader::open(path).map_err(read_error)?;
    let spec = reader.spec();
    let format = StreamFormat {
        sample_rate: spec.sample_rate,
        channels: spec.channels,
    };
    let title = title.or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()));
    let block = BLOCK_FRAMES * usize::from(spec.channels.max(1));
    let samples: Box<dyn Iterator<Item = Result<f32, hound::Error>>> = match spec.sample_format {
        hound::SampleFormat::Float => Box::new(reader.samples::<f32>()),
        hound::SampleFormat::Int => {
            if !(1..=32).contains(&spec.bits_per_sample) {
                return Err(ImportError::Read(format!(
                    "{name}: {}-bit samples",
                    spec.bits_per_sample
                )));
            }
            let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
            Box::new(
                reader
                    .samples::<i32>()
                    .map(move |s| s.map(|v| v as f32 * scale)),
            )
        }
    };
    let mut samples = samples;
    let mut next = |buf: &mut Vec<f32>| -> Result<(), ImportError> {
        buf.clear();
        for s in samples.by_ref().take(block) {
            buf.push(s.map_err(read_error)?);
        }
        Ok(())
    };
    import(format, &mut next, title, services, settings, vad, cancel)
}

/// **Worker.** Imports samples the shell decoded: interleaved, in `format`.
pub fn import_samples(
    samples: &[f32],
    format: StreamFormat,
    title: Option<String>,
    services: &ImportServices,
    settings: &ImportSettings,
    vad: &VadSource,
    cancel: &CancelToken,
) -> Result<ImportOutcome, ImportError> {
    let block = BLOCK_FRAMES * usize::from(format.channels.max(1));
    let mut chunks = samples.chunks(block);
    let mut next = |buf: &mut Vec<f32>| -> Result<(), ImportError> {
        buf.clear();
        if let Some(chunk) = chunks.next() {
            buf.extend_from_slice(chunk);
        }
        Ok(())
    };
    import(format, &mut next, title, services, settings, vad, cancel)
}

/// The import itself. `next` fills its buffer with the next interleaved block, or leaves it empty
/// at the end.
fn import(
    format: StreamFormat,
    next: &mut dyn FnMut(&mut Vec<f32>) -> Result<(), ImportError>,
    title: Option<String>,
    services: &ImportServices,
    settings: &ImportSettings,
    vad: &VadSource,
    cancel: &CancelToken,
) -> Result<ImportOutcome, ImportError> {
    if format.channels == 0 || format.sample_rate == 0 {
        return Err(ImportError::Format(format));
    }
    let started_at_unix_ms = services.clock.unix_ms();
    let warnings = RefCell::new(Vec::new());
    let emit = |event: MeetingEvent| {
        if let MeetingEvent::Warning(w) = event {
            warnings.borrow_mut().push(w);
        }
    };
    let (vad, error) = vad.open();
    if let Some(error) = error {
        emit(MeetingEvent::Warning(MeetingWarning::VadFailed {
            channel: Channel::Mic,
            phase: Phase::Final,
            error,
        }));
    }
    let mut pass =
        SpeechPass::new(vad, settings.vad, settings.regions).map_err(ImportError::Regions)?;
    let ctx = Pass {
        engine: services.engine.as_ref(),
        context: settings.context.as_deref(),
        cancel,
        emit: &emit,
    };
    let mut report = offline::report(Channel::Mic);
    let mut segments: Vec<Segment> = Vec::new();
    // Frames read from the file: its length, as the pass's captured audio.
    let mut frames: u64 = 0;
    {
        let mut on_region = |region: Region| -> Result<(), Stop> {
            segments.extend(offline::transcribe(
                &ctx,
                Channel::Mic,
                &region.audio,
                region.start,
                &mut report,
            )?);
            Ok(())
        };
        let mut resampler =
            StreamResampler::new(format.sample_rate).map_err(|_| ImportError::Format(format))?;
        let (mut buf, mut mono, mut out, mut regions) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let stop = |s: Stop| match s {
            Stop::Cancelled => ImportError::Cancelled,
            Stop::Window(e) => ImportError::Regions(e),
            // An import reads no chunks.
            Stop::Chunks(e) => ImportError::Read(e.to_string()),
        };
        loop {
            next(&mut buf)?;
            if buf.is_empty() {
                break;
            }
            frames += (buf.len() / usize::from(format.channels)) as u64;
            mono.clear();
            Downmix::Average.apply(&buf, format.channels, &mut mono);
            out.clear();
            out.reserve(resampler.max_output_frames(mono.len()));
            resampler
                .push(&mono, &mut out)
                .map_err(|e| ImportError::Read(e.to_string()))?;
            pass.push(&out, &mut regions)
                .map_err(ImportError::Regions)?;
            for region in regions.drain(..) {
                on_region(region).map_err(stop)?;
            }
        }
        out.clear();
        resampler
            .finish(&mut out)
            .map_err(|e| ImportError::Read(e.to_string()))?;
        pass.push(&out, &mut regions)
            .map_err(ImportError::Regions)?;
        pass.finish(&mut regions);
        for region in regions.drain(..) {
            on_region(region).map_err(stop)?;
        }
    }
    if let Some(error) = pass.vad_error().cloned() {
        emit(MeetingEvent::Warning(MeetingWarning::VadFailed {
            channel: Channel::Mic,
            phase: Phase::Final,
            error,
        }));
    }
    let detection = pass.detection();
    report.captured_ms = frames * 1_000 / u64::from(format.sample_rate);
    report.audible_ms = pass.audible_ms();
    report.speech_ms = pass.speech_ms();
    if little_speech_heard(report.audible_ms, report.speech_ms) {
        emit(MeetingEvent::Warning(MeetingWarning::LittleSpeechHeard {
            channel: Channel::Mic,
            audible_ms: report.audible_ms,
            speech_ms: report.speech_ms,
        }));
    }
    let warnings = warnings.into_inner();
    if ink_core::store::word_count(&segments) == 0 {
        log::info!(
            "import: no words ({} regions, {} empty, {} failed)",
            report.regions,
            report.empty_regions,
            report.failed_regions
        );
        return Err(ImportError::NothingHeard {
            pass: report,
            warnings,
        });
    }
    let record = save(services, title, started_at_unix_ms, &segments)?;
    log::info!(
        "import: {} regions, {} words, {} empty, {} failed",
        report.regions,
        report.word_count,
        report.empty_regions,
        report.failed_regions
    );
    Ok(ImportOutcome {
        record,
        pass: report,
        detection,
        warnings,
    })
}

/// One record with the import's segments, or none: a half-written one is deleted.
fn save(
    services: &ImportServices,
    title: Option<String>,
    started_at_unix_ms: i64,
    segments: &[Segment],
) -> Result<RecordId, ImportError> {
    let store = &services.store;
    let id = store
        .create_record(NewRecord {
            kind: RecordKind::FileImport,
            title,
            started_at_unix_ms,
            source_app: None,
            // The file stays where the user keeps it; no chunks are made.
            audio_dir: None,
        })
        .map_err(ImportError::Store)?;
    let ended = services.clock.unix_ms().max(started_at_unix_ms);
    let filled = store
        .append_segments(&id, segments)
        .and_then(|()| store.finish_record(&id, ended));
    if let Err(error) = filled {
        if let Err(cleanup) = store.delete_record(&id) {
            log::warn!("import: a half-saved record could not be removed: {cleanup}");
        }
        return Err(ImportError::Store(error));
    }
    Ok(id)
}
