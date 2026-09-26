//! The IOProcs, and the only code in this crate that runs on a realtime thread.
//!
//! **Realtime zone.** [`input_proc`] and [`tone_proc`] run on the HAL's IO thread. Everything they
//! reach must not allocate, lock, block, log or call Objective-C: they read atomics, convert a
//! timestamp, and hand the buffer to the sink (`ink-audio`'s ring producer, which copies and
//! returns). The whole body of each callback runs inside the [`RealtimeGuard`], so tests wrap it
//! in `assert_no_alloc` (I4): the unit tests here with synthetic buffer lists, and the capture
//! checklist binary on real devices.
//!
//! **Lifetime.** A context is boxed and handed to the HAL as the IOProc's client data. It is freed
//! only after the IOProc is stopped and destroyed *and* its [`Gate`] shows no callback in flight;
//! if a callback never leaves, the context is leaked rather than freed under it.
//!
//! `unsafe impl Sync` appears only in this file, each with its reason.
#![cfg(target_os = "macos")]

use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{self, NonNull};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use ink_audio::RealtimeGuard;
use ink_core::{AudioBlock, AudioSink, Clock, PlatformError, SourceStats, StreamFormat};
use objc2_core_audio::{
    AudioDeviceCreateIOProcID, AudioDeviceDestroyIOProcID, AudioDeviceIOProc, AudioDeviceIOProcID,
    AudioDeviceStart, AudioDeviceStop, AudioObjectID,
};
use objc2_core_audio_types::{AudioBuffer, AudioBufferList, AudioTimeStamp, AudioTimeStampFlags};

use super::hal::{HalError, ObjectId, check};
use crate::clock::MacClock;

/// How long [`RunningIo::stop`] waits for a callback in flight to leave, after the HAL has been
/// told to stop. A callback is a few hundred microseconds; this only matters if one hangs.
const LEAVE_TIMEOUT: Duration = Duration::from_secs(1);

/// Tracks callbacks in flight, so a context is never freed under one.
///
/// A callback enters (increments), then checks `closing`; the owner sets `closing`, then waits
/// for the count to reach zero. Both sides use `SeqCst`, so either the callback sees `closing` and
/// does nothing, or the owner sees it in flight and waits.
#[derive(Debug, Default)]
pub(crate) struct Gate {
    active: AtomicU32,
    closing: AtomicBool,
}

impl Gate {
    /// **Realtime.** `false` once closing: the callback must return without touching anything.
    fn enter(&self) -> bool {
        self.active.fetch_add(1, Ordering::SeqCst);
        if self.closing.load(Ordering::SeqCst) {
            self.active.fetch_sub(1, Ordering::SeqCst);
            return false;
        }
        true
    }

    /// **Realtime.**
    fn leave(&self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }

    fn close(&self) {
        self.closing.store(true, Ordering::SeqCst);
    }

    /// Waits until no callback is inside. `false` on timeout.
    fn wait_idle(&self, timeout: Duration) -> bool {
        let start = Instant::now();
        while self.active.load(Ordering::SeqCst) != 0 {
            if start.elapsed() > timeout {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        true
    }
}

/// A context an IOProc runs with.
pub(crate) trait IoContext: Send + Sync {
    /// Its in-flight gate.
    fn gate(&self) -> &Gate;
}

/// What an input IOProc has done, readable from any thread while it runs.
#[derive(Debug, Default)]
pub(crate) struct IoCounters {
    callbacks: AtomicU64,
    frames: AtomicU64,
    discontinuities: AtomicU64,
    skipped: AtomicU64,
    untimed: AtomicU64,
    panics: AtomicU64,
}

impl IoCounters {
    pub(crate) fn snapshot(&self) -> IoStats {
        let load = |a: &AtomicU64| a.load(Ordering::Relaxed);
        IoStats {
            callbacks: load(&self.callbacks),
            frames: load(&self.frames),
            discontinuities: load(&self.discontinuities),
            skipped: load(&self.skipped),
            untimed: load(&self.untimed),
            panics: load(&self.panics),
        }
    }
}

/// A capture stream's counters so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IoStats {
    /// IOProc calls. Zero on the far end while nothing plays (a tap-only aggregate is silent then,
    /// by design); zero on a started mic means a stalled device.
    pub callbacks: u64,
    /// Frames handed to the sink.
    pub frames: u64,
    /// Jumps in the device's sample time: audio lost before the IOProc saw it.
    pub discontinuities: u64,
    /// Callbacks whose buffer was not the format the stream was opened with (a device that
    /// changed format under us), so it was not delivered rather than mislabelled. Each is a gap,
    /// and `stop` reports it among the discontinuities.
    pub skipped: u64,
    /// Callbacks without a valid host time, stamped with the clock's time instead.
    pub untimed: u64,
    /// Panics caught on the IO thread (the sink's or this crate's). After the first, the stream
    /// stops delivering; `stop` reports it as an error.
    pub panics: u64,
}

/// An input IOProc's context: the sink, the guard and the stream's opened format.
pub(crate) struct InputContext {
    gate: Gate,
    /// Touched only by the IO thread between registration and destruction (see `Sync` below).
    sink: UnsafeCell<Box<dyn AudioSink>>,
    guard: RealtimeGuard,
    format: StreamFormat,
    clock: MacClock,
    /// The sample time the next callback should start at, as f64 bits; NaN before the first.
    next_sample_time: AtomicU64,
    counters: Arc<IoCounters>,
}

// SAFETY: the only non-`Sync` field is `sink`. The HAL calls an IOProc from one IO thread at a
// time, so `sink` is used by one thread while the IOProc is registered; the owner touches it again
// only in `into_sink`, which takes the context by value after `RunningIo::stop` has destroyed the
// IOProc and seen the gate empty. Everything else is atomics or `Sync` already.
unsafe impl Sync for InputContext {}

impl IoContext for InputContext {
    fn gate(&self) -> &Gate {
        &self.gate
    }
}

impl InputContext {
    pub(crate) fn new(
        sink: Box<dyn AudioSink>,
        guard: RealtimeGuard,
        format: StreamFormat,
        clock: MacClock,
        counters: Arc<IoCounters>,
    ) -> Box<Self> {
        Box::new(Self {
            gate: Gate::default(),
            sink: UnsafeCell::new(sink),
            guard,
            format,
            clock,
            next_sample_time: AtomicU64::new(f64::NAN.to_bits()),
            counters,
        })
    }

    /// **Realtime.** Buffer 0 as samples, if it is the format this stream was opened with.
    fn samples<'a>(&self, list: &'a AudioBufferList) -> Option<&'a [f32]> {
        if list.mNumberBuffers == 0 {
            return None;
        }
        // Buffer 0 is inside the declared one-element array, so no pointer arithmetic is needed.
        let buffer: &AudioBuffer = &list.mBuffers[0];
        let channels = usize::from(self.format.channels);
        let len = buffer.mDataByteSize as usize / size_of::<f32>();
        if buffer.mData.is_null()
            || buffer.mNumberChannels as usize != channels
            || !len.is_multiple_of(channels)
            || !buffer.mData.cast::<f32>().is_aligned()
        {
            return None;
        }
        // SAFETY: the HAL's buffer holds `mDataByteSize` bytes of the stream's virtual format,
        // which was checked to be native 32-bit float when the stream was opened; the pointer is
        // non-null and aligned (checked), and valid for the length of the callback.
        Some(unsafe { std::slice::from_raw_parts(buffer.mData.cast::<f32>(), len) })
    }

    /// **Realtime.** One callback's work.
    fn on_input(&self, list: &AudioBufferList, time: &AudioTimeStamp) {
        let c = &self.counters;
        c.callbacks.fetch_add(1, Ordering::Relaxed);
        if c.panics.load(Ordering::Relaxed) != 0 {
            // The sink may be half-way through a push that unwound: never call it again.
            return;
        }
        let Some(samples) = self.samples(list) else {
            // Counted as its own gap; the next delivered block starts a fresh continuity check,
            // so the same loss is not counted twice.
            c.skipped.fetch_add(1, Ordering::Relaxed);
            self.next_sample_time
                .store(f64::NAN.to_bits(), Ordering::Relaxed);
            return;
        };
        if samples.is_empty() {
            return;
        }
        let frames = (samples.len() / usize::from(self.format.channels)) as u64;
        let flags = time.mFlags;
        let host_time_ns = if flags.contains(AudioTimeStampFlags::HostTimeValid) {
            self.clock.ticks_to_ns(time.mHostTime)
        } else {
            c.untimed.fetch_add(1, Ordering::Relaxed);
            self.clock.now_ns()
        };
        if flags.contains(AudioTimeStampFlags::SampleTimeValid) {
            let expected = f64::from_bits(self.next_sample_time.load(Ordering::Relaxed));
            if !expected.is_nan() && (time.mSampleTime - expected).abs() > 0.5 {
                c.discontinuities.fetch_add(1, Ordering::Relaxed);
            }
            self.next_sample_time.store(
                (time.mSampleTime + frames as f64).to_bits(),
                Ordering::Relaxed,
            );
        }
        let block = AudioBlock {
            samples,
            format: self.format,
            host_time_ns,
        };
        // SAFETY: only the IO thread reaches here while the IOProc is registered (see `Sync`).
        let sink = unsafe { &mut *self.sink.get() };
        sink.push(&block);
        c.frames.fetch_add(frames, Ordering::Relaxed);
    }

    /// The sink back, once the IOProc is gone, so it is dropped on the owner's thread.
    pub(crate) fn into_sink(self) -> Box<dyn AudioSink> {
        self.sink.into_inner()
    }
}

/// The input IOProc. **Realtime.**
///
/// # Safety
///
/// `client` must be the [`InputContext`] registered with the IOProc, alive for the call (the
/// [`RunningIo`] that registered it guarantees this), and the HAL passes a valid buffer list and
/// timestamp for the length of the call.
pub(crate) unsafe extern "C-unwind" fn input_proc(
    _device: AudioObjectID,
    _now: NonNull<AudioTimeStamp>,
    input: NonNull<AudioBufferList>,
    input_time: NonNull<AudioTimeStamp>,
    _output: NonNull<AudioBufferList>,
    _output_time: NonNull<AudioTimeStamp>,
    client: *mut c_void,
) -> i32 {
    // SAFETY: per the contract above.
    let Some(context) = (unsafe { client.cast::<InputContext>().as_ref() }) else {
        return 0;
    };
    if !context.gate.enter() {
        return 0;
    }
    // SAFETY: per the contract above; the HAL never modifies them during the call.
    let (list, time) = unsafe { (input.as_ref(), input_time.as_ref()) };
    // An unwind must never cross into the HAL. The guard is inside, so the whole body (sink
    // included) is what the I4 tests check.
    let caught = catch_unwind(AssertUnwindSafe(|| {
        (context.guard)(&mut || context.on_input(list, time));
    }));
    if caught.is_err() {
        context.counters.panics.fetch_add(1, Ordering::Relaxed);
    }
    context.gate.leave();
    0
}

/// The probe's tone generator: an output IOProc that writes a sine into every output channel once
/// `playing` is set, and silence before.
pub(crate) struct ToneContext {
    gate: Gate,
    guard: RealtimeGuard,
    /// Radians per frame.
    step: f32,
    amplitude: f32,
    /// The oscillator phase, as f32 bits. Only the IO thread writes it.
    phase: AtomicU32,
    playing: AtomicBool,
    callbacks: AtomicU64,
    panics: AtomicU64,
}

impl IoContext for ToneContext {
    fn gate(&self) -> &Gate {
        &self.gate
    }
}

impl ToneContext {
    /// A sine of `hz` at `amplitude` for an output running at `sample_rate`.
    pub(crate) fn new(
        guard: RealtimeGuard,
        hz: f32,
        amplitude: f32,
        sample_rate: u32,
    ) -> Box<Self> {
        Box::new(Self {
            gate: Gate::default(),
            guard,
            step: std::f32::consts::TAU * hz / sample_rate.max(1) as f32,
            amplitude,
            phase: AtomicU32::new(0),
            playing: AtomicBool::new(false),
            callbacks: AtomicU64::new(0),
            panics: AtomicU64::new(0),
        })
    }

    /// Starts or stops the tone. **Any thread.**
    pub(crate) fn set_playing(&self, playing: bool) {
        self.playing.store(playing, Ordering::Release);
    }

    /// Output callbacks so far.
    pub(crate) fn callbacks(&self) -> u64 {
        self.callbacks.load(Ordering::Relaxed)
    }

    /// Panics caught on the IO thread.
    pub(crate) fn panics(&self) -> u64 {
        self.panics.load(Ordering::Relaxed)
    }

    /// **Realtime.** Fills every output buffer. The HAL zeroes them before the call, so silence
    /// needs no writing.
    ///
    /// # Safety
    ///
    /// `list` must be the HAL's output list for this call: `mNumberBuffers` buffers, each valid for
    /// `mDataByteSize` writable bytes of native 32-bit float (the probe checks the output stream's
    /// format before starting).
    unsafe fn on_output(&self, list: NonNull<AudioBufferList>) {
        self.callbacks.fetch_add(1, Ordering::Relaxed);
        if !self.playing.load(Ordering::Acquire) {
            return;
        }
        let start = f32::from_bits(self.phase.load(Ordering::Relaxed));
        let mut end = start;
        // SAFETY: per the contract; the list is variable-length, so its buffers are reached
        // through a raw pointer, `mNumberBuffers` of them.
        let (count, buffers) = unsafe {
            let list = list.as_ptr();
            (
                (*list).mNumberBuffers as usize,
                ptr::addr_of_mut!((*list).mBuffers).cast::<AudioBuffer>(),
            )
        };
        for i in 0..count {
            // SAFETY: `i < mNumberBuffers`.
            let buffer = unsafe { &mut *buffers.add(i) };
            let channels = (buffer.mNumberChannels as usize).max(1);
            let len = buffer.mDataByteSize as usize / size_of::<f32>();
            let data = buffer.mData.cast::<f32>();
            if data.is_null() || !data.is_aligned() {
                continue;
            }
            // SAFETY: per the contract: `len` floats, writable, aligned (checked).
            let out = unsafe { std::slice::from_raw_parts_mut(data, len) };
            let mut phase = start;
            for frame in out.chunks_exact_mut(channels) {
                frame.fill(self.amplitude * phase.sin());
                phase = (phase + self.step) % std::f32::consts::TAU;
            }
            end = phase;
        }
        self.phase.store(end.to_bits(), Ordering::Relaxed);
    }
}

/// The tone IOProc. **Realtime.**
///
/// # Safety
///
/// `client` must be the [`ToneContext`] registered with the IOProc, alive for the call, and
/// `output` the HAL's output list for this call.
pub(crate) unsafe extern "C-unwind" fn tone_proc(
    _device: AudioObjectID,
    _now: NonNull<AudioTimeStamp>,
    _input: NonNull<AudioBufferList>,
    _input_time: NonNull<AudioTimeStamp>,
    output: NonNull<AudioBufferList>,
    _output_time: NonNull<AudioTimeStamp>,
    client: *mut c_void,
) -> i32 {
    // SAFETY: per the contract above.
    let Some(context) = (unsafe { client.cast::<ToneContext>().as_ref() }) else {
        return 0;
    };
    if !context.gate.enter() {
        return 0;
    }
    let caught = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `output` is the HAL's output list for this call.
        (context.guard)(&mut || unsafe { context.on_output(output) });
    }));
    if caught.is_err() {
        context.panics.fetch_add(1, Ordering::Relaxed);
    }
    context.gate.leave();
    0
}

/// An IOProc registered and started on a device, owning its context.
pub(crate) struct RunningIo<C: IoContext> {
    device: ObjectId,
    proc_id: AudioDeviceIOProcID,
    context: NonNull<C>,
}

// SAFETY: the handle holds a device id, the IOProc id (a function pointer) and a pointer to a
// context that is `Send + Sync`; moving the handle to another thread moves only the right to stop
// the IOProc, which the HAL allows from any thread.
unsafe impl<C: IoContext> Send for RunningIo<C> {}

impl<C: IoContext> RunningIo<C> {
    /// Registers `proc` on `device` with `context` and starts IO. On failure the context comes
    /// back, so a sink inside it is dropped by the caller, never leaked.
    pub(crate) fn start(
        device: ObjectId,
        proc: AudioDeviceIOProc,
        context: Box<C>,
    ) -> Result<Self, (Box<C>, HalError)> {
        let raw = Box::into_raw(context);
        let mut proc_id: AudioDeviceIOProcID = None;
        // SAFETY: `proc` is one of this module's IOProcs, whose client data is a `C`; `raw` is a
        // live boxed `C` that stays allocated until `stop` or `Drop` has destroyed the IOProc;
        // `proc_id` is a live local the HAL writes.
        let status = unsafe {
            AudioDeviceCreateIOProcID(device, proc, raw.cast(), NonNull::from(&mut proc_id))
        };
        if let Err(e) = check(status, "registering the IOProc") {
            // SAFETY: registration failed, so the HAL holds no copy of `raw`.
            return Err((unsafe { Box::from_raw(raw) }, e));
        }
        // SAFETY: `proc_id` was just registered on `device`.
        let status = unsafe { AudioDeviceStart(device, proc_id) };
        if let Err(e) = check(status, "starting the audio device") {
            // SAFETY: as above; destroying the never-started IOProc releases the HAL's copy.
            unsafe { AudioDeviceDestroyIOProcID(device, proc_id) };
            // SAFETY: the IOProc is destroyed, so nothing else refers to `raw`.
            return Err((unsafe { Box::from_raw(raw) }, e));
        }
        Ok(Self {
            device,
            proc_id,
            // SAFETY: `Box::into_raw` never returns null.
            context: unsafe { NonNull::new_unchecked(raw) },
        })
    }

    /// The context, for reading its counters while IO runs.
    pub(crate) fn context(&self) -> &C {
        // SAFETY: the context is alive until `stop` or `Drop`, which take `self`.
        unsafe { self.context.as_ref() }
    }

    /// Stops IO and unregisters the IOProc. The context comes back once no callback can touch it;
    /// `None` if a callback never left, in which case it is leaked (never freed under a running
    /// callback) and the error says so.
    pub(crate) fn stop(self) -> (Option<Box<C>>, Result<(), String>) {
        let this = std::mem::ManuallyDrop::new(self);
        this.shutdown()
    }

    fn shutdown(&self) -> (Option<Box<C>>, Result<(), String>) {
        self.context().gate().close();
        // SAFETY: `proc_id` is registered on `device` (only `shutdown` destroys it, and it runs
        // once: from `stop`, which disarms `Drop`, or from `Drop`).
        let stopped = check(
            unsafe { AudioDeviceStop(self.device, self.proc_id) },
            "stopping the audio device",
        );
        // SAFETY: as above.
        let destroyed = check(
            unsafe { AudioDeviceDestroyIOProcID(self.device, self.proc_id) },
            "unregistering the IOProc",
        );
        if !self.context().gate().wait_idle(LEAVE_TIMEOUT) {
            return (
                None,
                Err(
                    "an audio callback did not return within 1 s of stopping; its context was \
                     leaked rather than freed under it"
                        .into(),
                ),
            );
        }
        // SAFETY: the IOProc is stopped and destroyed and no callback is inside the gate, so
        // nothing else refers to the context; it came from `Box::into_raw` in `start`.
        let context = unsafe { Box::from_raw(self.context.as_ptr()) };
        let result = match stopped.and(destroyed) {
            Ok(()) => Ok(()),
            Err(e) => Err(e.to_string()),
        };
        (Some(context), result)
    }
}

impl<C: IoContext> Drop for RunningIo<C> {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

/// Stops an input IOProc, drops its sink, and turns the counters into [`SourceStats`], or an error
/// when the sink panicked or teardown failed. Skipped callbacks are audio that never reached the
/// sink, so they count as discontinuities.
pub(crate) fn finish(
    running: RunningIo<InputContext>,
    counters: &IoCounters,
    what: &str,
) -> Result<SourceStats, PlatformError> {
    let (context, teardown) = running.stop();
    drop(context.map(|c| c.into_sink()));
    let stats = counters.snapshot();
    if stats.panics > 0 {
        return Err(PlatformError::Failed(format!(
            "{what}: the capture sink panicked on the audio thread; delivery stopped after {} \
             frames",
            stats.frames
        )));
    }
    teardown.map_err(|e| PlatformError::Device(format!("{what}: {e}")))?;
    Ok(SourceStats {
        frames: stats.frames,
        discontinuities: stats.discontinuities + stats.skipped,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use std::hint::black_box;
    use std::mem::MaybeUninit;

    use assert_no_alloc::{AllocDisabler, assert_no_alloc, violation_count};
    use ink_audio::capture_ring;

    use super::*;
    use crate::clock::MacClock;

    // I4: this crate's unit tests run on the counting allocator (warn mode), so a test can prove
    // a callback allocation-free and the control test can prove the guard fires.
    #[global_allocator]
    static ALLOCATOR: AllocDisabler = AllocDisabler;

    /// A guard that runs each callback under `assert_no_alloc` and counts what it catches.
    pub(crate) fn counting_guard() -> (RealtimeGuard, Arc<AtomicU64>, Arc<AtomicU64>) {
        let violations = Arc::new(AtomicU64::new(0));
        let calls = Arc::new(AtomicU64::new(0));
        let (v, c) = (violations.clone(), calls.clone());
        let guard: RealtimeGuard = Arc::new(move |work: &mut dyn FnMut()| {
            let before = violation_count();
            assert_no_alloc(work);
            v.fetch_add(u64::from(violation_count() - before), Ordering::Relaxed);
            c.fetch_add(1, Ordering::Relaxed);
        });
        (guard, violations, calls)
    }

    fn stamp(sample_time: f64, host_ticks: u64) -> AudioTimeStamp {
        // SAFETY: AudioTimeStamp is plain data; all zeros is a valid value.
        let mut t: AudioTimeStamp = unsafe { MaybeUninit::zeroed().assume_init() };
        t.mSampleTime = sample_time;
        t.mHostTime = host_ticks;
        t.mFlags = AudioTimeStampFlags::SampleHostTimeValid;
        t
    }

    fn list(samples: &mut [f32], channels: u32) -> AudioBufferList {
        AudioBufferList {
            mNumberBuffers: 1,
            mBuffers: [AudioBuffer {
                mNumberChannels: channels,
                mDataByteSize: size_of_val(samples) as u32,
                mData: samples.as_mut_ptr().cast(),
            }],
        }
    }

    fn empty_list() -> AudioBufferList {
        AudioBufferList {
            mNumberBuffers: 0,
            mBuffers: [AudioBuffer {
                mNumberChannels: 0,
                mDataByteSize: 0,
                mData: ptr::null_mut(),
            }],
        }
    }

    /// Calls the input IOProc as the HAL would.
    fn call_input(context: &InputContext, input: &mut AudioBufferList, time: &mut AudioTimeStamp) {
        let mut out = empty_list();
        let mut zero = stamp(0.0, 0);
        // SAFETY: a live context and live, well-formed lists and stamps, as the HAL passes them.
        unsafe {
            input_proc(
                0,
                NonNull::from(&mut zero),
                NonNull::from(input),
                NonNull::from(time),
                NonNull::from(&mut out),
                NonNull::from(&mut zero.clone()),
                ptr::from_ref(context).cast_mut().cast(),
            );
        }
    }

    const MONO_48K: StreamFormat = StreamFormat {
        sample_rate: 48_000,
        channels: 1,
    };

    #[test]
    fn control_the_guard_catches_an_allocation() {
        let (guard, violations, calls) = counting_guard();
        guard(&mut || {
            black_box(Vec::<u8>::with_capacity(32));
        });
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            violations.load(Ordering::Relaxed),
            2,
            "an allocation and its free"
        );
    }

    /// I4: the input IOProc, whole body under the guard, into `ink-audio`'s ring.
    #[test]
    fn the_input_ioproc_into_the_ring_allocates_nothing() {
        let clock = MacClock::new().unwrap();
        let (guard, violations, calls) = counting_guard();
        let (producer, mut consumer) = capture_ring(MONO_48K, Duration::from_secs(2)).unwrap();
        let counters = Arc::new(IoCounters::default());
        let context =
            InputContext::new(Box::new(producer), guard, MONO_48K, clock, counters.clone());
        let mut samples = vec![0.25f32; 512];
        let mut input = list(&mut samples, 1);
        for i in 0..200u64 {
            let mut time = stamp(i as f64 * 512.0, 1_000_000 + i * 256);
            call_input(&context, &mut input, &mut time);
            assert!(consumer.pop().is_some(), "block {i} reached the ring");
        }
        assert_eq!(
            calls.load(Ordering::Relaxed),
            200,
            "the guard wrapped every callback"
        );
        assert_eq!(violations.load(Ordering::Relaxed), 0, "and none allocated");
        let stats = counters.snapshot();
        assert_eq!(stats.frames, 200 * 512);
        assert_eq!(stats.discontinuities, 0);
        assert_eq!(stats.skipped, 0);
    }

    /// I4 control: the same path with a sink that allocates is caught, so the test above cannot
    /// pass vacuously.
    #[test]
    fn control_an_allocating_sink_is_caught_through_the_ioproc() {
        struct Allocates;
        impl AudioSink for Allocates {
            fn push(&mut self, block: &AudioBlock<'_>) {
                black_box(block.samples.to_vec());
            }
        }
        let (guard, violations, _) = counting_guard();
        let context = InputContext::new(
            Box::new(Allocates),
            guard,
            MONO_48K,
            MacClock::new().unwrap(),
            Arc::new(IoCounters::default()),
        );
        let mut samples = vec![0.5f32; 64];
        let mut input = list(&mut samples, 1);
        call_input(&context, &mut input, &mut stamp(0.0, 1));
        assert_eq!(violations.load(Ordering::Relaxed), 2);
    }

    /// I4: the probe's tone generator.
    #[test]
    fn the_tone_ioproc_allocates_nothing_and_writes_every_channel() {
        let (guard, violations, calls) = counting_guard();
        let context = ToneContext::new(guard, 1_000.0, 0.1, 48_000);
        context.set_playing(true);
        let mut samples = vec![0.0f32; 2 * 480];
        let mut output = list(&mut samples, 2);
        let mut input = empty_list();
        let mut zero = stamp(0.0, 0);
        for _ in 0..10 {
            // SAFETY: live context and lists, as the HAL passes them.
            unsafe {
                tone_proc(
                    0,
                    NonNull::from(&mut zero),
                    NonNull::from(&mut input),
                    NonNull::from(&mut zero.clone()),
                    NonNull::from(&mut output),
                    NonNull::from(&mut zero.clone()),
                    ptr::from_ref(&*context).cast_mut().cast(),
                );
            }
        }
        assert_eq!(calls.load(Ordering::Relaxed), 10);
        assert_eq!(violations.load(Ordering::Relaxed), 0);
        let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
        assert!((0.09..=0.1).contains(&peak), "peak {peak}");
        assert!(
            samples.chunks(2).all(|f| f[0] == f[1]),
            "both channels carry the tone"
        );
    }

    #[test]
    fn a_buffer_in_another_format_is_skipped_not_mislabelled() {
        let (guard, _, _) = counting_guard();
        let (producer, mut consumer) = capture_ring(MONO_48K, Duration::from_secs(1)).unwrap();
        let counters = Arc::new(IoCounters::default());
        let context = InputContext::new(
            Box::new(producer),
            guard,
            MONO_48K,
            MacClock::new().unwrap(),
            counters.clone(),
        );
        let mut stereo = vec![0.1f32; 256];
        call_input(&context, &mut list(&mut stereo, 2), &mut stamp(0.0, 1));
        call_input(&context, &mut empty_list(), &mut stamp(128.0, 2));
        assert!(consumer.pop().is_none());
        assert_eq!(counters.snapshot().skipped, 2);
        assert_eq!(counters.snapshot().callbacks, 2);
    }

    #[test]
    fn a_jump_in_sample_time_counts_as_a_discontinuity() {
        let (guard, _, _) = counting_guard();
        let (producer, _consumer) = capture_ring(MONO_48K, Duration::from_secs(1)).unwrap();
        let counters = Arc::new(IoCounters::default());
        let context = InputContext::new(
            Box::new(producer),
            guard,
            MONO_48K,
            MacClock::new().unwrap(),
            counters.clone(),
        );
        let mut samples = vec![0.1f32; 100];
        let mut input = list(&mut samples, 1);
        call_input(&context, &mut input, &mut stamp(0.0, 1));
        call_input(&context, &mut input, &mut stamp(100.0, 2));
        call_input(&context, &mut input, &mut stamp(1_000.0, 3)); // 800 frames lost
        call_input(&context, &mut input, &mut stamp(1_100.0, 4));
        assert_eq!(counters.snapshot().discontinuities, 1);
    }

    /// A skipped buffer is one gap: it is not counted again as a jump when delivery resumes.
    #[test]
    fn a_skipped_buffer_is_one_gap_not_two() {
        let (guard, _, _) = counting_guard();
        let (producer, _consumer) = capture_ring(MONO_48K, Duration::from_secs(1)).unwrap();
        let counters = Arc::new(IoCounters::default());
        let context = InputContext::new(
            Box::new(producer),
            guard,
            MONO_48K,
            MacClock::new().unwrap(),
            counters.clone(),
        );
        let mut mono = vec![0.1f32; 100];
        let mut stereo = vec![0.1f32; 200];
        call_input(&context, &mut list(&mut mono, 1), &mut stamp(0.0, 1));
        call_input(&context, &mut list(&mut stereo, 2), &mut stamp(100.0, 2));
        call_input(&context, &mut list(&mut mono, 1), &mut stamp(200.0, 3));
        let stats = counters.snapshot();
        assert_eq!((stats.skipped, stats.discontinuities), (1, 0));
        assert_eq!(stats.frames, 200);
    }

    #[test]
    fn host_time_is_converted_on_the_clock_timebase() {
        let clock = MacClock::new().unwrap();
        let (guard, _, _) = counting_guard();
        let (producer, mut consumer) = capture_ring(MONO_48K, Duration::from_secs(1)).unwrap();
        let context = InputContext::new(Box::new(producer), guard, MONO_48K, clock, Arc::default());
        let mut samples = vec![0.1f32; 48];
        let ticks = 24_000_000 * 5;
        call_input(&context, &mut list(&mut samples, 1), &mut stamp(0.0, ticks));
        let block = consumer.pop().unwrap();
        assert_eq!(block.block.host_time_ns, clock.ticks_to_ns(ticks));
        assert_eq!(block.block.format, MONO_48K);
    }

    #[test]
    fn a_panicking_sink_is_caught_counted_and_never_called_again() {
        struct Panics(u32);
        impl AudioSink for Panics {
            fn push(&mut self, _: &AudioBlock<'_>) {
                self.0 += 1;
                panic!("sink failure");
            }
        }
        let counters = Arc::new(IoCounters::default());
        let context = InputContext::new(
            Box::new(Panics(0)),
            ink_audio::unguarded(),
            MONO_48K,
            MacClock::new().unwrap(),
            counters.clone(),
        );
        let mut samples = vec![0.1f32; 48];
        let mut input = list(&mut samples, 1);
        call_input(&context, &mut input, &mut stamp(0.0, 1));
        call_input(&context, &mut input, &mut stamp(48.0, 2));
        assert_eq!(counters.snapshot().panics, 1);
        assert_eq!(counters.snapshot().callbacks, 2);
    }

    #[test]
    fn a_closed_gate_keeps_callbacks_out() {
        let counters = Arc::new(IoCounters::default());
        let context = InputContext::new(
            Box::new(capture_ring(MONO_48K, Duration::from_secs(1)).unwrap().0),
            ink_audio::unguarded(),
            MONO_48K,
            MacClock::new().unwrap(),
            counters.clone(),
        );
        context.gate().close();
        let mut samples = vec![0.1f32; 48];
        call_input(&context, &mut list(&mut samples, 1), &mut stamp(0.0, 1));
        assert_eq!(counters.snapshot().callbacks, 0);
        assert!(context.gate().wait_idle(Duration::from_millis(10)));
    }
}
