//! Permissions: checking without prompting, and asking only when the user asks.
//!
//! | Permission | `check` | `request` |
//! |---|---|---|
//! | Microphone | `AVCaptureDevice` authorization status | the system prompt if never asked, else the settings pane |
//! | System Audio | the self-tap tone probe ([`tone`]), once the app has asked | the prompt (by running the probe) the first time, else the settings pane |
//! | Accessibility | `AXIsProcessTrusted` | the settings pane |
//! | Input Monitoring | `CGPreflightListenEventAccess` | the settings pane |
//!
//! **System Audio has no public non-prompting query**, and a denied tap delivers silence rather
//! than failing, so the probe listens for its own tone. Running the probe before the app has ever asked
//! would make macOS show its prompt, which `check` must never do. So until the app has asked (the
//! shell persists that and passes it to [`MacPermissionProbe::with_system_audio_asked`], and
//! [`request`](PermissionProbe::request) sets it), `check(SystemAudio)` answers `NotDetermined`:
//! from the app's side, never asked.
//!
//! Accessibility and Input Monitoring cannot tell "never asked" from "refused": both read `Denied`.
//! Accessibility is requested through the settings pane rather than the prompting trust call, which
//! this crate never makes (the input half's rule).
#![cfg(target_os = "macos")]

pub mod tone;

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use block2::RcBlock;
use ink_core::{Permission, PermissionProbe, PermissionState, PlatformError};
use objc2::msg_send;
use objc2::runtime::{AnyClass, Bool};
use objc2_app_kit::NSWorkspace;
use objc2_core_graphics::CGPreflightListenEventAccess;
use objc2_foundation::{NSString, NSURL};

use crate::ax;
use crate::clock::MacClock;
use tone::ProbeReport;

// SAFETY: `AVMediaTypeAudio` is declared in `AVFoundation/AVMediaFormat.h` as
// `AVMediaType const AVMediaTypeAudio`, an immutable `NSString *` that lives for the process.
#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVMediaTypeAudio: &'static NSString;
}

/// `AVAuthorizationStatus` values (`AVCaptureDevice.h`).
mod av_status {
    pub const NOT_DETERMINED: isize = 0;
    pub const RESTRICTED: isize = 1;
    pub const DENIED: isize = 2;
    pub const AUTHORIZED: isize = 3;
}

fn capture_device_class() -> Option<&'static AnyClass> {
    AnyClass::get(c"AVCaptureDevice")
}

/// The microphone permission as `AVCaptureDevice` reports it. Never prompts. Restricted (by a
/// profile) reads as `Denied`.
pub(crate) fn microphone() -> PermissionState {
    let Some(class) = capture_device_class() else {
        return PermissionState::Unknown;
    };
    // SAFETY: `+[AVCaptureDevice authorizationStatusForMediaType:]` takes an `AVMediaType`
    // (`NSString *`) and returns `AVAuthorizationStatus` (`NSInteger`); the constant is a live,
    // immutable framework string.
    let status: isize =
        unsafe { msg_send![class, authorizationStatusForMediaType: AVMediaTypeAudio] };
    match status {
        av_status::AUTHORIZED => PermissionState::Granted,
        av_status::DENIED | av_status::RESTRICTED => PermissionState::Denied,
        av_status::NOT_DETERMINED => PermissionState::NotDetermined,
        _ => PermissionState::Unknown,
    }
}

/// Shows the microphone prompt. Returns at once; the answer arrives later and `check` reads it.
fn prompt_for_microphone() -> Result<(), PlatformError> {
    let class = capture_device_class().ok_or(PlatformError::Unsupported(
        "AVCaptureDevice is not available",
    ))?;
    // The answer is read back through `check`; the handler only has to exist.
    let handler = RcBlock::new(|_granted: Bool| {});
    // SAFETY: `+[AVCaptureDevice requestAccessForMediaType:completionHandler:]` takes an
    // `AVMediaType` and a `void (^)(BOOL)` block, which it copies; returns nothing.
    let () = unsafe {
        msg_send![
            class,
            requestAccessForMediaType: AVMediaTypeAudio,
            completionHandler: &*handler
        ]
    };
    Ok(())
}

/// A pane of System Settings > Privacy & Security.
fn settings_url(anchor: &str) -> String {
    format!("x-apple.systempreferences:com.apple.preference.security?{anchor}")
}

fn open_settings(anchor: &str) -> Result<(), PlatformError> {
    let url = NSURL::URLWithString(&NSString::from_str(&settings_url(anchor)))
        .ok_or_else(|| PlatformError::Failed(format!("bad settings URL for {anchor}")))?;
    if NSWorkspace::sharedWorkspace().openURL(&url) {
        Ok(())
    } else {
        Err(PlatformError::Failed(format!(
            "could not open System Settings at {anchor}"
        )))
    }
}

/// The settings pane that lists each permission. System Audio lives in "Screen & System Audio
/// Recording", whose anchor is the screen-capture one.
fn settings_anchor(permission: Permission) -> &'static str {
    match permission {
        Permission::Microphone => "Privacy_Microphone",
        Permission::SystemAudio => "Privacy_ScreenCapture",
        Permission::Accessibility => "Privacy_Accessibility",
        Permission::InputMonitoring => "Privacy_ListenEvent",
        // Newer permissions have no pane here yet: the security pane's top.
        _ => "",
    }
}

/// The last definite System Audio verdict a probe reached in this process: what the far end's
/// start gate reads. A probe that could not answer leaves it as it was.
#[derive(Debug, Default)]
pub(crate) struct SystemAudioVerdict(AtomicU8);

impl SystemAudioVerdict {
    const NONE: u8 = 0;
    const GRANTED: u8 = 1;
    const DENIED: u8 = 2;

    /// Records `state` if it is definite (Granted or Denied).
    fn record(&self, state: PermissionState) {
        let value = match state {
            PermissionState::Granted => Self::GRANTED,
            PermissionState::Denied => Self::DENIED,
            _ => return,
        };
        self.0.store(value, Ordering::SeqCst);
    }

    /// The last definite verdict, if any.
    pub(crate) fn get(&self) -> Option<PermissionState> {
        match self.0.load(Ordering::SeqCst) {
            Self::GRANTED => Some(PermissionState::Granted),
            Self::DENIED => Some(PermissionState::Denied),
            _ => None,
        }
    }
}

/// The process-wide verdict: [`MacPermissionProbe`] writes it, the far end's start gate reads it.
static SYSTEM_AUDIO_VERDICT: SystemAudioVerdict =
    SystemAudioVerdict(AtomicU8::new(SystemAudioVerdict::NONE));

/// The last definite System Audio verdict of any probe in this process.
pub(crate) fn system_audio_verdict() -> Option<PermissionState> {
    SYSTEM_AUDIO_VERDICT.get()
}

type ProbeFn = Arc<dyn Fn() -> Result<ProbeReport, PlatformError> + Send + Sync>;
type OpenSettingsFn = Arc<dyn Fn(&str) -> Result<(), PlatformError> + Send + Sync>;

/// [`PermissionProbe`] for macOS.
pub struct MacPermissionProbe {
    system_audio_asked: AtomicBool,
    /// One probe at a time; also keeps the last result for diagnostics.
    last_probe: Mutex<Option<Result<ProbeReport, PlatformError>>>,
    probe: ProbeFn,
    open_settings: OpenSettingsFn,
    verdict: &'static SystemAudioVerdict,
}

impl MacPermissionProbe {
    /// A probe that treats System Audio as never asked (see the module docs).
    pub fn new(clock: MacClock) -> Self {
        Self::with_seams(
            Arc::new(move || tone::run(clock)),
            Arc::new(open_settings),
            &SYSTEM_AUDIO_VERDICT,
        )
    }

    /// The seams tests replace: the tone probe, opening System Settings, and where the verdict is
    /// recorded.
    fn with_seams(
        probe: ProbeFn,
        open_settings: OpenSettingsFn,
        verdict: &'static SystemAudioVerdict,
    ) -> Self {
        Self {
            system_audio_asked: AtomicBool::new(false),
            last_probe: Mutex::new(None),
            probe,
            open_settings,
            verdict,
        }
    }

    /// Whether the app has asked for System Audio before (the shell persists it). While false,
    /// `check(SystemAudio)` answers `NotDetermined` instead of running the probe, because the probe
    /// would make macOS prompt.
    pub fn with_system_audio_asked(self, asked: bool) -> Self {
        self.system_audio_asked.store(asked, Ordering::Relaxed);
        self
    }

    /// Whether System Audio has been asked for, by the shell's flag or a `request` in this process.
    /// The shell persists this after a `request`.
    pub fn system_audio_asked(&self) -> bool {
        self.system_audio_asked.load(Ordering::Relaxed)
    }

    /// **Worker.** Runs the tone probe now, whatever has been asked, and returns what it measured.
    /// It prompts if macOS has never asked. A Granted or Denied verdict is recorded for the far
    /// end's start gate (a far end refuses to start after Denied); no answer changes nothing.
    pub fn probe_system_audio(&self) -> Result<ProbeReport, PlatformError> {
        self.run_probe(true)
    }

    fn run_probe(&self, record: bool) -> Result<ProbeReport, PlatformError> {
        let mut last = self
            .last_probe
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let result = (self.probe)();
        if let (true, Ok(report)) = (record, &result) {
            self.verdict.record(report.verdict.permission());
        }
        *last = Some(result.clone());
        result
    }

    /// The last probe's result, if one ran.
    pub fn last_probe(&self) -> Option<Result<ProbeReport, PlatformError>> {
        self.last_probe
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The state a probe result supports: an error is no answer.
fn probed_state(result: &Result<ProbeReport, PlatformError>) -> PermissionState {
    match result {
        Ok(report) => report.verdict.permission(),
        Err(_) => PermissionState::Unknown,
    }
}

impl PermissionProbe for MacPermissionProbe {
    /// Never prompts. `SystemAudio` takes about a second once asked for (the probe plays a muted
    /// tone through the default output), so call it before a meeting and when the user comes back
    /// from System Settings, not on a timer.
    fn check(&self, permission: Permission) -> PermissionState {
        match permission {
            Permission::Microphone => microphone(),
            Permission::SystemAudio if !self.system_audio_asked() => PermissionState::NotDetermined,
            Permission::SystemAudio => probed_state(&self.probe_system_audio()),
            Permission::Accessibility => granted_or_denied(ax::is_process_trusted()),
            Permission::InputMonitoring => granted_or_denied(CGPreflightListenEventAccess()),
            _ => PermissionState::Unknown,
        }
    }

    /// Call only when the user asks. macOS prompts once per permission; after that the answer is
    /// changed in System Settings, which is what this opens.
    fn request(&self, permission: Permission) -> Result<(), PlatformError> {
        match permission {
            Permission::Microphone => match microphone() {
                PermissionState::NotDetermined => prompt_for_microphone(),
                PermissionState::Granted => Ok(()),
                _ => (self.open_settings)(settings_anchor(permission)),
            },
            Permission::SystemAudio => {
                if !self.system_audio_asked.swap(true, Ordering::Relaxed) {
                    // Starting a tap is what makes macOS ask. The probe's verdict here is not the
                    // answer (the prompt may still be up), so it is not recorded; the shell checks
                    // again afterwards. A probe that could not run is an error, returned.
                    return self.run_probe(false).map(|_| ());
                }
                match self.probe_system_audio()?.verdict.permission() {
                    PermissionState::Granted => Ok(()),
                    _ => (self.open_settings)(settings_anchor(permission)),
                }
            }
            _ => (self.open_settings)(settings_anchor(permission)),
        }
    }
}

fn granted_or_denied(granted: bool) -> PermissionState {
    if granted {
        PermissionState::Granted
    } else {
        PermissionState::Denied
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicU8;

    use super::*;
    use tone::ToneVerdict;

    /// What the fake probe hears next.
    const HEARD: u8 = 0;
    const SILENCE: u8 = 1;
    const NO_AUDIO: u8 = 2;
    const FAILS: u8 = 3;

    fn report(verdict: ToneVerdict) -> ProbeReport {
        ProbeReport {
            verdict,
            tap_callbacks: 10,
            output_callbacks: 10,
            samples: 100,
            tone_fraction: 0.0,
            peak: 0.0,
        }
    }

    /// A probe with a scripted tone probe, a recorded settings pane, and its own verdict cell (so
    /// tests never touch the process-wide one).
    fn fake(asked: bool) -> (MacPermissionProbe, Arc<AtomicU8>, Arc<Mutex<Vec<String>>>) {
        let hears = Arc::new(AtomicU8::new(HEARD));
        let opened = Arc::new(Mutex::new(Vec::new()));
        let (h, o) = (hears.clone(), opened.clone());
        let probe = MacPermissionProbe::with_seams(
            Arc::new(move || match h.load(Ordering::SeqCst) {
                HEARD => Ok(report(ToneVerdict::Heard)),
                SILENCE => Ok(report(ToneVerdict::Silence)),
                NO_AUDIO => Ok(report(ToneVerdict::NoAudio)),
                _ => Err(PlatformError::Device("no default output device".into())),
            }),
            Arc::new(move |anchor: &str| {
                o.lock().unwrap().push(anchor.to_owned());
                Ok(())
            }),
            Box::leak(Box::new(SystemAudioVerdict::default())),
        )
        .with_system_audio_asked(asked);
        (probe, hears, opened)
    }

    /// The user revokes System Audio after granting it: with the asked flag set, `check` runs the
    /// probe again and reads Denied. A stale "asked" never means Granted.
    #[test]
    fn revoking_after_a_grant_reads_denied_not_a_stale_granted() {
        let (probe, hears, _) = fake(true);
        assert_eq!(
            probe.check(Permission::SystemAudio),
            PermissionState::Granted
        );
        assert_eq!(probe.verdict.get(), Some(PermissionState::Granted));
        hears.store(SILENCE, Ordering::SeqCst); // revoked in System Settings
        assert_eq!(
            probe.check(Permission::SystemAudio),
            PermissionState::Denied
        );
        assert_eq!(probe.verdict.get(), Some(PermissionState::Denied));
    }

    /// A probe that cannot answer is Unknown, never the last Granted.
    #[test]
    fn a_probe_that_cannot_answer_is_never_granted() {
        let (probe, hears, _) = fake(true);
        assert_eq!(
            probe.check(Permission::SystemAudio),
            PermissionState::Granted
        );
        for no_answer in [NO_AUDIO, FAILS] {
            hears.store(no_answer, Ordering::SeqCst);
            assert_eq!(
                probe.check(Permission::SystemAudio),
                PermissionState::Unknown
            );
        }
        // The last definite verdict stands for the far end's gate; it is not upgraded.
        hears.store(SILENCE, Ordering::SeqCst);
        probe.check(Permission::SystemAudio);
        hears.store(FAILS, Ordering::SeqCst);
        probe.check(Permission::SystemAudio);
        assert_eq!(probe.verdict.get(), Some(PermissionState::Denied));
    }

    #[test]
    fn request_returns_the_probe_error() {
        let (probe, hears, opened) = fake(false);
        hears.store(FAILS, Ordering::SeqCst);
        assert!(matches!(
            probe.request(Permission::SystemAudio),
            Err(PlatformError::Device(_))
        ));
        assert!(
            probe.system_audio_asked(),
            "asked, even though the probe failed"
        );
        assert!(
            probe.request(Permission::SystemAudio).is_err(),
            "and again once asked"
        );
        assert!(opened.lock().unwrap().is_empty());
    }

    #[test]
    fn request_opens_settings_only_when_the_probe_says_not_granted() {
        let (probe, hears, opened) = fake(true);
        assert!(probe.request(Permission::SystemAudio).is_ok());
        assert!(
            opened.lock().unwrap().is_empty(),
            "granted: nothing to open"
        );
        hears.store(SILENCE, Ordering::SeqCst);
        assert!(probe.request(Permission::SystemAudio).is_ok());
        assert_eq!(*opened.lock().unwrap(), ["Privacy_ScreenCapture"]);
    }

    #[test]
    fn system_audio_is_never_asked_until_the_app_has_asked() {
        let probe = MacPermissionProbe::new(MacClock::new().unwrap());
        // No probe runs, so no prompt: the answer comes straight from the flag.
        assert_eq!(
            probe.check(Permission::SystemAudio),
            PermissionState::NotDetermined
        );
        assert!(probe.last_probe().is_none(), "the probe did not run");
        assert!(!probe.system_audio_asked());
        let asked = MacPermissionProbe::new(MacClock::new().unwrap()).with_system_audio_asked(true);
        assert!(asked.system_audio_asked());
    }

    #[test]
    fn a_probe_error_is_no_answer() {
        assert_eq!(
            probed_state(&Err(PlatformError::Device("gone".into()))),
            PermissionState::Unknown
        );
    }

    #[test]
    fn every_permission_has_a_settings_pane() {
        for permission in [
            Permission::Microphone,
            Permission::SystemAudio,
            Permission::Accessibility,
            Permission::InputMonitoring,
        ] {
            let anchor = settings_anchor(permission);
            assert!(anchor.starts_with("Privacy_"), "{permission:?}");
            assert!(settings_url(anchor).starts_with("x-apple.systempreferences:"));
        }
    }

    /// Runs on CI: none of these prompts or needs a grant, and each gives a definite answer or an
    /// honest Unknown.
    #[test]
    fn the_non_prompting_checks_answer() {
        let probe = MacPermissionProbe::new(MacClock::new().unwrap());
        let mic = probe.check(Permission::Microphone);
        assert_ne!(mic, PermissionState::Unknown, "AVFoundation answered");
        for permission in [Permission::Accessibility, Permission::InputMonitoring] {
            let state = probe.check(permission);
            assert!(
                matches!(state, PermissionState::Granted | PermissionState::Denied),
                "{permission:?}: {state:?}"
            );
        }
    }
}
