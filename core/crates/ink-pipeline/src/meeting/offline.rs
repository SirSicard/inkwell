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

use ink_audio::{ChunkInfo, ChunkStore, Downmix, LevelMeter, StreamResampler, WindowError};
use ink_core::{
    CancelToken, Channel, DiarizeInput, EngineError, MAX_DIARIZE_WINDOW, OfflineEngine, Segment,
    SpeakerTurn, StreamFormat, TranscribeOptions,
};
use ink_echo::{Alignment, CancellerConfig, EchoCanceller, EchoError, PathFinder, PathReport};

use super::diarize::{speaker_changes, speaker_of};
use super::echo::{EchoEvidence, ErleMeter, Following, far_playing};
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
    /// Echo cancellation refused the audio (the sides out of step: a bug, since they are fed a
    /// second at a time).
    Echo(EchoError),
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
    /// Every sample the readable chunks hold is exactly zero, and there was at least one: the
    /// side captured no data at all (a denied capture that still called back).
    pub only_zeros: bool,
}

/// Zeros handed out at once while filling a gap: a gap of minutes never becomes one buffer.
const GAP_BLOCK: usize = 16_000;

/// One side's chunks, read back onto the meeting timeline (host time `t0_ns` is sample 0) and
/// handed out in order as 16 kHz mono, gaps filled with digital silence: at most a chunk's audio,
/// or a second of a gap's silence, at a time.
pub(crate) struct SideSamples<'a> {
    audio: &'a ChunkStore,
    channel: Channel,
    t0_ns: u64,
    chunks: std::vec::IntoIter<ChunkInfo>,
    /// The run of chunks being resampled as one stream: its format, its resampler, and the index
    /// the next chunk must have to continue it.
    run: Option<(StreamFormat, StreamResampler, u64)>,
    /// The next sample's position in the meeting.
    at: u64,
    /// Samples still to drop because they come from before the start.
    skip: u64,
    /// Silence still to hand out before `stashed`: a gap.
    zeros: u64,
    /// A chunk read and placed after a gap, waiting for the gap's silence to go first.
    stashed: Option<(ChunkInfo, Vec<f32>)>,
    mono: Vec<f32>,
    out: Vec<f32>,
    done: bool,
    read: SideRead,
    /// Every sample decoded, as stored: for telling a side of exact zeros from a quiet one.
    stored: LevelMeter,
}

impl<'a> SideSamples<'a> {
    /// The samples of `channel`'s chunks in `audio`. Lists the chunks now.
    pub(crate) fn open(audio: &'a ChunkStore, channel: Channel, t0_ns: u64) -> Result<Self, Stop> {
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
            only_zeros: false,
        };
        Ok(Self {
            audio,
            channel,
            t0_ns,
            chunks: list.chunks.into_iter(),
            run: None,
            at: 0,
            skip: 0,
            zeros: 0,
            stashed: None,
            mono: Vec::new(),
            out: Vec::new(),
            done: false,
            read,
            stored: LevelMeter::new(),
        })
    }

    /// Appends the next samples to `buf`: at least one, unless the side is read to its end, when
    /// it returns false.
    pub(crate) fn next_into(&mut self, buf: &mut Vec<f32>) -> Result<bool, Stop> {
        let from = buf.len();
        while buf.len() == from {
            if self.zeros > 0 {
                let n = self.zeros.min(GAP_BLOCK as u64) as usize;
                self.zeros -= n as u64;
                self.emit(&[0.0; GAP_BLOCK][..n], buf);
            } else if let Some((chunk, data)) = self.stashed.take() {
                self.resample(&chunk, &data, buf);
            } else if self.done {
                return Ok(false);
            } else {
                self.step(buf)?;
            }
        }
        Ok(true)
    }

    /// What was read: the chunks, and whether every stored sample was zero.
    pub(crate) fn summary(&self) -> SideRead {
        SideRead {
            only_zeros: self.stored.all_zero(),
            ..self.read
        }
    }

    /// Reads one chunk, or ends the side after the last.
    fn step(&mut self, buf: &mut Vec<f32>) -> Result<(), Stop> {
        let Some(chunk) = self.chunks.next() else {
            self.end_run(buf);
            self.done = true;
            return Ok(());
        };
        let data = match (chunk.format_estimated, self.audio.read(&chunk)) {
            (false, Ok(data)) => data,
            _ => {
                // The chunk after it starts a new run, placed by its own host time, so what
                // follows a hole stays where it was said.
                self.read.skipped += 1;
                self.end_run(buf);
                return Ok(());
            }
        };
        self.stored.add(&data);
        let continues = self.run.as_ref().is_some_and(|(format, _, next)| {
            !chunk.after_gap && *format == chunk.format && *next == chunk.index
        });
        if continues {
            self.resample(&chunk, &data, buf);
            return Ok(());
        }
        self.end_run(buf);
        let Ok(resampler) = StreamResampler::new(chunk.format.sample_rate) else {
            self.read.skipped += 1;
            return Ok(());
        };
        self.run = Some((chunk.format, resampler, chunk.index));
        if !chunk.host_time_estimated {
            self.place(ns_to_samples(
                i128::from(chunk.host_time_ns) - i128::from(self.t0_ns),
            ));
        }
        // The gap's silence goes first.
        self.stashed = Some((chunk, data));
        Ok(())
    }

    /// Downmixes and resamples one chunk of the current run into `buf`.
    fn resample(&mut self, chunk: &ChunkInfo, data: &[f32], buf: &mut Vec<f32>) {
        let Some((_, resampler, next)) = &mut self.run else {
            return;
        };
        *next = chunk.index + 1;
        self.mono.clear();
        Downmix::for_channel(self.channel).apply(data, chunk.format.channels, &mut self.mono);
        let mut out = std::mem::take(&mut self.out);
        out.clear();
        out.reserve(resampler.max_output_frames(self.mono.len()));
        match resampler.push(&self.mono, &mut out) {
            Ok(()) => self.emit(&out, buf),
            Err(_) => {
                self.read.skipped += 1;
                self.run = None;
            }
        }
        self.out = out;
    }

    /// Ends the current run: its resampler's last samples go out. A flush that fails loses the
    /// run's last few milliseconds, and counts as a skipped chunk.
    fn end_run(&mut self, buf: &mut Vec<f32>) {
        let Some((_, mut resampler, _)) = self.run.take() else {
            return;
        };
        let mut out = std::mem::take(&mut self.out);
        out.clear();
        match resampler.finish(&mut out) {
            Ok(()) => self.emit(&out, buf),
            Err(_) => self.read.skipped += 1,
        }
        self.out = out;
    }

    /// A new run starts at meeting position `at`: silence fills a gap before it, and whatever of
    /// it lies before the meeting's start is dropped. A run that claims to start before the audio
    /// already placed goes on from there instead: audio is never dropped for a clock's sake.
    fn place(&mut self, at: i64) {
        if at < 0 && self.at == 0 {
            self.skip = at.unsigned_abs();
            return;
        }
        self.zeros = (at.max(0) as u64).saturating_sub(self.at);
    }

    /// Hands samples out, less those from before the start.
    fn emit(&mut self, samples: &[f32], buf: &mut Vec<f32>) {
        let drop = self.skip.min(samples.len() as u64) as usize;
        self.skip -= drop as u64;
        let samples = &samples[drop..];
        buf.extend_from_slice(samples);
        self.at += samples.len() as u64;
    }
}

/// One side's samples ([`SideSamples`]) run through a [`SpeechPass`], handed out a region at a
/// time as the caller pulls them.
///
/// It holds one chunk of audio while decoding it, the pass's own buffers (about a window), and the
/// regions of the last window the pass closed; never the side's recording.
pub(crate) struct SideReader<'a> {
    samples: SideSamples<'a>,
    pass: SpeechPass,
    buf: Vec<f32>,
    ready: VecDeque<Region>,
    fresh: Vec<Region>,
    done: bool,
}

impl<'a> SideReader<'a> {
    /// A reader over `channel`'s chunks in `audio`, cut by `pass`. Lists the chunks now.
    pub(crate) fn open(
        audio: &'a ChunkStore,
        channel: Channel,
        t0_ns: u64,
        pass: SpeechPass,
    ) -> Result<Self, Stop> {
        Ok(Self {
            samples: SideSamples::open(audio, channel, t0_ns)?,
            pass,
            buf: Vec::new(),
            ready: VecDeque::new(),
            fresh: Vec::new(),
            done: false,
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
            self.buf.clear();
            if self.samples.next_into(&mut self.buf)? {
                self.pass.push(&self.buf, &mut self.fresh)?;
            } else {
                self.pass.finish(&mut self.fresh);
                self.done = true;
            }
            self.ready.extend(self.fresh.drain(..));
        }
    }

    /// What was read, once the reader has handed out its last region.
    pub(crate) fn summary(&self) -> SideRead {
        SideRead {
            audible_ms: self.pass.audible_ms(),
            speech_ms: self.pass.speech_ms(),
            ..self.samples.summary()
        }
    }

    /// The error the pass's VAD failed with, if it did.
    pub(crate) fn vad_error(&self) -> Option<&EngineError> {
        self.pass.vad_error()
    }
}

/// Audio pushed into the path search or the canceller at once, per side: a second, so neither
/// side ever runs more than about that far ahead of the other (both refuse 10 s).
const PIECE: usize = 16_000;

/// A side's samples with a read position, for feeding two sides in step.
struct Feed<'a> {
    samples: SideSamples<'a>,
    buf: Vec<f32>,
    off: usize,
    ended: bool,
    /// Samples handed on so far.
    fed: u64,
}

impl<'a> Feed<'a> {
    fn new(samples: SideSamples<'a>) -> Self {
        Self {
            samples,
            buf: Vec::new(),
            off: 0,
            ended: false,
            fed: 0,
        }
    }

    /// Up to a second of the samples before `until` (counted from the side's start), silence
    /// once the side has ended: empty only at `until`.
    fn exactly(&mut self, until: u64) -> Result<&[f32], Stop> {
        static ZEROS: [f32; PIECE] = [0.0; PIECE];
        let want = until.saturating_sub(self.fed).min(PIECE as u64) as usize;
        if want == 0 {
            return Ok(&[]);
        }
        let fed = self.fed;
        if !self.take(want)?.is_empty() {
            let n = (self.fed - fed) as usize;
            return Ok(&self.buf[self.off - n..self.off]);
        }
        self.fed += want as u64;
        Ok(&ZEROS[..want])
    }

    /// Up to `max` next samples, empty once the side has ended.
    fn take(&mut self, max: usize) -> Result<&[f32], Stop> {
        if self.off == self.buf.len() && !self.ended {
            self.buf.clear();
            self.off = 0;
            self.ended = !self.samples.next_into(&mut self.buf)?;
        }
        let n = max.min(self.buf.len() - self.off);
        let piece = &self.buf[self.off..self.off + n];
        self.off += n;
        self.fed += n as u64;
        Ok(piece)
    }
}

/// The echo path over the whole recording: both sides read back from their chunks, from the
/// meeting's start, into a fresh [`PathFinder`] (never the live one), and its consensus once at
/// the end. `None` when either side's chunks cannot be listed: the passes that follow report that.
pub(crate) fn fit_path(
    audio: &ChunkStore,
    t0_ns: u64,
    cancel: &CancelToken,
) -> Result<Option<(PathReport, Following)>, Stop> {
    let (Ok(mic), Ok(far)) = (
        SideSamples::open(audio, Channel::Mic, t0_ns),
        SideSamples::open(audio, Channel::Far, t0_ns),
    ) else {
        return Ok(None);
    };
    let (mut mic, mut far) = (Feed::new(mic), Feed::new(far));
    let mut finder = PathFinder::new();
    // Whether the mic follows the far end, for a path the search cannot fit.
    let mut following = Following::default();
    loop {
        if cancel.is_cancelled() {
            return Err(Stop::Cancelled);
        }
        let m = mic.take(PIECE)?;
        if m.is_empty() {
            break;
        }
        finder.push(m, &[]).map_err(Stop::Echo)?;
        following.push_mic(m);
        // The far end up to where the mic is: past the end of either side there is nothing to
        // compare.
        while far.fed < mic.fed {
            let want = (mic.fed - far.fed) as usize;
            let f = far.take(want.min(PIECE))?;
            if f.is_empty() {
                return Ok(Some((finder.estimate(), following)));
            }
            finder.push(&[], f).map_err(Stop::Echo)?;
            following.push_far(f);
        }
    }
    Ok(Some((finder.estimate(), following)))
}

/// Recorded audio AEC3 is run through once before the pass, its output discarded, samples: 20 s.
/// AEC3 needs about 10 s of far-end audio to converge (11–12 dB ERLE over the first 10 s of the
/// gate's real take, 24 dB after), and while it converges its full output still carries speech the
/// VAD hears, so the linear output there would be transcribed as the user's. The final pass has
/// the whole recording: it adapts AEC3 on the meeting's own start, then cancels from the start.
pub(crate) const PRIME: usize = 20 * 16_000;

/// Both sides read back from their chunks and put through [`EchoCanceller`] along a path, in
/// step, a second of the mic at a time, from the meeting's start: AEC3 first adapted on the first
/// [`PRIME`] samples (see there), so the frames handed out are numbered from the meeting's start.
pub(crate) struct Cancelled<'a> {
    mic: Feed<'a>,
    far: Feed<'a>,
    canceller: EchoCanceller,
    /// Frames of the priming run still coming out of AEC3, to be dropped.
    primed_frames: u64,
    done: bool,
}

impl<'a> Cancelled<'a> {
    /// Opens both sides (twice: the priming run reads their starts), and primes AEC3.
    pub(crate) fn open(audio: &'a ChunkStore, t0_ns: u64, path: Alignment) -> Result<Self, Stop> {
        let mut canceller =
            EchoCanceller::new(path, CancellerConfig::default()).map_err(Stop::Echo)?;
        // The priming run: the first PRIME mic samples, and the far end's samples that line up
        // with them (PRIME / (1 + drift) of them), each padded with silence to that length. The
        // real run then follows on both at once, still on the path: mic sample PRIME + k meets
        // far sample PRIME / (1 + drift) + n exactly where mic sample k meets far sample n, to
        // within half a sample.
        let far_prime = (PRIME as f64 / (1.0 + path.drift)).round() as u64;
        let mut mic = Feed::new(SideSamples::open(audio, Channel::Mic, t0_ns)?);
        let mut far = Feed::new(SideSamples::open(audio, Channel::Far, t0_ns)?);
        let mut dropped = 0u64;
        let mut drop = |_: &ink_echo::EchoFrame| dropped += 1;
        while mic.fed < PRIME as u64 || far.fed < far_prime {
            let m = mic.exactly(PRIME as u64)?;
            canceller.push(m, &[], &mut drop).map_err(Stop::Echo)?;
            let f = far.exactly(far_prime)?;
            canceller.push(&[], f, &mut drop).map_err(Stop::Echo)?;
        }
        Ok(Self {
            mic: Feed::new(SideSamples::open(audio, Channel::Mic, t0_ns)?),
            far: Feed::new(SideSamples::open(audio, Channel::Far, t0_ns)?),
            canceller,
            primed_frames: (PRIME / ink_echo::FRAME) as u64 - dropped,
            done: false,
        })
    }

    /// A second of the mic, and the far end up to the same point (silence after its end); every
    /// frame that comes out goes to `on_frame`, numbered from the meeting's start. After the
    /// mic's last sample it flushes the canceller and returns false.
    pub(crate) fn step(
        &mut self,
        mut on_frame: impl FnMut(&ink_echo::EchoFrame),
    ) -> Result<bool, Stop> {
        if self.done {
            return Ok(false);
        }
        let skip = &mut self.primed_frames;
        let shift = (PRIME / ink_echo::FRAME) as u64;
        let mut take = |f: &ink_echo::EchoFrame| {
            if *skip > 0 {
                *skip -= 1;
                return;
            }
            let mut frame = f.clone();
            frame.index -= shift;
            on_frame(&frame);
        };
        let (mic, far, canceller) = (&mut self.mic, &mut self.far, &mut self.canceller);
        let m = mic.take(PIECE)?;
        if m.is_empty() {
            canceller.finish(&mut take).map_err(Stop::Echo)?;
            self.done = true;
            return Ok(false);
        }
        canceller.push(m, &[], &mut take).map_err(Stop::Echo)?;
        // The far end up to the same point: silence once it has ended.
        while far.fed < mic.fed {
            let f = far.exactly(mic.fed)?;
            canceller.push(&[], f, &mut take).map_err(Stop::Echo)?;
        }
        Ok(true)
    }

    /// What reading the mic found.
    pub(crate) fn mic_summary(&self) -> SideRead {
        self.mic.samples.summary()
    }
}

/// The mic along a found echo path ([`Cancelled`]), then a paired [`SpeechPass`] that judges
/// AEC3's full output and cuts its regions from the linear output. It hands out regions as the
/// caller pulls them, and keeps the VAD's verdicts for the duplicate-line check, and the ERLE.
///
/// It holds a chunk of each side, a second of the canceller's output, and the pass's buffers:
/// never the recording.
pub(crate) struct EchoReader<'a> {
    cancelled: Cancelled<'a>,
    pass: SpeechPass,
    erle: ErleMeter,
    /// Per frame from the meeting's start: whether the far end played.
    far: Vec<bool>,
    full: Vec<f32>,
    linear: Vec<f32>,
    mic: Vec<f32>,
    ready: VecDeque<Region>,
    fresh: Vec<Region>,
    done: bool,
}

impl<'a> EchoReader<'a> {
    /// A reader over the mic, cancelled along `path` with the far end as reference, cut by
    /// `pass` (a [`SpeechPass::paired`]). Lists both sides' chunks and primes AEC3 now.
    pub(crate) fn open(
        audio: &'a ChunkStore,
        t0_ns: u64,
        path: Alignment,
        pass: SpeechPass,
    ) -> Result<Self, Stop> {
        Ok(Self {
            cancelled: Cancelled::open(audio, t0_ns, path)?,
            pass,
            erle: ErleMeter::default(),
            far: Vec::new(),
            full: Vec::new(),
            linear: Vec::new(),
            mic: Vec::new(),
            ready: VecDeque::new(),
            fresh: Vec::new(),
            done: false,
        })
    }

    /// The next speech region of the linear output, in order, or `None` once the mic is read.
    pub(crate) fn next_region(&mut self) -> Result<Option<Region>, Stop> {
        loop {
            if let Some(region) = self.ready.pop_front() {
                return Ok(Some(region));
            }
            if self.done {
                return Ok(None);
            }
            let (full, linear, mic, erle, far) = (
                &mut self.full,
                &mut self.linear,
                &mut self.mic,
                &mut self.erle,
                &mut self.far,
            );
            let more = self.cancelled.step(|f| {
                erle.observe(f);
                far.push(far_playing(f));
                full.extend_from_slice(&f.full[..f.len]);
                linear.extend_from_slice(&f.linear[..f.len]);
                mic.extend_from_slice(&f.mic[..f.len]);
            })?;
            self.pass
                .push_paired(&self.full, &self.linear, &self.mic, &mut self.fresh)?;
            self.full.clear();
            self.linear.clear();
            self.mic.clear();
            if !more {
                self.pass.finish(&mut self.fresh);
                self.done = true;
            }
            self.ready.extend(self.fresh.drain(..));
        }
    }

    /// What was read (the mic's chunks; audible and speech time on the full output), once the
    /// reader has handed out its last region.
    pub(crate) fn summary(&self) -> SideRead {
        SideRead {
            audible_ms: self.pass.audible_ms(),
            speech_ms: self.pass.speech_ms(),
            ..self.cancelled.mic_summary()
        }
    }

    /// The error the pass's VAD failed with, if it did.
    pub(crate) fn vad_error(&self) -> Option<&EngineError> {
        self.pass.vad_error()
    }

    /// The VAD's verdicts over the full output, and the ERLE measured: what the duplicate-line
    /// check and the report need once the mic is read.
    pub(crate) fn into_evidence(mut self) -> (EchoEvidence, ErleMeter) {
        let heard = self.pass.take_evidence();
        (EchoEvidence::new(heard, self.far), self.erle)
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
        live: false,
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
        backlogged_finals: 0,
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
