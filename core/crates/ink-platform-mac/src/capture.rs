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
//! **Silence is never taken for quiet.** `ink_audio::levels` tells digital zeros from a quiet room,
//! and far-end idle (no callbacks: nothing is playing) from a stalled mic.
#![cfg(target_os = "macos")]

pub(crate) mod hal;
mod io;
mod mic;
mod routing;
mod tap;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

    /// Not yet: the HAL's device-list and default-device listeners come with the Mac's device
    /// branch (through `io`'s listener seam and its leak-safe teardown). Until then the core reads
    /// the devices when it opens a mic or is asked.
    fn watch_devices(&self, _: EventSink<DeviceChange>) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported("device change notifications"))
    }

    fn unwatch_devices(&self) {}
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
