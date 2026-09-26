//! Which microphone to record, and how a device's transport reads.
//!
//! **The rule (decided in the gate, measured with earbuds):** when the output is Bluetooth,
//! record the built-in mic. A Bluetooth headset mic is 16 kHz call-link audio, gates to digital
//! zeros while the user is silent, and opening it drops the headset's playback to the call
//! profile. With earbuds no echo reached either mic, and the built-in mic was no worse. The headset
//! mic stays available as a setting (a noisy room is where it probably wins).
//!
//! Pure: it decides over [`DeviceInfo`]s, so every case is tested without hardware.

use ink_core::{DeviceInfo, Transport};
use objc2_core_audio::{
    kAudioDeviceTransportTypeAggregate, kAudioDeviceTransportTypeBluetooth,
    kAudioDeviceTransportTypeBluetoothLE, kAudioDeviceTransportTypeBuiltIn,
    kAudioDeviceTransportTypeUSB, kAudioDeviceTransportTypeVirtual,
};

/// `kAudioDeviceTransportTypeAutoAggregate` ('fgrp'). Declared here because the binding crate files
/// it under the deprecated header, which this crate does not enable.
const TRANSPORT_AUTO_AGGREGATE: u32 = u32::from_be_bytes(*b"fgrp");

/// The core's [`Transport`] for a raw `kAudioDevicePropertyTransportType`.
#[allow(non_upper_case_globals)] // Core Audio's constant names, matched as they are spelled.
pub fn transport(raw: u32) -> Transport {
    match raw {
        kAudioDeviceTransportTypeBuiltIn => Transport::BuiltIn,
        kAudioDeviceTransportTypeBluetooth | kAudioDeviceTransportTypeBluetoothLE => {
            Transport::Bluetooth
        }
        kAudioDeviceTransportTypeUSB => Transport::Usb,
        kAudioDeviceTransportTypeAggregate
        | TRANSPORT_AUTO_AGGREGATE
        | kAudioDeviceTransportTypeVirtual => Transport::Virtual,
        // Unknown (0), PCI, FireWire, HDMI, DisplayPort, AirPlay, AVB, Thunderbolt, Continuity.
        _ => Transport::Other,
    }
}

/// Why a microphone was chosen. The shell shows it, so "why is it using the laptop mic?" has an
/// answer on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MicRouteReason {
    /// The caller named the device.
    Requested,
    /// The output is not Bluetooth: the system default input.
    DefaultInput,
    /// The output is Bluetooth: the built-in mic, by the routing rule.
    BuiltInForBluetoothOutput,
    /// The output is Bluetooth and the headset-mic setting is on: the headset's mic, with 16 kHz
    /// call audio and zeros while the user is silent.
    HeadsetMicSetting,
    /// The output is Bluetooth but this Mac has no built-in mic: the default input.
    NoBuiltInMic,
    /// No default input is set: the first input device.
    FirstInput,
}

/// The microphone to record and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MicRoute<'a> {
    /// The device.
    pub device: &'a DeviceInfo,
    /// Why.
    pub reason: MicRouteReason,
}

/// Chooses the microphone when the caller names none.
///
/// `inputs` are the input devices (the default among them marked `is_default`); `output` is the
/// default output; `headset_mic` is the user's setting. `None` when there is no input at all.
pub fn route_mic<'a>(
    inputs: &'a [DeviceInfo],
    output: Option<&DeviceInfo>,
    headset_mic: bool,
) -> Option<MicRoute<'a>> {
    let default = || {
        inputs
            .iter()
            .find(|d| d.is_default)
            .map(|device| (device, MicRouteReason::DefaultInput))
            .or_else(|| inputs.first().map(|d| (d, MicRouteReason::FirstInput)))
    };
    let bluetooth_output = output.filter(|o| o.transport == Transport::Bluetooth);
    let (device, reason) = match bluetooth_output {
        None => default()?,
        Some(output) if headset_mic => {
            // The headset's own mic: the Bluetooth input that shares its name (macOS lists a
            // headset as one output and one input device), else any Bluetooth input, else the
            // default.
            let bluetooth = |d: &&DeviceInfo| d.transport == Transport::Bluetooth;
            inputs
                .iter()
                .filter(bluetooth)
                .find(|d| d.name == output.name)
                .or_else(|| inputs.iter().find(bluetooth))
                .map(|d| (d, MicRouteReason::HeadsetMicSetting))
                .or_else(default)?
        }
        Some(_) => match inputs.iter().find(|d| d.transport == Transport::BuiltIn) {
            Some(built_in) => (built_in, MicRouteReason::BuiltInForBluetoothOutput),
            None => {
                let (device, _) = default()?;
                (device, MicRouteReason::NoBuiltInMic)
            }
        },
    };
    Some(MicRoute { device, reason })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ink_core::DeviceId;

    fn device(uid: &str, name: &str, transport: Transport, is_default: bool) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId(uid.into()),
            name: name.into(),
            transport,
            is_default,
        }
    }

    fn built_in() -> DeviceInfo {
        device(
            "builtin-mic",
            "Built-in Microphone",
            Transport::BuiltIn,
            false,
        )
    }

    fn earbuds_in(is_default: bool) -> DeviceInfo {
        device(
            "buds:input",
            "Example Buds",
            Transport::Bluetooth,
            is_default,
        )
    }

    fn earbuds_out() -> DeviceInfo {
        device("buds:output", "Example Buds", Transport::Bluetooth, true)
    }

    fn speakers() -> DeviceInfo {
        device(
            "builtin-speakers",
            "Built-in Speakers",
            Transport::BuiltIn,
            true,
        )
    }

    #[test]
    fn bluetooth_output_records_the_built_in_mic() {
        // macOS makes the earbuds the default input when they connect; the rule overrides that.
        let inputs = [earbuds_in(true), built_in()];
        let route = route_mic(&inputs, Some(&earbuds_out()), false).unwrap();
        assert_eq!(route.device.id.0, "builtin-mic");
        assert_eq!(route.reason, MicRouteReason::BuiltInForBluetoothOutput);
    }

    #[test]
    fn the_headset_mic_is_a_setting() {
        let inputs = [built_in(), earbuds_in(false)];
        let route = route_mic(&inputs, Some(&earbuds_out()), true).unwrap();
        assert_eq!(route.device.id.0, "buds:input");
        assert_eq!(route.reason, MicRouteReason::HeadsetMicSetting);
    }

    #[test]
    fn the_headset_setting_prefers_the_headset_that_is_playing() {
        let other = device("other:input", "Other Headset", Transport::Bluetooth, false);
        let inputs = [built_in(), other, earbuds_in(false)];
        let route = route_mic(&inputs, Some(&earbuds_out()), true).unwrap();
        assert_eq!(route.device.id.0, "buds:input");
    }

    #[test]
    fn wired_or_built_in_output_records_the_default_input() {
        let usb = device("usb-mic", "USB Mic", Transport::Usb, true);
        let inputs = [built_in(), usb];
        let route = route_mic(&inputs, Some(&speakers()), false).unwrap();
        assert_eq!(route.device.id.0, "usb-mic");
        assert_eq!(route.reason, MicRouteReason::DefaultInput);
        // The setting only matters with Bluetooth output.
        let route = route_mic(&inputs, Some(&speakers()), true).unwrap();
        assert_eq!(route.reason, MicRouteReason::DefaultInput);
    }

    #[test]
    fn bluetooth_output_without_a_built_in_mic_falls_back_to_the_default_and_says_so() {
        let inputs = [earbuds_in(true)];
        let route = route_mic(&inputs, Some(&earbuds_out()), false).unwrap();
        assert_eq!(route.device.id.0, "buds:input");
        assert_eq!(route.reason, MicRouteReason::NoBuiltInMic);
    }

    #[test]
    fn no_output_no_default_and_no_inputs_are_handled() {
        let inputs = [built_in()];
        let route = route_mic(&inputs, None, false).unwrap();
        assert_eq!(route.reason, MicRouteReason::FirstInput);
        assert_eq!(route_mic(&[], Some(&earbuds_out()), false), None);
        assert_eq!(route_mic(&[], None, true), None);
    }

    #[test]
    fn transports_map_to_the_core_kinds() {
        assert_eq!(
            transport(kAudioDeviceTransportTypeBuiltIn),
            Transport::BuiltIn
        );
        assert_eq!(
            transport(kAudioDeviceTransportTypeBluetooth),
            Transport::Bluetooth
        );
        assert_eq!(
            transport(kAudioDeviceTransportTypeBluetoothLE),
            Transport::Bluetooth
        );
        assert_eq!(transport(kAudioDeviceTransportTypeUSB), Transport::Usb);
        assert_eq!(
            transport(kAudioDeviceTransportTypeAggregate),
            Transport::Virtual
        );
        assert_eq!(
            transport(kAudioDeviceTransportTypeVirtual),
            Transport::Virtual
        );
        assert_eq!(transport(0), Transport::Other, "unknown");
        assert_eq!(transport(u32::from_be_bytes(*b"hdmi")), Transport::Other);
    }
}
