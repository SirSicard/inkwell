//! Operating-system services: capture control, meeting detection, hotkeys, text insertion,
//! focus, permissions, and the machine's free disk space and memory.
//!
//! `ink-platform-mac` (S2.1a, S2.1b) and `ink-platform-win` (S3.1) implement these. Every method
//! that needs the main thread on its OS hops there inside the implementation; callers stay on
//! worker threads.

use std::path::Path;
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

/// Why the platform's routing picks an input when the caller names none
/// ([`CaptureControl::automatic_input`]): what Settings calls "Automatic".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AutoReason {
    /// The output is not Bluetooth: the system default input.
    DefaultInput,
    /// The output is Bluetooth: a mic that is not the headset's (the built-in one; on Windows a
    /// USB mic may be the one kept), because a classic headset mic is 16 kHz call audio.
    BuiltInForBluetoothOutput,
    /// The output is Bluetooth and no other mic exists (a Mac without a built-in mic; on Windows,
    /// every mic is Bluetooth): the default input.
    NoBuiltInMic,
    /// No default input is set: the first input.
    FirstInput,
    /// An LE Audio headset, whose own mic keeps full quality (Windows).
    LeAudioHeadset,
    /// The platform's own headset-mic switch is on: the Bluetooth headset's mic. Nothing in the
    /// core turns that switch on since the user picks a mic (`audio.input`); it goes when the
    /// platforms drop the switch.
    HeadsetMicSetting,
}

/// The input [`CaptureControl::open_mic`] opens when it is given no device, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoInput {
    /// The device.
    pub device: DeviceInfo,
    /// Why the routing picks it.
    pub reason: AutoReason,
}

/// What changed among the devices ([`CaptureControl::watch_devices`]). The core reads the lists
/// again whatever the kind; the kind is for the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeviceChange {
    /// A device was added or removed, or one changed state (enabled, disabled, renamed).
    Devices,
    /// The default input changed.
    DefaultInput,
    /// The default output changed.
    DefaultOutput,
}

/// Opens capture streams and reports devices.
pub trait CaptureControl: Send + Sync {
    /// **Worker.** Input devices, default first.
    fn input_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError>;

    /// **Worker.** Output devices, default first, for the output picker (Windows: which output a
    /// meeting's far end records). [`PlatformError::Unsupported`] where the far end does not
    /// depend on an output (macOS: the process tap hears the app wherever it plays).
    fn output_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError>;

    /// **Worker.** The current default output device, if there is one. Mic routing and the echo
    /// decision read its transport.
    fn default_output(&self) -> Result<Option<DeviceInfo>, PlatformError>;

    /// **Worker.** The input `open_mic(None)` would open now, and why; `None` when there is no
    /// input at all. Opens nothing.
    fn automatic_input(&self) -> Result<Option<AutoInput>, PlatformError>;

    /// **Worker.** A microphone stream on `device`, or on the routing default when `None`. A
    /// stream whose device goes away (unplugged, disconnected) reports [`AudioSource::ended`]: a
    /// meeting then opens the mic again elsewhere.
    fn open_mic(&self, device: Option<&DeviceId>) -> Result<Box<dyn AudioSource>, PlatformError>;

    /// **Worker.** A far-end stream for `target`.
    fn open_far_end(&self, target: &FarEndTarget) -> Result<Box<dyn AudioSource>, PlatformError>;

    /// **Worker.** Starts telling `on_change` when devices come or go, or a default changes: an OS
    /// notification (Core Audio property listeners, `IMMNotificationClient`), never a poll.
    /// `on_change` runs on a callback thread (never a realtime one) and must not block: the core
    /// only enqueues, and coalesces what arrives together. Several notifications for one change
    /// are fine. Calling it again replaces the callback. [`PlatformError::Unsupported`] where the
    /// platform cannot watch yet: the core then reads the devices only when it opens a mic or is
    /// asked.
    fn watch_devices(&self, on_change: EventSink<DeviceChange>) -> Result<(), PlatformError>;

    /// **Worker.** Stops watching. When it returns, the callback will not run again. A no-op when
    /// not watching.
    fn unwatch_devices(&self);
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
    /// Detection stopped on its own: the OS stopped answering, or the watcher hit a bug. Which apps
    /// hold the microphone is unknown from here on, and nothing more arrives until
    /// [`MeetingDetector::start`] is called again. Sent once. The core treats meeting state as
    /// unknown, tells the user, and never restarts detection in a loop.
    Lost {
        /// What went wrong, for the log and the UI. Never audio or transcript content.
        reason: String,
    },
}

/// Watches for applications that start and stop using the microphone. The core debounces the
/// signals and applies its allowlist; the platform reports what the OS says, minus its own
/// daemons (for example CoreSpeech on macOS).
pub trait MeetingDetector: Send + Sync {
    /// **Worker.** Starts watching. `on_signal` runs on a callback thread and must not block.
    /// Calling `start` again replaces the callback. If detection stops without [`stop`](Self::stop)
    /// (the OS stops answering), the callback gets [`MeetingSignal::Lost`] once and nothing after.
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
    /// loop. If a hold was in progress, [`HotkeyEvent::Cancelled`] arrives first.
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

/// The machine's free disk space and memory: a model download checks the space before it fetches
/// a byte, and the core suggests a language model's size by the memory.
pub trait SystemInfo: Send + Sync {
    /// **Worker.** The bytes this user may still write on the volume holding `path`, which must
    /// exist (quotas included where the OS keeps them).
    fn free_disk_bytes(&self, path: &Path) -> Result<u64, PlatformError>;

    /// **Worker.** The machine's physical memory in bytes, as the OS reports it (a little under
    /// what is fitted: firmware and graphics keep some).
    fn total_memory_bytes(&self) -> Result<u64, PlatformError>;
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
