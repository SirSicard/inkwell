//! Nemotron-3-Diarization through NeMo-Speech.cpp's C API (`engine-nemo`): ink-core's
//! [`Diarizer`] for the far end (architecture rules 5 and 10).
//!
//! - **Final pass:** [`Diarizer::diarize`] runs the `v3-offline` preset (larger chunks and caches
//!   of the same streaming state machine; the stateless full-attention path is limited to a few
//!   minutes of audio) over the whole channel, with the library's default segmentation.
//! - **Live labels:** [`Diarizer::open_stream`] runs the model's default low-latency V3 preset and
//!   reports each turn once it can no longer change.
//!
//! # Building
//!
//! The library is not built by cargo. `native/build-nemo-speech.sh` builds and installs
//! NeMo-Speech.cpp at the pinned commit `97a15af`; `build.rs` links the install prefix named by
//! `NEMO_SPEECH_DIR` and refuses headers from any other commit, because the declarations below are
//! written by hand against that commit's `nemo_speech/diar.h`. NeMo links its own ggml as separate
//! shared libraries, so none of its ggml symbols enter the Rust link: this adapter is indifferent
//! to how the llama.cpp adapter builds its ggml, as long as the two sets of shared libraries are
//! not installed under the same names in one directory.
//!
//! # Threads, and what the header promises
//!
//! Every call here is a **worker** call. The pinned `nemo_speech/diar.h` says, at lines 84-86:
//!
//! > The model handle must outlive every stream opened from it. Streams are single-threaded by
//! > contract; independent streams may run on different threads (compute serializes internally).
//!
//! That is all it says about threads, so the types below claim no more:
//!
//! - A stream is `Send` but not `Sync`: one thread at a time uses it, and it may move between
//!   threads. Its error message is read on the thread that made the failing call
//!   (`asr.h:290-291`: "Thread-local last error message for the most recent failed call on this
//!   thread").
//! - The model handle is used only under a mutex (open a stream, destroy), so it is never used
//!   from two threads at once. The header gives it no thread affinity, so it may be used and
//!   destroyed from any thread. Streams opened from it run concurrently, as lines 85-86 allow.

#![warn(clippy::undocumented_unsafe_blocks)]

use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::fmt;
use std::path::Path;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use ink_core::{
    CANONICAL_RATE, CancelToken, Diarizer, EngineError, EngineInfo, EngineStream, EventSink,
    SpeakerId, SpeakerTurn,
};

use crate::lock;
use crate::model_dir::ModelDir;
use crate::registry::EngineRow;
use crate::residency::Loader;

/// The preset the final pass runs.
pub const OFFLINE_PRESET: &str = "v3-offline";

/// [`OFFLINE_PRESET`] as the C string the library takes.
const OFFLINE_PRESET_C: &CStr = c"v3-offline";

/// Audio per push in the final pass: 10 s. Cancellation is checked between pushes.
const PUSH_SAMPLES: usize = 10 * CANONICAL_RATE as usize;

/// How far behind the labelled audio a live turn must end before it is reported, in seconds.
/// The library's segmentation (hysteresis, 0.229 s onset pad, 0.079 s offset pad, gaps under
/// 0.296 s filled) can still move a turn's end, or join it to the next, within about 0.6 s of the
/// newest label; a second leaves margin.
const LIVE_SETTLE_S: f64 = 1.0;

/// Where the model runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NemoDevice {
    /// The CPU.
    Cpu,
    /// A GPU by index: Metal on macOS (index 0).
    Gpu(u16),
}

impl NemoDevice {
    fn index(self) -> i32 {
        match self {
            Self::Cpu => -1,
            Self::Gpu(ix) => i32::from(ix),
        }
    }
}

/// The C API of `nemo_speech/diar.h` (and the shared status and error accessor from `asr.h`) at
/// NeMo-Speech.cpp `97a15af`. `build.rs` checks both headers against that commit.
mod ffi {
    use std::ffi::{c_char, c_int};
    use std::marker::{PhantomData, PhantomPinned};

    /// `nemo_speech_diar_model`: opaque.
    #[repr(C)]
    pub struct DiarModel {
        _data: (),
        _marker: PhantomData<(*mut u8, PhantomPinned)>,
    }

    /// `nemo_speech_diar_stream`: opaque.
    #[repr(C)]
    pub struct DiarStream {
        _data: (),
        _marker: PhantomData<(*mut u8, PhantomPinned)>,
    }

    /// `nemo_speech_asr_status`, a C enum (int-sized on every supported target). Kept as an
    /// integer: an unknown value must not be undefined behaviour.
    pub type Status = c_int;
    pub const OK: Status = 0;
    pub const ERROR_INVALID_ARGUMENT: Status = 1;
    pub const ERROR_OUT_OF_MEMORY: Status = 2;
    pub const ERROR_RUNTIME: Status = 3;
    pub const ERROR_CANCELLED: Status = 4;

    /// `nemo_speech_diar_model_config`. Append-only in the C ABI; `size` says how much of it the
    /// caller filled (`asr.h:13-14`).
    #[repr(C)]
    pub struct DiarModelConfig {
        pub size: usize,
        pub model_path: *const c_char,
        pub gpu: i32,
        pub preset: *const c_char,
        pub chunk_frames: i32,
        pub right_context_frames: i32,
        pub left_context_frames: i32,
        pub fifo_frames: i32,
        pub spkcache_frames: i32,
        pub update_period_frames: i32,
    }

    /// `nemo_speech_diar_segment`.
    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default)]
    pub struct DiarSegment {
        pub start_time: f64,
        pub end_time: f64,
        /// 1-based.
        pub speaker: i32,
    }

    /// `nemo_speech_diar_segmentation_config`: only ever passed as NULL (the library's defaults),
    /// so it is declared opaque.
    #[repr(C)]
    pub struct DiarSegmentationConfig {
        _data: (),
        _marker: PhantomData<(*mut u8, PhantomPinned)>,
    }

    // Linked by build.rs, from the prefix NEMO_SPEECH_DIR names.
    unsafe extern "C" {
        pub fn nemo_speech_diar_create(
            cfg: *const DiarModelConfig,
            out: *mut *mut DiarModel,
        ) -> Status;
        pub fn nemo_speech_diar_destroy(model: *mut DiarModel);
        pub fn nemo_speech_diar_num_speakers(model: *const DiarModel) -> i32;
        pub fn nemo_speech_diar_seconds_per_frame(model: *const DiarModel) -> f64;
        pub fn nemo_speech_diar_stream_open(
            model: *mut DiarModel,
            out: *mut *mut DiarStream,
        ) -> Status;
        pub fn nemo_speech_diar_stream_push_f32(
            stream: *mut DiarStream,
            samples: *const f32,
            n_samples: usize,
            sample_rate: i32,
        ) -> Status;
        pub fn nemo_speech_diar_stream_finish(stream: *mut DiarStream) -> Status;
        pub fn nemo_speech_diar_stream_close(stream: *mut DiarStream);
        pub fn nemo_speech_diar_frame_count(stream: *const DiarStream) -> i64;
        pub fn nemo_speech_diar_segments(
            stream: *const DiarStream,
            cfg: *const DiarSegmentationConfig,
            out: *mut DiarSegment,
            capacity: usize,
            count: *mut usize,
        ) -> Status;
        pub fn nemo_speech_asr_last_error() -> *const c_char;
    }
}

/// The error for a failed call, with the library's message for this thread.
fn status_error(what: &str, status: ffi::Status) -> EngineError {
    if status == ffi::ERROR_CANCELLED {
        return EngineError::Cancelled;
    }
    let kind = match status {
        ffi::ERROR_INVALID_ARGUMENT => "invalid argument",
        ffi::ERROR_OUT_OF_MEMORY => "out of memory",
        ffi::ERROR_RUNTIME => "runtime error",
        _ => "unknown status",
    };
    // SAFETY: the library returns a pointer to its thread-local, NUL-terminated message (or
    // NULL), valid until the next call on this thread; it is copied before any other call.
    let message = unsafe {
        let ptr = ffi::nemo_speech_asr_last_error();
        if ptr.is_null() {
            String::new()
        } else {
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    };
    // The library's messages name arguments and resources, never audio (I5).
    EngineError::Failed(format!(
        "NeMo-Speech.cpp {what}: {kind} ({status}): {message}"
    ))
}

fn check(what: &str, status: ffi::Status) -> Result<(), EngineError> {
    if status == ffi::OK {
        Ok(())
    } else {
        Err(status_error(what, status))
    }
}

/// A model handle. Only [`Model`] holds one, behind its mutex.
struct ModelHandle(NonNull<ffi::DiarModel>);

// SAFETY: `diar.h:84-86` (quoted in the module docs) binds threads only for streams and gives the
// model handle no thread affinity. The handle is only ever used through `Model::handle`'s mutex,
// so no two threads use it at once, and it may be used and destroyed from whichever thread holds
// the lock. It is not `Sync`; the mutex provides that.
unsafe impl Send for ModelHandle {}

/// One loaded model with one geometry preset. `Send + Sync` through its mutex.
struct Model {
    /// Every call on the handle (opening a stream, destroying it) holds this lock. The header does
    /// not promise that concurrent opens are safe.
    handle: Mutex<ModelHandle>,
    speakers: i32,
    seconds_per_frame: f64,
}

impl Model {
    fn load(path: &CStr, preset: Option<&CStr>, device: NemoDevice) -> Result<Self, EngineError> {
        let cfg = ffi::DiarModelConfig {
            size: size_of::<ffi::DiarModelConfig>(),
            model_path: path.as_ptr(),
            gpu: device.index(),
            preset: preset.map_or(std::ptr::null(), CStr::as_ptr),
            // `diar.h:39-41`: "Individual geometry overrides in coarse 80 ms encoder frames,
            // applied on top of the preset (<= 0 = keep preset value; left context: < 0 keeps it,
            // 0 is a valid explicit value)". So zero, and -1 for the left context, keep the
            // preset's geometry.
            chunk_frames: 0,
            right_context_frames: 0,
            left_context_frames: -1,
            fifo_frames: 0,
            spkcache_frames: 0,
            update_period_frames: 0,
        };
        let mut out = std::ptr::null_mut();
        // SAFETY: `cfg` and the strings it points to outlive the call, and its `size` covers the
        // whole struct as declared in the pinned header; `out` is a valid place for the handle.
        let status = unsafe { ffi::nemo_speech_diar_create(&cfg, &mut out) };
        // On a failure `out` is not read: the header defines no handle then, so there is nothing
        // this side may free. (At the pinned commit, `c_api.cpp:489` clears `*out` first and the
        // handle is only released to it on success, lines 515-517, so nothing is left behind.)
        check("loading the model", status)?;
        let ptr = NonNull::new(out).ok_or_else(|| {
            EngineError::Failed("NeMo-Speech.cpp returned no model handle".into())
        })?;
        // SAFETY: `ptr` is a live handle from a successful create, not yet shared.
        let (speakers, seconds_per_frame) = unsafe {
            (
                ffi::nemo_speech_diar_num_speakers(ptr.as_ptr()),
                ffi::nemo_speech_diar_seconds_per_frame(ptr.as_ptr()),
            )
        };
        // Built before the check below, so a rejected model is still destroyed.
        let model = Self {
            handle: Mutex::new(ModelHandle(ptr)),
            speakers,
            seconds_per_frame,
        };
        if speakers < 1 || !(seconds_per_frame > 0.0 && seconds_per_frame.is_finite()) {
            return Err(EngineError::Failed(format!(
                "NeMo-Speech.cpp model reports {speakers} speakers at {seconds_per_frame} s per frame"
            )));
        }
        Ok(model)
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        let handle = self
            .handle
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: the handle is live and destroyed once, here. No stream outlives it
        // (`diar.h:84`): each holds an `Arc<Model>`, so this runs after the last one closed.
        unsafe { ffi::nemo_speech_diar_destroy(handle.0.as_ptr()) }
    }
}

/// One diarization stream, closed on drop.
struct Stream {
    ptr: NonNull<ffi::DiarStream>,
    /// Keeps the model alive for as long as the stream (`diar.h:84`). The stream is closed in
    /// `Drop::drop`, which runs before any field is dropped, so this `Arc` is released only
    /// after the close.
    model: Arc<Model>,
    finished: bool,
}

// SAFETY: `diar.h:85`: "Streams are single-threaded by contract". A `Stream` is owned and every
// call takes `&self` or `&mut self` of it, so one thread at a time uses it; it is `Send` (it may
// move between calls) and not `Sync`. Streams run alongside each other, as `diar.h:85-86`
// allows, and never touch the model handle after opening.
unsafe impl Send for Stream {}

impl Stream {
    fn open(model: &Arc<Model>) -> Result<Self, EngineError> {
        let handle = lock(&model.handle);
        let mut out = std::ptr::null_mut();
        // SAFETY: the model handle is live (the `Arc` holds it) and used under its lock; `out` is
        // a valid place for the handle.
        let status = unsafe { ffi::nemo_speech_diar_stream_open(handle.0.as_ptr(), &mut out) };
        drop(handle);
        // On a failure `out` is not read, as for the model: the header defines no handle then.
        // (At the pinned commit, `c_api.cpp:544` clears `*out` first and releases the stream to
        // it only on success, lines 545-547.)
        check("opening a stream", status)?;
        let ptr = NonNull::new(out).ok_or_else(|| {
            EngineError::Failed("NeMo-Speech.cpp returned no stream handle".into())
        })?;
        Ok(Self {
            ptr,
            model: Arc::clone(model),
            finished: false,
        })
    }

    fn push(&mut self, samples: &[f32]) -> Result<(), EngineError> {
        if samples.is_empty() {
            return Ok(());
        }
        // SAFETY: the stream is live and used by this thread alone; `samples` is valid for its
        // length for the duration of the call, and the library copies what it keeps.
        let status = unsafe {
            ffi::nemo_speech_diar_stream_push_f32(
                self.ptr.as_ptr(),
                samples.as_ptr(),
                samples.len(),
                CANONICAL_RATE as i32,
            )
        };
        check("diarizing", status)
    }

    fn finish(&mut self) -> Result<(), EngineError> {
        if self.finished {
            return Ok(());
        }
        // SAFETY: the stream is live and used by this thread alone.
        let status = unsafe { ffi::nemo_speech_diar_stream_finish(self.ptr.as_ptr()) };
        check("finishing a stream", status)?;
        self.finished = true;
        Ok(())
    }

    fn frame_count(&self) -> i64 {
        // SAFETY: the stream is live.
        unsafe { ffi::nemo_speech_diar_frame_count(self.ptr.as_ptr()) }
    }

    /// Every segment so far, under the library's default segmentation, into `out` (its
    /// allocation is reused). `diar.h:131-134`: segments come "sorted by start time", by a
    /// "two-call pattern" (the count, then the copy); the header offers no way to read only the
    /// new ones.
    fn segments_into(&self, out: &mut Vec<ffi::DiarSegment>) -> Result<(), EngineError> {
        let mut count = 0usize;
        // SAFETY: the stream is live; a NULL buffer with capacity 0 asks only for the count,
        // written to a valid place. A NULL config selects the library's defaults.
        let status = unsafe {
            ffi::nemo_speech_diar_segments(
                self.ptr.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
                &mut count,
            )
        };
        check("counting segments", status)?;
        out.clear();
        out.resize(count, ffi::DiarSegment::default());
        let mut written = 0usize;
        // SAFETY: as above, with a buffer of `count` segments; nothing touched the stream since
        // the count was taken (it is used by this thread alone).
        let status = unsafe {
            ffi::nemo_speech_diar_segments(
                self.ptr.as_ptr(),
                std::ptr::null(),
                out.as_mut_ptr(),
                out.len(),
                &mut written,
            )
        };
        check("reading segments", status)?;
        out.truncate(written);
        Ok(())
    }

    /// Every segment so far, as [`segments_into`](Self::segments_into) gives them.
    fn segments(&self) -> Result<Vec<ffi::DiarSegment>, EngineError> {
        let mut out = Vec::new();
        self.segments_into(&mut out)?;
        Ok(out)
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // SAFETY: the handle is live and closed once, here; the model it came from is alive
        // (this stream's `Arc` is released only after this body).
        unsafe { ffi::nemo_speech_diar_stream_close(self.ptr.as_ptr()) }
    }
}

/// A segment as a turn: checked, in milliseconds, speaker `n` (1-based) labelled `spk{n-1}`.
fn turn(segment: &ffi::DiarSegment, speakers: i32) -> Result<SpeakerTurn, EngineError> {
    let ffi::DiarSegment {
        start_time,
        end_time,
        speaker,
    } = *segment;
    let valid = start_time.is_finite()
        && end_time.is_finite()
        && start_time >= 0.0
        && end_time >= start_time
        && (1..=speakers).contains(&speaker);
    if !valid {
        return Err(EngineError::Failed(format!(
            "NeMo-Speech.cpp returned a malformed segment: {start_time}..{end_time} s, speaker {speaker} of {speakers}"
        )));
    }
    Ok(SpeakerTurn {
        speaker: SpeakerId(format!("spk{}", speaker - 1)),
        start_ms: ms(start_time),
        end_ms: ms(end_time),
    })
}

/// Seconds as whole milliseconds (non-negative and finite, checked by the caller).
fn ms(seconds: f64) -> u64 {
    (seconds * 1_000.0).round() as u64
}

fn check_samples(audio: &[f32]) -> Result<(), EngineError> {
    if audio.iter().all(|s| s.is_finite()) {
        Ok(())
    } else {
        Err(EngineError::Failed(
            "audio for the diarizer holds a non-finite sample".into(),
        ))
    }
}

/// Nemotron-3-Diarization on NeMo-Speech.cpp: implements [`Diarizer`]. `Send + Sync`.
///
/// Each preset's model is loaded when first used (the final pass's at the first
/// [`diarize`](Diarizer::diarize), the live one at the first
/// [`open_stream`](Diarizer::open_stream)) and kept until this value is dropped. A failed load
/// is returned and tried again on the next call.
pub struct NemoDiarizer {
    info: EngineInfo,
    path: CString,
    device: NemoDevice,
    offline: Mutex<Option<Arc<Model>>>,
    live: Mutex<Option<Arc<Model>>>,
}

impl fmt::Debug for NemoDiarizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NemoDiarizer")
            .field("id", &self.info.id)
            .field("device", &self.device)
            .finish_non_exhaustive()
    }
}

impl NemoDiarizer {
    /// **Worker.** A diarizer over the GGUF at `model`, described by `info`, on `device`. A
    /// missing file is [`EngineError::ModelMissing`]; nothing is loaded yet.
    pub fn new(model: &Path, info: EngineInfo, device: NemoDevice) -> Result<Self, EngineError> {
        if !model.is_file() {
            return Err(EngineError::ModelMissing(format!(
                "{}: no model file at {}",
                info.id,
                model.display()
            )));
        }
        let path = model
            .to_str()
            .and_then(|s| CString::new(s).ok())
            .ok_or_else(|| {
                EngineError::Failed(format!(
                    "{}: model path {} is not valid UTF-8 without NUL bytes",
                    info.id,
                    model.display()
                ))
            })?;
        Ok(Self {
            info,
            path,
            device,
            offline: Mutex::new(None),
            live: Mutex::new(None),
        })
    }

    /// The model for the final pass or for live labels, loading it on first use.
    ///
    /// The slot's lock is held while its model loads, deliberately: a concurrent first call for
    /// the same preset waits instead of loading a second copy. It blocks nothing else; the other
    /// preset has its own slot.
    fn model(&self, live: bool) -> Result<Arc<Model>, EngineError> {
        // No preset selects the model's own low-latency default.
        let (slot, preset) = if live {
            (&self.live, None)
        } else {
            (&self.offline, Some(OFFLINE_PRESET_C))
        };
        let mut slot = lock(slot);
        if let Some(model) = slot.as_ref() {
            return Ok(Arc::clone(model));
        }
        let model = Arc::new(Model::load(&self.path, preset, self.device)?);
        *slot = Some(Arc::clone(&model));
        Ok(model)
    }
}

impl Diarizer for NemoDiarizer {
    fn info(&self) -> EngineInfo {
        self.info.clone()
    }

    fn diarize(
        &self,
        audio: &[f32],
        cancel: &CancelToken,
    ) -> Result<Vec<SpeakerTurn>, EngineError> {
        if cancel.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        check_samples(audio)?;
        if audio.is_empty() {
            return Ok(Vec::new());
        }
        let model = self.model(false)?;
        let mut stream = Stream::open(&model)?;
        for chunk in audio.chunks(PUSH_SAMPLES) {
            if cancel.is_cancelled() {
                return Err(EngineError::Cancelled);
            }
            stream.push(chunk)?;
        }
        if cancel.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        stream.finish()?;
        stream
            .segments()?
            .iter()
            .map(|s| turn(s, model.speakers))
            .collect()
    }

    fn open_stream(
        &self,
        turns: EventSink<SpeakerTurn>,
    ) -> Result<Box<dyn EngineStream>, EngineError> {
        let model = self.model(true)?;
        Ok(Box::new(LiveLabels {
            stream: Stream::open(&model)?,
            turns,
            settler: Settler::default(),
            segments: Vec::new(),
            labelled_frames: 0,
        }))
    }
}

/// Which live turns to report: each once, once it can no longer change, in start order.
///
/// The library hands back every segment so far, sorted by start. Every segment that starts
/// before the first one not yet reported has been reported, and cannot change: it ended before an
/// earlier horizon, and a new segment only ever starts near the newest label (the 0.229 s onset
/// pad at most before it), which is after anything settled. So only the segments from the first
/// unreported one on are considered, and only their keys are remembered.
#[derive(Debug, Default)]
struct Settler {
    /// The start (seconds) of the earliest segment not yet reported.
    watermark: f64,
    /// Reported turns starting at or after the watermark, by (start, end, speaker).
    reported: HashSet<(u64, u64, String)>,
}

impl Settler {
    /// The turns among `segments` (sorted by start) that end at or before `horizon` seconds
    /// (all of them when `None`) and were not reported before, in start order.
    fn settle(
        &mut self,
        segments: &[ffi::DiarSegment],
        horizon: Option<f64>,
        speakers: i32,
    ) -> Result<Vec<SpeakerTurn>, EngineError> {
        let tail = &segments[segments.partition_point(|s| s.start_time < self.watermark)..];
        let mut new = Vec::new();
        let mut first_unreported: Option<f64> = None;
        for segment in tail {
            let turn = turn(segment, speakers)?;
            let key = (turn.start_ms, turn.end_ms, turn.speaker.0.clone());
            if self.reported.contains(&key) {
                continue;
            }
            if horizon.is_some_and(|h| segment.end_time > h) {
                first_unreported.get_or_insert(segment.start_time);
                continue;
            }
            self.reported.insert(key);
            new.push(turn);
        }
        // Everything before the first unreported segment is reported; with none left, the last
        // start stays the watermark, so its turn is not reported again.
        self.watermark = first_unreported
            .or_else(|| tail.last().map(|s| s.start_time))
            .unwrap_or(self.watermark);
        let watermark_ms = ms(self.watermark);
        self.reported.retain(|(start, _, _)| *start >= watermark_ms);
        Ok(new)
    }
}

/// A live diarization stream: reports each turn once it is settled.
struct LiveLabels {
    stream: Stream,
    turns: EventSink<SpeakerTurn>,
    settler: Settler,
    /// The segment buffer, reused from read to read.
    segments: Vec<ffi::DiarSegment>,
    /// Frames labelled when the segments were last read.
    labelled_frames: i64,
}

impl LiveLabels {
    /// Reads the segments and reports the newly settled turns.
    fn report(&mut self, horizon: Option<f64>) -> Result<(), EngineError> {
        self.stream.segments_into(&mut self.segments)?;
        let speakers = self.stream.model.speakers;
        for turn in self.settler.settle(&self.segments, horizon, speakers)? {
            (self.turns)(turn);
        }
        Ok(())
    }
}

impl EngineStream for LiveLabels {
    fn push(&mut self, audio: &[f32]) -> Result<(), EngineError> {
        check_samples(audio)?;
        self.stream.push(audio)?;
        // Labels advance a chunk at a time; the segments are read again only when they have.
        let frames = self.stream.frame_count();
        if frames != self.labelled_frames {
            self.labelled_frames = frames;
            let horizon = frames as f64 * self.stream.model.seconds_per_frame - LIVE_SETTLE_S;
            self.report(Some(horizon))?;
        }
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<(), EngineError> {
        self.stream.finish()?;
        self.report(None)
    }
}

/// Loads the diarizer row for [`Residency`](crate::Residency): its single GGUF, installed under
/// `dir`, on `device`.
#[derive(Clone, Debug)]
pub struct NemoLoader {
    dir: ModelDir,
    device: NemoDevice,
}

impl NemoLoader {
    /// Loads from rows installed under `dir`, running on `device`.
    pub fn new(dir: ModelDir, device: NemoDevice) -> Self {
        Self { dir, device }
    }
}

impl Loader<NemoDiarizer> for NemoLoader {
    fn load(&self, row: &EngineRow) -> Result<NemoDiarizer, EngineError> {
        let [file] = row.files.as_slice() else {
            return Err(EngineError::Failed(format!(
                "row {} has {} files; the diarizer takes one GGUF",
                row.id,
                row.files.len()
            )));
        };
        NemoDiarizer::new(&self.dir.file_path(row, file), row.info(), self.device)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(start_time: f64, end_time: f64, speaker: i32) -> ffi::DiarSegment {
        ffi::DiarSegment {
            start_time,
            end_time,
            speaker,
        }
    }

    #[test]
    fn a_segment_becomes_a_turn_in_milliseconds_with_a_zero_based_label() {
        let t = turn(&segment(1.2344, 2.5, 3), 8).unwrap();
        assert_eq!(
            t,
            SpeakerTurn {
                speaker: SpeakerId("spk2".into()),
                start_ms: 1_234,
                end_ms: 2_500,
            }
        );
    }

    #[test]
    fn a_malformed_segment_is_an_error_not_a_turn() {
        for s in [
            segment(f64::NAN, 1.0, 1),
            segment(0.0, f64::INFINITY, 1),
            segment(-0.5, 1.0, 1),
            segment(2.0, 1.0, 1),
            segment(0.0, 1.0, 0),
            segment(0.0, 1.0, 9),
        ] {
            assert!(matches!(turn(&s, 8), Err(EngineError::Failed(_))), "{s:?}");
        }
    }

    fn spk(n: usize) -> SpeakerId {
        SpeakerId(format!("spk{n}"))
    }

    /// The (start ms, speaker) of each reported turn.
    fn starts(turns: &[SpeakerTurn]) -> Vec<(u64, SpeakerId)> {
        turns
            .iter()
            .map(|t| (t.start_ms, t.speaker.clone()))
            .collect()
    }

    #[test]
    fn live_turns_are_reported_once_each_when_settled_in_start_order() {
        let mut settler = Settler::default();
        // A ends by 3 s; B (another speaker) is still open.
        let segments = vec![segment(0.0, 1.0, 1), segment(0.5, 5.0, 2)];
        let turns = settler.settle(&segments, Some(3.0), 8).unwrap();
        assert_eq!(starts(&turns), [(0, spk(0))]);
        // B grows; C starts and ends inside it and settles first.
        let segments = vec![
            segment(0.0, 1.0, 1),
            segment(0.5, 5.2, 2),
            segment(3.0, 4.0, 1),
        ];
        let turns = settler.settle(&segments, Some(5.0), 8).unwrap();
        assert_eq!(starts(&turns), [(3_000, spk(0))]);
        // B settles, and D after it.
        let segments = vec![
            segment(0.0, 1.0, 1),
            segment(0.5, 5.3, 2),
            segment(3.0, 4.0, 1),
            segment(6.0, 7.0, 1),
        ];
        let turns = settler.settle(&segments, Some(8.0), 8).unwrap();
        assert_eq!(starts(&turns), [(500, spk(1)), (6_000, spk(0))]);
        assert_eq!(turns[0].end_ms, 5_300);
        // At the end nothing is left, and nothing comes twice.
        assert!(settler.settle(&segments, None, 8).unwrap().is_empty());
    }

    #[test]
    fn at_the_end_every_unreported_turn_is_reported() {
        let mut settler = Settler::default();
        let segments = vec![segment(0.0, 1.0, 1), segment(0.5, 5.0, 2)];
        settler.settle(&segments, Some(3.0), 8).unwrap();
        let turns = settler.settle(&segments, None, 8).unwrap();
        assert_eq!(starts(&turns), [(500, spk(1))]);
    }

    #[test]
    fn turns_before_the_first_unreported_one_are_not_looked_at_again() {
        // After the first two are reported, only the tail is considered: a turn before the
        // watermark is not even validated (this one is malformed and would be an error).
        let mut settler = Settler::default();
        let segments = vec![segment(0.0, 1.0, 1), segment(1.5, 2.0, 2)];
        assert_eq!(settler.settle(&segments, Some(4.0), 8).unwrap().len(), 2);
        let segments = vec![
            segment(0.0, f64::NAN, 1),
            segment(1.5, 2.0, 2),
            segment(5.0, 6.0, 1),
        ];
        let turns = settler.settle(&segments, Some(8.0), 8).unwrap();
        assert_eq!(starts(&turns), [(5_000, spk(0))]);
        // And what it remembers stays bounded by the unsettled tail.
        assert!(settler.reported.len() <= 1, "{:?}", settler.reported);
    }

    #[test]
    fn a_malformed_turn_in_the_tail_is_an_error() {
        let mut settler = Settler::default();
        let segments = vec![segment(0.0, 1.0, 9)];
        assert!(matches!(
            settler.settle(&segments, Some(4.0), 8),
            Err(EngineError::Failed(_))
        ));
    }

    #[test]
    fn statuses_map_to_engine_errors() {
        assert_eq!(check("x", ffi::OK), Ok(()));
        assert_eq!(
            check("x", ffi::ERROR_CANCELLED),
            Err(EngineError::Cancelled)
        );
        for status in [
            ffi::ERROR_INVALID_ARGUMENT,
            ffi::ERROR_OUT_OF_MEMORY,
            ffi::ERROR_RUNTIME,
            42,
        ] {
            assert!(
                matches!(check("x", status), Err(EngineError::Failed(_))),
                "{status}"
            );
        }
    }

    #[test]
    fn the_c_preset_is_the_documented_one() {
        assert_eq!(OFFLINE_PRESET_C.to_str(), Ok(OFFLINE_PRESET));
    }

    #[test]
    fn non_finite_audio_is_refused() {
        assert!(check_samples(&[0.0, 0.5]).is_ok());
        assert!(check_samples(&[0.0, f32::NAN]).is_err());
        assert!(check_samples(&[f32::INFINITY]).is_err());
    }
}
