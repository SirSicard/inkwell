//! Capture: the microphone and the far end, as two independent streams (the dual graph).
//!
//! ```text
//!  input device ──IOProc──► MacMicSource ──push──► ring (ink-audio)      mic, its own clock
//!  process tap ─► tap-only aggregate ──IOProc──► MacFarEndSource ──push──► ring   far end
//! ```
//!
//! The two are aligned by the host time on each block, which both stamp from the same mach clock
//! ([`MacClock`]). A single aggregate holding mic and tap would share a clock, but a Bluetooth mic
//! inside an aggregate delivers digital zeros, and people take calls on earbuds; so the mic never
//! goes into an aggregate.
//!
//! **Mic routing** ([`route_mic`]): with Bluetooth output, the built-in mic, unless the headset-mic
//! setting is on ([`MacCapture::set_headset_mic`]).
//!
//! **Threads.** Every method here is **worker**; the IOProcs are the realtime part (`io`, I4).
//! The sink passed to `start` is `ink-audio`'s ring producer.
//!
//! **Device changes** ([`MacCapture::watch_devices`](CaptureControl::watch_devices)): HAL property
//! listeners on the system object (the device list, the default input and the default output),
//! whose callbacks run on a HAL notification thread and only hand the change to the core, which
//! enqueues it. A mic stream also listens to its own device's `DeviceIsAlive`, so a mic that goes
//! mid-meeting reads as ended ([`MacMicSource`]).
//!
//! **Silence is never taken for quiet.** `ink_audio::levels` tells digital zeros from a quiet room,
//! and far-end idle (no callbacks: nothing is playing) from a stalled mic.
#![cfg(target_os = "macos")]

pub(crate) mod hal;
mod io;
mod mic;
mod routing;
mod tap;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use ink_audio::{RealtimeGuard, unguarded};
use ink_core::{
    AudioSource, AutoInput, AutoReason, CaptureControl, DeviceChange, DeviceId, DeviceInfo,
    EventSink, FarEndTarget, PlatformError,
};
use objc2_core_audio::CATapMuteBehavior;

pub use io::{IoStats, leaked_contexts};
pub use mic::MacMicSource;
pub use routing::{MicRoute, MicRouteReason, route_mic, transport};
pub use tap::MacFarEndSource;

pub(crate) use hal::{HalDevice, HalError, ObjectId, ProcessHal, SystemHal};
use io::{
    DEVICE_WATCH_PROPERTIES, DeviceWatchContext, IoHal, Listeners, SYSTEM_IO, device_watch_listener,
};
pub(crate) use io::{RunningIo, ToneContext, tone_proc};
pub(crate) use tap::TapScope;

use crate::clock::MacClock;
use hal::Direction;

/// This process's pid.
pub(crate) fn own_pid() -> i32 {
    // A pid always fits in pid_t (i32); the fallback is never a real process.
    i32::try_from(std::process::id()).unwrap_or(-1)
}

/// The core's view of devices in one direction: those with channels that way, the default first,
/// never one of this crate's own private aggregates.
fn device_infos(
    devices: &[HalDevice],
    default: Option<ObjectId>,
    channels: impl Fn(&HalDevice) -> u32,
) -> Vec<DeviceInfo> {
    let mut out: Vec<DeviceInfo> = devices
        .iter()
        .filter(|d| channels(d) > 0 && !d.uid.starts_with(tap::AGGREGATE_UID_PREFIX))
        .map(|d| DeviceInfo {
            id: DeviceId(d.uid.clone()),
            name: d.name.clone(),
            transport: routing::transport(d.transport),
            is_default: Some(d.id) == default,
        })
        .collect();
    // Stable: the HAL's order after the default.
    out.sort_by_key(|d| !d.is_default);
    out
}

/// [`CaptureControl`] for macOS.
pub struct MacCapture {
    clock: MacClock,
    guard: RealtimeGuard,
    headset_mic: AtomicBool,
    processes: Arc<dyn ProcessHal>,
    /// The device watch's listeners, while watching. The lock is held only to swap them; the
    /// callbacks never take it.
    watch: Mutex<Option<Listeners<DeviceWatchContext>>>,
}

/// Adds the device watch's listeners on the system object, telling `on_change`.
fn watch_on(
    hal: &'static dyn IoHal,
    on_change: EventSink<DeviceChange>,
) -> Result<Listeners<DeviceWatchContext>, HalError> {
    // SAFETY: `device_watch_listener` reads its client data as a `DeviceWatchContext`, inside its
    // gate.
    unsafe {
        Listeners::register_on(
            hal,
            hal::SYSTEM,
            &DEVICE_WATCH_PROPERTIES,
            Some(device_watch_listener),
            DeviceWatchContext::new(on_change),
            "listening for audio device changes",
        )
    }
}

impl MacCapture {
    /// Capture on `clock`'s timebase, the one every block's host time uses. It holds no OS
    /// resources until a stream is opened.
    pub fn new(clock: MacClock) -> Self {
        Self {
            clock,
            guard: unguarded(),
            headset_mic: AtomicBool::new(false),
            processes: Arc::new(SystemHal),
            watch: Mutex::new(None),
        }
    }

    /// Runs every IOProc callback of streams opened from now on through `guard` (I4: tests and the
    /// checklist binary pass an `assert_no_alloc` guard).
    pub fn with_realtime_guard(mut self, guard: RealtimeGuard) -> Self {
        self.guard = guard;
        self
    }

    /// The user's setting: record the Bluetooth headset's own mic when the output is Bluetooth,
    /// instead of the built-in mic. Off by default.
    pub fn set_headset_mic(&self, on: bool) {
        self.headset_mic.store(on, Ordering::Relaxed);
    }

    /// Whether the headset-mic setting is on.
    pub fn headset_mic(&self) -> bool {
        self.headset_mic.load(Ordering::Relaxed)
    }

    /// **Worker.** The mic `open_mic(None)` would record, and why. `None` when there is no input.
    pub fn mic_route(&self) -> Result<Option<(DeviceInfo, MicRouteReason)>, PlatformError> {
        let devices = hal::devices()?;
        let inputs = self.inputs(&devices)?;
        let output = self.output(&devices)?;
        Ok(route_mic(&inputs, output.as_ref(), self.headset_mic())
            .map(|route| (route.device.clone(), route.reason)))
    }

    /// **Worker.** [`CaptureControl::open_mic`], as the concrete type (its counters are readable).
    pub fn open_mic_source(
        &self,
        device: Option<&DeviceId>,
    ) -> Result<MacMicSource, PlatformError> {
        let devices = hal::devices()?;
        let uid = match device {
            Some(requested) => requested.clone(),
            None => {
                let inputs = self.inputs(&devices)?;
                let output = self.output(&devices)?;
                route_mic(&inputs, output.as_ref(), self.headset_mic())
                    .map(|route| route.device.id.clone())
                    .ok_or_else(|| PlatformError::Device("no input device".into()))?
            }
        };
        let device = devices
            .into_iter()
            .find(|d| d.uid == uid.0 && d.input_channels > 0)
            .ok_or_else(|| PlatformError::Device(format!("no input device with UID {}", uid.0)))?;
        MacMicSource::open(device, self.clock, self.guard.clone())
    }

    /// **Worker.** [`CaptureControl::open_far_end`], as the concrete type.
    pub fn open_far_end_source(
        &self,
        target: &FarEndTarget,
    ) -> Result<MacFarEndSource, PlatformError> {
        let scope = tap::tap_scope(self.processes.as_ref(), target, own_pid())?;
        MacFarEndSource::open(
            scope,
            CATapMuteBehavior::Unmuted,
            true,
            self.clock,
            self.guard.clone(),
        )
    }

    fn inputs(&self, devices: &[HalDevice]) -> Result<Vec<DeviceInfo>, PlatformError> {
        let default = hal::default_device(Direction::Input)?;
        Ok(device_infos(devices, default, |d| d.input_channels))
    }

    fn output(&self, devices: &[HalDevice]) -> Result<Option<DeviceInfo>, PlatformError> {
        let Some(default) = hal::default_device(Direction::Output)? else {
            return Ok(None);
        };
        Ok(device_infos(devices, Some(default), |d| d.output_channels)
            .into_iter()
            .find(|d| d.is_default))
    }
}

impl CaptureControl for MacCapture {
    fn input_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        self.inputs(&hal::devices()?)
    }

    /// None: the far end is a process tap, which hears its app whatever output it plays to, so
    /// there is nothing to pick.
    fn output_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        Err(PlatformError::Unsupported(
            "an output picker: the far end is tapped from its app, whatever it plays to",
        ))
    }

    fn default_output(&self) -> Result<Option<DeviceInfo>, PlatformError> {
        self.output(&hal::devices()?)
    }

    /// [`MacCapture::mic_route`], in the core's words.
    fn automatic_input(&self) -> Result<Option<AutoInput>, PlatformError> {
        Ok(self.mic_route()?.map(|(device, why)| AutoInput {
            device,
            reason: auto_reason(why),
        }))
    }

    /// With `None`, the routed mic ([`MacCapture::mic_route`]).
    fn open_mic(&self, device: Option<&DeviceId>) -> Result<Box<dyn AudioSource>, PlatformError> {
        Ok(Box::new(self.open_mic_source(device)?))
    }

    /// `AllOutput` taps every process except this one. `Apps` taps those apps' processes and the
    /// helpers under their bundle ids, and fails if none has an audio process.
    fn open_far_end(&self, target: &FarEndTarget) -> Result<Box<dyn AudioSource>, PlatformError> {
        Ok(Box::new(self.open_far_end_source(target)?))
    }

    /// HAL listeners for the device list and both defaults (the module docs). A second call
    /// removes the first's listeners before adding the new ones; if those cannot be added, nothing
    /// is watched (the core logs the error and reads the devices when it opens a mic).
    fn watch_devices(&self, on_change: EventSink<DeviceChange>) -> Result<(), PlatformError> {
        let mut watch = self.watch.lock().unwrap_or_else(PoisonError::into_inner);
        // The old listeners go first (their drop waits out a notification in flight), so the old
        // callback never runs after the new one is in place.
        drop(watch.take());
        *watch = Some(watch_on(&SYSTEM_IO, on_change)?);
        Ok(())
    }

    /// Removes the listeners; when it returns no notification is inside the old callback (one
    /// still in flight after a second is the leak case `leaked_contexts` counts, and its gate
    /// keeps it from calling the core).
    fn unwatch_devices(&self) {
        let old = self
            .watch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        drop(old);
    }
}

/// The core's [`AutoReason`] for the routing's reason.
fn auto_reason(why: MicRouteReason) -> AutoReason {
    match why {
        MicRouteReason::BuiltInForBluetoothOutput => AutoReason::BuiltInForBluetoothOutput,
        MicRouteReason::HeadsetMicSetting => AutoReason::HeadsetMicSetting,
        MicRouteReason::NoBuiltInMic => AutoReason::NoBuiltInMic,
        MicRouteReason::FirstInput => AutoReason::FirstInput,
        // `Requested` is never the routing's answer (it routes only when nothing is named).
        MicRouteReason::DefaultInput | MicRouteReason::Requested => AutoReason::DefaultInput,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ink_core::Transport;
    use objc2_core_audio::{kAudioDeviceTransportTypeBluetooth, kAudioDeviceTransportTypeBuiltIn};

    fn hal_device(id: ObjectId, uid: &str, transport: u32, inputs: u32, outputs: u32) -> HalDevice {
        HalDevice {
            id,
            uid: uid.into(),
            name: uid.into(),
            transport,
            input_channels: inputs,
            output_channels: outputs,
        }
    }

    #[test]
    fn inputs_list_the_default_first_and_never_our_own_aggregate() {
        let devices = [
            hal_device(1, "builtin-mic", kAudioDeviceTransportTypeBuiltIn, 1, 0),
            hal_device(2, "speakers", kAudioDeviceTransportTypeBuiltIn, 0, 2),
            hal_device(3, "buds", kAudioDeviceTransportTypeBluetooth, 1, 2),
            hal_device(
                4,
                &format!("{}1234", tap::AGGREGATE_UID_PREFIX),
                0x6772_7570,
                1,
                0,
            ),
        ];
        let inputs = device_infos(&devices, Some(3), |d| d.input_channels);
        let uids: Vec<&str> = inputs.iter().map(|d| d.id.0.as_str()).collect();
        assert_eq!(uids, ["buds", "builtin-mic"]);
        assert!(inputs[0].is_default);
        assert_eq!(inputs[0].transport, Transport::Bluetooth);
        let outputs = device_infos(&devices, Some(2), |d| d.output_channels);
        let uids: Vec<&str> = outputs.iter().map(|d| d.id.0.as_str()).collect();
        assert_eq!(uids, ["speakers", "buds"]);
    }

    #[test]
    fn no_default_is_no_default() {
        let devices = [hal_device(1, "mic", kAudioDeviceTransportTypeBuiltIn, 1, 0)];
        let inputs = device_infos(&devices, None, |d| d.input_channels);
        assert!(!inputs[0].is_default);
    }

    // Local only: these talk to the audio server. They open nothing and need no permission.

    #[test]
    #[ignore = "talks to the local audio server"]
    fn the_real_devices_list_and_route() {
        let capture = MacCapture::new(MacClock::new().unwrap());
        let inputs = capture.input_devices().expect("inputs");
        assert!(inputs.iter().filter(|d| d.is_default).count() <= 1);
        if let Some(first) = inputs.first() {
            assert!(first.is_default || inputs.iter().all(|d| !d.is_default));
        }
        let output = capture.default_output().expect("output");
        let route = capture.mic_route().expect("route");
        assert_eq!(route.is_some(), !inputs.is_empty(), "{output:?}");
    }

    /// A private aggregate device of this process alone (invisible to every other app; the
    /// listings leave it out by its UID prefix): something to add and remove on the real HAL
    /// without touching any setting or device of the user's.
    fn private_aggregate() -> ObjectId {
        use objc2_core_audio::{
            AudioHardwareCreateAggregateDevice, kAudioAggregateDeviceIsPrivateKey,
            kAudioAggregateDeviceNameKey, kAudioAggregateDeviceUIDKey,
        };
        use objc2_core_foundation::{CFBoolean, CFDictionary, CFString, CFType};
        let key = |k: &std::ffi::CStr| CFString::from_str(&k.to_string_lossy());
        let keys = [
            key(kAudioAggregateDeviceNameKey),
            key(kAudioAggregateDeviceUIDKey),
            key(kAudioAggregateDeviceIsPrivateKey),
        ];
        let name = CFString::from_str("Inkwell watch test");
        let uid = CFString::from_str(&format!(
            "{}watch-test-{}",
            tap::AGGREGATE_UID_PREFIX,
            std::process::id()
        ));
        let values: [&CFType; 3] = [&name, &uid, CFBoolean::new(true)];
        let keys: Vec<&CFString> = keys.iter().map(|k| &**k).collect();
        let description = CFDictionary::<CFString, CFType>::from_slices(&keys, &values);
        let mut id = hal::UNKNOWN;
        // SAFETY: a well-formed description and a live out-parameter.
        let status = unsafe {
            AudioHardwareCreateAggregateDevice(
                (*description).as_ref(),
                std::ptr::NonNull::from(&mut id),
            )
        };
        hal::check(status, "creating a test aggregate").expect("a private aggregate");
        id
    }

    fn wait_for(what: &str, done: impl Fn() -> bool) {
        let start = std::time::Instant::now();
        while !done() {
            assert!(
                start.elapsed() < std::time::Duration::from_secs(5),
                "{what} within 5 s"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// The real HAL tells the watch when a device comes and goes, and tells a stream's alive
    /// listener when its device dies; after unwatching nothing more arrives, and nothing leaks.
    #[test]
    #[ignore = "talks to the local audio server"]
    fn the_real_hal_tells_the_watch_and_the_alive_listener() {
        use std::sync::atomic::AtomicUsize;
        let capture = MacCapture::new(MacClock::new().unwrap());
        let leaked = leaked_contexts();
        let devices = Arc::new(AtomicUsize::new(0));
        let d = devices.clone();
        capture
            .watch_devices(Arc::new(move |change| {
                if change == DeviceChange::Devices {
                    d.fetch_add(1, Ordering::SeqCst);
                }
            }))
            .expect("watching");
        let aggregate = private_aggregate();
        wait_for("a device-list notification", || {
            devices.load(Ordering::SeqCst) > 0
        });

        let gone = Arc::new(AtomicBool::new(false));
        let alive = mic::watch_alive(
            &SYSTEM_IO,
            aggregate,
            Arc::new(move || {
                hal::get::<u32>(
                    aggregate,
                    objc2_core_audio::kAudioDevicePropertyDeviceIsAlive,
                    objc2_core_audio::kAudioObjectPropertyScopeGlobal,
                    "alive",
                )
                .map(|a| a != 0)
            }),
            gone.clone(),
        )
        .expect("listening");
        assert!(!gone.load(Ordering::Acquire), "alive while it exists");
        // And a format listener, as a mic stream has: its removal too is refused once the device
        // is gone ('!obj'), and must not leak.
        let format = io::FormatListener::register(
            aggregate,
            io::FormatWatch::new(
                ink_core::StreamFormat {
                    sample_rate: 48_000,
                    channels: 1,
                },
                Arc::default(),
            ),
            Arc::new(|| Err("not read".into())),
        )
        .expect("listening for the rate");
        let before = devices.load(Ordering::SeqCst);
        // SAFETY: created above, destroyed once.
        let status = unsafe { objc2_core_audio::AudioHardwareDestroyAggregateDevice(aggregate) };
        hal::check(status, "destroying the test aggregate").unwrap();
        wait_for("a notification that it went", || {
            devices.load(Ordering::SeqCst) > before
        });
        wait_for("the alive listener marking it gone", || {
            gone.load(Ordering::Acquire)
        });
        drop(alive);
        drop(format);

        capture.unwatch_devices();
        let after = devices.load(Ordering::SeqCst);
        let again = private_aggregate();
        std::thread::sleep(std::time::Duration::from_millis(300));
        // SAFETY: created above, destroyed once.
        unsafe { objc2_core_audio::AudioHardwareDestroyAggregateDevice(again) };
        assert_eq!(
            devices.load(Ordering::SeqCst),
            after,
            "nothing after unwatch"
        );
        assert_eq!(leaked_contexts(), leaked, "nothing leaked");
    }

    #[test]
    #[ignore = "talks to the local audio server"]
    fn a_routed_mic_opens_without_starting() {
        let capture = MacCapture::new(MacClock::new().unwrap());
        if capture.input_devices().unwrap().is_empty() {
            return;
        }
        let mic = capture.open_mic_source(None).expect("open");
        let format = mic.format();
        assert!(format.sample_rate >= 8_000, "{format:?}");
        assert!(format.channels >= 1);
        assert_eq!(mic.stats(), IoStats::default(), "nothing started");
    }
}
