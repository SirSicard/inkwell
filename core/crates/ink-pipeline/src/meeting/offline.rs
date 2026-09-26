//! The final pass over one side: its recorded chunks, read back onto the meeting's timeline, cut
//! into levelled speech regions ([`SpeechPass`]), and transcribed one region per engine call.
//!
//! # Reading the chunks back
//!
//! A side's chunks are read in order, one at a time (ten seconds each, never the session), then
//! downmixed and resampled as capture did. The first chunk, and each one after a gap or a format
//! change, is placed by its host time: sample 0 is the meeting's start stamp, lost audio becomes
//! digital silence (which the VAD never calls speech), and audio from before the start is dropped.
//! A chunk whose host time recovery could only estimate continues where the last one ended. A
//! chunk that cannot be read is skipped and counted: the pass has a gap there, and says so.
//!
//! # One region at a time
//!
//! [`SideReader`] hands the regions out as the caller pulls them, so a pass holds one chunk being
//! decoded, the speech pass's buffers (about a window) and the regions of the window last closed,
//! never the side's recording. A far end with a diarizer is read twice: [`RegionWindows`] streams
//! its regions into the diarizer (which pulls them through ink-core's `DiarizeInput`, a window at
//! a time), keeping only where each region sits; then each region is read again and transcribed,
//! cut where the speaker changes. The second read repeats the first exactly (the same audio, a
//! fresh VAD, the same settings), and the turns are on the meeting's timeline anyway, so the cuts
//! do not depend on it. `tests/far_pass_memory.rs` measures the bound: the same peak for a
//! 4-minute and a 12-minute far end.
//!
//! # Holding the engine to the VAD
//!
//! Every region the VAD found goes to the engine, and every one that comes back without words is
//! reported ([`MeetingWarning::EmptySpeechRegion`]): an engine returns a segment for everything it
//! is given, so its segments say nothing about where the speech was, and only this cross-check
//! notices speech that was dropped.

use std::collections::VecDeque;

use ink_audio::{ChunkInfo, ChunkStore, Downmix, StreamResampler, WindowError};
use ink_core::{
    CancelToken, Channel, DiarizeInput, EngineError, MAX_DIARIZE_WINDOW, OfflineEngine, Segment,
    SpeakerTurn, StreamFormat, TranscribeOptions,
};

use super::diarize::{speaker_changes, speaker_of};
use super::events::{ChannelPass, MeetingEvent, MeetingWarning};
use super::timeline::ns_to_samples;
use crate::speech::{Region, SpeechPass, samples_to_ms};

/// Why a pass stopped before the end.
#[derive(Debug)]
pub(crate) enum Stop {
    /// The cancel token was set.
    Cancelled,
    /// The chunk directory could not be listed.
    Chunks(ink_audio::ChunkError),
    /// The windowing refused its sizes (a bad [`RegionConfig`](crate::speech::RegionConfig)).
    Window(WindowError),
}

impl From<WindowError> for Stop {
    fn from(e: WindowError) -> Self {
        Self::Window(e)
    }
}

/// What a pass needs besides its audio.
pub(crate) struct Pass<'a> {
    pub engine: &'a dyn OfflineEngine,
    pub context: Option<&'a str>,
    pub cancel: &'a CancelToken,
    pub emit: &'a dyn Fn(MeetingEvent),
}

/// What reading a side's chunks found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SideRead {
    /// Chunk files, readable or not.
    pub chunks: usize,
    /// Audio in the readable ones, ms.
    pub captured_ms: u64,
    /// Chunk files that could not be used (unreadable, or their audio failed to resample): the
    /// pass has a gap at each.
    pub skipped: usize,
    /// Time above the audible floor, ms ([`SpeechPass::audible_ms`]).
    pub audible_ms: u64,
    /// Time the VAD found as speech, ms ([`SpeechPass::speech_ms`]).
    pub speech_ms: u64,
}

/// Zeros fed per push while filling a gap.
const GAP_BLOCK: usize = 16_000;

/// One side's chunks, read back onto the meeting timeline (host time `t0_ns` is sample 0) and run
/// through a [`SpeechPass`], handed out a region at a time as the caller pulls them.
///
/// It holds one chunk of audio while decoding it, the pass's own buffers (about a window), and the
/// regions of the last window the pass closed; never the side's recording.
pub(crate) struct SideReader<'a> {
    audio: &'a ChunkStore,
    channel: Channel,
    t0_ns: u64,
    chunks: std::vec::IntoIter<ChunkInfo>,
    pass: SpeechPass,
    /// The run of chunks being resampled as one stream: its format, its resampler, and the index
    /// the next chunk must have to continue it.
    run: Option<(StreamFormat, StreamResampler, u64)>,
    /// The next sample's position in the meeting.
    at: u64,
    /// Samples still to drop because they come from before the start.
    skip: u64,
    ready: VecDeque<Region>,
    fresh: Vec<Region>,
    mono: Vec<f32>,
    out: Vec<f32>,
    done: bool,
    read: SideRead,
}

impl<'a> SideReader<'a> {
    /// A reader over `channel`'s chunks in `audio`, cut by `pass`. Lists the chunks now.
    pub(crate) fn open(
        audio: &'a ChunkStore,
        channel: Channel,
        t0_ns: u64,
        pass: SpeechPass,
    ) -> Result<Self, Stop> {
        let list = audio.chunks(channel).map_err(Stop::Chunks)?;
        let read = SideRead {
            chunks: list.chunks.len() + list.unreadable.len(),
            captured_ms: list
                .chunks
                .iter()
                .filter(|c| !c.format_estimated)
                .map(|c| c.frames * 1_000 / u64::from(c.format.sample_rate.max(1)))
                .sum(),
            skipped: list.unreadable.len(),
            audible_ms: 0,
            speech_ms: 0,
        };
        Ok(Self {
            audio,
            channel,
            t0_ns,
            chunks: list.chunks.into_iter(),
            pass,
            run: None,
            at: 0,
            skip: 0,
            ready: VecDeque::new(),
            fresh: Vec::new(),
            mono: Vec::new(),
            out: Vec::new(),
            done: false,
            read,
        })
    }

    /// The next speech region, in order, or `None` once the side is read.
    pub(crate) fn next_region(&mut self) -> Result<Option<Region>, Stop> {
        loop {
            if let Some(region) = self.ready.pop_front() {
                return Ok(Some(region));
            }
            if self.done {
                return Ok(None);
            }
            self.step()?;
        }
    }

    /// What was read, once the reader has handed out its last region.
    pub(crate) fn summary(&self) -> SideRead {
        SideRead {
            audible_ms: self.pass.audible_ms(),
            speech_ms: self.pass.speech_ms(),
            ..self.read
        }
    }

    /// The error the pass's VAD failed with, if it did.
    pub(crate) fn vad_error(&self) -> Option<&EngineError> {
        self.pass.vad_error()
    }

    /// Reads one chunk into the pass, or ends the side after the last.
    fn step(&mut self) -> Result<(), Stop> {
        let Some(chunk) = self.chunks.next() else {
            self.end_run()?;
            self.pass.finish(&mut self.fresh);
            self.ready.extend(self.fresh.drain(..));
            self.done = true;
            return Ok(());
        };
        let data = match (chunk.format_estimated, self.audio.read(&chunk)) {
            (false, Ok(data)) => data,
            _ => {
                // The chunk after it starts a new run, placed by its own host time, so what
                // follows a hole stays where it was said.
                self.read.skipped += 1;
                return self.end_run();
            }
        };
        let continues = self.run.as_ref().is_some_and(|(format, _, next)| {
            !chunk.after_gap && *format == chunk.format && *next == chunk.index
        });
        if !continues {
            self.end_run()?;
            let Ok(resampler) = StreamResampler::new(chunk.format.sample_rate) else {
                self.read.skipped += 1;
                return Ok(());
            };
            self.run = Some((chunk.format, resampler, chunk.index));
            if !chunk.host_time_estimated {
                self.place(ns_to_samples(
                    i128::from(chunk.host_time_ns) - i128::from(self.t0_ns),
                ))?;
            }
        }
        let Some((_, resampler, next)) = &mut self.run else {
            return Ok(());
        };
        *next = chunk.index + 1;
        self.mono.clear();
        Downmix::for_channel(self.channel).apply(&data, chunk.format.channels, &mut self.mono);
        let mut out = std::mem::take(&mut self.out);
        out.clear();
        out.reserve(resampler.max_output_frames(self.mono.len()));
        let resampled = resampler.push(&self.mono, &mut out);
        let pushed = match resampled {
            Ok(()) => self.push(&out),
            Err(_) => {
                self.read.skipped += 1;
                self.run = None;
                Ok(())
            }
        };
        self.out = out;
        pushed
    }

    /// Ends the current run: its resampler's last samples go into the pass. A flush that fails
    /// loses the run's last few milliseconds, and counts as a skipped chunk.
    fn end_run(&mut self) -> Result<(), Stop> {
        let Some((_, mut resampler, _)) = self.run.take() else {
            return Ok(());
        };
        let mut out = std::mem::take(&mut self.out);
        out.clear();
        let pushed = match resampler.finish(&mut out) {
            Ok(()) => self.push(&out),
            Err(_) => {
                self.read.skipped += 1;
                Ok(())
            }
        };
        self.out = out;
        pushed
    }

    /// A new run starts at meeting position `at`: silence fills a gap before it, and whatever of
    /// it lies before the meeting's start is dropped. A run that claims to start before the audio
    /// already placed goes on from there instead: audio is never dropped for a clock's sake.
    fn place(&mut self, at: i64) -> Result<(), Stop> {
        if at < 0 && self.at == 0 {
            self.skip = at.unsigned_abs();
            return Ok(());
        }
        let at = at.max(0) as u64;
        let zeros = [0.0f32; GAP_BLOCK];
        while self.at < at {
            let n = (at - self.at).min(GAP_BLOCK as u64) as usize;
            self.push(&zeros[..n])?;
        }
        Ok(())
    }

    fn push(&mut self, samples: &[f32]) -> Result<(), Stop> {
        let drop = self.skip.min(samples.len() as u64) as usize;
        self.skip -= drop as u64;
        let samples = &samples[drop..];
        self.pass.push(samples, &mut self.fresh)?;
        self.at += samples.len() as u64;
        self.ready.extend(self.fresh.drain(..));
        Ok(())
    }
}

/// A far end's speech regions as a diarizer's input: pulled from a [`SideReader`] one region at a
/// time, joined end to end (no silence between them), in windows of at most
/// [`MAX_DIARIZE_WINDOW`]. It keeps only the region being handed over and, per region, where it
/// sits (three numbers), to map the turns back onto the meeting.
pub(crate) struct RegionWindows<'r, 'a> {
    reader: &'r mut SideReader<'a>,
    current: Option<Region>,
    offset: usize,
    /// Per region: (start in the joined stream, start in the meeting, length), in samples.
    pub(crate) pieces: Vec<(u64, u64, u64)>,
    joined: u64,
    /// Why reading stopped early, if it did: the diarizer saw the end of its input there.
    pub(crate) failed: Option<Stop>,
}

impl<'r, 'a> RegionWindows<'r, 'a> {
    pub(crate) fn new(reader: &'r mut SideReader<'a>) -> Self {
        Self {
            reader,
            current: None,
            offset: 0,
            pieces: Vec::new(),
            joined: 0,
            failed: None,
        }
    }
}

impl DiarizeInput for RegionWindows<'_, '_> {
    fn next_window(&mut self) -> Option<&[f32]> {
        while self
            .current
            .as_ref()
            .is_none_or(|r| self.offset >= r.audio.len())
        {
            self.current = None;
            match self.reader.next_region() {
                Ok(Some(region)) => {
                    let len = region.audio.len() as u64;
                    self.pieces.push((self.joined, region.start, len));
                    self.joined += len;
                    self.current = Some(region);
                    self.offset = 0;
                }
                Ok(None) => return None,
                Err(stop) => {
                    self.failed = Some(stop);
                    return None;
                }
            }
        }
        let region = self.current.as_ref()?;
        let start = self.offset;
        let end = (start + MAX_DIARIZE_WINDOW).min(region.audio.len());
        self.offset = end;
        Some(&region.audio[start..end])
    }
}

/// Transcribes one stretch of levelled speech, starting at meeting sample `start`, into segments
/// on the meeting's timeline, and accounts for it in `report`.
pub(crate) fn transcribe(
    ctx: &Pass<'_>,
    channel: Channel,
    audio: &[f32],
    start: u64,
    report: &mut ChannelPass,
) -> Result<Vec<Segment>, Stop> {
    if ctx.cancel.is_cancelled() {
        return Err(Stop::Cancelled);
    }
    let (start_ms, end_ms) = (
        samples_to_ms(start),
        samples_to_ms(start + audio.len() as u64),
    );
    report.regions += 1;
    let options = TranscribeOptions {
        channel,
        context: ctx.context.map(str::to_owned),
        cancel: ctx.cancel.clone(),
    };
    let transcript = match ctx.engine.transcribe(audio, &options) {
        Ok(t) => t,
        Err(EngineError::Cancelled) => return Err(Stop::Cancelled),
        Err(error) => {
            report.failed_regions += 1;
            log::warn!(
                "meeting final pass: the {channel:?} region at {start_ms} ms failed: {error}"
            );
            (ctx.emit)(MeetingEvent::Warning(MeetingWarning::FinalEngineFailed {
                channel,
                start_ms,
                end_ms,
                error,
            }));
            return Ok(Vec::new());
        }
    };
    let segments: Vec<Segment> = transcript
        .segments
        .into_iter()
        .filter(|s| !s.text.trim().is_empty())
        .map(|s| {
            let seg_start = (start_ms + s.start_ms).min(end_ms);
            Segment {
                channel,
                start_ms: seg_start,
                end_ms: (start_ms + s.end_ms).clamp(seg_start, end_ms),
                text: s.text.trim().to_owned(),
                speaker: None,
            }
        })
        .collect();
    if segments.is_empty() {
        report.empty_regions += 1;
        log::warn!(
            "meeting final pass: the VAD heard speech in the {channel:?} region at {start_ms} ms and the engine returned no words"
        );
        (ctx.emit)(MeetingEvent::Warning(MeetingWarning::EmptySpeechRegion {
            channel,
            start_ms,
            end_ms,
        }));
    }
    report.word_count += ink_core::store::word_count(&segments);
    Ok(segments)
}

/// A new, empty report for `channel`.
pub(crate) fn report(channel: Channel) -> ChannelPass {
    ChannelPass {
        channel,
        chunks_written: None,
        chunks: 0,
        captured_ms: 0,
        audible_ms: 0,
        regions: 0,
        empty_regions: 0,
        failed_regions: 0,
        word_count: 0,
        speech_ms: 0,
    }
}

/// Transcribes one region. With `labels` (rule 5's turns, on the meeting's timeline), the region
/// is first cut where the speaker changes, so no engine call spans two speakers, and each segment
/// gets the speaker holding most of it.
pub(crate) fn transcribe_region(
    ctx: &Pass<'_>,
    channel: Channel,
    region: &Region,
    labels: Option<&[SpeakerTurn]>,
    report: &mut ChannelPass,
) -> Result<Vec<Segment>, Stop> {
    let cuts = labels.map_or_else(Vec::new, |turns| {
        speaker_changes(region.start_ms()..region.end_ms(), turns)
    });
    let ends = cuts
        .iter()
        .map(|&ms| ((ms * 16).saturating_sub(region.start) as usize).min(region.audio.len()))
        .chain(std::iter::once(region.audio.len()));
    let mut segments = Vec::new();
    let mut from = 0usize;
    for to in ends {
        if to <= from {
            continue;
        }
        let start = region.start + from as u64;
        let mut piece = transcribe(ctx, channel, &region.audio[from..to], start, report)?;
        if let Some(turns) = labels {
            for s in &mut piece {
                s.speaker = speaker_of(s.start_ms..s.end_ms.max(s.start_ms + 1), turns);
            }
        }
        segments.extend(piece);
        from = to;
    }
    Ok(segments)
}
