//! Which microphone, and which output, Inkwell records: the user's choice in Settings > Sound
//! (`audio.input`, `audio.output`), resolved against the devices connected now.
//!
//! **One mic for everything.** Dictation, meetings and the mic test all open the mic this module
//! picks ([`pick_mic`]); the owner chose one choice over one per job (2026-10-05).
//!
//! **The choice.** `audio.input` is `auto` (the default) or a device's id as the OS gives it (a
//! Core Audio UID, a WASAPI endpoint id): an opaque token, printable and at most
//! [`MAX_DEVICE_BYTES`]. Setting it checks the device is connected now, and the core remembers its
//! name and transport beside it ([`INPUT_DEVICE_KEY`]), because ids are not forever: Windows gives
//! a USB mic a new endpoint id on another port. `audio.output` (Windows) is `default` or an
//! output's id, kept the same way.
//!
//! **The rule** ([`resolve_input`]), in order:
//!
//! 1. the chosen device, by id;
//! 2. else a connected device with the chosen one's name and transport (the same mic on another
//!    port), still the user's choice;
//! 3. else Automatic, the platform's routing ([`CaptureControl::automatic_input`]: with
//!    Bluetooth output, not the headset's call-quality mic), said as `chosen_missing` so the
//!    screens can say "isn't connected; using ... until it is";
//! 4. else nothing: an error, never a stream of silence.
//!
//! With `auto`, step 3 alone, with the routing's own reason. Pure apart from [`pick_mic`] and
//! [`Choices`], which read the platform and the store; the rest is tested as a table.
//!
//! **Coalescing.** A device change arrives as several OS notifications (a headset connecting adds
//! an input, an output and changes both defaults within a second). [`Coalescer`] turns a burst into
//! one read: [`QUIET`] after the last notification, and at most [`MAX_WAIT`] after the first.

use std::sync::Arc;
use std::time::Duration;

use ink_core::{AutoInput, AutoReason, CaptureControl, DeviceId, DeviceInfo, Store, Transport};
use serde_json::{Value, json};

use crate::capture::{MicInfo, transport_name};

/// The setting naming the mic: [`AUTO`] or a device's id.
pub const INPUT_KEY: &str = "audio.input";
/// The setting naming the output a meeting's far end records (Windows): [`DEFAULT`] or a device's
/// id.
pub const OUTPUT_KEY: &str = "audio.output";
/// The core's own setting beside [`INPUT_KEY`]: the chosen mic's id, name and transport as they
/// were when it was chosen (`{"id","name","transport"}`). Not a shell setting.
pub const INPUT_DEVICE_KEY: &str = "audio.input.device";
/// The same for [`OUTPUT_KEY`].
pub const OUTPUT_DEVICE_KEY: &str = "audio.output.device";
/// [`INPUT_KEY`]'s value for Automatic.
pub const AUTO: &str = "auto";
/// [`OUTPUT_KEY`]'s value for the default output.
pub const DEFAULT: &str = "default";
/// The longest device id the settings take, in bytes: far beyond any OS's.
pub const MAX_DEVICE_BYTES: usize = 512;
/// How long a burst of device notifications must go quiet before the devices are read.
pub const QUIET: Duration = Duration::from_millis(300);
/// The longest a burst delays that read, from its first notification.
pub const MAX_WAIT: Duration = Duration::from_secs(1);

/// Whether `value` can be a device's id in a setting: not empty, at most [`MAX_DEVICE_BYTES`], one
/// line with no control characters, and not one of the keywords. Says nothing about whether it is
/// connected.
pub fn is_device_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_DEVICE_BYTES
        && value != AUTO
        && value != DEFAULT
        && !value
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
}

/// A device the user chose, as remembered: its id, and its name and transport when it was chosen
/// (absent for a choice stored without them).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wanted {
    /// The OS's id.
    pub id: DeviceId,
    /// Its name when chosen.
    pub name: Option<String>,
    /// How it connected when chosen.
    pub transport: Option<Transport>,
}

impl Wanted {
    /// The remembered device, for [`INPUT_DEVICE_KEY`] or [`OUTPUT_DEVICE_KEY`].
    pub fn of(device: &DeviceInfo) -> Self {
        Self {
            id: device.id.clone(),
            name: Some(device.name.clone()),
            transport: Some(device.transport),
        }
    }

    /// Its JSON, as stored and as the events carry it (`AudioWanted`).
    pub fn json(&self) -> Value {
        let mut v = json!({"id": self.id.0});
        if let Some(name) = &self.name {
            v["name"] = name.as_str().into();
        }
        if let Some(t) = self.transport {
            v["transport"] = transport_name(t).into();
        }
        v
    }

    /// Reads a stored remembered device; `None` when it is not one.
    fn parse(stored: &str) -> Option<Self> {
        let v: Value = serde_json::from_str(stored).ok()?;
        Some(Self {
            id: DeviceId(v.get("id")?.as_str()?.to_owned()),
            name: v.get("name").and_then(Value::as_str).map(str::to_owned),
            transport: v
                .get("transport")
                .and_then(Value::as_str)
                .and_then(parse_transport),
        })
    }
}

/// A transport for its schema word.
pub fn parse_transport(name: &str) -> Option<Transport> {
    Some(match name {
        "built_in" => Transport::BuiltIn,
        "bluetooth" => Transport::Bluetooth,
        "usb" => Transport::Usb,
        "virtual" => Transport::Virtual,
        "other" => Transport::Other,
        _ => return None,
    })
}

/// The mic the user chose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputChoice {
    /// Automatic: the platform's routing.
    Auto,
    /// This device.
    Device(Wanted),
}

/// The output the user chose a meeting's far end to record (Windows).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputChoice {
    /// The default output, followed as it changes.
    Default,
    /// This device.
    Device(Wanted),
}

/// Reads one choice: `None` for the keyword (or unset), else the device with what is remembered
/// of it (only when the remembered id is this one: a write between the two settings never pairs
/// one device's id with another's name).
fn read_choice(
    store: &dyn Store,
    key: &str,
    device_key: &str,
    keyword: &str,
) -> Result<Option<Wanted>, String> {
    let Some(id) = store.setting(key).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    if id == keyword {
        return Ok(None);
    }
    let remembered = store
        .setting(device_key)
        .map_err(|e| e.to_string())?
        .as_deref()
        .and_then(Wanted::parse)
        .filter(|w| w.id.0 == id);
    Ok(Some(remembered.unwrap_or(Wanted {
        id: DeviceId(id),
        name: None,
        transport: None,
    })))
}

/// **Worker.** The mic choice as stored. One that cannot be read is Automatic, logged: a mic that
/// opens beats dictation or a meeting that cannot start over a setting.
pub fn input_choice(store: &dyn Store) -> InputChoice {
    match read_choice(store, INPUT_KEY, INPUT_DEVICE_KEY, AUTO) {
        Ok(Some(w)) => InputChoice::Device(w),
        Ok(None) => InputChoice::Auto,
        Err(e) => {
            log::error!("the mic choice could not be read ({e}); using Automatic");
            InputChoice::Auto
        }
    }
}

/// **Worker.** The output choice as stored; one that cannot be read is the default, logged.
pub fn output_choice(store: &dyn Store) -> OutputChoice {
    match read_choice(store, OUTPUT_KEY, OUTPUT_DEVICE_KEY, DEFAULT) {
        Ok(Some(w)) => OutputChoice::Device(w),
        Ok(None) => OutputChoice::Default,
        Err(e) => {
            log::error!("the output choice could not be read ({e}); using the default output");
            OutputChoice::Default
        }
    }
}

/// Where a meeting reads the user's device choices: at its start, and again when its mic goes
/// (the user may just have picked another).
#[derive(Clone)]
pub struct Choices {
    store: Arc<dyn Store>,
}

impl Choices {
    /// The choices in `store`.
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store }
    }

    /// **Worker.** [`input_choice`] now.
    pub fn input(&self) -> InputChoice {
        input_choice(self.store.as_ref())
    }

    /// **Worker.** [`output_choice`] now.
    pub fn output(&self) -> OutputChoice {
        output_choice(self.store.as_ref())
    }
}

/// Why a mic records: the user's choice, Automatic in its place, or Automatic chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MicReason {
    /// The device the user chose (found by id, or by its name and transport).
    Chosen,
    /// The user chose a device that is not connected: Automatic's pick instead.
    ChosenMissing,
    /// Automatic, chosen, for the routing's reason.
    Auto(AutoReason),
}

impl MicReason {
    /// Its schema word (`MicReason`).
    pub fn word(self) -> &'static str {
        match self {
            Self::Chosen => "chosen",
            Self::ChosenMissing => "chosen_missing",
            Self::Auto(AutoReason::DefaultInput) => "default_input",
            Self::Auto(AutoReason::BuiltInForBluetoothOutput) => "built_in_for_bluetooth_output",
            Self::Auto(AutoReason::NoBuiltInMic) => "no_built_in_mic",
            Self::Auto(AutoReason::FirstInput) => "first_input",
            Self::Auto(AutoReason::LeAudioHeadset) => "le_audio_headset",
            Self::Auto(AutoReason::HeadsetMicSetting) => "headset_mic_setting",
            // The core's enum is non-exhaustive: a reason added there is said to be unknown,
            // never passed off as another one.
            Self::Auto(_) => "unknown",
        }
    }
}

/// The mic to open, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picked {
    /// The device.
    pub device: DeviceInfo,
    /// Why.
    pub reason: MicReason,
    /// The chosen device it stands in for, when that one is not connected
    /// ([`MicReason::ChosenMissing`]).
    pub wanted: Option<Wanted>,
}

impl Picked {
    /// The mic as a meeting reports it; `transport` as the opened stream says (it may know better
    /// than the list).
    pub fn mic_info(&self, transport: Transport) -> MicInfo {
        MicInfo {
            name: self.device.name.clone(),
            transport,
            reason: self.reason.word(),
            wanted: self.wanted.clone(),
        }
    }

    /// Its JSON in the events (`AudioInput`).
    pub fn json(&self) -> Value {
        json!({
            "id": self.device.id.0,
            "name": self.device.name,
            "transport": transport_name(self.device.transport),
            "reason": self.reason.word(),
        })
    }
}

/// The connected device the user chose: by id, else the first with its remembered name and
/// transport.
pub fn find_wanted<'a>(devices: &'a [DeviceInfo], wanted: &Wanted) -> Option<&'a DeviceInfo> {
    devices.iter().find(|d| d.id == wanted.id).or_else(|| {
        let (name, transport) = (wanted.name.as_ref()?, wanted.transport?);
        devices
            .iter()
            .find(|d| &d.name == name && d.transport == transport)
    })
}

/// The mic to open for `choice` (the module docs' rule): `inputs` are connected now, `automatic`
/// is the platform's routing now. `None`: there is no mic to open.
pub fn resolve_input(
    inputs: &[DeviceInfo],
    choice: &InputChoice,
    automatic: Option<AutoInput>,
) -> Option<Picked> {
    match choice {
        InputChoice::Device(wanted) => match find_wanted(inputs, wanted) {
            Some(device) => Some(Picked {
                device: device.clone(),
                reason: MicReason::Chosen,
                wanted: None,
            }),
            None => automatic.map(|auto| Picked {
                device: auto.device,
                reason: MicReason::ChosenMissing,
                wanted: Some(wanted.clone()),
            }),
        },
        InputChoice::Auto => automatic.map(|auto| Picked {
            device: auto.device,
            reason: MicReason::Auto(auto.reason),
            wanted: None,
        }),
    }
}

/// **Worker.** [`resolve_input`] over the devices connected now. The platform's routing is asked
/// only when the choice is Automatic or not connected. Errors name the device, never audio.
pub fn pick_mic(capture: &dyn CaptureControl, choice: &InputChoice) -> Result<Picked, String> {
    let inputs = capture
        .input_devices()
        .map_err(|e| format!("the microphones: {e}"))?;
    let chosen_here = match choice {
        InputChoice::Device(wanted) => find_wanted(&inputs, wanted).is_some(),
        InputChoice::Auto => false,
    };
    let automatic = if chosen_here {
        None
    } else {
        capture
            .automatic_input()
            .map_err(|e| format!("the microphone: {e}"))?
    };
    resolve_input(&inputs, choice, automatic).ok_or_else(|| "there is no microphone".to_owned())
}

/// Why a meeting's far end records an output (Windows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputReason {
    /// The output the user chose.
    Chosen,
    /// The user chose an output that is not connected: the default instead.
    ChosenMissing,
    /// The default output, chosen.
    Default,
}

impl OutputReason {
    /// Its schema word (`OutputReason`).
    pub fn word(self) -> &'static str {
        match self {
            Self::Chosen => "chosen",
            Self::ChosenMissing => "chosen_missing",
            Self::Default => "default_output",
        }
    }
}

/// The output a far end records, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickedOutput {
    /// The device.
    pub device: DeviceInfo,
    /// Why.
    pub reason: OutputReason,
}

impl PickedOutput {
    /// Its JSON in the events (`AudioOutput`).
    pub fn json(&self) -> Value {
        json!({
            "id": self.device.id.0,
            "name": self.device.name,
            "transport": transport_name(self.device.transport),
            "reason": self.reason.word(),
        })
    }
}

/// The output for `choice` among `outputs` (default marked): the chosen one, by id or by name and
/// transport, else the default. `None`: no output at all. For the Windows far end, which pins its
/// loopback to it (feat/audio-devices-win).
pub fn resolve_output(outputs: &[DeviceInfo], choice: &OutputChoice) -> Option<PickedOutput> {
    let default = || outputs.iter().find(|d| d.is_default).or(outputs.first());
    match choice {
        OutputChoice::Device(wanted) => match find_wanted(outputs, wanted) {
            Some(device) => Some((device, OutputReason::Chosen)),
            None => default().map(|d| (d, OutputReason::ChosenMissing)),
        },
        OutputChoice::Default => default().map(|d| (d, OutputReason::Default)),
    }
    .map(|(device, reason)| PickedOutput {
        device: device.clone(),
        reason,
    })
}

/// A device's JSON in the events (`AudioDevice`).
pub fn device_json(d: &DeviceInfo) -> Value {
    json!({
        "id": d.id.0,
        "name": d.name,
        "transport": transport_name(d.transport),
        "is_default": d.is_default,
    })
}

/// Turns a burst of device notifications into one read (the module docs). Times are host ns on
/// the core's clock; it keeps no clock of its own, so it is tested with a mock one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Coalescer {
    /// The burst's first and latest notification, while one is pending.
    pending: Option<(u64, u64)>,
}

impl Coalescer {
    /// A notification at `now_ns`.
    pub fn changed(&mut self, now_ns: u64) {
        let first = self.pending.map_or(now_ns, |(first, _)| first);
        self.pending = Some((first, now_ns));
    }

    /// When the pending burst is due to be read, if one is pending.
    pub fn deadline_ns(&self) -> Option<u64> {
        let ns = |d: Duration| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX);
        self.pending.map(|(first, last)| {
            last.saturating_add(ns(QUIET))
                .min(first.saturating_add(ns(MAX_WAIT)))
        })
    }

    /// Whether the burst is due at `now_ns`; if so it is taken, and nothing is pending.
    pub fn take_due(&mut self, now_ns: u64) -> bool {
        if self.deadline_ns().is_some_and(|d| now_ns >= d) {
            self.pending = None;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use ink_core::Clock;
    use ink_core::mock::{MemStore, MockClock, MockPlatform};

    use super::*;

    fn device(id: &str, name: &str, transport: Transport, is_default: bool) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId(id.into()),
            name: name.into(),
            transport,
            is_default,
        }
    }

    fn built_in() -> DeviceInfo {
        device("built-in", "Built-in Microphone", Transport::BuiltIn, false)
    }

    fn usb(id: &str) -> DeviceInfo {
        device(id, "USB Microphone", Transport::Usb, true)
    }

    fn buds() -> DeviceInfo {
        device("buds", "Example Buds", Transport::Bluetooth, false)
    }

    fn auto(d: &DeviceInfo, reason: AutoReason) -> Option<AutoInput> {
        Some(AutoInput {
            device: d.clone(),
            reason,
        })
    }

    fn chose(d: &DeviceInfo) -> InputChoice {
        InputChoice::Device(Wanted::of(d))
    }

    /// (inputs, choice, Automatic, the device and reason word expected).
    type Case = (
        Vec<DeviceInfo>,
        InputChoice,
        Option<AutoInput>,
        Option<(&'static str, &'static str)>,
    );

    /// The rule, case by case.
    #[test]
    fn the_resolver_takes_the_choice_then_its_name_then_automatic_then_nothing() {
        let routed = auto(&built_in(), AutoReason::BuiltInForBluetoothOutput);
        let bare = InputChoice::Device(Wanted {
            id: DeviceId("buds".into()),
            name: None,
            transport: None,
        });
        let cases: Vec<Case> = vec![
            // Automatic: the routing's pick and reason.
            (
                vec![usb("usb-1"), built_in()],
                InputChoice::Auto,
                routed.clone(),
                Some(("built-in", "built_in_for_bluetooth_output")),
            ),
            // The chosen Bluetooth mic is taken even with Bluetooth output: the user's call.
            (
                vec![built_in(), buds()],
                chose(&buds()),
                routed.clone(),
                Some(("buds", "chosen")),
            ),
            // The same mic on another port (a new endpoint id): found by name and transport.
            (
                vec![built_in(), usb("usb-2")],
                chose(&usb("usb-1")),
                routed.clone(),
                Some(("usb-2", "chosen")),
            ),
            // Not connected: Automatic stands in, said as such.
            (
                vec![built_in()],
                chose(&buds()),
                routed.clone(),
                Some(("built-in", "chosen_missing")),
            ),
            // A choice stored without its name is found by id only.
            (
                vec![built_in(), buds()],
                bare.clone(),
                routed.clone(),
                Some(("buds", "chosen")),
            ),
            (
                vec![built_in()],
                bare,
                routed.clone(),
                Some(("built-in", "chosen_missing")),
            ),
            // A name alone is not enough: another transport is another device.
            (
                vec![device("x", "USB Microphone", Transport::Bluetooth, false)],
                chose(&usb("usb-1")),
                auto(&buds(), AutoReason::DefaultInput),
                Some(("buds", "chosen_missing")),
            ),
            // No mic at all: nothing, never silence.
            (Vec::new(), InputChoice::Auto, None, None),
            (Vec::new(), chose(&buds()), None, None),
        ];
        for (i, (inputs, choice, automatic, want)) in cases.into_iter().enumerate() {
            let got = resolve_input(&inputs, &choice, automatic);
            let got_words = got
                .as_ref()
                .map(|p| (p.device.id.0.as_str(), p.reason.word()));
            assert_eq!(got_words, want, "case {i}");
            if let Some(p) = got {
                assert_eq!(
                    p.wanted.is_some(),
                    p.reason == MicReason::ChosenMissing,
                    "case {i}: only a stand-in names what it stands in for"
                );
            }
        }
    }

    /// Every word the resolver says is one the event schema allows, and each its own.
    #[test]
    fn each_mic_reason_is_a_schema_word() {
        let reasons = [
            MicReason::Chosen,
            MicReason::ChosenMissing,
            MicReason::Auto(AutoReason::DefaultInput),
            MicReason::Auto(AutoReason::BuiltInForBluetoothOutput),
            MicReason::Auto(AutoReason::NoBuiltInMic),
            MicReason::Auto(AutoReason::FirstInput),
            MicReason::Auto(AutoReason::LeAudioHeadset),
            MicReason::Auto(AutoReason::HeadsetMicSetting),
        ];
        let words: Vec<&str> = reasons.iter().map(|r| r.word()).collect();
        let mut distinct = words.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), words.len(), "{words:?}");
        let schema: Value = serde_json::from_str(crate::schema::EVENTS_SCHEMA).unwrap();
        let allowed = schema["$defs"]["MicReason"]["enum"].as_array().unwrap();
        for word in words.iter().chain(&["unknown"]) {
            assert!(allowed.iter().any(|a| a == word), "{word}");
        }
        let outputs = schema["$defs"]["OutputReason"]["enum"].as_array().unwrap();
        for r in [
            OutputReason::Chosen,
            OutputReason::ChosenMissing,
            OutputReason::Default,
        ] {
            assert!(outputs.iter().any(|a| a == r.word()), "{}", r.word());
        }
    }

    #[test]
    fn the_output_resolver_takes_the_choice_else_the_default() {
        let speakers = device("speakers", "Speakers", Transport::BuiltIn, true);
        let dock = device("dock", "Dock Audio", Transport::Usb, false);
        let outputs = vec![speakers.clone(), dock.clone()];
        let pick = |choice: &OutputChoice| {
            resolve_output(&outputs, choice).map(|p| (p.device.id.0, p.reason))
        };
        assert_eq!(
            pick(&OutputChoice::Default),
            Some(("speakers".into(), OutputReason::Default))
        );
        assert_eq!(
            pick(&OutputChoice::Device(Wanted::of(&dock))),
            Some(("dock".into(), OutputReason::Chosen))
        );
        let gone = device("tv", "Television", Transport::Other, false);
        assert_eq!(
            pick(&OutputChoice::Device(Wanted::of(&gone))),
            Some(("speakers".into(), OutputReason::ChosenMissing))
        );
        assert_eq!(resolve_output(&[], &OutputChoice::Default), None);
    }

    #[test]
    fn a_device_token_is_one_printable_line_of_at_most_512_bytes() {
        assert!(is_device_token("BuiltInMicrophoneDevice"));
        assert!(is_device_token(
            "{0.0.1.00000000}.{6a3a1b52-2c1e-4c3e-9f1a-1b2c3d4e5f60}"
        ));
        assert!(is_device_token(
            "AppleUSBAudioEngine:Vendor:USB Microphone:1:1"
        ));
        assert!(is_device_token(&"x".repeat(MAX_DEVICE_BYTES)));
        assert!(!is_device_token(&"x".repeat(MAX_DEVICE_BYTES + 1)));
        // 171 three-byte characters are 513 bytes.
        assert!(!is_device_token(&"\u{2603}".repeat(171)));
        for bad in [
            "",
            "auto",
            "default",
            "a\nb",
            "a\u{0}b",
            "a\u{2028}b",
            "tab\there",
        ] {
            assert!(!is_device_token(bad), "{bad:?}");
        }
    }

    /// The remembered name pairs only with its own id; a choice without one is found by id.
    #[test]
    fn the_choice_reads_its_remembered_device_only_for_its_own_id() {
        let store = MemStore::default();
        assert_eq!(input_choice(&store), InputChoice::Auto);
        store.set_setting(INPUT_KEY, AUTO).unwrap();
        assert_eq!(input_choice(&store), InputChoice::Auto);
        let wanted = Wanted::of(&buds());
        store
            .set_settings(&[
                (INPUT_DEVICE_KEY, &wanted.json().to_string()),
                (INPUT_KEY, "buds"),
            ])
            .unwrap();
        assert_eq!(input_choice(&store), InputChoice::Device(wanted));
        store.set_setting(INPUT_KEY, "other").unwrap();
        assert_eq!(
            input_choice(&store),
            InputChoice::Device(Wanted {
                id: DeviceId("other".into()),
                name: None,
                transport: None
            })
        );
        assert_eq!(output_choice(&store), OutputChoice::Default);
        store.set_setting(OUTPUT_KEY, "dock").unwrap();
        assert!(matches!(output_choice(&store), OutputChoice::Device(w) if w.id.0 == "dock"));
    }

    /// Over a platform: the routing is asked only when it is needed, and no mic is an error.
    #[test]
    fn pick_mic_reads_the_platform_and_refuses_no_mic() {
        let mock = MockPlatform::new().with_devices(vec![built_in(), buds()], None);
        let picked = pick_mic(&mock, &chose(&buds())).unwrap();
        assert_eq!(
            (picked.device.id.0.as_str(), picked.reason),
            ("buds", MicReason::Chosen)
        );
        let picked = pick_mic(&mock, &InputChoice::Auto).unwrap();
        assert_eq!(
            (picked.device.id.0.as_str(), picked.reason),
            ("built-in", MicReason::Auto(AutoReason::FirstInput))
        );
        let none = MockPlatform::new().with_devices(Vec::new(), None);
        assert_eq!(
            pick_mic(&none, &InputChoice::Auto),
            Err("there is no microphone".into())
        );
    }

    /// A burst is read once: 300 ms after its last notification, and never later than 1 s after
    /// its first, however long it goes on.
    #[test]
    fn a_burst_of_notifications_is_read_once_quiet_or_capped() {
        let clock = MockClock::new(1_000_000_000, 0);
        let ms = 1_000_000;
        let mut c = Coalescer::default();
        assert_eq!(c.deadline_ns(), None);
        assert!(!c.take_due(clock.now_ns()));

        c.changed(clock.now_ns());
        assert_eq!(c.deadline_ns(), Some(clock.now_ns() + 300 * ms));
        clock.advance_ns(200 * ms);
        c.changed(clock.now_ns());
        clock.advance_ns(299 * ms);
        assert!(!c.take_due(clock.now_ns()), "not quiet yet");
        clock.advance_ns(ms);
        assert!(c.take_due(clock.now_ns()), "quiet for 300 ms");
        assert!(!c.take_due(clock.now_ns()), "taken once");
        assert_eq!(c.deadline_ns(), None);

        // A notification every 200 ms never goes quiet: read once a second, at 1 s and 2 s.
        let start = clock.now_ns();
        let mut reads = Vec::new();
        for _ in 0..10 {
            c.changed(clock.now_ns());
            clock.advance_ns(200 * ms);
            if c.take_due(clock.now_ns()) {
                reads.push((clock.now_ns() - start) / ms);
            }
        }
        assert_eq!(reads, [1_000, 2_000]);
    }
}
