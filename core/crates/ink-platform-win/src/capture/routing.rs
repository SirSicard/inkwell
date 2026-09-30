//! Which microphone to record, and how an endpoint's transport reads.
//!
//! **The rule** is the Mac's (measured in the gate with earbuds), plus one Windows case: when the
//! output is Bluetooth, record a mic that is not Bluetooth. A classic Bluetooth headset mic is
//! 16 kHz call-link audio (HFP), gates to digital zeros while the user is silent, and opening it
//! drops the headset's playback to the call profile. **LE Audio is the exception, and preferred:**
//! its mic runs at 32 kHz and does not take the playback down with it, so when the output is a
//! Bluetooth headset whose own mic runs at 32 kHz or more, that mic is recorded. (No classic
//! Bluetooth voice link runs at 32 kHz, so the rate is the test.) The classic headset mic stays
//! available as a setting.
//!
//! "Its own mic" is the input endpoint in the same **container** as the output: Windows groups a
//! physical device's endpoints under one container id, where names differ per profile
//! ("Headphones (X Stereo)", "Headset (X Hands-Free)").
//!
//! Pure: it decides over [`Endpoint`]s, so every case is tested without hardware.

use ink_core::{DeviceInfo, Transport};

/// An endpoint as routing sees it: the core's view plus what Windows adds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    /// The core's view: id, name, transport, default.
    pub info: DeviceInfo,
    /// The physical device it belongs to (`PKEY_Device_ContainerId`), when Windows reports one.
    pub container: Option<String>,
    /// The shared-mode rate the endpoint runs at (`PKEY_AudioEngine_DeviceFormat`), when readable.
    pub rate: Option<u32>,
}

/// The lowest rate of an LE Audio voice link; nothing classic reaches it.
pub const LE_AUDIO_MIN_RATE: u32 = 32_000;

/// `EndpointFormFactor::DigitalAudioDisplayDevice` (HDMI, DisplayPort).
const FORM_FACTOR_DISPLAY: u32 = 9;

/// The core's [`Transport`] for an endpoint, from its device's enumerator
/// (`PKEY_Device_EnumeratorName`: `BTHENUM`, `USB`, `HDAUDIO`, `SWD`...) and its form factor
/// (`PKEY_AudioEndpoint_FormFactor`).
pub fn transport(enumerator: &str, form_factor: Option<u32>) -> Transport {
    let enumerator = enumerator.to_ascii_uppercase();
    if enumerator.starts_with("BTH") {
        // BTHENUM (classic profiles), BTHHFENUM (hands-free), BTHLE* (LE Audio).
        return Transport::Bluetooth;
    }
    if enumerator == "USB" {
        return Transport::Usb;
    }
    if form_factor == Some(FORM_FACTOR_DISPLAY) {
        // Audio over a monitor cable: neither built in nor an accessory the routing rule knows.
        return Transport::Other;
    }
    match enumerator.as_str() {
        // The machine's own audio: the HD Audio bus and the vendors' own buses for it.
        "HDAUDIO" | "INTELAUDIO" | "ACP" | "AMDACP" => Transport::BuiltIn,
        // Software devices and root-enumerated drivers: virtual cables, voice changers, filters.
        "SWD" | "ROOT" | "MMDEVAPI" => Transport::Virtual,
        _ => Transport::Other,
    }
}

/// Why a microphone was chosen. The shell shows it, so "why is it using that mic?" has an answer
/// on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MicRouteReason {
    /// The caller named the device.
    Requested,
    /// The output is not Bluetooth: the system default input.
    DefaultInput,
    /// The output is an LE Audio headset: its own mic, at 32 kHz or more.
    LeAudioHeadset,
    /// The output is classic Bluetooth: a mic that is not Bluetooth, by the routing rule.
    NotBluetoothForBluetoothOutput,
    /// The output is Bluetooth and the headset-mic setting is on: the headset's own mic.
    HeadsetMicSetting,
    /// The output is Bluetooth but every mic is Bluetooth: the default input.
    OnlyBluetoothMics,
    /// No default input is set: the first input device.
    FirstInput,
}

/// The microphone to record and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MicRoute<'a> {
    /// The device.
    pub device: &'a Endpoint,
    /// Why.
    pub reason: MicRouteReason,
}

/// Chooses the microphone when the caller names none.
///
/// `inputs` are the active capture endpoints (the default among them marked `is_default`);
/// `output` is the default render endpoint; `headset_mic` is the user's setting. `None` when there
/// is no input at all.
pub fn route_mic<'a>(
    inputs: &'a [Endpoint],
    output: Option<&Endpoint>,
    headset_mic: bool,
) -> Option<MicRoute<'a>> {
    let default = || {
        inputs
            .iter()
            .find(|d| d.info.is_default)
            .map(|device| (device, MicRouteReason::DefaultInput))
            .or_else(|| inputs.first().map(|d| (d, MicRouteReason::FirstInput)))
    };
    let bluetooth = |d: &&Endpoint| d.info.transport == Transport::Bluetooth;
    let Some(output) = output.filter(|o| o.info.transport == Transport::Bluetooth) else {
        let (device, reason) = default()?;
        return Some(MicRoute { device, reason });
    };
    // The headset's own mic: a Bluetooth input in the output's container.
    let own_mic = inputs
        .iter()
        .filter(bluetooth)
        .find(|d| d.container.is_some() && d.container == output.container);
    let (device, reason) = if let Some(mic) =
        own_mic.filter(|d| d.rate.is_some_and(|rate| rate >= LE_AUDIO_MIN_RATE))
    {
        (mic, MicRouteReason::LeAudioHeadset)
    } else if headset_mic {
        own_mic
            .or_else(|| inputs.iter().find(bluetooth))
            .map(|d| (d, MicRouteReason::HeadsetMicSetting))
            .or_else(default)?
    } else {
        // Not Bluetooth: the default if it qualifies, else built in, then USB, then anything.
        let not_bluetooth = |d: &&Endpoint| d.info.transport != Transport::Bluetooth;
        let preferred = inputs
            .iter()
            .filter(not_bluetooth)
            .find(|d| d.info.is_default)
            .or_else(|| {
                [Transport::BuiltIn, Transport::Usb]
                    .into_iter()
                    .find_map(|t| inputs.iter().find(|d| d.info.transport == t))
            })
            .or_else(|| inputs.iter().find(not_bluetooth));
        match preferred {
            Some(device) => (device, MicRouteReason::NotBluetoothForBluetoothOutput),
            None => {
                let (device, _) = default()?;
                (device, MicRouteReason::OnlyBluetoothMics)
            }
        }
    };
    Some(MicRoute { device, reason })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ink_core::DeviceId;

    fn endpoint(
        id: &str,
        transport: Transport,
        is_default: bool,
        container: Option<&str>,
        rate: Option<u32>,
    ) -> Endpoint {
        Endpoint {
            info: DeviceInfo {
                id: DeviceId(id.into()),
                name: id.into(),
                transport,
                is_default,
            },
            container: container.map(Into::into),
            rate,
        }
    }

    fn chosen(route: Option<MicRoute<'_>>) -> (&str, MicRouteReason) {
        let route = route.expect("a route");
        (route.device.info.id.0.as_str(), route.reason)
    }

    #[test]
    fn transports_by_enumerator() {
        assert_eq!(transport("BTHENUM", Some(5)), Transport::Bluetooth);
        assert_eq!(transport("BTHHFENUM", Some(5)), Transport::Bluetooth);
        assert_eq!(transport("BthLEDevice", Some(3)), Transport::Bluetooth);
        assert_eq!(transport("USB", Some(4)), Transport::Usb);
        assert_eq!(transport("HDAUDIO", Some(1)), Transport::BuiltIn);
        assert_eq!(transport("HDAUDIO", Some(9)), Transport::Other, "HDMI");
        assert_eq!(transport("SWD", None), Transport::Virtual);
        assert_eq!(transport("ROOT", Some(4)), Transport::Virtual);
        assert_eq!(transport("", None), Transport::Other);
        assert_eq!(transport("SOMETHINGNEW", Some(1)), Transport::Other);
    }

    #[test]
    fn wired_output_records_the_default_input() {
        let inputs = [
            endpoint("webcam", Transport::Usb, false, Some("c1"), Some(48_000)),
            endpoint(
                "line-in",
                Transport::BuiltIn,
                true,
                Some("c2"),
                Some(48_000),
            ),
        ];
        let speakers = endpoint("speakers", Transport::BuiltIn, true, Some("c2"), None);
        assert_eq!(
            chosen(route_mic(&inputs, Some(&speakers), false)),
            ("line-in", MicRouteReason::DefaultInput)
        );
        assert_eq!(
            chosen(route_mic(&inputs, None, false)),
            ("line-in", MicRouteReason::DefaultInput)
        );
    }

    #[test]
    fn no_default_input_takes_the_first() {
        let inputs = [endpoint("a", Transport::Usb, false, None, None)];
        assert_eq!(
            chosen(route_mic(&inputs, None, false)),
            ("a", MicRouteReason::FirstInput)
        );
        assert!(route_mic(&[], None, false).is_none());
    }

    #[test]
    fn classic_bluetooth_output_records_a_mic_that_is_not_bluetooth() {
        // Windows has made the headset the default input, as it does when one connects.
        let inputs = [
            endpoint(
                "headset-mic",
                Transport::Bluetooth,
                true,
                Some("bt"),
                Some(16_000),
            ),
            endpoint("webcam", Transport::Usb, false, Some("cam"), Some(48_000)),
            endpoint(
                "line-in",
                Transport::BuiltIn,
                false,
                Some("pc"),
                Some(48_000),
            ),
        ];
        let buds = endpoint("buds", Transport::Bluetooth, true, Some("bt"), Some(48_000));
        assert_eq!(
            chosen(route_mic(&inputs, Some(&buds), false)),
            ("line-in", MicRouteReason::NotBluetoothForBluetoothOutput),
            "built in before USB when the default is the headset"
        );
    }

    #[test]
    fn a_default_that_is_not_bluetooth_is_kept_under_bluetooth_output() {
        let inputs = [
            endpoint(
                "line-in",
                Transport::BuiltIn,
                false,
                Some("pc"),
                Some(48_000),
            ),
            endpoint("usb-mic", Transport::Usb, true, Some("mic"), Some(48_000)),
            endpoint(
                "headset-mic",
                Transport::Bluetooth,
                false,
                Some("bt"),
                Some(16_000),
            ),
        ];
        let buds = endpoint("buds", Transport::Bluetooth, true, Some("bt"), None);
        assert_eq!(
            chosen(route_mic(&inputs, Some(&buds), false)),
            ("usb-mic", MicRouteReason::NotBluetoothForBluetoothOutput)
        );
    }

    #[test]
    fn an_le_audio_headset_records_its_own_mic() {
        let inputs = [
            endpoint(
                "line-in",
                Transport::BuiltIn,
                true,
                Some("pc"),
                Some(48_000),
            ),
            endpoint(
                "other-bt",
                Transport::Bluetooth,
                false,
                Some("x"),
                Some(32_000),
            ),
            endpoint(
                "le-mic",
                Transport::Bluetooth,
                false,
                Some("le"),
                Some(32_000),
            ),
        ];
        let le = endpoint(
            "le-buds",
            Transport::Bluetooth,
            true,
            Some("le"),
            Some(48_000),
        );
        assert_eq!(
            chosen(route_mic(&inputs, Some(&le), false)),
            ("le-mic", MicRouteReason::LeAudioHeadset),
            "its own container, not another Bluetooth mic"
        );
    }

    #[test]
    fn a_16_khz_headset_mic_is_not_le_audio() {
        let inputs = [
            endpoint(
                "line-in",
                Transport::BuiltIn,
                false,
                Some("pc"),
                Some(48_000),
            ),
            endpoint(
                "hfp-mic",
                Transport::Bluetooth,
                true,
                Some("bt"),
                Some(16_000),
            ),
        ];
        let buds = endpoint("buds", Transport::Bluetooth, true, Some("bt"), None);
        assert_eq!(
            chosen(route_mic(&inputs, Some(&buds), false)).1,
            MicRouteReason::NotBluetoothForBluetoothOutput
        );
        // An unreadable rate is not evidence of LE Audio either.
        let unknown = [
            endpoint("line-in", Transport::BuiltIn, false, Some("pc"), None),
            endpoint("mic", Transport::Bluetooth, true, Some("bt"), None),
        ];
        assert_eq!(chosen(route_mic(&unknown, Some(&buds), false)).0, "line-in");
    }

    #[test]
    fn the_headset_mic_setting_takes_the_headsets_own_mic() {
        let inputs = [
            endpoint(
                "line-in",
                Transport::BuiltIn,
                true,
                Some("pc"),
                Some(48_000),
            ),
            endpoint(
                "other-bt",
                Transport::Bluetooth,
                false,
                Some("x"),
                Some(16_000),
            ),
            endpoint(
                "hfp-mic",
                Transport::Bluetooth,
                false,
                Some("bt"),
                Some(16_000),
            ),
        ];
        let buds = endpoint("buds", Transport::Bluetooth, true, Some("bt"), None);
        assert_eq!(
            chosen(route_mic(&inputs, Some(&buds), true)),
            ("hfp-mic", MicRouteReason::HeadsetMicSetting)
        );
        // No container match: any Bluetooth mic.
        let loose = endpoint("buds", Transport::Bluetooth, true, None, None);
        assert_eq!(
            chosen(route_mic(&inputs, Some(&loose), true)),
            ("other-bt", MicRouteReason::HeadsetMicSetting)
        );
        // No Bluetooth mic at all: the default.
        let wired = [endpoint("line-in", Transport::BuiltIn, true, None, None)];
        assert_eq!(
            chosen(route_mic(&wired, Some(&buds), true)),
            ("line-in", MicRouteReason::DefaultInput)
        );
    }

    #[test]
    fn only_bluetooth_mics_falls_back_to_the_default() {
        let inputs = [
            endpoint(
                "hfp-a",
                Transport::Bluetooth,
                false,
                Some("a"),
                Some(16_000),
            ),
            endpoint("hfp-b", Transport::Bluetooth, true, Some("b"), Some(16_000)),
        ];
        let buds = endpoint("buds", Transport::Bluetooth, true, Some("a"), None);
        assert_eq!(
            chosen(route_mic(&inputs, Some(&buds), false)),
            ("hfp-b", MicRouteReason::OnlyBluetoothMics)
        );
    }
}
