//! Operating-system services: capture control, meeting detection, hotkeys, text insertion,
//! focus and permissions.
//!
//! `ink-platform-mac` (S2.1a, S2.1b) and `ink-platform-win` (S3.1) implement these. Every method
//! that needs the main thread on its OS hops there inside the implementation; callers stay on
//! worker threads.

use std::sync::Arc;

use crate::audio::AudioSource;
use crate::clock::Clock;
use crate::error::PlatformError;
use crate::threading::EventSink;

/// An audio device, as the OS identifies it (a Core Audio UID, a WASAPI endpoint id).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DeviceId(pub String);

/// How a device connects. Echo cancellation and mic routing depend on it: with Bluetooth output
/// the built-in mic is recorded (S0.3), and headphones need no echo cancellation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Transport {
    /// Built into the machine.
    BuiltIn,
    /// Bluetooth: 16 kHz call audio on the headset mic, which also outputs digital zeros while
    /// the user is silent.
    Bluetooth,
    /// USB.
    Usb,
    /// An aggregate or virtual device.
    Virtual,
    /// Anything the OS reports that the list above does not cover.
    Other,
}

/// An input or output device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The OS identifier.
    pub id: DeviceId,
    /// The name the OS shows the user.
    pub name: String,
    /// How it connects.
    pub transport: Transport,
    /// Whether it is the current default for its direction.
    pub is_default: bool,
}

/// An application, as detection, focus and far-end capture see it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AppRef {
    /// The bundle identifier on macOS, the executable name on Windows.
    pub id: String,
    /// The process, when one is known.
    pub pid: Option<u32>,
    /// The name to show the user.
    pub name: String,
}

/// What the far-end source captures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FarEndTarget {
    /// Everything the default output plays. The Windows default, because per-process loopback
    /// is silent on new Teams.
    AllOutput,
    /// Only these applications (and, on Windows, their process trees).
    Apps(Vec<AppRef>),
}

/// Opens capture streams and reports devices.
pub trait CaptureControl: Send + Sync {
    /// **Worker.** Input devices, default first.
    fn input_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError>;

    /// **Worker.** The current default output device, if there is one. Mic routing and the echo
    /// decision read its transport.
    fn default_output(&self) -> Result<Option<DeviceInfo>, PlatformError>;

    /// **Worker.** A microphone stream on `device`, or on the routing default when `None`.
    fn open_mic(&self, device: Option<&DeviceId>) -> Result<Box<dyn AudioSource>, PlatformError>;

    /// **Worker.** A far-end stream for `target`.
    fn open_far_end(&self, target: &FarEndTarget) -> Result<Box<dyn AudioSource>, PlatformError>;
}

/// A change in which applications hold the microphone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeetingSignal {
    /// `app` started capturing from an input device.
    MicInUse {
        /// The application.
        app: AppRef,
    },
    /// `app` stopped capturing.
    MicReleased {
        /// The application.
        app: AppRef,
    },
}

/// Watches for applications that start and stop using the microphone. The core debounces the
/// signals and applies its allowlist; the platform reports what the OS says, minus its own
/// daemons (for example CoreSpeech on macOS).
pub trait MeetingDetector: Send + Sync {
    /// **Worker.** Starts watching. `on_signal` runs on a callback thread and must not block.
    /// Calling `start` again replaces the callback.
    fn start(&self, on_signal: EventSink<MeetingSignal>) -> Result<(), PlatformError>;

    /// **Worker.** Stops watching. When it returns, the callback will not run again.
    fn stop(&self);
}

/// A hotkey, as a platform token (for example `"fn"`, `"right_option"`, `"ctrl+shift+space"`).
/// The platform rejects tokens it cannot bind with [`PlatformError::Unsupported`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HotkeyBinding(pub String);

/// What the hotkey did. The hold-versus-toggle state machine lives in the core, not here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// Went down, at host time `at_ns` on the [`Clock`] timebase.
    Pressed {
        /// Host time in nanoseconds.
        at_ns: u64,
    },
    /// Came up, at host time `at_ns`.
    Released {
        /// Host time in nanoseconds.
        at_ns: u64,
    },
    /// The OS stopped delivering key events (a disabled event tap, lost focus of a low-level
    /// hook). The core ends any hold in progress rather than leaving it stuck down.
    Cancelled,
    /// The OS removed the hotkey (for example, Accessibility was revoked mid-session). It also
    /// ends any hold in progress, and nothing more arrives until [`HotkeySource::start`] is
    /// called again. The core re-checks permissions and tells the user; it never retries in a
    /// loop.
    Lost,
}

/// A global hotkey.
pub trait HotkeySource: Send + Sync {
    /// **Worker.** Starts listening for `binding`. `on_event` runs on the platform's event thread,
    /// which the OS disables if it is slow, so it must only enqueue. Calling `start` again
    /// replaces the binding and the callback.
    fn start(
        &self,
        binding: &HotkeyBinding,
        on_event: EventSink<HotkeyEvent>,
    ) -> Result<(), PlatformError>;

    /// **Worker.** Stops listening. When it returns, the callback will not run again.
    fn stop(&self);
}

/// How an insertion went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsertOutcome {
    /// Pasted through the clipboard, and the previous clipboard restored.
    Pasted,
    /// Typed as Unicode key events or written through accessibility (the fallbacks).
    Typed,
    /// Secure Input (macOS) or an elevated target (Windows) blocks synthetic input. Nothing was
    /// inserted, and the UI says so instead of failing silently.
    Blocked,
    /// The text is in, by paste or by a fallback, but the previous clipboard could not be put
    /// back fully. It is a success, never retried (a retry would insert the text twice); the UI
    /// says once that the clipboard changed.
    InsertedClipboardNotRestored,
}

/// Puts text into the focused application.
pub trait TextInserter: Send + Sync {
    /// **Worker.** Inserts `text` at the focused caret. It may block while the target reads the
    /// clipboard and the previous contents are restored. It never prompts for a permission.
    fn insert(&self, text: &str) -> Result<InsertOutcome, PlatformError>;
}

/// What has keyboard focus.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FocusInfo {
    /// The frontmost application, when the OS reports one.
    pub app: Option<AppRef>,
    /// Whether secure text entry is on, so insertion would be blocked.
    pub secure_input: bool,
}

/// Reads focus and selection.
pub trait FocusReader: Send + Sync {
    /// **Worker.** The frontmost application and the secure-input state.
    fn focus(&self) -> Result<FocusInfo, PlatformError>;

    /// **Worker.** The selected text in the focused application, for voice editing. `None` when
    /// nothing is selected.
    fn selected_text(&self) -> Result<Option<String>, PlatformError>;
}

/// A permission the app depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Permission {
    /// Microphone capture.
    Microphone,
    /// System-audio capture (a process tap on macOS).
    SystemAudio,
    /// Accessibility: synthetic input, reading focus and the selection, and an active (blocking)
    /// event tap, which is what the macOS hotkey is.
    Accessibility,
    /// Input Monitoring: only for a listen-only event tap. None is planned on macOS (the hotkey tap
    /// is active, under Accessibility), and the Windows low-level keyboard hook needs neither.
    InputMonitoring,
}

/// The state of one permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionState {
    /// Granted, and verified where the OS allows (a denied system-audio tap delivers silence, so
    /// macOS checks it with a self-tap tone probe).
    Granted,
    /// Refused.
    Denied,
    /// Never asked.
    NotDetermined,
    /// The OS gives no reliable answer.
    Unknown,
}

/// Checks and requests permissions.
pub trait PermissionProbe: Send + Sync {
    /// **Worker.** The current state. Never prompts. It may take up to about a second where a probe
    /// is needed.
    fn check(&self, permission: Permission) -> PermissionState;

    /// **Worker.** Shows the system prompt or the settings pane. Call it only in response to the
    /// user asking.
    fn request(&self, permission: Permission) -> Result<(), PlatformError>;
}

/// Every platform service, bundled for wiring the pipeline and the C ABI.
#[derive(Clone)]
pub struct Platform {
    /// Devices and capture streams.
    pub capture: Arc<dyn CaptureControl>,
    /// Meeting detection.
    pub meetings: Arc<dyn MeetingDetector>,
    /// The global hotkey.
    pub hotkeys: Arc<dyn HotkeySource>,
    /// Text insertion.
    pub inserter: Arc<dyn TextInserter>,
    /// Focus and selection.
    pub focus: Arc<dyn FocusReader>,
    /// Permissions.
    pub permissions: Arc<dyn PermissionProbe>,
    /// The host clock that capture timestamps use.
    pub clock: Arc<dyn Clock>,
}
