//! A meeting's capture from this machine's devices: the mic and the far end, opened for
//! `meeting.start`, never started here (the meeting run starts them once its chain has).
//!
//! The platform decides which mic (with Bluetooth output, the built-in one, unless the user's
//! headset-mic setting says otherwise) and taps the far end: the meeting's app when one is known,
//! everything this machine plays otherwise (the app itself is never tapped). Errors name the
//! device or the app, never audio.

use ink_core::{AppRef, Transport};
use ink_pipeline::meeting::watchdog::{FarDelivery, Routing};

use crate::meeting::CaptureSide;

/// The mic a meeting records, as the shell may show it ("why is it using the laptop mic?").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MicInfo {
    /// The device's name as the OS shows it.
    pub name: String,
    /// How it connects.
    pub transport: Transport,
    /// Why it was chosen: a schema word (`default_input`, `built_in_for_bluetooth_output`,
    /// `headset_mic_setting`, `no_built_in_mic`, `first_input`, `requested`).
    pub reason: &'static str,
}

/// Both sides of a meeting's capture, opened and not started.
pub struct Opened {
    /// The mic, then the far end.
    pub sides: Vec<CaptureSide>,
    /// How they are routed, for the silent-channel watchdog.
    pub routing: Routing,
    /// The mic, when the platform says which it is.
    pub mic: Option<MicInfo>,
}

/// Opens a meeting's capture on this machine.
pub trait MeetingCapture: Send + Sync {
    /// **Worker.** The mic and the far end for a meeting with `app` (`None`: everything this
    /// machine plays), opened and not started. `headset_mic`: the user's setting.
    fn open(&self, app: Option<&AppRef>, headset_mic: bool) -> Result<Opened, String>;
}

/// No devices: this platform's capture is not built yet (Windows until S3.1), or a test core.
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
            _ => "default_input",
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
            let far = match app {
                Some(app) => self
                    .capture
                    .open_far_end_source(&FarEndTarget::Apps(vec![app.clone()]))
                    .or_else(|e| {
                        // The app has no audio process the tap can name (it may play through a
                        // helper under another id): everything this Mac plays is the far end
                        // instead, which still leaves this app out.
                        log::warn!(
                            "meeting: the far end of {} could not be tapped ({e}); tapping everything this Mac plays",
                            app.id
                        );
                        self.capture
                            .open_far_end_source(&FarEndTarget::AllOutput)
                    }),
                None => self.capture.open_far_end_source(&FarEndTarget::AllOutput),
            }
            .map_err(|e| format!("the other side's sound: {e}"))?;
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
            })
        }
    }
}
