//! A meeting's capture from this machine's devices: the mic and the far end, opened for
//! `meeting.start`, never started here (the meeting run starts them once its chain has).
//!
//! The platform decides which mic (with Bluetooth output, the built-in one, unless the user's
//! headset-mic setting says otherwise) and taps the far end: the meeting's app when one is known,
//! everything this machine plays otherwise (on the Mac, Inkwell itself is never tapped). Errors
//! name the device or the app, never audio.
//!
//! On Windows ([`WinMeetingCapture`]) the far end of an app is S0.4's plan: process loopback,
//! which hears the app alone, for Zoom and the browsers; device loopback of the output the app
//! plays to for every other app (per-process loopback is silent on new Teams), which hears
//! everything that device plays, Inkwell's own sounds included, and is said as such (`far_end`
//! "everything"). Device loopback is bound to one output, so it moves with the call ([`Follow`]):
//! a headset plugged in mid-call, an output unplugged, a new default under Record now.

use ink_core::{AppRef, AudioSource, Transport};
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
    /// Everything this machine plays (on the Mac, except this app): Record now, which names no
    /// app, a Windows app recorded from its output device, and replays, which have no devices.
    #[default]
    Everything,
    /// Everything this machine plays (on the Mac, except this app), because the meeting's app
    /// could not be heard alone: other apps' sound is recorded too, and the shell must say so
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

/// A side opened again elsewhere ([`Follow::moved`]): its new source, opened and not started, and
/// what it records (a device's name, for the log).
pub type Reopened = (Box<dyn AudioSource>, String);

/// A side whose device can change under a meeting: while it records, the pump asks it every
/// [`FOLLOW_INTERVAL`](crate::meeting::FOLLOW_INTERVAL), and at once when its source ended by
/// itself, whether to open the side again elsewhere. **Pump**, every method.
pub trait Follow: Send {
    /// A new source for the side, opened and not started, and what it records (a device's name,
    /// for the log), when the side should move: its device went, or its app plays elsewhere now.
    /// `None`: it stays. With `again` (its source ended by itself, or could not be replaced), it
    /// opens the side again wherever it should be, even where it was. Errors name the device or
    /// the app, never audio.
    fn moved(&mut self, again: bool) -> Result<Option<Reopened>, String>;
}

/// Opens a meeting's capture on this machine.
pub trait MeetingCapture: Send + Sync {
    /// **Worker.** The mic and the far end for a meeting with `app` (`None`: everything this
    /// machine plays), opened and not started. `headset_mic`: the user's setting.
    fn open(&self, app: Option<&AppRef>, headset_mic: bool) -> Result<Opened, String>;

    /// **Worker.** What the far end of `app` would record, when the platform knows without
    /// opening anything: on Windows, everything for an app its plan gives device loopback.
    /// `None` when only [`open`](Self::open) can tell (the default; the Mac's tap may find no
    /// process for the app, and an app given process loopback may not be running).
    fn planned_far(&self, _app: &AppRef) -> Option<FarScope> {
        None
    }
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
            // The tap follows its process whatever output it plays to: nothing moves it.
            let side = |source: Box<dyn AudioSource>| CaptureSide {
                source,
                ring: DEFAULT_RING_DURATION,
                start_at: None,
                follow: None,
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
pub use win::{FarHears, WinDevices, WinMeetingCapture};

#[cfg(windows)]
mod win {
    use std::sync::Arc;

    use ink_audio::DEFAULT_RING_DURATION;
    use ink_core::{AppRef, AudioSource, DeviceId, FarEndTarget, PlatformError};
    use ink_platform_win::WinCapture;
    use ink_platform_win::capture::{Endpoint, MicRouteReason};

    use super::*;

    /// What a far end opened by [`WinDevices::open_far`] hears.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum FarHears {
        /// One process tree alone (process loopback), wherever it plays.
        App,
        /// Everything one output device plays (device loopback).
        Device {
            /// The endpoint's id.
            id: String,
            /// Its name as Windows shows it.
            name: String,
        },
    }

    /// What [`WinMeetingCapture`] asks of the platform: [`WinCapture`] in the app, a stand-in
    /// that opens replay sources in the tests (CI has no devices). **Worker**, every method (and
    /// the meeting's pump, for a far end that moves).
    pub trait WinDevices: Send + Sync {
        /// The mic a meeting records with the headset-mic setting at `headset_mic`, and why;
        /// `None` when there is no input at all.
        fn route_mic(
            &self,
            headset_mic: bool,
        ) -> Result<Option<(Endpoint, MicRouteReason)>, PlatformError>;
        /// Opens the mic `device`, not started.
        fn open_mic(&self, device: &DeviceId) -> Result<Box<dyn AudioSource>, PlatformError>;
        /// Opens the far end for `target`, not started, and what it hears.
        fn open_far(
            &self,
            target: &FarEndTarget,
        ) -> Result<(Box<dyn AudioSource>, FarHears), PlatformError>;
        /// Whether a device-loopback far end opened for `target` on the output `endpoint` should
        /// be opened again (the platform's `far_end_moved`). Opens nothing.
        fn far_moved(&self, target: &FarEndTarget, endpoint: &str) -> Result<bool, PlatformError>;
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
        ) -> Result<(Box<dyn AudioSource>, FarHears), PlatformError> {
            let far = self.open_far_end_source(target)?;
            let hears = match far.endpoint() {
                Some(id) if !far.is_process_loopback() => FarHears::Device {
                    id: id.to_owned(),
                    name: far.device_name().to_owned(),
                },
                _ => FarHears::App,
            };
            Ok((Box::new(far), hears))
        }

        fn far_moved(&self, target: &FarEndTarget, endpoint: &str) -> Result<bool, PlatformError> {
            self.far_end_moved(target, endpoint)
        }
    }

    /// [`MeetingCapture`] on Windows: the routed mic (WASAPI) and the far end by S0.4's plan
    /// (the module docs), both stamped on the performance counter.
    pub struct WinMeetingCapture {
        devices: Arc<dyn WinDevices>,
    }

    impl WinMeetingCapture {
        /// Capture from `devices`.
        pub fn new(devices: impl WinDevices + 'static) -> Self {
            Self {
                devices: Arc::new(devices),
            }
        }
    }

    /// A device-loopback far end, moved with its call ([`Follow`]): to where the app plays now,
    /// or to the new default output when it records all output.
    struct FollowOutput {
        devices: Arc<dyn WinDevices>,
        /// What it was opened for: the app, or all output (Record now, or an app that could not
        /// be heard alone).
        target: FarEndTarget,
        /// The output it records now.
        endpoint: String,
        /// Why the last question could not be answered, logged once until it can.
        unasked: Option<String>,
    }

    impl Follow for FollowOutput {
        fn moved(&mut self, again: bool) -> Result<Option<Reopened>, String> {
            if !again {
                match self.devices.far_moved(&self.target, &self.endpoint) {
                    Ok(moved) => {
                        self.unasked = None;
                        if !moved {
                            return Ok(None);
                        }
                    }
                    Err(e) => {
                        // Where the call plays could not be read: it stays where it is, which
                        // may well be right, and is asked again next time.
                        let e = e.to_string();
                        if self.unasked.as_ref() != Some(&e) {
                            log::warn!(
                                "meeting: where the other side plays could not be read: {e}"
                            );
                            self.unasked = Some(e);
                        }
                        return Ok(None);
                    }
                }
            }
            let theirs = |e: PlatformError| format!("the other side's sound: {e}");
            let (far, hears) = self.devices.open_far(&self.target).map_err(theirs)?;
            let what = match hears {
                FarHears::Device { id, name } => {
                    self.endpoint = id;
                    name
                }
                // Not for these targets (the plan gives process loopback only to Zoom and the
                // browsers, which are never followed); it would hear the app wherever it plays.
                FarHears::App => "the app alone".to_owned(),
            };
            Ok(Some((far, what)))
        }
    }

    /// The schema word (`MicReason`) for why Windows chose a mic.
    fn reason(r: MicRouteReason) -> &'static str {
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
        fn planned_far(&self, app: &AppRef) -> Option<FarScope> {
            // The plan's own list, compared as it compares it: any other app is heard by device
            // loopback, whatever runs. One on the list is known only once opened (not running:
            // everything instead).
            let alone = ink_platform_win::capture::PROCESS_LOOPBACK_APPS
                .iter()
                .any(|exe| exe.eq_ignore_ascii_case(&app.id));
            (!alone).then_some(FarScope::Everything)
        }

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
                    .map(|opened| (opened, FarEndTarget::AllOutput))
            };
            let (far, scope) = match app {
                Some(app) => {
                    let target = FarEndTarget::Apps(vec![app.clone()]);
                    match self.devices.open_far(&target) {
                        Ok(opened @ (_, FarHears::App)) => (Ok((opened, target)), FarScope::App),
                        // Device loopback of the output the app plays to (Teams, and every app
                        // but Zoom and the browsers): by plan, not a fallback, and it hears
                        // everything that device plays, so it is said as everything.
                        Ok(opened) => (Ok((opened, target)), FarScope::Everything),
                        Err(e) => {
                            // Zoom or a browser that is no longer running, or an output that
                            // cannot be opened: the default output instead. Other apps' sound is
                            // then in the recording, so the shell is told
                            // (meeting.far_end_fallback).
                            log::warn!(
                                "meeting: the far end of {} could not be opened ({e}); recording everything the default output plays",
                                app.id
                            );
                            (everything(), FarScope::EverythingInstead(e.to_string()))
                        }
                    }
                }
                None => (everything(), FarScope::Everything),
            };
            let ((far, hears), target) = far.map_err(|e| format!("the other side's sound: {e}"))?;
            // Device loopback is bound to its output: it moves with the call. Process loopback
            // hears its app wherever it plays.
            let follow = match hears {
                FarHears::Device { id, .. } => Some(Box::new(FollowOutput {
                    devices: Arc::clone(&self.devices),
                    target,
                    endpoint: id,
                    unasked: None,
                }) as Box<dyn Follow>),
                FarHears::App => None,
            };
            let side = |source: Box<dyn AudioSource>, follow| CaptureSide {
                source,
                ring: DEFAULT_RING_DURATION,
                start_at: None,
                follow,
            };
            Ok(Opened {
                // A mic that is switched is not followed: it stops, and the watchdog says so.
                sides: vec![side(mic, None), side(far, follow)],
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
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
            all_output_fails: AtomicBool,
            /// What `far_moved` answers.
            moved: AtomicBool,
            /// Device-loopback far ends opened so far: each is on the output "out-N".
            outputs: AtomicUsize,
            /// What was asked: the headset setting, the mic opened, each far-end target, each
            /// question whether the far end moved.
            asked: Mutex<Vec<String>>,
        }

        impl Devices {
            /// A device-loopback far end on the next output.
            fn on_device(&self) -> FarHears {
                let n = self.outputs.fetch_add(1, Ordering::Relaxed);
                FarHears::Device {
                    id: format!("out-{n}"),
                    name: format!("Speakers {n}"),
                }
            }
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
                all_output_fails: AtomicBool::new(false),
                moved: AtomicBool::new(false),
                outputs: AtomicUsize::new(0),
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
            ) -> Result<(Box<dyn AudioSource>, FarHears), PlatformError> {
                let far = || Box::new(Stub(Channel::Far)) as Box<dyn AudioSource>;
                match target {
                    FarEndTarget::AllOutput => {
                        self.asked.lock().unwrap().push("far all output".into());
                        if self.all_output_fails.load(Ordering::Relaxed) {
                            return Err(PlatformError::Device("no output device".into()));
                        }
                        Ok((far(), self.on_device()))
                    }
                    FarEndTarget::Apps(apps) => {
                        let ids: Vec<&str> = apps.iter().map(|a| a.id.as_str()).collect();
                        self.asked.lock().unwrap().push(format!("far {ids:?}"));
                        match self.app_far {
                            AppFar::Alone => Ok((far(), FarHears::App)),
                            AppFar::Device => Ok((far(), self.on_device())),
                            AppFar::Fails => {
                                Err(PlatformError::Device("Zoom.exe is not running".into()))
                            }
                        }
                    }
                }
            }

            fn far_moved(
                &self,
                target: &FarEndTarget,
                endpoint: &str,
            ) -> Result<bool, PlatformError> {
                let what = match target {
                    FarEndTarget::AllOutput => "all output".to_owned(),
                    FarEndTarget::Apps(apps) => apps[0].id.clone(),
                };
                self.asked
                    .lock()
                    .unwrap()
                    .push(format!("moved? {what} on {endpoint}"));
                Ok(self.moved.load(Ordering::Relaxed))
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
            let silent = devices(AppFar::Fails);
            silent.all_output_fails.store(true, Ordering::Relaxed);
            let err = WinMeetingCapture::new(leak(silent))
                .open(Some(&app("Zoom.exe")), false)
                .err()
                .unwrap();
            assert!(err.starts_with("the other side's sound: "), "{err}");
        }

        /// S3.5b: a device-loopback far end follows its call; process loopback (Zoom) and the
        /// mic have nothing to follow. It asks whether it moved, opens again for the same target
        /// when it did (or when told to open again), and from then on asks about its new output.
        #[test]
        fn a_device_loopback_far_end_follows_its_call() {
            let d = leak(devices(AppFar::Alone));
            let zoom = WinMeetingCapture::new(d)
                .open(Some(&app("Zoom.exe")), false)
                .unwrap();
            assert!(zoom.sides.iter().all(|s| s.follow.is_none()));

            let d = leak(devices(AppFar::Device));
            let mut opened = WinMeetingCapture::new(d)
                .open(Some(&app("ms-teams.exe")), false)
                .unwrap();
            assert!(opened.sides[0].follow.is_none(), "the mic is not followed");
            let follow = opened.sides[1].follow.as_mut().expect("the far end moves");
            assert!(follow.moved(false).unwrap().is_none(), "it stays");
            d.moved.store(true, Ordering::Relaxed);
            let (source, what) = follow.moved(false).unwrap().expect("it moved");
            assert_eq!(source.channel(), Channel::Far);
            assert_eq!(what, "Speakers 1");
            d.moved.store(false, Ordering::Relaxed);
            let (_, what) = follow.moved(true).unwrap().expect("opened again");
            assert_eq!(what, "Speakers 2");
            assert!(follow.moved(false).unwrap().is_none());
            assert_eq!(
                d.asked.lock().unwrap()[2..],
                [
                    "far [\"ms-teams.exe\"]",
                    "moved? ms-teams.exe on out-0",
                    "moved? ms-teams.exe on out-0",
                    "far [\"ms-teams.exe\"]",
                    // Told to open again: nothing asked, opened where it should be.
                    "far [\"ms-teams.exe\"]",
                    "moved? ms-teams.exe on out-2",
                ]
            );
        }

        /// An app that could not be heard alone, and Record now, follow the default output; one
        /// that cannot be opened again says why, as the start does.
        #[test]
        fn a_far_end_of_all_output_follows_the_default_output() {
            for app_opened in [Some(app("Zoom.exe")), None] {
                let d = leak(devices(AppFar::Fails));
                let mut opened = WinMeetingCapture::new(d)
                    .open(app_opened.as_ref(), false)
                    .unwrap();
                let follow = opened.sides[1].follow.as_mut().expect("it moves");
                assert!(follow.moved(false).unwrap().is_none());
                assert_eq!(
                    d.asked.lock().unwrap().last().unwrap(),
                    "moved? all output on out-0"
                );
                // Its output was the only one, and it went.
                d.all_output_fails.store(true, Ordering::Relaxed);
                let err = follow.moved(true).err().unwrap();
                assert!(err.starts_with("the other side's sound: "), "{err}");
            }
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
