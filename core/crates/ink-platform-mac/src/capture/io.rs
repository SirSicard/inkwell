//! The IOProcs, and the only code in this crate that runs on a realtime thread.
//!
//! **Realtime zone.** [`input_proc`] and [`tone_proc`] run on the HAL's IO thread. Everything they
//! reach must not allocate, lock, block, log or call Objective-C: they read atomics, convert a
//! timestamp, and hand the buffer to the sink (`ink-audio`'s ring producer, which copies and
//! returns). The whole body of each callback runs inside the [`RealtimeGuard`], so tests wrap it
//! in `assert_no_alloc` (I4): the unit tests here with synthetic buffer lists, and the capture
//! checklist binary on real devices.
//!
//! **Panics (decided).** A panic on the IO thread is a bug. `catch_unwind` stops it at the IOProc
//! boundary, so it never unwinds into the HAL. Before that, Rust's panic hook runs once and prints
//! the message, which allocates and writes to stderr on the realtime thread: that one callback
//! breaks the realtime rules and may glitch. The alternative, a process-wide silent hook, would
//! hide every panic in the app, so none is installed. The cost is bounded: a panic happens at most
//! once per stream, because the stream short-circuits after it (every later callback returns at
//! once without touching the sink or the output), and `stop` reports it as an error.
//!
//! **Format changes.** A stream's blocks carry the format it was opened with. A [`FormatListener`]
//! on the device (on the HAL's notification thread, not the IO thread) re-reads the format when
//! the device's nominal rate changes; if it differs, the IOProc delivers nothing more (one atomic
//! load per callback), counts the refusals and the gap, and `stop` reports the change as an error.
//! Recovery, reopening at the new rate mid-session, is left to the device-change work (S2.8).
//!
//! **Lifetime.** A context is boxed and handed to the HAL as the IOProc's (or listener's) client
//! data. It is freed only after the IOProc is stopped and destroyed (the listener removed) *and*
//! its [`Gate`] shows no callback in flight; if a callback never leaves, the context is leaked
//! rather than freed under it. It is also leaked when the HAL refuses to unregister the IOProc or
//! remove the listener, since the HAL may then still call it; [`leaked_contexts`] counts both.
//!
//! **Device listeners.** [`Listeners`] holds property listeners on one object with one context, on
//! the same rules: the device watch on the system object (the device list and both defaults,
//! [`device_watch_listener`], which hands each change to the core's callback and returns) and a
//! mic's device-alive listener ([`alive_listener`], which sets a flag). Both run on a HAL
//! notification thread, never the IO thread.
//!
//! **A device that is gone** (unplugged mid-stream) refuses its teardown: removing a listener
//! answers `'!obj'`, destroying an IOProc `'!dev'` (measured). The HAL calls nothing of an object
//! it no longer has, so a refusal on an object that reads as gone ([`IoHal::is_gone`]) counts as
//! done, and the context is freed rather than leaked.
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
use ink_core::{
    AudioBlock, AudioSink, Clock, DeviceChange, EventSink, PlatformError, SourceStats, StreamFormat,
};
use objc2_core_audio::{
    AudioDeviceCreateIOProcID, AudioDeviceDestroyIOProcID, AudioDeviceIOProc, AudioDeviceIOProcID,
    AudioDeviceStart, AudioDeviceStop, AudioObjectAddPropertyListener, AudioObjectID,
    AudioObjectPropertyAddress, AudioObjectPropertyListenerProc, AudioObjectPropertySelector,
    AudioObjectRemovePropertyListener, kAudioDevicePropertyDeviceIsAlive,
    kAudioDevicePropertyNominalSampleRate, kAudioHardwareBadDeviceError,
    kAudioHardwareBadObjectError, kAudioHardwarePropertyDefaultInputDevice,
    kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyDevices,
    kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal,
};
use objc2_core_audio_types::{AudioBuffer, AudioBufferList, AudioTimeStamp, AudioTimeStampFlags};

use super::hal::{HalError, ObjectId, check};
use crate::clock::MacClock;

/// How long [`RunningIo::stop`] waits for a callback in flight to leave, after the HAL has been
/// told to stop. A callback is a few hundred microseconds; this only matters if one hangs.
const LEAVE_TIMEOUT: Duration = Duration::from_secs(1);

/// Contexts leaked in this process (see [`leaked_contexts`]).
static LEAKED_CONTEXTS: AtomicU64 = AtomicU64::new(0);

/// How many IOProc or listener contexts this process has leaked because teardown could not prove
/// the HAL would never call them again: an IOProc it refused to unregister, a listener it refused
/// to remove, or a callback that never returned. Each is small, but holds its stream's sink (the
/// ring producer) for good. **Any thread.** A non-zero count is worth reporting.
pub fn leaked_contexts() -> u64 {
    LEAKED_CONTEXTS.load(Ordering::Relaxed)
}

fn leak() {
    LEAKED_CONTEXTS.fetch_add(1, Ordering::Relaxed);
}

/// The HAL calls that register and unregister IOProcs and property listeners, behind a seam so
/// tests can make teardown fail. Each method is the HAL function of the same name.
pub(crate) trait IoHal: Sync {
    /// `AudioDeviceCreateIOProcID`.
    ///
    /// # Safety
    ///
    /// `proc` must be an IOProc whose client data is what `client` points to, alive until the
    /// IOProc is destroyed.
    unsafe fn create_ioproc(
        &self,
        device: ObjectId,
        proc: AudioDeviceIOProc,
        client: *mut c_void,
        id: &mut AudioDeviceIOProcID,
    ) -> i32;
    /// `AudioDeviceStart`.
    ///
    /// # Safety
    ///
    /// `id` must be registered on `device`.
    unsafe fn start(&self, device: ObjectId, id: AudioDeviceIOProcID) -> i32;
    /// `AudioDeviceStop`.
    ///
    /// # Safety
    ///
    /// `id` must be registered on `device`.
    unsafe fn stop(&self, device: ObjectId, id: AudioDeviceIOProcID) -> i32;
    /// `AudioDeviceDestroyIOProcID`.
    ///
    /// # Safety
    ///
    /// `id` must be registered on `device`, and destroyed once.
    unsafe fn destroy_ioproc(&self, device: ObjectId, id: AudioDeviceIOProcID) -> i32;
    /// `AudioObjectAddPropertyListener`.
    ///
    /// # Safety
    ///
    /// `listener` must take what `client` points to, alive until the listener is removed.
    unsafe fn add_listener(
        &self,
        device: ObjectId,
        address: &AudioObjectPropertyAddress,
        listener: AudioObjectPropertyListenerProc,
        client: *mut c_void,
    ) -> i32;
    /// `AudioObjectRemovePropertyListener`.
    ///
    /// # Safety
    ///
    /// The arguments must be those the listener was added with.
    unsafe fn remove_listener(
        &self,
        device: ObjectId,
        address: &AudioObjectPropertyAddress,
        listener: AudioObjectPropertyListenerProc,
        client: *mut c_void,
    ) -> i32;
    /// Whether `object` is gone from the HAL (a device unplugged): reading it fails as a bad
    /// object or device. A refused teardown on a gone object is no teardown left to do: the HAL
    /// calls nothing of an object it no longer has. Measured: removing a listener from a destroyed
    /// device answers '!obj', destroying its IOProc '!dev'.
    fn is_gone(&self, object: ObjectId) -> bool {
        let _ = object;
        false
    }
}

/// Whether a teardown call that answered `status` on `object` left nothing registered: it worked,
/// or the object itself is gone.
fn torn_down(hal: &dyn IoHal, object: ObjectId, status: i32) -> bool {
    status == 0 || hal.is_gone(object)
}

/// [`IoHal`] on the real audio server.
pub(crate) struct SystemIo;

/// The one real [`IoHal`].
pub(crate) static SYSTEM_IO: SystemIo = SystemIo;

impl IoHal for SystemIo {
    unsafe fn create_ioproc(
        &self,
        device: ObjectId,
        proc: AudioDeviceIOProc,
        client: *mut c_void,
        id: &mut AudioDeviceIOProcID,
    ) -> i32 {
        // SAFETY: the caller upholds the trait's contract; `id` is a live out-parameter.
        unsafe { AudioDeviceCreateIOProcID(device, proc, client, NonNull::from(id)) }
    }

    unsafe fn start(&self, device: ObjectId, id: AudioDeviceIOProcID) -> i32 {
        // SAFETY: the caller upholds the trait's contract.
        unsafe { AudioDeviceStart(device, id) }
    }

    unsafe fn stop(&self, device: ObjectId, id: AudioDeviceIOProcID) -> i32 {
        // SAFETY: the caller upholds the trait's contract.
        unsafe { AudioDeviceStop(device, id) }
    }

    unsafe fn destroy_ioproc(&self, device: ObjectId, id: AudioDeviceIOProcID) -> i32 {
        // SAFETY: the caller upholds the trait's contract.
        unsafe { AudioDeviceDestroyIOProcID(device, id) }
    }

    unsafe fn add_listener(
        &self,
        device: ObjectId,
        address: &AudioObjectPropertyAddress,
        listener: AudioObjectPropertyListenerProc,
        client: *mut c_void,
    ) -> i32 {
        // SAFETY: the caller upholds the trait's contract; `address` is live for the call.
        unsafe { AudioObjectAddPropertyListener(device, NonNull::from(address), listener, client) }
    }

    unsafe fn remove_listener(
        &self,
        device: ObjectId,
        address: &AudioObjectPropertyAddress,
        listener: AudioObjectPropertyListenerProc,
        client: *mut c_void,
    ) -> i32 {
        // SAFETY: the caller upholds the trait's contract; `address` is live for the call.
        unsafe {
            AudioObjectRemovePropertyListener(device, NonNull::from(address), listener, client)
        }
    }

    fn is_gone(&self, object: ObjectId) -> bool {
        match super::hal::get::<u32>(
            object,
            kAudioDevicePropertyDeviceIsAlive,
            kAudioObjectPropertyScopeGlobal,
            "reading whether a device is still there",
        ) {
            Ok(_) => false,
            Err(e) => {
                e.status == kAudioHardwareBadObjectError || e.status == kAudioHardwareBadDeviceError
            }
        }
    }
}

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
    /// Set by the [`FormatWatch`] (notification thread), read by the IOProc.
    format_changed: AtomicBool,
    rate_changes: AtomicU64,
    new_rate: AtomicU32,
    refused: AtomicU64,
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
            rate_changes: load(&self.rate_changes),
            new_rate: self.new_rate.load(Ordering::Relaxed),
            refused: load(&self.refused),
        }
    }

    /// Whether the device's format changed since the stream was opened. **Any thread.**
    pub(crate) fn format_changed(&self) -> bool {
        self.format_changed.load(Ordering::Acquire)
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
    /// Times the device's format was found changed mid-session (normally 0 or 1). After the first,
    /// the stream delivers nothing more, and `stop` reports it as an error.
    pub rate_changes: u64,
    /// The sample rate the device changed to; 0 when it could not be read.
    pub new_rate: u32,
    /// Callbacks not delivered because the format had changed. The gap counts once among the
    /// discontinuities.
    pub refused: u64,
}

/// Watches a stream's format for a change from the one it was opened with.
pub(crate) struct FormatWatch {
    opened: StreamFormat,
    counters: Arc<IoCounters>,
}

impl FormatWatch {
    pub(crate) fn new(opened: StreamFormat, counters: Arc<IoCounters>) -> Self {
        Self { opened, counters }
    }

    /// Records what a re-read of the device's format found. **Not realtime** (the HAL's
    /// notification thread). A format that cannot be read counts as changed: delivery stops rather
    /// than risk mislabelled audio.
    pub(crate) fn observe(&self, current: Result<StreamFormat, String>) {
        let rate = match current {
            Ok(format) if format == self.opened => return,
            Ok(format) => format.sample_rate,
            Err(_) => 0,
        };
        let c = &self.counters;
        c.new_rate.store(rate, Ordering::Relaxed);
        c.rate_changes.fetch_add(1, Ordering::Relaxed);
        c.format_changed.store(true, Ordering::Release);
    }
}

/// Reads a device's current capture format (the HAL in production, a fake in tests).
pub(crate) type FormatReader = Arc<dyn Fn() -> Result<StreamFormat, String> + Send + Sync>;

/// The property whose change makes the listener re-read the format.
pub(crate) const FORMAT_PROPERTY: AudioObjectPropertyAddress = AudioObjectPropertyAddress {
    mSelector: kAudioDevicePropertyNominalSampleRate,
    mScope: kAudioObjectPropertyScopeGlobal,
    mElement: kAudioObjectPropertyElementMain,
};

/// The listener's client data.
pub(crate) struct FormatListenerContext {
    gate: Gate,
    watch: FormatWatch,
    read: FormatReader,
}

impl FormatListenerContext {
    pub(crate) fn new(watch: FormatWatch, read: FormatReader) -> Box<Self> {
        Box::new(Self {
            gate: Gate::default(),
            watch,
            read,
        })
    }
}

/// The nominal-rate listener. Runs on a HAL notification thread (not realtime), so it may read
/// the device's format, which is IPC.
///
/// # Safety
///
/// `client` must be the [`FormatListenerContext`] registered with the listener, alive for the
/// call ([`FormatListener`] guarantees this).
pub(crate) unsafe extern "C-unwind" fn format_listener(
    _object: AudioObjectID,
    _count: u32,
    _addresses: NonNull<AudioObjectPropertyAddress>,
    client: *mut c_void,
) -> i32 {
    // SAFETY: per the contract above.
    let Some(context) = (unsafe { client.cast::<FormatListenerContext>().as_ref() }) else {
        return 0;
    };
    if !context.gate.enter() {
        return 0;
    }
    // An unwind must never cross into the HAL; a failed read of any kind stops delivery.
    let read = catch_unwind(AssertUnwindSafe(|| (context.read)()))
        .unwrap_or_else(|_| Err("the format read panicked".into()));
    context.watch.observe(read);
    context.gate.leave();
    0
}

/// A registered nominal-rate listener, owning its context; removed on drop.
pub(crate) struct FormatListener {
    hal: &'static dyn IoHal,
    device: ObjectId,
    context: NonNull<FormatListenerContext>,
}

// SAFETY: the handle holds a device id and a pointer to a context that is `Send + Sync` (a gate,
// atomics behind an `Arc`, and a `Send + Sync` reader); removing the listener is allowed from any
// thread.
unsafe impl Send for FormatListener {}

impl FormatListener {
    /// Listens for `device`'s nominal rate changing. It then re-reads the format with `read` and
    /// tells `watch`. Call [`check_now`](Self::check_now) after registering, to catch a change
    /// before the listener existed.
    pub(crate) fn register(
        device: ObjectId,
        watch: FormatWatch,
        read: FormatReader,
    ) -> Result<Self, HalError> {
        Self::register_on(&SYSTEM_IO, device, watch, read)
    }

    pub(crate) fn register_on(
        hal: &'static dyn IoHal,
        device: ObjectId,
        watch: FormatWatch,
        read: FormatReader,
    ) -> Result<Self, HalError> {
        let raw = Box::into_raw(FormatListenerContext::new(watch, read));
        // SAFETY: `format_listener` takes a `FormatListenerContext`, and `raw` stays allocated
        // until `Drop` has removed the listener and seen its gate empty (or is leaked).
        let status = unsafe {
            hal.add_listener(device, &FORMAT_PROPERTY, Some(format_listener), raw.cast())
        };
        if let Err(e) = check(status, "listening for the device's sample rate") {
            // SAFETY: registration failed, so the HAL holds no copy of `raw`.
            drop(unsafe { Box::from_raw(raw) });
            return Err(e);
        }
        Ok(Self {
            hal,
            device,
            // SAFETY: `Box::into_raw` never returns null.
            context: unsafe { NonNull::new_unchecked(raw) },
        })
    }

    /// Reads the format once now, as a notification would.
    pub(crate) fn check_now(&self) {
        // SAFETY: the context is alive until `Drop`.
        let context = unsafe { self.context.as_ref() };
        context.watch.observe((context.read)());
    }
}

impl Drop for FormatListener {
    fn drop(&mut self) {
        // SAFETY: the context is alive until freed below.
        let context = unsafe { self.context.as_ref() };
        context.gate.close();
        // SAFETY: the same device, address, listener and client data it was registered with.
        let status = unsafe {
            self.hal.remove_listener(
                self.device,
                &FORMAT_PROPERTY,
                Some(format_listener),
                self.context.as_ptr().cast(),
            )
        };
        // Freed only when the HAL confirms the listener is gone (or the device is, and the
        // listener with it) and no notification is inside. Otherwise a later notification could
        // still read the context (its gate lives inside it), so it is leaked and counted, never
        // freed.
        if torn_down(self.hal, self.device, status) && context.gate.wait_idle(LEAVE_TIMEOUT) {
            // SAFETY: removed, and no notification is inside the gate; the context came from
            // `Box::into_raw` in `register_on`.
            drop(unsafe { Box::from_raw(self.context.as_ptr()) });
        } else {
            leak();
        }
    }
}

/// Property listeners on one audio object that share one context: added together, removed
/// together when this drops. Every notification runs on a HAL notification thread (never the IO
/// thread), enters the context's [`Gate`], and must return quickly. The context is freed only when
/// the HAL confirms every listener is removed and no notification is inside; otherwise it is
/// leaked and counted ([`leaked_contexts`]), the same rule as [`FormatListener`].
pub(crate) struct Listeners<C: IoContext> {
    hal: &'static dyn IoHal,
    object: ObjectId,
    addresses: &'static [AudioObjectPropertyAddress],
    listener: AudioObjectPropertyListenerProc,
    context: NonNull<C>,
}

// SAFETY: the handle holds an object id, static addresses, a function pointer and a pointer to a
// context that is `Send + Sync` (`IoContext`); removing the listeners is allowed from any thread.
unsafe impl<C: IoContext> Send for Listeners<C> {}

impl<C: IoContext> Listeners<C> {
    /// Adds `listener` for each of `addresses` on `object`, with `context` as its client data. If
    /// one cannot be added, those already added are removed and the error says which failed.
    ///
    /// # Safety
    ///
    /// `listener` must read its client data as a `C`, and only through [`IoContext::gate`]'s
    /// enter and leave around anything else it touches.
    pub(crate) unsafe fn register_on(
        hal: &'static dyn IoHal,
        object: ObjectId,
        addresses: &'static [AudioObjectPropertyAddress],
        listener: AudioObjectPropertyListenerProc,
        context: Box<C>,
        what: &'static str,
    ) -> Result<Self, HalError> {
        let raw = Box::into_raw(context);
        for (added, address) in addresses.iter().enumerate() {
            // SAFETY: per this function's contract `listener` takes a `C`, and `raw` stays
            // allocated until every listener is removed and the gate is empty (or is leaked).
            let status = unsafe { hal.add_listener(object, address, listener, raw.cast()) };
            if let Err(e) = check(status, what) {
                let this = Self {
                    hal,
                    object,
                    addresses: &addresses[..added],
                    listener,
                    // SAFETY: `Box::into_raw` never returns null.
                    context: unsafe { NonNull::new_unchecked(raw) },
                };
                // Removes what was added, then frees the context (or leaks it, counted).
                drop(this);
                return Err(e);
            }
        }
        Ok(Self {
            hal,
            object,
            addresses,
            listener,
            // SAFETY: `Box::into_raw` never returns null.
            context: unsafe { NonNull::new_unchecked(raw) },
        })
    }

    /// The context, for a first check by hand after registering.
    pub(crate) fn context(&self) -> &C {
        // SAFETY: the context is alive until `Drop`.
        unsafe { self.context.as_ref() }
    }
}

impl<C: IoContext> Drop for Listeners<C> {
    fn drop(&mut self) {
        self.context().gate().close();
        let mut refused = false;
        for address in self.addresses {
            // SAFETY: the same object, address, listener and client data it was added with.
            let status = unsafe {
                self.hal.remove_listener(
                    self.object,
                    address,
                    self.listener,
                    self.context.as_ptr().cast(),
                )
            };
            refused |= status != 0;
        }
        // A device unplugged mid-stream answers '!obj': its listeners went with it. Asked once.
        let removed = !refused || self.hal.is_gone(self.object);
        // A listener still added could be called later and would read the context (its gate lives
        // inside it), so it is leaked, never freed.
        if removed && self.context().gate().wait_idle(LEAVE_TIMEOUT) {
            // SAFETY: every listener is removed and no notification is inside the gate; the
            // context came from `Box::into_raw` in `register_on`.
            drop(unsafe { Box::from_raw(self.context.as_ptr()) });
        } else {
            leak();
        }
    }
}

/// The system object's properties the device watch listens to: the device list and the two
/// defaults.
pub(crate) const DEVICE_WATCH_PROPERTIES: [AudioObjectPropertyAddress; 3] = [
    AudioObjectPropertyAddress {
        mSelector: kAudioHardwarePropertyDevices,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    },
    AudioObjectPropertyAddress {
        mSelector: kAudioHardwarePropertyDefaultInputDevice,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    },
    AudioObjectPropertyAddress {
        mSelector: kAudioHardwarePropertyDefaultOutputDevice,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    },
];

/// What a notification about `selector` on the system object means for the core; `None` for a
/// property the watch does not listen to. Pure.
pub(crate) fn device_change(selector: AudioObjectPropertySelector) -> Option<DeviceChange> {
    // Compared, not matched: the HAL's constants are lower case, which a pattern takes for a
    // binding name.
    [
        (kAudioHardwarePropertyDevices, DeviceChange::Devices),
        (
            kAudioHardwarePropertyDefaultInputDevice,
            DeviceChange::DefaultInput,
        ),
        (
            kAudioHardwarePropertyDefaultOutputDevice,
            DeviceChange::DefaultOutput,
        ),
    ]
    .into_iter()
    .find_map(|(s, change)| (s == selector).then_some(change))
}

/// The device watch's client data: the core's callback, which only enqueues.
pub(crate) struct DeviceWatchContext {
    gate: Gate,
    sink: EventSink<DeviceChange>,
}

impl IoContext for DeviceWatchContext {
    fn gate(&self) -> &Gate {
        &self.gate
    }
}

impl DeviceWatchContext {
    pub(crate) fn new(sink: EventSink<DeviceChange>) -> Box<Self> {
        Box::new(Self {
            gate: Gate::default(),
            sink,
        })
    }

    /// Tells the core about each property in `selectors` it listens to, in order. A panic in the
    /// core's callback stops here; it never unwinds into the HAL.
    fn notify(&self, selectors: impl Iterator<Item = AudioObjectPropertySelector>) {
        for change in selectors.filter_map(device_change) {
            let _ = catch_unwind(AssertUnwindSafe(|| (self.sink)(change)));
        }
    }
}

/// The device watch's listener on the system object. Runs on a HAL notification thread: it reads
/// which properties changed and hands each to the core's callback, which only enqueues, so the
/// HAL's thread is let go of at once. The devices themselves are read later, on the core's thread.
///
/// # Safety
///
/// `client` must be the [`DeviceWatchContext`] registered with the listener, alive for the call
/// ([`Listeners`] guarantees this), and `addresses` must point to `count` addresses (the HAL's
/// contract).
pub(crate) unsafe extern "C-unwind" fn device_watch_listener(
    _object: AudioObjectID,
    count: u32,
    addresses: NonNull<AudioObjectPropertyAddress>,
    client: *mut c_void,
) -> i32 {
    // SAFETY: per the contract above.
    let Some(context) = (unsafe { client.cast::<DeviceWatchContext>().as_ref() }) else {
        return 0;
    };
    if !context.gate.enter() {
        return 0;
    }
    // SAFETY: the HAL passes `count` live addresses for the call (u32 always fits a usize here).
    let changed = unsafe { std::slice::from_raw_parts(addresses.as_ptr(), count as usize) };
    context.notify(changed.iter().map(|a| a.mSelector));
    context.gate.leave();
    0
}

/// The property that says a device died: unplugged, switched off, gone from the HAL.
pub(crate) const ALIVE_PROPERTY: [AudioObjectPropertyAddress; 1] = [AudioObjectPropertyAddress {
    mSelector: kAudioDevicePropertyDeviceIsAlive,
    mScope: kAudioObjectPropertyScopeGlobal,
    mElement: kAudioObjectPropertyElementMain,
}];

/// Reads whether a device is alive (the HAL in production, a fake in tests).
pub(crate) type AliveReader = Arc<dyn Fn() -> Result<bool, HalError> + Send + Sync>;

/// A stream's device-alive listener's client data: it marks the stream's device gone.
pub(crate) struct AliveContext {
    gate: Gate,
    read: AliveReader,
    gone: Arc<AtomicBool>,
}

impl IoContext for AliveContext {
    fn gate(&self) -> &Gate {
        &self.gate
    }
}

impl AliveContext {
    pub(crate) fn new(read: AliveReader, gone: Arc<AtomicBool>) -> Box<Self> {
        Box::new(Self {
            gate: Gate::default(),
            read,
            gone,
        })
    }

    /// Reads the device's state once (a notification, or the first look after registering). A
    /// device that cannot be read is taken as gone: its object is what went. **Not realtime.**
    pub(crate) fn check_now(&self) {
        let alive = catch_unwind(AssertUnwindSafe(|| (self.read)()))
            .ok()
            .and_then(Result::ok)
            .unwrap_or(false);
        if !alive {
            self.gone.store(true, Ordering::Release);
        }
    }
}

/// The device-alive listener. Runs on a HAL notification thread (not realtime), so it may read the
/// property, which is IPC; it only sets a flag the stream's owner reads.
///
/// # Safety
///
/// `client` must be the [`AliveContext`] registered with the listener, alive for the call
/// ([`Listeners`] guarantees this).
pub(crate) unsafe extern "C-unwind" fn alive_listener(
    _object: AudioObjectID,
    _count: u32,
    _addresses: NonNull<AudioObjectPropertyAddress>,
    client: *mut c_void,
) -> i32 {
    // SAFETY: per the contract above.
    let Some(context) = (unsafe { client.cast::<AliveContext>().as_ref() }) else {
        return 0;
    };
    if !context.gate.enter() {
        return 0;
    }
    context.check_now();
    context.gate.leave();
    0
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
        if c.format_changed.load(Ordering::Acquire) {
            // The blocks would carry the old format: deliver nothing. The whole run of refusals
            // is one gap.
            if c.refused.fetch_add(1, Ordering::Relaxed) == 0 {
                c.discontinuities.fetch_add(1, Ordering::Relaxed);
            }
            self.next_sample_time
                .store(f64::NAN.to_bits(), Ordering::Relaxed);
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
        // After a panic, write nothing more: silence (the HAL zeroed the buffers).
        if self.panics.load(Ordering::Relaxed) != 0 || !self.playing.load(Ordering::Acquire) {
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
    hal: &'static dyn IoHal,
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
    /// back, so a sink inside it is dropped by the caller; `None` only if the HAL also refused to
    /// unregister the IOProc, in which case it is leaked and counted, never freed.
    pub(crate) fn start(
        device: ObjectId,
        proc: AudioDeviceIOProc,
        context: Box<C>,
    ) -> Result<Self, (Option<Box<C>>, HalError)> {
        Self::start_on(&SYSTEM_IO, device, proc, context)
    }

    pub(crate) fn start_on(
        hal: &'static dyn IoHal,
        device: ObjectId,
        proc: AudioDeviceIOProc,
        context: Box<C>,
    ) -> Result<Self, (Option<Box<C>>, HalError)> {
        let raw = Box::into_raw(context);
        let mut proc_id: AudioDeviceIOProcID = None;
        // SAFETY: `proc` is one of this module's IOProcs, whose client data is a `C`; `raw` is a
        // live boxed `C` that stays allocated until `stop` or `Drop` has destroyed the IOProc.
        let status = unsafe { hal.create_ioproc(device, proc, raw.cast(), &mut proc_id) };
        if let Err(e) = check(status, "registering the IOProc") {
            // SAFETY: registration failed, so the HAL holds no copy of `raw`.
            return Err((Some(unsafe { Box::from_raw(raw) }), e));
        }
        // SAFETY: `proc_id` was just registered on `device`.
        let status = unsafe { hal.start(device, proc_id) };
        if let Err(e) = check(status, "starting the audio device") {
            // SAFETY: as above; destroying the never-started IOProc releases the HAL's copy.
            if unsafe { hal.destroy_ioproc(device, proc_id) } != 0 {
                // Still registered: the HAL may call it, so the context is never freed.
                leak();
                return Err((None, e));
            }
            // SAFETY: the IOProc is destroyed, so nothing else refers to `raw`.
            return Err((Some(unsafe { Box::from_raw(raw) }), e));
        }
        Ok(Self {
            hal,
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
    /// `None` if the HAL refused to unregister the IOProc or a callback never left, in which case
    /// it is leaked and counted (never freed under a callback) and the error says so.
    pub(crate) fn stop(self) -> (Option<Box<C>>, Result<(), String>) {
        let this = std::mem::ManuallyDrop::new(self);
        this.shutdown()
    }

    fn shutdown(&self) -> (Option<Box<C>>, Result<(), String>) {
        self.context().gate().close();
        // SAFETY: `proc_id` is registered on `device` (only `shutdown` destroys it, and it runs
        // once: from `stop`, which disarms `Drop`, or from `Drop`).
        let stopped = check(
            unsafe { self.hal.stop(self.device, self.proc_id) },
            "stopping the audio device",
        );
        // SAFETY: as above.
        let status = unsafe { self.hal.destroy_ioproc(self.device, self.proc_id) };
        // A device unplugged mid-stream answers '!dev': its IOProcs went with it, and so did
        // the need to stop it.
        let gone = status != 0 && self.hal.is_gone(self.device);
        let stopped = if gone { Ok(()) } else { stopped };
        let destroyed = if gone {
            Ok(())
        } else {
            check(status, "unregistering the IOProc")
        };
        if let Err(e) = destroyed {
            // Still registered: the HAL may call it again, so the context is never freed.
            leak();
            return (
                None,
                Err(format!(
                    "{e}; the IOProc's context was leaked rather than freed while registered"
                )),
            );
        }
        if !self.context().gate().wait_idle(LEAVE_TIMEOUT) {
            leak();
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
        let result = stopped.map_err(|e| e.to_string());
        (Some(context), result)
    }
}

impl<C: IoContext> Drop for RunningIo<C> {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

/// Starts an input stream on `device` in `format` under a format watch: registers the listener,
/// checks the format once (a change since opening is an error, never mislabelled audio), then
/// starts IO. On failure the sink is dropped, so the ring's consumer sees it abandoned.
#[expect(
    clippy::too_many_arguments,
    reason = "one call site per source; a struct adds nothing"
)]
pub(crate) fn start_input(
    device: ObjectId,
    format: StreamFormat,
    sink: Box<dyn AudioSink>,
    guard: RealtimeGuard,
    clock: MacClock,
    counters: &Arc<IoCounters>,
    read: FormatReader,
    what: &str,
) -> Result<(RunningIo<InputContext>, FormatListener), PlatformError> {
    let watch = FormatWatch::new(format, Arc::clone(counters));
    let listener = FormatListener::register(device, watch, read)?;
    listener.check_now();
    if counters.format_changed() {
        return Err(PlatformError::Device(format!(
            "{what} changed format since it was opened; open it again"
        )));
    }
    let context = InputContext::new(sink, guard, format, clock, Arc::clone(counters));
    let running = RunningIo::start(device, Some(input_proc), context)
        .map_err(|(_context, error)| PlatformError::from(error))?;
    Ok((running, listener))
}

/// Stops an input IOProc, drops its sink, and reports the session: [`session_result`], or the
/// teardown's error.
pub(crate) fn finish(
    running: RunningIo<InputContext>,
    counters: &IoCounters,
    what: &str,
    opened_rate: u32,
) -> Result<SourceStats, PlatformError> {
    let (context, teardown) = running.stop();
    drop(context.map(|c| c.into_sink()));
    let stats = session_result(counters.snapshot(), what, opened_rate)?;
    teardown.map_err(|e| PlatformError::Device(format!("{what}: {e}")))?;
    Ok(stats)
}

/// A finished session's [`SourceStats`], or an error when it was cut short: the sink panicked, or
/// the device's format changed. Skipped callbacks are audio that never reached the sink, so they
/// count as discontinuities.
pub(crate) fn session_result(
    stats: IoStats,
    what: &str,
    opened_rate: u32,
) -> Result<SourceStats, PlatformError> {
    if stats.panics > 0 {
        return Err(PlatformError::Failed(format!(
            "{what}: the capture sink panicked on the audio thread; delivery stopped after {} \
             frames",
            stats.frames
        )));
    }
    if stats.rate_changes > 0 {
        let now = match stats.new_rate {
            0 => "an unreadable format".to_owned(),
            rate => format!("{rate} Hz"),
        };
        return Err(PlatformError::Device(format!(
            "{what}: the device changed from {opened_rate} Hz to {now} mid-session; delivery \
             stopped after {} frames and {} callbacks were not delivered; open the stream again",
            stats.frames, stats.refused
        )));
    }
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

    const MONO_44K: StreamFormat = StreamFormat {
        sample_rate: 44_100,
        channels: 1,
    };

    /// The device's rate changes mid-session: from the next callback nothing is delivered (never
    /// audio labelled with the old rate). The gap is one discontinuity; the refusals and the change
    /// are counted separately.
    #[test]
    fn a_mid_session_rate_change_stops_delivery_and_is_counted() {
        let (guard, violations, _) = counting_guard();
        let counters = Arc::new(IoCounters::default());
        let watch = FormatWatch::new(MONO_48K, counters.clone());
        let (producer, mut consumer) = capture_ring(MONO_48K, Duration::from_secs(1)).unwrap();
        let context = InputContext::new(
            Box::new(producer),
            guard,
            MONO_48K,
            MacClock::new().unwrap(),
            counters.clone(),
        );
        let mut samples = vec![0.1f32; 480];
        let mut input = list(&mut samples, 1);
        for i in 0..2 {
            call_input(&context, &mut input, &mut stamp(i as f64 * 480.0, 1 + i));
        }
        watch.observe(Ok(MONO_44K)); // on the HAL's notification thread
        for i in 2..5 {
            call_input(&context, &mut input, &mut stamp(i as f64 * 480.0, 1 + i));
        }
        let mut delivered = 0;
        while consumer.pop().is_some() {
            delivered += 1;
        }
        assert_eq!(delivered, 2, "nothing after the change");
        let stats = counters.snapshot();
        assert_eq!(
            (stats.rate_changes, stats.refused, stats.discontinuities),
            (1, 3, 1)
        );
        assert_eq!(stats.new_rate, 44_100);
        assert_eq!(stats.frames, 960);
        assert_eq!(
            violations.load(Ordering::Relaxed),
            0,
            "the refusal path allocates nothing"
        );
    }

    /// The listener re-reads the format through its seam (here a fake HAL read) and marks a change.
    #[test]
    fn the_format_listener_reads_through_its_seam_and_marks_a_change() {
        let counters = Arc::new(IoCounters::default());
        let reads = Arc::new(AtomicU64::new(0));
        let r = reads.clone();
        let context = FormatListenerContext::new(
            FormatWatch::new(MONO_48K, counters.clone()),
            Arc::new(move || {
                r.fetch_add(1, Ordering::Relaxed);
                Ok(MONO_44K)
            }),
        );
        let mut address = FORMAT_PROPERTY;
        // SAFETY: a live context and address, as the HAL passes them.
        unsafe {
            format_listener(
                7,
                1,
                NonNull::from(&mut address),
                ptr::from_ref(&*context).cast_mut().cast(),
            );
        }
        assert_eq!(reads.load(Ordering::Relaxed), 1);
        let stats = counters.snapshot();
        assert_eq!((stats.rate_changes, stats.new_rate), (1, 44_100));
    }

    #[test]
    fn a_notification_with_the_same_format_changes_nothing() {
        let counters = Arc::new(IoCounters::default());
        let watch = FormatWatch::new(MONO_48K, counters.clone());
        watch.observe(Ok(MONO_48K));
        assert_eq!(counters.snapshot().rate_changes, 0);
        assert!(!counters.format_changed());
    }

    /// A format that cannot be read is treated as changed: delivery stops rather than risk
    /// mislabelled audio.
    #[test]
    fn an_unreadable_format_is_treated_as_a_change() {
        let counters = Arc::new(IoCounters::default());
        let watch = FormatWatch::new(MONO_48K, counters.clone());
        watch.observe(Err("reading a stream's format failed".into()));
        assert!(counters.format_changed());
        assert_eq!(counters.snapshot().new_rate, 0);
    }

    /// Stopping a stream whose format changed is an error that names the change, so the session
    /// is reported as cut short; skipped buffers count as discontinuities.
    #[test]
    fn a_rate_change_ends_the_session_with_an_error_naming_it() {
        let changed = IoStats {
            callbacks: 10,
            frames: 960,
            rate_changes: 1,
            new_rate: 44_100,
            refused: 8,
            discontinuities: 1,
            ..IoStats::default()
        };
        match session_result(changed, "microphone", 48_000) {
            Err(PlatformError::Device(message)) => {
                assert!(
                    message.contains("48000") && message.contains("44100"),
                    "{message}"
                );
            }
            other => panic!("{other:?}"),
        }
        let skipped = IoStats {
            frames: 100,
            skipped: 2,
            discontinuities: 1,
            ..IoStats::default()
        };
        assert_eq!(
            session_result(skipped, "far end", 48_000),
            Ok(SourceStats {
                frames: 100,
                discontinuities: 3
            })
        );
    }

    /// A fake of the registration calls: everything succeeds except the teardown call a test
    /// fails. Nothing is really registered, so no callback ever runs.
    struct FakeIo {
        destroy: i32,
        remove: i32,
        /// What `is_gone` answers: the device was unplugged.
        gone: bool,
    }

    const REFUSED: i32 = i32::from_be_bytes(*b"nope");

    impl IoHal for FakeIo {
        unsafe fn create_ioproc(
            &self,
            _device: ObjectId,
            proc: AudioDeviceIOProc,
            _client: *mut c_void,
            id: &mut AudioDeviceIOProcID,
        ) -> i32 {
            *id = proc;
            0
        }
        unsafe fn start(&self, _device: ObjectId, _id: AudioDeviceIOProcID) -> i32 {
            0
        }
        unsafe fn stop(&self, _device: ObjectId, _id: AudioDeviceIOProcID) -> i32 {
            0
        }
        unsafe fn destroy_ioproc(&self, _device: ObjectId, _id: AudioDeviceIOProcID) -> i32 {
            self.destroy
        }
        unsafe fn add_listener(
            &self,
            _device: ObjectId,
            _address: &AudioObjectPropertyAddress,
            _listener: AudioObjectPropertyListenerProc,
            _client: *mut c_void,
        ) -> i32 {
            0
        }
        unsafe fn remove_listener(
            &self,
            _device: ObjectId,
            _address: &AudioObjectPropertyAddress,
            _listener: AudioObjectPropertyListenerProc,
            _client: *mut c_void,
        ) -> i32 {
            self.remove
        }
        fn is_gone(&self, _: ObjectId) -> bool {
            self.gone
        }
    }

    static TEARDOWN_WORKS: FakeIo = FakeIo {
        destroy: 0,
        remove: 0,
        gone: false,
    };
    static DESTROY_FAILS: FakeIo = FakeIo {
        destroy: REFUSED,
        remove: 0,
        gone: false,
    };
    static REMOVE_FAILS: FakeIo = FakeIo {
        destroy: 0,
        remove: REFUSED,
        gone: false,
    };
    /// A device unplugged mid-stream, as measured: its IOProc's destroy answers '!dev', its
    /// listeners' removal '!obj'.
    static DEVICE_WENT: FakeIo = FakeIo {
        destroy: kAudioHardwareBadDeviceError,
        remove: kAudioHardwareBadObjectError,
        gone: true,
    };

    /// The IOProc and the rate listener of a device that went are torn down with it: their
    /// contexts are freed (the ring's sink with them), not leaked, and the stop is no error.
    #[test]
    fn a_device_that_went_frees_its_ioproc_and_listener_contexts() {
        let counters = Arc::new(IoCounters::default());
        let context = InputContext::new(
            Box::new(
                capture_ring(MONO_48K, Duration::from_millis(100))
                    .unwrap()
                    .0,
            ),
            ink_audio::unguarded(),
            MONO_48K,
            MacClock::new().unwrap(),
            counters.clone(),
        );
        let Ok(running) = RunningIo::start_on(&DEVICE_WENT, 1, Some(input_proc), context) else {
            panic!("the fake starts");
        };
        let (context, result) = running.stop();
        assert!(result.is_ok(), "{result:?}");
        drop(context.expect("handed back"));
        assert_eq!(Arc::strong_count(&counters), 1, "freed");

        let marker = Arc::new(());
        let m = marker.clone();
        let read: FormatReader = Arc::new(move || {
            let _held = &m;
            Ok(MONO_48K)
        });
        let listener = FormatListener::register_on(
            &DEVICE_WENT,
            1,
            FormatWatch::new(MONO_48K, Arc::default()),
            read,
        )
        .expect("the fake registers");
        drop(listener);
        assert_eq!(Arc::strong_count(&marker), 1, "freed");
    }

    /// If the IOProc cannot be unregistered the HAL may still call it, so its context is leaked,
    /// never freed, and the leak is counted and reported.
    #[test]
    fn a_failed_ioproc_destroy_leaks_the_context_and_counts_it() {
        let open = |counters: &Arc<IoCounters>| {
            InputContext::new(
                Box::new(
                    capture_ring(MONO_48K, Duration::from_millis(100))
                        .unwrap()
                        .0,
                ),
                ink_audio::unguarded(),
                MONO_48K,
                MacClock::new().unwrap(),
                counters.clone(),
            )
        };
        let counters = Arc::new(IoCounters::default());
        let before = leaked_contexts();
        let Ok(running) = RunningIo::start_on(&DESTROY_FAILS, 1, Some(input_proc), open(&counters))
        else {
            panic!("the fake starts");
        };
        let (context, result) = running.stop();
        assert!(context.is_none(), "not handed back to be freed");
        let error = result.expect_err("the failure is reported");
        assert!(error.contains("leaked"), "{error}");
        assert!(leaked_contexts() > before, "and counted");
        assert_eq!(
            Arc::strong_count(&counters),
            2,
            "the context is still alive"
        );

        // Control: when unregistering works, the context comes back and is freed.
        let counters = Arc::new(IoCounters::default());
        let Ok(running) =
            RunningIo::start_on(&TEARDOWN_WORKS, 1, Some(input_proc), open(&counters))
        else {
            panic!("the fake starts");
        };
        let (context, result) = running.stop();
        assert!(result.is_ok());
        drop(context);
        assert_eq!(Arc::strong_count(&counters), 1, "freed");
    }

    /// If the rate listener cannot be removed a later notification may still arrive, so its
    /// context is leaked, never freed, and the leak is counted.
    #[test]
    fn a_failed_listener_removal_leaks_the_context_and_counts_it() {
        let register = |hal: &'static FakeIo, marker: &Arc<()>| {
            let m = marker.clone();
            let read: FormatReader = Arc::new(move || {
                let _held = &m;
                Ok(MONO_48K)
            });
            let watch = FormatWatch::new(MONO_48K, Arc::default());
            FormatListener::register_on(hal, 1, watch, read).expect("the fake registers")
        };
        let marker = Arc::new(());
        let before = leaked_contexts();
        drop(register(&REMOVE_FAILS, &marker));
        assert!(leaked_contexts() > before, "counted");
        assert_eq!(Arc::strong_count(&marker), 2, "the context is still alive");

        // Control: a removal that works frees the context.
        let marker = Arc::new(());
        drop(register(&TEARDOWN_WORKS, &marker));
        assert_eq!(Arc::strong_count(&marker), 1, "freed");
    }

    /// A fake that counts listener calls and fails the add it is told to (0-based).
    struct CountingIo {
        adds: AtomicU32,
        removes: AtomicU32,
        fail_add: u32,
        remove: i32,
        /// What `is_gone` answers.
        gone: bool,
    }

    impl CountingIo {
        const fn new(fail_add: u32, remove: i32) -> Self {
            Self {
                adds: AtomicU32::new(0),
                removes: AtomicU32::new(0),
                fail_add,
                remove,
                gone: false,
            }
        }

        const fn gone(remove: i32) -> Self {
            Self {
                gone: true,
                ..Self::new(u32::MAX, remove)
            }
        }
    }

    impl IoHal for CountingIo {
        unsafe fn create_ioproc(
            &self,
            _: ObjectId,
            _: AudioDeviceIOProc,
            _: *mut c_void,
            _: &mut AudioDeviceIOProcID,
        ) -> i32 {
            REFUSED
        }
        unsafe fn start(&self, _: ObjectId, _: AudioDeviceIOProcID) -> i32 {
            REFUSED
        }
        unsafe fn stop(&self, _: ObjectId, _: AudioDeviceIOProcID) -> i32 {
            REFUSED
        }
        unsafe fn destroy_ioproc(&self, _: ObjectId, _: AudioDeviceIOProcID) -> i32 {
            REFUSED
        }
        unsafe fn add_listener(
            &self,
            _: ObjectId,
            _: &AudioObjectPropertyAddress,
            _: AudioObjectPropertyListenerProc,
            _: *mut c_void,
        ) -> i32 {
            if self.adds.fetch_add(1, Ordering::SeqCst) == self.fail_add {
                REFUSED
            } else {
                0
            }
        }
        unsafe fn remove_listener(
            &self,
            _: ObjectId,
            _: &AudioObjectPropertyAddress,
            _: AudioObjectPropertyListenerProc,
            _: *mut c_void,
        ) -> i32 {
            self.removes.fetch_add(1, Ordering::SeqCst);
            self.remove
        }
        fn is_gone(&self, _: ObjectId) -> bool {
            self.gone
        }
    }

    /// A device-watch sink that records what it is told, holding `marker` (to see it freed).
    fn recording_sink(
        marker: &Arc<()>,
    ) -> (
        EventSink<DeviceChange>,
        Arc<std::sync::Mutex<Vec<DeviceChange>>>,
    ) {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (s, m) = (seen.clone(), marker.clone());
        let sink: EventSink<DeviceChange> = Arc::new(move |change| {
            let _held = &m;
            s.lock().unwrap().push(change);
        });
        (sink, seen)
    }

    fn register_watch(
        hal: &'static CountingIo,
        sink: EventSink<DeviceChange>,
    ) -> Result<Listeners<DeviceWatchContext>, HalError> {
        // SAFETY: `device_watch_listener` takes a `DeviceWatchContext`.
        unsafe {
            Listeners::register_on(
                hal,
                1,
                &DEVICE_WATCH_PROPERTIES,
                Some(device_watch_listener),
                DeviceWatchContext::new(sink),
                "watching",
            )
        }
    }

    #[test]
    fn the_watch_names_each_property_it_listens_to_and_nothing_else() {
        let changes: Vec<_> = DEVICE_WATCH_PROPERTIES
            .iter()
            .map(|a| device_change(a.mSelector))
            .collect();
        assert_eq!(
            changes,
            [
                Some(DeviceChange::Devices),
                Some(DeviceChange::DefaultInput),
                Some(DeviceChange::DefaultOutput)
            ]
        );
        assert_eq!(device_change(kAudioDevicePropertyNominalSampleRate), None);
        assert_eq!(device_change(kAudioDevicePropertyDeviceIsAlive), None);
    }

    /// The HAL's notification hands each change to the core's callback, in order, and returns; a
    /// property it does not listen to is not passed on, and a closed gate passes nothing.
    #[test]
    fn a_notification_is_handed_to_the_core_and_a_closed_gate_hands_nothing() {
        let marker = Arc::new(());
        let (sink, seen) = recording_sink(&marker);
        let context = DeviceWatchContext::new(sink);
        let mut addresses = [
            DEVICE_WATCH_PROPERTIES[2],
            FORMAT_PROPERTY,
            DEVICE_WATCH_PROPERTIES[0],
        ];
        let call = |addresses: &mut [AudioObjectPropertyAddress]| {
            // SAFETY: a live context and `len` live addresses, as the HAL passes them.
            unsafe {
                device_watch_listener(
                    1,
                    addresses.len() as u32,
                    NonNull::from(&mut addresses[0]),
                    ptr::from_ref(&*context).cast_mut().cast(),
                )
            }
        };
        assert_eq!(call(&mut addresses), 0);
        assert_eq!(
            *seen.lock().unwrap(),
            [DeviceChange::DefaultOutput, DeviceChange::Devices]
        );
        context.gate().close();
        call(&mut addresses);
        assert_eq!(seen.lock().unwrap().len(), 2, "nothing after closing");
        assert!(context.gate().wait_idle(Duration::from_millis(10)));
    }

    /// A panic in the core's callback is stopped at the listener: it never unwinds into the HAL,
    /// and the gate is left.
    #[test]
    fn a_panicking_callback_never_unwinds_into_the_hal() {
        let sink: EventSink<DeviceChange> = Arc::new(|_| panic!("the core's bug"));
        let context = DeviceWatchContext::new(sink);
        let mut address = DEVICE_WATCH_PROPERTIES[0];
        // SAFETY: a live context and one live address.
        let status = unsafe {
            device_watch_listener(
                1,
                1,
                NonNull::from(&mut address),
                ptr::from_ref(&*context).cast_mut().cast(),
            )
        };
        assert_eq!(status, 0);
        assert!(context.gate().wait_idle(Duration::from_millis(10)));
    }

    #[test]
    fn the_watch_adds_three_listeners_and_removes_them_all_freeing_its_context() {
        static HAL: CountingIo = CountingIo::new(u32::MAX, 0);
        let marker = Arc::new(());
        let (sink, _) = recording_sink(&marker);
        let watch = register_watch(&HAL, sink).expect("registers");
        assert_eq!(HAL.adds.load(Ordering::SeqCst), 3);
        drop(watch);
        assert_eq!(HAL.removes.load(Ordering::SeqCst), 3);
        assert_eq!(Arc::strong_count(&marker), 1, "freed");
    }

    /// One listener refused: those already added are removed, the context is freed, and the
    /// error is the HAL's.
    #[test]
    fn a_refused_listener_removes_those_added_before_it() {
        static HAL: CountingIo = CountingIo::new(1, 0);
        let marker = Arc::new(());
        let (sink, _) = recording_sink(&marker);
        let Err(error) = register_watch(&HAL, sink) else {
            panic!("the second add is refused");
        };
        assert_eq!(error.status, REFUSED);
        assert_eq!(HAL.adds.load(Ordering::SeqCst), 2, "stopped at the refusal");
        assert_eq!(HAL.removes.load(Ordering::SeqCst), 1, "the first removed");
        assert_eq!(Arc::strong_count(&marker), 1, "freed");
    }

    /// A listener the HAL will not remove may still be called, so the context (and the core's
    /// callback in it) is leaked, never freed, and counted.
    #[test]
    fn a_watch_whose_removal_is_refused_leaks_its_context() {
        static HAL: CountingIo = CountingIo::new(u32::MAX, REFUSED);
        let marker = Arc::new(());
        let (sink, _) = recording_sink(&marker);
        let before = leaked_contexts();
        drop(register_watch(&HAL, sink).expect("registers"));
        assert_eq!(HAL.removes.load(Ordering::SeqCst), 3, "every removal tried");
        assert!(leaked_contexts() > before, "counted");
        assert_eq!(Arc::strong_count(&marker), 2, "still alive");
    }

    /// Removing a listener from an object that is gone is refused ('!obj'), but the listeners
    /// went with it: the context is freed, not leaked (each unplugged mic would otherwise leak
    /// one). The same refusal on an object still there leaks, as any refusal does.
    #[test]
    fn a_listener_on_a_gone_object_is_freed_not_leaked() {
        static GONE: CountingIo = CountingIo::gone(kAudioHardwareBadObjectError);
        static THERE: CountingIo = CountingIo::new(u32::MAX, kAudioHardwareBadObjectError);
        let marker = Arc::new(());
        let (sink, _) = recording_sink(&marker);
        drop(register_watch(&GONE, sink).expect("registers"));
        assert_eq!(Arc::strong_count(&marker), 1, "freed");
        let (sink, _) = recording_sink(&marker);
        drop(register_watch(&THERE, sink).expect("registers"));
        assert_eq!(Arc::strong_count(&marker), 2, "leaked");
    }

    #[test]
    fn the_alive_listener_marks_a_dead_or_unreadable_device_gone() {
        let gone_after = |read: AliveReader| {
            let gone = Arc::new(AtomicBool::new(false));
            let context = AliveContext::new(read, gone.clone());
            let mut address = ALIVE_PROPERTY[0];
            // SAFETY: a live context and one live address.
            unsafe {
                alive_listener(
                    1,
                    1,
                    NonNull::from(&mut address),
                    ptr::from_ref(&*context).cast_mut().cast(),
                );
            }
            gone.load(Ordering::Acquire)
        };
        assert!(!gone_after(Arc::new(|| Ok(true))), "alive");
        assert!(gone_after(Arc::new(|| Ok(false))), "dead");
        assert!(
            gone_after(Arc::new(|| Err(HalError {
                what: "reading",
                status: REFUSED
            }))),
            "unreadable: its object went"
        );
        assert!(
            gone_after(Arc::new(|| panic!("a bug"))),
            "a panic is taken as gone"
        );
    }

    /// Once gone, a device stays gone for that stream: a later notification saying alive (the HAL
    /// reusing the object) does not bring it back.
    #[test]
    fn gone_stays_gone() {
        let gone = Arc::new(AtomicBool::new(false));
        let alive = Arc::new(AtomicBool::new(false));
        let a = alive.clone();
        let context =
            AliveContext::new(Arc::new(move || Ok(a.load(Ordering::SeqCst))), gone.clone());
        context.check_now();
        alive.store(true, Ordering::SeqCst);
        context.check_now();
        assert!(gone.load(Ordering::Acquire));
    }

    /// After a panic the tone generator short-circuits: it writes nothing more.
    #[test]
    fn a_panicked_tone_generator_writes_nothing_more() {
        let first = Arc::new(AtomicBool::new(true));
        let f = first.clone();
        let guard: RealtimeGuard = Arc::new(move |work: &mut dyn FnMut()| {
            assert!(
                !f.swap(false, Ordering::SeqCst),
                "a bug on the first callback"
            );
            work();
        });
        let context = ToneContext::new(guard, 1_000.0, 0.1, 48_000);
        context.set_playing(true);
        let mut samples = vec![0.0f32; 96];
        let mut output = list(&mut samples, 1);
        let mut input = empty_list();
        let mut zero = stamp(0.0, 0);
        for _ in 0..2 {
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
        assert_eq!(context.panics(), 1);
        assert!(
            samples.iter().all(|&s| s == 0.0),
            "nothing written after the panic"
        );
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
