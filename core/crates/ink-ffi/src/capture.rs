//! A meeting's capture from this machine's devices: the mic and the far end, opened for
//! `meeting.start`, never started here (the meeting run starts them once its chain has).
//!
//! The mic is the user's choice in Settings > Sound, or Automatic (the platform's routing: with
//! Bluetooth output, not the headset's call-quality mic), resolved by [`devices`](crate::devices).
//! The far end is tapped: the meeting's app when one is known, everything this machine plays
//! otherwise (on the Mac, Inkwell itself is never tapped). Errors name the device or the app,
//! never audio.
//!
//! **The mic moves only when it goes** ([`FollowMic`]). A meeting keeps the mic it started with:
//! a mic plugged in mid-call, or a new default, changes nothing (the owner's call, 2026-10-05).
//! When its own mic goes (unplugged, a headset switched off: the source ends by itself), it opens
//! the mic again from the choice as it is then, which may be Automatic standing in for a chosen
//! mic that left, and the shell is told (`meeting.mic_switched`).
//!
//! On Windows ([`WinMeetingCapture`]) the far end of an app is S0.4's plan: process loopback,
//! which hears the app alone, for Zoom and the browsers; device loopback of the output the app
//! plays to for every other app (per-process loopback is silent on new Teams), which hears
//! everything that device plays, Inkwell's own sounds included, and is said as such (`far_end`
//! "everything"). Device loopback is bound to one output, so it moves with the call ([`Follow`]):
//! a headset plugged in mid-call, an output unplugged, a new default under Record now.

use std::sync::Arc;

use ink_core::{AppRef, AudioSource, Transport};
#[cfg(any(target_os = "macos", windows))]
use ink_pipeline::meeting::watchdog::FarDelivery;
use ink_pipeline::meeting::watchdog::Routing;

use crate::devices::{Choices, InputChoice, Wanted};
use crate::meeting::CaptureSide;

/// The mic a meeting records, as the shell may show it ("why is it using the laptop mic?").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MicInfo {
    /// The device's name as the OS shows it.
    pub name: String,
    /// How it connects.
    pub transport: Transport,
    /// Why it was chosen: a schema word (`chosen`, `chosen_missing`, or Automatic's reason:
    /// `default_input`, `built_in_for_bluetooth_output`, `no_built_in_mic`, `first_input`,
    /// `le_audio_headset` (Windows), or `unknown` for a reason this build does not name).
    pub reason: &'static str,
    /// The chosen mic this one stands in for, while that one is not connected (`chosen_missing`).
    pub wanted: Option<Wanted>,
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

    /// For a mic side: the mic it records now (after a move, the new one), for
    /// `meeting.mic_switched`. `None` for a far end.
    fn mic(&self) -> Option<&MicInfo> {
        None
    }
}

/// Opens a meeting's capture on this machine.
pub trait MeetingCapture: Send + Sync {
    /// **Worker.** The mic and the far end for a meeting with `app` (`None`: everything this
    /// machine plays), opened and not started. The mic is `choices`' input, read now; a mic that
    /// goes mid-meeting reads it again ([`FollowMic`]). `choices`' output is for the Windows far
    /// end to pin its loopback to (the Windows device branch; until then it follows the default).
    fn open(&self, app: Option<&AppRef>, choices: &Choices) -> Result<Opened, String>;

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
    fn open(&self, _: Option<&AppRef>, _: &Choices) -> Result<Opened, String> {
        Err("this platform cannot capture a meeting yet".into())
    }
}

/// Opens a meeting's mic for a choice, not started, and says which mic it is.
pub type OpenMic =
    Arc<dyn Fn(&InputChoice) -> Result<(Box<dyn AudioSource>, MicInfo), String> + Send + Sync>;

/// A meeting's mic, opened again only when it goes (the module docs): asked every
/// [`FOLLOW_INTERVAL`](crate::meeting::FOLLOW_INTERVAL), it stays; told its source ended, it opens
/// the mic again from the choice as it is now. No device is polled.
pub struct FollowMic {
    open: OpenMic,
    choices: Choices,
    mic: MicInfo,
}

impl FollowMic {
    /// Follows a mic opened as `mic`, opening again through `open` with `choices`' input.
    pub fn new(open: OpenMic, choices: Choices, mic: MicInfo) -> Self {
        Self { open, choices, mic }
    }
}

impl Follow for FollowMic {
    fn moved(&mut self, again: bool) -> Result<Option<Reopened>, String> {
        if !again {
            // A new mic, or a new default, never takes a meeting's mic away from it.
            return Ok(None);
        }
        let (source, mic) = (self.open)(&self.choices.input())?;
        let what = mic.name.clone();
        self.mic = mic;
        Ok(Some((source, what)))
    }

    fn mic(&self) -> Option<&MicInfo> {
        Some(&self.mic)
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

    use super::*;

    /// [`MeetingCapture`] on the Mac: the chosen (or routed) mic's own IOProc and a process tap.
    pub struct MacMeetingCapture {
        capture: Arc<MacCapture>,
    }

    impl MacMeetingCapture {
        /// Capture on the platform clock.
        pub fn new(capture: MacCapture) -> Self {
            Self {
                capture: Arc::new(capture),
            }
        }
    }

    /// **Worker** (or the meeting's pump, when its mic went). The mic for `choice`, opened and
    /// not started, and which it is.
    fn open_mic(
        capture: &MacCapture,
        choice: &InputChoice,
    ) -> Result<(Box<dyn AudioSource>, MicInfo), String> {
        let picked = crate::devices::pick_mic(capture, choice)?;
        let mic = capture
            .open_mic_source(Some(&picked.device.id))
            .map_err(|e| format!("the microphone {}: {e}", picked.device.name))?;
        let transport = mic.transport();
        Ok((Box::new(mic), picked.mic_info(transport)))
    }

    impl MeetingCapture for MacMeetingCapture {
        fn open(&self, app: Option<&AppRef>, choices: &Choices) -> Result<Opened, String> {
            let (mic, info) = open_mic(&self.capture, &choices.input())?;
            let transport = info.transport;
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
            // The tap follows its process whatever output it plays to: nothing moves it. The mic
            // opens again when it goes.
            let capture = Arc::clone(&self.capture);
            let follow = FollowMic::new(
                Arc::new(move |choice: &InputChoice| open_mic(&capture, choice)),
                choices.clone(),
                info.clone(),
            );
            let side =
                |source: Box<dyn AudioSource>, follow: Option<Box<dyn Follow>>| CaptureSide {
                    source,
                    ring: DEFAULT_RING_DURATION,
                    start_at: None,
                    follow,
                };
            Ok(Opened {
                sides: vec![side(mic, Some(Box::new(follow))), side(Box::new(far), None)],
                routing: Routing {
                    mic: transport,
                    far: FarDelivery::WhilePlaying,
                },
                mic: Some(info),
                far: scope,
            })
        }
    }
}

#[cfg(windows)]
pub use win::{FarHears, WinDevices, WinMeetingCapture};

#[cfg(windows)]
mod win {
    use ink_audio::DEFAULT_RING_DURATION;
    use ink_core::{AppRef, AudioSource, DeviceId, FarEndTarget, PlatformError};
    use ink_platform_win::WinCapture;

    use super::*;
    use crate::devices::Picked;

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
        /// The mic a meeting records for the user's `choice`, and why
        /// ([`crate::devices::pick_mic`]). Errors name the device.
        fn pick_mic(&self, choice: &InputChoice) -> Result<Picked, String>;
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
        fn pick_mic(&self, choice: &InputChoice) -> Result<Picked, String> {
            crate::devices::pick_mic(self, choice)
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

    /// **Worker** (or the meeting's pump, when its mic went). The mic for `choice`, opened and
    /// not started, and which it is.
    fn open_mic(
        devices: &dyn WinDevices,
        choice: &InputChoice,
    ) -> Result<(Box<dyn AudioSource>, MicInfo), String> {
        let picked = devices.pick_mic(choice)?;
        let mic = devices
            .open_mic(&picked.device.id)
            .map_err(|e| format!("the microphone {}: {e}", picked.device.name))?;
        Ok((mic, picked.mic_info(picked.device.transport)))
    }

    impl MeetingCapture for WinMeetingCapture {
        fn planned_far(&self, app: &AppRef) -> Option<FarScope> {
            // The plan's own test: any other app is heard by device loopback, whatever runs. One
            // on the list is known only once opened (not running: everything instead).
            (!ink_platform_win::capture::is_process_loopback_app(&app.id))
                .then_some(FarScope::Everything)
        }

        /// `choices`' output is not read yet: the far end follows the default output until the
        /// Windows device branch pins it.
        fn open(&self, app: Option<&AppRef>, choices: &Choices) -> Result<Opened, String> {
            let (mic, info) = open_mic(self.devices.as_ref(), &choices.input())?;
            let transport = info.transport;
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
            // The mic opens again when it goes (never for a new mic or a new default).
            let devices = Arc::clone(&self.devices);
            let follow_mic = FollowMic::new(
                Arc::new(move |choice: &InputChoice| open_mic(devices.as_ref(), choice)),
                choices.clone(),
                info.clone(),
            );
            Ok(Opened {
                sides: vec![
                    side(mic, Some(Box::new(follow_mic) as Box<dyn Follow>)),
                    side(far, follow),
                ],
                routing: Routing {
                    mic: transport,
                    // Loopback delivers nothing while nothing plays: idle, not stalled.
                    far: FarDelivery::WhilePlaying,
                },
                mic: Some(info),
                far: scope,
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        use ink_core::mock::MemStore;
        use ink_core::{AudioSink, AutoReason, Channel, DeviceInfo, SourceStats, StreamFormat};

        use super::*;
        use crate::devices::MicReason;

        /// The choices of a library with none set: Automatic.
        fn choices() -> Choices {
            Choices::new(Arc::new(MemStore::default()))
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
            mic: Option<Picked>,
            app_far: AppFar,
            all_output_fails: AtomicBool,
            /// What `far_moved` answers.
            moved: AtomicBool,
            /// Device-loopback far ends opened so far: each is on the output "out-N".
            outputs: AtomicUsize,
            /// What was asked: the mic picked (for which choice), the mic opened, each far-end
            /// target, each question whether the far end moved.
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

        fn picked(name: &str, transport: Transport, reason: AutoReason) -> Picked {
            Picked {
                device: DeviceInfo {
                    id: DeviceId(format!("{{0.0.1.00000000}}.{name}")),
                    name: name.into(),
                    transport,
                    is_default: true,
                },
                reason: MicReason::Auto(reason),
                wanted: None,
            }
        }

        fn devices(app_far: AppFar) -> Devices {
            Devices {
                mic: Some(picked(
                    "Microphone (USB Audio)",
                    Transport::Usb,
                    AutoReason::DefaultInput,
                )),
                app_far,
                all_output_fails: AtomicBool::new(false),
                moved: AtomicBool::new(false),
                outputs: AtomicUsize::new(0),
                asked: Mutex::new(Vec::new()),
            }
        }

        impl WinDevices for &'static Devices {
            fn pick_mic(&self, choice: &InputChoice) -> Result<Picked, String> {
                let what = match choice {
                    InputChoice::Auto => "auto".to_owned(),
                    InputChoice::Device(w) => w.id.0.clone(),
                };
                self.asked.lock().unwrap().push(format!("pick {what}"));
                self.mic
                    .clone()
                    .ok_or_else(|| "there is no microphone".into())
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
                .open(Some(&app("Zoom.exe")), &choices())
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
                    wanted: None,
                })
            );
            assert_eq!(
                *d.asked.lock().unwrap(),
                [
                    "pick auto",
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
                .open(Some(&app("ms-teams.exe")), &choices())
                .unwrap();
            assert_eq!(opened.far, FarScope::Everything);
            assert_eq!(opened.far.name(), "everything");
            assert_eq!(d.asked.lock().unwrap()[0], "pick auto");
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
                .open(Some(&app("Zoom.exe")), &choices())
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
            let opened = WinMeetingCapture::new(d).open(None, &choices()).unwrap();
            assert_eq!(opened.far, FarScope::Everything);
            assert_eq!(d.asked.lock().unwrap()[2..], ["far all output"]);
        }

        #[test]
        fn a_meeting_without_a_mic_or_an_output_does_not_open() {
            let mut none = devices(AppFar::Alone);
            none.mic = None;
            assert_eq!(
                WinMeetingCapture::new(leak(none))
                    .open(None, &choices())
                    .err(),
                Some("there is no microphone".into())
            );
            let silent = devices(AppFar::Fails);
            silent.all_output_fails.store(true, Ordering::Relaxed);
            let err = WinMeetingCapture::new(leak(silent))
                .open(Some(&app("Zoom.exe")), &choices())
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
                .open(Some(&app("Zoom.exe")), &choices())
                .unwrap();
            assert!(zoom.sides[1].follow.is_none(), "process loopback stays");

            let d = leak(devices(AppFar::Device));
            let mut opened = WinMeetingCapture::new(d)
                .open(Some(&app("ms-teams.exe")), &choices())
                .unwrap();
            assert!(
                opened.sides[0].follow.is_some(),
                "the mic opens again when it goes"
            );
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
                    .open(app_opened.as_ref(), &choices())
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
            d.mic = Some(picked(
                "Headset (Earbuds)",
                Transport::Bluetooth,
                AutoReason::LeAudioHeadset,
            ));
            let opened = WinMeetingCapture::new(leak(d))
                .open(None, &choices())
                .unwrap();
            assert_eq!(opened.routing.mic, Transport::Bluetooth);
            assert_eq!(opened.mic.unwrap().reason, "le_audio_headset");
        }

        /// The mic stays through every interval's question (no device is asked), and opens again
        /// from the choice only when told its source ended.
        #[test]
        fn the_mic_opens_again_only_when_it_goes() {
            let d = leak(devices(AppFar::Alone));
            let mut opened = WinMeetingCapture::new(d)
                .open(Some(&app("Zoom.exe")), &choices())
                .unwrap();
            let asked = d.asked.lock().unwrap().len();
            let follow = opened.sides[0].follow.as_mut().unwrap();
            assert!(follow.moved(false).unwrap().is_none());
            assert_eq!(d.asked.lock().unwrap().len(), asked, "nothing asked");
            let (source, what) = follow.moved(true).unwrap().expect("opened again");
            assert_eq!(source.channel(), Channel::Mic);
            assert_eq!(what, "Microphone (USB Audio)");
            assert_eq!(follow.mic().unwrap().reason, "default_input");
            assert_eq!(
                d.asked.lock().unwrap()[asked..],
                ["pick auto", "mic {0.0.1.00000000}.Microphone (USB Audio)"]
            );
        }
    }
}
