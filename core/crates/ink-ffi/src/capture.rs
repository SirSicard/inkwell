//! A meeting's capture from this machine's devices: the mic and the far end, opened for
//! `meeting.start`, never started here (the meeting run starts them once its chain has).
//!
//! The platform decides which mic (with Bluetooth output, the built-in one, unless the user's
//! headset-mic setting says otherwise) and taps the far end: the meeting's app when one is known,
//! everything this machine plays otherwise (the app itself is never tapped). Errors name the
//! device or the app, never audio.
//!
//! On Windows ([`WinMeetingCapture`]) the far end of an app is S0.4's plan: process loopback,
//! which hears the app alone, for Zoom and the browsers; device loopback of the output the app
//! plays to for every other app (per-process loopback is silent on new Teams), which hears
//! everything that device plays and is said as such (`far_end` "everything").

use ink_core::{AppRef, Transport};
#[cfg(any(target_os = "macos", windows))]
use ink_pipeline::meeting::watchdog::FarDelivery;
use ink_pipeline::meeting::watchdog::Routing;

use crate::meeting::CaptureSide;

/// The mic a meeting records, as the shell may show it ("why is it using the laptop mic?").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MicInfo {
    /// The device's name as the OS shows it.
    pub name: String,
    /// How it connects.
    pub transport: Transport,
    /// Why it was chosen: a schema word (`default_input`, `built_in_for_bluetooth_output`,
    /// `headset_mic_setting`, `no_built_in_mic`, `first_input`, `requested`, `le_audio_headset`
    /// (Windows), or `unknown` for a reason this build does not name).
    pub reason: &'static str,
}

/// What a meeting's far end records.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum FarScope {
    /// The meeting's app alone.
    App,
    /// Everything this machine plays, except this app: Record now, which names no app (and
    /// replays, which have no devices).
    #[default]
    Everything,
    /// Everything this machine plays, except this app, because the meeting's app could not be
    /// tapped alone: other apps' sound is recorded too, and the shell must say so
    /// (`meeting.far_end_fallback`). Why, as the platform said it.
    EverythingInstead(String),
}

impl FarScope {
    /// Its schema word (`FarEnd`).
    pub fn name(&self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Everything | Self::EverythingInstead(_) => "everything",
        }
    }
}

/// Both sides of a meeting's capture, opened and not started.
pub struct Opened {
    /// The mic, then the far end.
    pub sides: Vec<CaptureSide>,
    /// How they are routed, for the silent-channel watchdog.
    pub routing: Routing,
    /// The mic, when the platform says which it is.
    pub mic: Option<MicInfo>,
    /// What the far end records.
    pub far: FarScope,
}

/// Opens a meeting's capture on this machine.
pub trait MeetingCapture: Send + Sync {
    /// **Worker.** The mic and the far end for a meeting with `app` (`None`: everything this
    /// machine plays), opened and not started. `headset_mic`: the user's setting.
    fn open(&self, app: Option<&AppRef>, headset_mic: bool) -> Result<Opened, String>;
}

/// No devices: a platform without capture, or a test core.
pub struct NoCapture;

impl MeetingCapture for NoCapture {
    fn open(&self, _: Option<&AppRef>, _: bool) -> Result<Opened, String> {
        Err("this platform cannot capture a meeting yet".into())
    }
}

/// A transport's schema word.
pub fn transport_name(t: Transport) -> &'static str {
    match t {
        Transport::BuiltIn => "built_in",
        Transport::Bluetooth => "bluetooth",
        Transport::Usb => "usb",
        Transport::Virtual => "virtual",
        _ => "other",
    }
}

#[cfg(target_os = "macos")]
pub use mac::MacMeetingCapture;

#[cfg(target_os = "macos")]
mod mac {
    use ink_audio::DEFAULT_RING_DURATION;
    use ink_core::{AppRef, AudioSource, FarEndTarget};
    use ink_platform_mac::MacCapture;
    use ink_platform_mac::capture::MicRouteReason;

    use super::*;

    /// [`MeetingCapture`] on the Mac: the routed mic's own IOProc and a process tap.
    pub struct MacMeetingCapture {
        capture: MacCapture,
    }

    impl MacMeetingCapture {
        /// Capture on the platform clock.
        pub fn new(capture: MacCapture) -> Self {
            Self { capture }
        }
    }

    fn reason(r: MicRouteReason) -> &'static str {
        match r {
            MicRouteReason::Requested => "requested",
            MicRouteReason::DefaultInput => "default_input",
            MicRouteReason::BuiltInForBluetoothOutput => "built_in_for_bluetooth_output",
            MicRouteReason::HeadsetMicSetting => "headset_mic_setting",
            MicRouteReason::NoBuiltInMic => "no_built_in_mic",
            MicRouteReason::FirstInput => "first_input",
            // The platform's enum is non-exhaustive: a reason added there is said to be unknown,
            // never passed off as another one.
            _ => "unknown",
        }
    }

    impl MeetingCapture for MacMeetingCapture {
        fn open(&self, app: Option<&AppRef>, headset_mic: bool) -> Result<Opened, String> {
            self.capture.set_headset_mic(headset_mic);
            let (device, why) = self
                .capture
                .mic_route()
                .map_err(|e| format!("the microphone: {e}"))?
                .ok_or("there is no microphone")?;
            let mic = self
                .capture
                .open_mic_source(Some(&device.id))
                .map_err(|e| format!("the microphone {}: {e}", device.name))?;
            let transport = mic.transport();
            let (far, scope) = match app {
                Some(app) => match self
                    .capture
                    .open_far_end_source(&FarEndTarget::Apps(vec![app.clone()]))
                {
                    Ok(far) => (Ok(far), FarScope::App),
                    Err(e) => {
                        // The app has no audio process the tap can name (it may play through a
                        // helper under another id): everything this Mac plays is the far end
                        // instead, which still leaves this app out. Other apps' sound is then in
                        // the recording, so the shell is told (meeting.far_end_fallback).
                        log::warn!(
                            "meeting: the far end of {} could not be tapped ({e}); tapping everything this Mac plays",
                            app.id
                        );
                        (
                            self.capture.open_far_end_source(&FarEndTarget::AllOutput),
                            FarScope::EverythingInstead(e.to_string()),
                        )
                    }
                },
                None => (
                    self.capture.open_far_end_source(&FarEndTarget::AllOutput),
                    FarScope::Everything,
                ),
            };
            let far = far.map_err(|e| format!("the other side's sound: {e}"))?;
            let side = |source: Box<dyn AudioSource>| CaptureSide {
                source,
                ring: DEFAULT_RING_DURATION,
                start_at: None,
            };
            Ok(Opened {
                sides: vec![side(Box::new(mic)), side(Box::new(far))],
                routing: Routing {
                    mic: transport,
                    far: FarDelivery::WhilePlaying,
                },
                mic: Some(MicInfo {
                    name: device.name,
                    transport,
                    reason: reason(why),
                }),
                far: scope,
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Review (S2.8): each reason the platform names has its own schema word, none of them
        /// "unknown", and every word (with "unknown") is one the event schema allows.
        #[test]
        fn each_mic_reason_is_its_own_schema_word() {
            let words: Vec<&str> = [
                MicRouteReason::Requested,
                MicRouteReason::DefaultInput,
                MicRouteReason::BuiltInForBluetoothOutput,
                MicRouteReason::HeadsetMicSetting,
                MicRouteReason::NoBuiltInMic,
                MicRouteReason::FirstInput,
            ]
            .into_iter()
            .map(reason)
            .collect();
            let mut distinct = words.clone();
            distinct.sort_unstable();
            distinct.dedup();
            assert_eq!(distinct.len(), words.len(), "{words:?}");
            assert!(!words.contains(&"unknown"));
            let schema: serde_json::Value =
                serde_json::from_str(crate::schema::EVENTS_SCHEMA).unwrap();
            let allowed = schema["$defs"]["MicReason"]["enum"].as_array().unwrap();
            for word in words.iter().chain(&["unknown"]) {
                assert!(allowed.iter().any(|a| a == word), "{word}");
            }
        }
    }
}

#[cfg(windows)]
pub use win::{WinDevices, WinMeetingCapture};

#[cfg(windows)]
mod win {
    use ink_audio::DEFAULT_RING_DURATION;
    use ink_core::{AppRef, AudioSource, DeviceId, FarEndTarget, PlatformError};
    use ink_platform_win::WinCapture;
    use ink_platform_win::capture::{Endpoint, MicRouteReason};

    use super::*;

    /// What [`WinMeetingCapture`] asks of the platform: [`WinCapture`] in the app, a stand-in
    /// that opens replay sources in the tests (CI has no devices). **Worker**, every method.
    pub trait WinDevices: Send + Sync {
        /// The mic a meeting records with the headset-mic setting at `headset_mic`, and why;
        /// `None` when there is no input at all.
        fn route_mic(
            &self,
            headset_mic: bool,
        ) -> Result<Option<(Endpoint, MicRouteReason)>, PlatformError>;
        /// Opens the mic `device`, not started.
        fn open_mic(&self, device: &DeviceId) -> Result<Box<dyn AudioSource>, PlatformError>;
        /// Opens the far end for `target`, not started, and whether it hears one process tree
        /// alone (process loopback) rather than everything an output device plays.
        fn open_far(
            &self,
            target: &FarEndTarget,
        ) -> Result<(Box<dyn AudioSource>, bool), PlatformError>;
    }

    impl WinDevices for WinCapture {
        fn route_mic(
            &self,
            headset_mic: bool,
        ) -> Result<Option<(Endpoint, MicRouteReason)>, PlatformError> {
            self.set_headset_mic(headset_mic);
            self.mic_route()
        }

        fn open_mic(&self, device: &DeviceId) -> Result<Box<dyn AudioSource>, PlatformError> {
            Ok(Box::new(self.open_mic_source(Some(device))?))
        }

        fn open_far(
            &self,
            target: &FarEndTarget,
        ) -> Result<(Box<dyn AudioSource>, bool), PlatformError> {
            let far = self.open_far_end_source(target)?;
            let alone = far.is_process_loopback();
            Ok((Box::new(far), alone))
        }
    }

    /// [`MeetingCapture`] on Windows: the routed mic (WASAPI) and the far end by S0.4's plan
    /// (the module docs), both stamped on the performance counter.
    pub struct WinMeetingCapture {
        devices: Box<dyn WinDevices>,
    }

    impl WinMeetingCapture {
        /// Capture from `devices`.
        pub fn new(devices: impl WinDevices + 'static) -> Self {
            Self {
                devices: Box::new(devices),
            }
        }
    }

    /// The schema word (`MicReason`) for why Windows chose a mic.
    pub(super) fn reason(r: MicRouteReason) -> &'static str {
        match r {
            MicRouteReason::Requested => "requested",
            MicRouteReason::DefaultInput => "default_input",
            MicRouteReason::LeAudioHeadset => "le_audio_headset",
            // Not the Bluetooth headset's call-quality mic because the output is Bluetooth: on
            // Windows the mic kept may be USB as well as built in (the schema's word says both).
            MicRouteReason::NotBluetoothForBluetoothOutput => "built_in_for_bluetooth_output",
            MicRouteReason::HeadsetMicSetting => "headset_mic_setting",
            // The output is Bluetooth and no other mic exists: the default, as the Mac's word
            // for a Mac without a built-in mic says.
            MicRouteReason::OnlyBluetoothMics => "no_built_in_mic",
            MicRouteReason::FirstInput => "first_input",
            // The platform's enum is non-exhaustive: a reason added there is said to be unknown,
            // never passed off as another one.
            _ => "unknown",
        }
    }

    impl MeetingCapture for WinMeetingCapture {
        fn open(&self, app: Option<&AppRef>, headset_mic: bool) -> Result<Opened, String> {
            let (device, why) = self
                .devices
                .route_mic(headset_mic)
                .map_err(|e| format!("the microphone: {e}"))?
                .ok_or("there is no microphone")?;
            let mic = self
                .devices
                .open_mic(&device.info.id)
                .map_err(|e| format!("the microphone {}: {e}", device.info.name))?;
            let transport = device.info.transport;
            let everything = || {
                self.devices
                    .open_far(&FarEndTarget::AllOutput)
                    .map(|(far, _)| far)
            };
            let (far, scope) = match app {
                Some(app) => match self
                    .devices
                    .open_far(&FarEndTarget::Apps(vec![app.clone()]))
                {
                    Ok((far, true)) => (Ok(far), FarScope::App),
                    // Device loopback of the output the app plays to (Teams, and every app but
                    // Zoom and the browsers): by plan, not a fallback, and it hears everything
                    // that device plays, so it is said as everything.
                    Ok((far, false)) => (Ok(far), FarScope::Everything),
                    Err(e) => {
                        // Zoom or a browser that is no longer running, or an output that cannot
                        // be opened: the default output instead. Other apps' sound is then in
                        // the recording, so the shell is told (meeting.far_end_fallback).
                        log::warn!(
                            "meeting: the far end of {} could not be opened ({e}); recording everything the default output plays",
                            app.id
                        );
                        (everything(), FarScope::EverythingInstead(e.to_string()))
                    }
                },
                None => (everything(), FarScope::Everything),
            };
            let far = far.map_err(|e| format!("the other side's sound: {e}"))?;
            let side = |source: Box<dyn AudioSource>| CaptureSide {
                source,
                ring: DEFAULT_RING_DURATION,
                start_at: None,
            };
            Ok(Opened {
                sides: vec![side(mic), side(far)],
                routing: Routing {
                    mic: transport,
                    // Loopback delivers nothing while nothing plays: idle, not stalled.
                    far: FarDelivery::WhilePlaying,
                },
                mic: Some(MicInfo {
                    name: device.info.name,
                    transport,
                    reason: reason(why),
                }),
                far: scope,
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use std::sync::Mutex;

        use ink_core::{AudioSink, Channel, DeviceInfo, SourceStats, StreamFormat};

        use super::*;

        /// Each reason Windows names has its own schema word, none of them "unknown", and every
        /// word is one the event schema allows.
        #[test]
        fn each_mic_reason_is_its_own_schema_word() {
            let words: Vec<&str> = [
                MicRouteReason::Requested,
                MicRouteReason::DefaultInput,
                MicRouteReason::LeAudioHeadset,
                MicRouteReason::NotBluetoothForBluetoothOutput,
                MicRouteReason::HeadsetMicSetting,
                MicRouteReason::OnlyBluetoothMics,
                MicRouteReason::FirstInput,
            ]
            .into_iter()
            .map(reason)
            .collect();
            let mut distinct = words.clone();
            distinct.sort_unstable();
            distinct.dedup();
            assert_eq!(distinct.len(), words.len(), "{words:?}");
            assert!(!words.contains(&"unknown"));
            let schema: serde_json::Value =
                serde_json::from_str(crate::schema::EVENTS_SCHEMA).unwrap();
            let allowed = schema["$defs"]["MicReason"]["enum"].as_array().unwrap();
            for word in words.iter().chain(&["unknown"]) {
                assert!(allowed.iter().any(|a| a == word), "{word}");
            }
        }

        /// A source that only says what it is (these tests never start one).
        struct Stub(Channel);

        impl AudioSource for Stub {
            fn channel(&self) -> Channel {
                self.0
            }

            fn format(&self) -> StreamFormat {
                StreamFormat {
                    sample_rate: 48_000,
                    channels: 1,
                }
            }

            fn start(&mut self, _: Box<dyn AudioSink>) -> Result<(), PlatformError> {
                Err(PlatformError::Failed("a stub".into()))
            }

            fn stop(&mut self) -> Result<SourceStats, PlatformError> {
                Ok(SourceStats::default())
            }
        }

        /// What the far end answers for an app: process loopback, device loopback, or an error.
        #[derive(Clone, Copy)]
        enum AppFar {
            Alone,
            Device,
            Fails,
        }

        struct Devices {
            mic: Option<(Endpoint, MicRouteReason)>,
            app_far: AppFar,
            all_output_fails: bool,
            /// What was asked: the headset setting, the mic opened, each far-end target.
            asked: Mutex<Vec<String>>,
        }

        fn endpoint(name: &str, transport: Transport) -> Endpoint {
            Endpoint {
                info: DeviceInfo {
                    id: DeviceId(format!("{{0.0.1.00000000}}.{name}")),
                    name: name.into(),
                    transport,
                    is_default: true,
                },
                container: None,
                rate: Some(48_000),
            }
        }

        fn devices(app_far: AppFar) -> Devices {
            Devices {
                mic: Some((
                    endpoint("Microphone (USB Audio)", Transport::Usb),
                    MicRouteReason::DefaultInput,
                )),
                app_far,
                all_output_fails: false,
                asked: Mutex::new(Vec::new()),
            }
        }

        impl WinDevices for &'static Devices {
            fn route_mic(
                &self,
                headset_mic: bool,
            ) -> Result<Option<(Endpoint, MicRouteReason)>, PlatformError> {
                self.asked
                    .lock()
                    .unwrap()
                    .push(format!("headset {headset_mic}"));
                Ok(self.mic.clone())
            }

            fn open_mic(&self, device: &DeviceId) -> Result<Box<dyn AudioSource>, PlatformError> {
                self.asked.lock().unwrap().push(format!("mic {}", device.0));
                Ok(Box::new(Stub(Channel::Mic)))
            }

            fn open_far(
                &self,
                target: &FarEndTarget,
            ) -> Result<(Box<dyn AudioSource>, bool), PlatformError> {
                let far = || Box::new(Stub(Channel::Far)) as Box<dyn AudioSource>;
                match target {
                    FarEndTarget::AllOutput => {
                        self.asked.lock().unwrap().push("far all output".into());
                        if self.all_output_fails {
                            return Err(PlatformError::Device("no output device".into()));
                        }
                        Ok((far(), false))
                    }
                    FarEndTarget::Apps(apps) => {
                        let ids: Vec<&str> = apps.iter().map(|a| a.id.as_str()).collect();
                        self.asked.lock().unwrap().push(format!("far {ids:?}"));
                        match self.app_far {
                            AppFar::Alone => Ok((far(), true)),
                            AppFar::Device => Ok((far(), false)),
                            AppFar::Fails => {
                                Err(PlatformError::Device("Zoom.exe is not running".into()))
                            }
                        }
                    }
                }
            }
        }

        fn leak(d: Devices) -> &'static Devices {
            Box::leak(Box::new(d))
        }

        fn app(id: &str) -> AppRef {
            AppRef {
                id: id.into(),
                pid: Some(4_321),
                name: id.trim_end_matches(".exe").into(),
            }
        }

        fn channels(opened: &Opened) -> Vec<Channel> {
            opened.sides.iter().map(|s| s.source.channel()).collect()
        }

        #[test]
        fn zoom_is_heard_alone_through_the_routed_mic() {
            let d = leak(devices(AppFar::Alone));
            let opened = WinMeetingCapture::new(d)
                .open(Some(&app("Zoom.exe")), false)
                .unwrap();
            assert_eq!(channels(&opened), [Channel::Mic, Channel::Far]);
            assert_eq!(opened.far, FarScope::App);
            assert_eq!(
                opened.routing,
                Routing {
                    mic: Transport::Usb,
                    far: FarDelivery::WhilePlaying
                }
            );
            assert_eq!(
                opened.mic,
                Some(MicInfo {
                    name: "Microphone (USB Audio)".into(),
                    transport: Transport::Usb,
                    reason: "default_input",
                })
            );
            assert_eq!(
                *d.asked.lock().unwrap(),
                [
                    "headset false",
                    "mic {0.0.1.00000000}.Microphone (USB Audio)",
                    "far [\"Zoom.exe\"]",
                ]
            );
        }

        /// Teams' far end is its output device, which hears everything that device plays: said
        /// as everything, and not as a fallback.
        #[test]
        fn an_app_on_device_loopback_is_said_to_record_everything() {
            let d = leak(devices(AppFar::Device));
            let opened = WinMeetingCapture::new(d)
                .open(Some(&app("ms-teams.exe")), true)
                .unwrap();
            assert_eq!(opened.far, FarScope::Everything);
            assert_eq!(opened.far.name(), "everything");
            assert_eq!(d.asked.lock().unwrap()[0], "headset true");
            assert!(
                !d.asked
                    .lock()
                    .unwrap()
                    .contains(&"far all output".to_owned())
            );
        }

        /// An app whose far end cannot be opened (Zoom gone, say): the default output instead,
        /// and why, for `meeting.far_end_fallback`.
        #[test]
        fn an_app_that_cannot_be_heard_falls_back_to_the_default_output_and_says_why() {
            let d = leak(devices(AppFar::Fails));
            let opened = WinMeetingCapture::new(d)
                .open(Some(&app("Zoom.exe")), false)
                .unwrap();
            assert_eq!(channels(&opened), [Channel::Mic, Channel::Far]);
            assert_eq!(
                opened.far,
                FarScope::EverythingInstead(
                    PlatformError::Device("Zoom.exe is not running".into()).to_string()
                )
            );
            assert_eq!(
                d.asked.lock().unwrap()[2..],
                ["far [\"Zoom.exe\"]", "far all output"]
            );
        }

        #[test]
        fn record_now_records_the_default_output() {
            let d = leak(devices(AppFar::Fails));
            let opened = WinMeetingCapture::new(d).open(None, false).unwrap();
            assert_eq!(opened.far, FarScope::Everything);
            assert_eq!(d.asked.lock().unwrap()[2..], ["far all output"]);
        }

        #[test]
        fn a_meeting_without_a_mic_or_an_output_does_not_open() {
            let mut none = devices(AppFar::Alone);
            none.mic = None;
            assert_eq!(
                WinMeetingCapture::new(leak(none)).open(None, false).err(),
                Some("there is no microphone".into())
            );
            let mut silent = devices(AppFar::Fails);
            silent.all_output_fails = true;
            let err = WinMeetingCapture::new(leak(silent))
                .open(Some(&app("Zoom.exe")), false)
                .err()
                .unwrap();
            assert!(err.starts_with("the other side's sound: "), "{err}");
        }

        /// A Bluetooth mic's routing reaches the watchdog, which then takes its zeros for the
        /// user's silence (S3.1: a classic headset gates to zeros).
        #[test]
        fn a_bluetooth_mic_is_routed_as_bluetooth() {
            let mut d = devices(AppFar::Alone);
            d.mic = Some((
                endpoint("Headset (Earbuds)", Transport::Bluetooth),
                MicRouteReason::LeAudioHeadset,
            ));
            let opened = WinMeetingCapture::new(leak(d)).open(None, false).unwrap();
            assert_eq!(opened.routing.mic, Transport::Bluetooth);
            assert_eq!(opened.mic.unwrap().reason, "le_audio_headset");
        }
    }
}
