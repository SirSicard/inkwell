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
//! # Holding the engine to the VAD
//!
//! Every region the VAD found goes to the engine, and every one that comes back without words is
//! reported ([`MeetingWarning::EmptySpeechRegion`]): an engine returns a segment for everything it
//! is given, so its segments say nothing about where the speech was, and only this cross-check
//! notices speech that was dropped.

use ink_audio::{ChunkStore, Downmix, StreamResampler, WindowError};
use ink_core::{
    CancelToken, Channel, Diarizer, EngineError, OfflineEngine, Segment, SpeakerTurn,
    TranscribeOptions,
};

use super::diarize::{rule5, speaker_changes, speaker_of, to_meeting};
use super::events::{ChannelPass, Diarization, MeetingEvent, MeetingWarning};
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

/// Zeros fed per push while filling a gap.
const GAP_BLOCK: usize = 16_000;

/// Reads `channel`'s chunks onto the meeting timeline (host time `t0_ns` is sample 0), through
/// `pass`, handing each region to `on_region` as it closes. Returns how many chunk files could not
/// be used (unreadable, or their audio failed to resample): the pass has a gap at each.
pub(crate) fn read_side(
    audio: &ChunkStore,
    channel: Channel,
    t0_ns: u64,
    pass: &mut SpeechPass,
    on_region: &mut dyn FnMut(Region) -> Result<(), Stop>,
) -> Result<usize, Stop> {
    let list = audio.chunks(channel).map_err(Stop::Chunks)?;
    let mut skipped = list.unreadable.len();
    let mut feed = Feed {
        pass,
        on_region,
        at: 0,
        skip: 0,
        regions: Vec::new(),
    };
    // The run of chunks being resampled as one stream: its format, its resampler, and the index
    // the next chunk must have to continue it.
    let mut run: Option<(ink_core::StreamFormat, StreamResampler, u64)> = None;
    let (mut mono, mut out) = (Vec::new(), Vec::new());
    for chunk in &list.chunks {
        let data = match (chunk.format_estimated, audio.read(chunk)) {
            (false, Ok(data)) => data,
            _ => {
                // The chunk after it starts a new run, placed by its own host time, so what
                // follows a hole stays where it was said.
                skipped += 1;
                skipped += end_run(&mut run, &mut out, &mut feed)?;
                continue;
            }
        };
        let continues = run.as_ref().is_some_and(|(format, _, next)| {
            !chunk.after_gap && *format == chunk.format && *next == chunk.index
        });
        if !continues {
            skipped += end_run(&mut run, &mut out, &mut feed)?;
            let Ok(resampler) = StreamResampler::new(chunk.format.sample_rate) else {
                skipped += 1;
                continue;
            };
            run = Some((chunk.format, resampler, chunk.index));
            if !chunk.host_time_estimated {
                feed.place(ns_to_samples(
                    i128::from(chunk.host_time_ns) - i128::from(t0_ns),
                ))?;
            }
        }
        let Some((_, resampler, next)) = &mut run else {
            continue;
        };
        *next = chunk.index + 1;
        mono.clear();
        Downmix::for_channel(channel).apply(&data, chunk.format.channels, &mut mono);
        out.clear();
        out.reserve(resampler.max_output_frames(mono.len()));
        if resampler.push(&mono, &mut out).is_err() {
            skipped += 1;
            run = None;
            continue;
        }
        feed.push(&out)?;
    }
    skipped += end_run(&mut run, &mut out, &mut feed)?;
    feed.pass.finish(&mut feed.regions);
    for region in feed.regions.drain(..) {
        (feed.on_region)(region)?;
    }
    Ok(skipped)
}

/// Ends the current run: its resampler's last samples go into the pass. Returns 1 when that flush
/// failed (the run's last few milliseconds are lost), else 0.
fn end_run(
    run: &mut Option<(ink_core::StreamFormat, StreamResampler, u64)>,
    out: &mut Vec<f32>,
    feed: &mut Feed<'_>,
) -> Result<usize, Stop> {
    let Some((_, mut resampler, _)) = run.take() else {
        return Ok(0);
    };
    out.clear();
    match resampler.finish(out) {
        Ok(()) => {
            feed.push(out)?;
            Ok(0)
        }
        Err(_) => Ok(1),
    }
}

/// Audio on its way into the pass, placed on the meeting timeline.
struct Feed<'a> {
    pass: &'a mut SpeechPass,
    on_region: &'a mut dyn FnMut(Region) -> Result<(), Stop>,
    /// The next sample's position in the meeting.
    at: u64,
    /// Samples still to drop because they come from before the start.
    skip: u64,
    regions: Vec<Region>,
}

impl Feed<'_> {
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
        self.pass.push(samples, &mut self.regions)?;
        self.at += samples.len() as u64;
        for region in self.regions.drain(..) {
            (self.on_region)(region)?;
        }
        Ok(())
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
    report.speech_ms += end_ms - start_ms;
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
        regions: 0,
        empty_regions: 0,
        failed_regions: 0,
        word_count: 0,
        speech_ms: 0,
    }
}

/// The far end's regions, diarized, then transcribed piece by piece with a speaker each. Without
/// a diarizer (or when it fails) the regions are transcribed whole and unlabelled.
pub(crate) fn far_with_speakers(
    ctx: &Pass<'_>,
    diarizer: &dyn Diarizer,
    regions: Vec<Region>,
    report: &mut ChannelPass,
) -> Result<(Vec<Segment>, Option<Diarization>), Stop> {
    let turns = match diarize(ctx, diarizer, &regions) {
        Ok(turns) => Some(turns),
        Err(EngineError::Cancelled) => return Err(Stop::Cancelled),
        Err(error) => {
            log::warn!("meeting final pass: diarization failed: {error}");
            (ctx.emit)(MeetingEvent::Warning(MeetingWarning::DiarizationFailed(
                error,
            )));
            None
        }
    };
    let decided = turns.as_deref().map(rule5);
    let labels = decided.as_ref().and_then(|r| r.turns.as_deref());
    let mut segments = Vec::new();
    for region in &regions {
        let cuts = labels.map_or_else(Vec::new, |turns| {
            speaker_changes(region.start_ms()..region.end_ms(), turns)
        });
        let mut from = 0usize;
        let ends = cuts
            .iter()
            .map(|&ms| ((ms * 16).saturating_sub(region.start) as usize).min(region.audio.len()))
            .chain(std::iter::once(region.audio.len()));
        for to in ends {
            if to <= from {
                continue;
            }
            let start = region.start + from as u64;
            let mut piece = transcribe(ctx, Channel::Far, &region.audio[from..to], start, report)?;
            if let Some(turns) = labels {
                for s in &mut piece {
                    s.speaker = speaker_of(s.start_ms..s.end_ms.max(s.start_ms + 1), turns);
                }
            }
            segments.extend(piece);
            from = to;
        }
    }
    let diarization = decided.map(|r| Diarization {
        clusters: r.clusters,
        substantial: r.substantial.len(),
        labelled: r.turns.is_some(),
        attributed: segments.iter().filter(|s| s.speaker.is_some()).count(),
    });
    Ok((segments, diarization))
}

/// Diarizes the regions end to end and maps the turns back onto the meeting.
fn diarize(
    ctx: &Pass<'_>,
    diarizer: &dyn Diarizer,
    regions: &[Region],
) -> Result<Vec<SpeakerTurn>, EngineError> {
    let total: usize = regions.iter().map(|r| r.audio.len()).sum();
    if total == 0 {
        return Ok(Vec::new());
    }
    // The diarizer takes one buffer: the far end's speech, not its silence (see the report for
    // what a long meeting costs here).
    let mut speech = Vec::with_capacity(total);
    let mut pieces = Vec::with_capacity(regions.len());
    for r in regions {
        pieces.push((speech.len() as u64, r.start, r.audio.len() as u64));
        speech.extend_from_slice(&r.audio);
    }
    let turns = diarizer.diarize(&speech, ctx.cancel)?;
    Ok(to_meeting(&turns, &pieces))
}
