//! Capture: the microphone and the far end, as two independent WASAPI streams.
//!
//! ```text
//!  capture endpoint ──packets──► WasapiSource (mic)  ──push──► ring (ink-audio)
//!  render endpoint  ──loopback─► WasapiSource (far)  ──push──► ring          device loopback
//!  VAD\Process_Loopback (pid) ─► WasapiSource (far)  ──push──► ring          process loopback
//! ```
//!
//! Both stamp their blocks from the performance counter ([`WinClock`]), so they align.
//!
//! **The far end ([`plan_far_end`]), S0.4's defaults.** Device loopback is the default: it hears
//! everything the device plays, and per-process loopback is known to be silent on new Teams.
//! For an app it is the device **that app plays to** (its render session's endpoint), not
//! necessarily the default. Zoom and the browsers get process-tree loopback instead, which hears
//! only the call. The call-app matrix (Teams, Zoom, Meet in Chrome and Edge) is not measured yet;
//! it is in `windows/S3.1-CHECKLIST.md`.
//!
//! **A device-loopback far end is bound to one output**, so a meeting asks, while it records,
//! whether it should move ([`far_end_moved`]): its output went, the default changed under "all
//! output", or the app now plays on another output. Process loopback follows its app anyway.
//!
//! **The output the user chose** (Settings > Sound, `audio.output`) is *pinned*: "all output"
//! (Record now, and an app that could not be heard alone) records it instead of the default, stays
//! on it when the default changes, records the default while it is unplugged, and goes back to it
//! when it returns. An app heard by its own output (Teams) or alone (Zoom) keeps its own.
//!
//! **Device changes** ([`WinCapture::watch_devices`](CaptureControl::watch_devices)): an
//! `IMMNotificationClient` on a thread of its own ([`watch`]), whose callbacks only hand the change
//! to the core, which enqueues it.
//!
//! **Mic routing** ([`route_mic`]): with Bluetooth output, a mic that is not Bluetooth, unless the
//! headset is LE Audio or the headset-mic setting is on.
//!
//! **Silence is never taken for quiet.** Loopback delivers nothing while nothing plays (the
//! watchdog's `FarDelivery::WhilePlaying`, as on the Mac); a silent packet is delivered as zeros.
//! The core's watchdog (`ink-pipeline`) judges both, always on.
//!
//! **Threads.** Every method here is **worker**. Each stream's thread is the realtime part
//! (`stream`, I4).
#![cfg(windows)]

pub(crate) mod devices;
mod loopback;
mod routing;
pub(crate) mod stream;
mod watch;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use ink_audio::{RealtimeGuard, unguarded};
use ink_core::{
    AppRef, AudioSink, AudioSource, AutoInput, AutoReason, CaptureControl, Channel, DeviceChange,
    DeviceId, DeviceInfo, EventSink, FarEndTarget, Permission, PermissionState, PlatformError,
    SourceStats, StreamFormat,
};

pub use routing::{Endpoint, LE_AUDIO_MIN_RATE, MicRoute, MicRouteReason, route_mic, transport};
pub use stream::IoStats;

use crate::clock::WinClock;
use crate::com::ComScope;
use crate::permissions;
use crate::process::ProcessTable;
use crate::sessions::{self, Session};
use devices::Flow;
use stream::{Counters, Running, StreamKind};

/// Apps whose far end is captured with process-tree loopback rather than device loopback: Zoom and
/// the browsers (S0.4). Matched case-insensitively against the executable name, in ASCII only
/// ([`is_process_loopback_app`]), as every lookup by executable here is: the core keeps Windows
/// identities lowercased the same way (`ink_ffi::calls::identity`, `to_ascii_lowercase`), and
/// decides from this list before opening whether an Always app is heard alone. A change to how
/// executables are matched here changes both.
pub const PROCESS_LOOPBACK_APPS: [&str; 5] = [
    "Zoom.exe",
    "chrome.exe",
    "msedge.exe",
    "brave.exe",
    "firefox.exe",
];

/// How the far end will be captured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FarPlan {
    /// Everything this render endpoint plays.
    Device {
        /// The endpoint id.
        endpoint: String,
        /// Why this endpoint.
        reason: FarReason,
    },
    /// One app's process tree.
    Process {
        /// The root of the tree.
        pid: u32,
        /// Its executable.
        exe: String,
    },
}

/// Why a device-loopback far end records the endpoint it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FarReason {
    /// All output was asked for: the default output.
    AllOutput,
    /// All output was asked for: the output the user chose (pinned).
    Pinned,
    /// The app plays to this endpoint (it has a session there).
    AppPlaysHere,
    /// The app has no session on any output yet: the default output.
    AppNotPlayingYet,
}

/// Whether `exe` is one of [`PROCESS_LOOPBACK_APPS`]: heard alone, by process loopback, while it
/// runs. Every other app is heard by device loopback (everything its output plays). Pure.
pub fn is_process_loopback_app(exe: &str) -> bool {
    PROCESS_LOOPBACK_APPS
        .iter()
        .any(|app| app.eq_ignore_ascii_case(exe))
}

/// Chooses how to capture `target`, from the default output, the pinned output (connected now, or
/// `None`), the render sessions and the processes running now. Pure.
pub(crate) fn plan_far_end(
    target: &FarEndTarget,
    default_output: Option<&str>,
    pinned: Option<&str>,
    render_sessions: &[Session],
    processes: &ProcessTable,
) -> Result<FarPlan, PlatformError> {
    let default = |reason| {
        default_output
            .map(|endpoint| FarPlan::Device {
                endpoint: endpoint.to_owned(),
                reason,
            })
            .ok_or_else(|| PlatformError::Device("no output device".into()))
    };
    let apps = match target {
        FarEndTarget::AllOutput => {
            return match pinned {
                Some(endpoint) => Ok(FarPlan::Device {
                    endpoint: endpoint.to_owned(),
                    reason: FarReason::Pinned,
                }),
                None => default(FarReason::AllOutput),
            };
        }
        FarEndTarget::Apps(apps) if apps.is_empty() => {
            return Err(PlatformError::Failed("no app to capture".into()));
        }
        FarEndTarget::Apps(apps) => apps,
    };
    if let [app] = apps.as_slice()
        && is_process_loopback_app(&app.id)
    {
        let pid = app
            .pid
            .filter(|&pid| {
                processes
                    .exe(pid)
                    .is_some_and(|exe| exe.eq_ignore_ascii_case(&app.id))
            })
            .and_then(|pid| processes.root_of(pid))
            .or_else(|| processes.roots_named(&app.id).first().copied())
            .ok_or_else(|| PlatformError::Device(format!("{} is not running", app.id)))?;
        return Ok(FarPlan::Process {
            pid,
            exe: processes.exe(pid).unwrap_or(&app.id).to_owned(),
        });
    }
    // Device loopback on the endpoint the app plays to: an active session first, then any.
    let belongs = |session: &&Session| belongs_to(session, apps, processes);
    let playing = render_sessions
        .iter()
        .filter(belongs)
        .find(|s| s.active)
        .or_else(|| render_sessions.iter().find(belongs));
    match playing {
        Some(session) => Ok(FarPlan::Device {
            endpoint: session.endpoint.clone(),
            reason: FarReason::AppPlaysHere,
        }),
        None => default(FarReason::AppNotPlayingYet),
    }
}

/// Whether `session`'s process is one of `apps` (by executable).
fn belongs_to(session: &Session, apps: &[AppRef], processes: &ProcessTable) -> bool {
    processes.exe(session.pid).is_some_and(|exe| {
        apps.iter()
            .any(|app: &AppRef| app.id.eq_ignore_ascii_case(exe))
    })
}

/// Whether a device-loopback far end recording the output `current` for `target` should be opened
/// again where [`plan_far_end`] would open it now: `current` is no longer an active output; for
/// all output, the pinned output when it is among `outputs` (so it goes back to it when it
/// returns), else the default, is another output; for apps, one of them plays (an active session)
/// and none plays on `current`. An app with no session to be seen (it may play through a helper) is
/// taken to play on the default output, as [`plan_far_end`] took it; one whose sessions are all
/// idle stays where it is (there is nothing to hear). Pure.
pub(crate) fn far_end_moved(
    target: &FarEndTarget,
    current: &str,
    outputs: &[String],
    default_output: Option<&str>,
    pinned: Option<&str>,
    render_sessions: &[Session],
    processes: &ProcessTable,
) -> bool {
    if !outputs.iter().any(|output| output == current) {
        return true;
    }
    let apps = match target {
        FarEndTarget::AllOutput => {
            let wanted = pinned
                .filter(|p| outputs.iter().any(|output| output == p))
                .or(default_output);
            return wanted.is_some_and(|w| w != current);
        }
        FarEndTarget::Apps(apps) => apps,
    };
    let sessions: Vec<&Session> = render_sessions
        .iter()
        .filter(|s| belongs_to(s, apps, processes))
        .collect();
    if sessions.is_empty() {
        return default_output.is_some_and(|d| d != current);
    }
    let mut active = sessions.iter().filter(|s| s.active).peekable();
    active.peek().is_some() && active.all(|s| s.endpoint != current)
}

/// [`CaptureControl`] for Windows.
pub struct WinCapture {
    clock: WinClock,
    guard: RealtimeGuard,
    headset_mic: AtomicBool,
    /// The device watch's thread, while watching. The lock is held only to swap it; the
    /// callbacks never take it.
    watch: Mutex<Option<watch::DeviceWatch>>,
}

impl WinCapture {
    /// Capture on `clock`'s timebase, the one every block's host time uses. It holds no OS
    /// resources until a stream is opened.
    pub fn new(clock: WinClock) -> Self {
        Self {
            clock,
            guard: unguarded(),
            headset_mic: AtomicBool::new(false),
            watch: Mutex::new(None),
        }
    }

    /// Runs every wake of streams opened from now on through `guard` (I4: tests and the checklist
    /// binary pass an `assert_no_alloc` guard).
    pub fn with_realtime_guard(mut self, guard: RealtimeGuard) -> Self {
        self.guard = guard;
        self
    }

    /// The user's setting: record a classic Bluetooth headset's own mic when the output is that
    /// headset. Off by default.
    pub fn set_headset_mic(&self, on: bool) {
        self.headset_mic.store(on, Ordering::Relaxed);
    }

    /// Whether the headset-mic setting is on.
    pub fn headset_mic(&self) -> bool {
        self.headset_mic.load(Ordering::Relaxed)
    }

    /// **Worker.** Active input endpoints, default first, with what routing reads.
    pub fn input_endpoints(&self) -> Result<Vec<Endpoint>, PlatformError> {
        let _com = ComScope::enter()?;
        let devices = devices::enumerator()?;
        Ok(sorted(devices::endpoints(&devices, Flow::Capture)?))
    }

    /// **Worker.** Active output endpoints, default first.
    pub fn output_endpoints(&self) -> Result<Vec<Endpoint>, PlatformError> {
        let _com = ComScope::enter()?;
        let devices = devices::enumerator()?;
        Ok(sorted(devices::endpoints(&devices, Flow::Render)?))
    }

    /// **Worker.** The mic `open_mic(None)` would record, and why. `None` when there is no input.
    pub fn mic_route(&self) -> Result<Option<(Endpoint, MicRouteReason)>, PlatformError> {
        let inputs = self.input_endpoints()?;
        let outputs = self.output_endpoints()?;
        let output = outputs.iter().find(|e| e.info.is_default);
        Ok(route_mic(&inputs, output, self.headset_mic())
            .map(|route| (route.device.clone(), route.reason)))
    }

    /// **Worker.** How `open_far_end_source(target, pinned)` would capture, without opening
    /// anything. `pinned` (an output's endpoint id, the user's choice) is used for all output while
    /// it is an active output; otherwise the default is.
    pub fn far_end_plan(
        &self,
        target: &FarEndTarget,
        pinned: Option<&str>,
    ) -> Result<FarPlan, PlatformError> {
        let _com = ComScope::enter()?;
        let devices = devices::enumerator()?;
        let default = devices::default_endpoint(&devices, Flow::Render)?
            .map(|d| devices::endpoint_id(&d))
            .transpose()?;
        let pinned = match (target, pinned) {
            (FarEndTarget::AllOutput, Some(pinned)) => devices::endpoints(&devices, Flow::Render)?
                .iter()
                .any(|e| e.info.id.0 == pinned)
                .then_some(pinned),
            _ => None,
        };
        let render = match target {
            FarEndTarget::AllOutput => Vec::new(),
            FarEndTarget::Apps(_) => sessions::read_all(&devices, Flow::Render)?.sessions,
        };
        let processes = match target {
            FarEndTarget::AllOutput => ProcessTable::default(),
            FarEndTarget::Apps(_) => ProcessTable::snapshot()?,
        };
        plan_far_end(target, default.as_deref(), pinned, &render, &processes)
    }

    /// **Worker** (or a meeting's pump). Whether a device-loopback far end opened for `target` on
    /// the output `current` should be opened again ([`far_end_moved`]), from the outputs, the
    /// pinned output (for all output), the render sessions and the processes now. Opens nothing.
    pub fn far_end_moved(
        &self,
        target: &FarEndTarget,
        current: &str,
        pinned: Option<&str>,
    ) -> Result<bool, PlatformError> {
        let _com = ComScope::enter()?;
        let devices = devices::enumerator()?;
        let outputs: Vec<String> = devices::endpoints(&devices, Flow::Render)?
            .into_iter()
            .map(|e| e.info.id.0)
            .collect();
        let default = devices::default_endpoint(&devices, Flow::Render)?
            .map(|d| devices::endpoint_id(&d))
            .transpose()?;
        let (render, processes) = match target {
            FarEndTarget::AllOutput => (Vec::new(), ProcessTable::default()),
            FarEndTarget::Apps(_) => (
                sessions::read_all(&devices, Flow::Render)?.sessions,
                ProcessTable::snapshot()?,
            ),
        };
        Ok(far_end_moved(
            target,
            current,
            &outputs,
            default.as_deref(),
            pinned,
            &render,
            &processes,
        ))
    }

    /// **Worker.** [`CaptureControl::open_mic`], as the concrete type (its counters are readable).
    pub fn open_mic_source(
        &self,
        device: Option<&DeviceId>,
    ) -> Result<WasapiSource, PlatformError> {
        let endpoint = match device {
            Some(requested) => {
                let inputs = self.input_endpoints()?;
                inputs
                    .into_iter()
                    .find(|e| e.info.id == *requested)
                    .ok_or_else(|| {
                        PlatformError::Device(format!("no input device with id {}", requested.0))
                    })?
            }
            None => self
                .mic_route()?
                .map(|(endpoint, _)| endpoint)
                .ok_or_else(|| PlatformError::Device("no input device".into()))?,
        };
        let name = endpoint.info.name.clone();
        WasapiSource::open(
            Channel::Mic,
            StreamKind::Mic {
                endpoint: endpoint.info.id.0,
            },
            name,
            self.clock,
            self.guard.clone(),
        )
    }

    /// **Worker.** [`CaptureControl::open_far_end`], as the concrete type, with all output pinned
    /// to `pinned` while it is connected ([`WinCapture::far_end_plan`]).
    pub fn open_far_end_source(
        &self,
        target: &FarEndTarget,
        pinned: Option<&str>,
    ) -> Result<WasapiSource, PlatformError> {
        let (kind, name) = match self.far_end_plan(target, pinned)? {
            FarPlan::Device { endpoint, .. } => {
                let name = self
                    .output_endpoints()?
                    .into_iter()
                    .find(|e| e.info.id.0 == endpoint)
                    .map_or_else(|| endpoint.clone(), |e| e.info.name);
                (StreamKind::DeviceLoopback { endpoint }, name)
            }
            FarPlan::Process { pid, exe } => (StreamKind::ProcessLoopback { pid }, exe),
        };
        WasapiSource::open(Channel::Far, kind, name, self.clock, self.guard.clone())
    }
}

/// Default first; otherwise the MMDevice API's order.
fn sorted(mut endpoints: Vec<Endpoint>) -> Vec<Endpoint> {
    endpoints.sort_by_key(|e| !e.info.is_default);
    endpoints
}

impl CaptureControl for WinCapture {
    fn input_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        Ok(self
            .input_endpoints()?
            .into_iter()
            .map(|e| e.info)
            .collect())
    }

    /// [`WinCapture::output_endpoints`]: the active render endpoints, default first.
    fn output_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        Ok(self
            .output_endpoints()?
            .into_iter()
            .map(|e| e.info)
            .collect())
    }

    fn default_output(&self) -> Result<Option<DeviceInfo>, PlatformError> {
        Ok(self
            .output_endpoints()?
            .into_iter()
            .map(|e| e.info)
            .find(|info| info.is_default))
    }

    /// [`WinCapture::mic_route`], in the core's words.
    fn automatic_input(&self) -> Result<Option<AutoInput>, PlatformError> {
        Ok(self.mic_route()?.map(|(endpoint, why)| AutoInput {
            device: endpoint.info,
            reason: auto_reason(why),
        }))
    }

    /// With `None`, the routed mic ([`WinCapture::mic_route`]).
    fn open_mic(&self, device: Option<&DeviceId>) -> Result<Box<dyn AudioSource>, PlatformError> {
        Ok(Box::new(self.open_mic_source(device)?))
    }

    /// `AllOutput` is device loopback of the default output. `Apps` follows [`plan_far_end`]:
    /// process-tree loopback for one of [`PROCESS_LOOPBACK_APPS`] (an error if it is not
    /// running), else device loopback of the endpoint the apps play to.
    fn open_far_end(&self, target: &FarEndTarget) -> Result<Box<dyn AudioSource>, PlatformError> {
        Ok(Box::new(self.open_far_end_source(target, None)?))
    }

    /// An `IMMNotificationClient` on its own thread: devices added, removed, changing state or
    /// renamed, and the console defaults (the module docs). A second call stops the first watch
    /// before starting the new one.
    fn watch_devices(&self, on_change: EventSink<DeviceChange>) -> Result<(), PlatformError> {
        let mut watch = self.watch.lock().unwrap_or_else(PoisonError::into_inner);
        // The old watch first: once it has stopped, its callback never runs again.
        if let Some(old) = watch.take() {
            old.stop();
        }
        *watch = Some(watch::DeviceWatch::start(on_change)?);
        Ok(())
    }

    /// Unregisters the notification client and stops its thread; when it returns, the callback
    /// will not run again.
    fn unwatch_devices(&self) {
        let old = self
            .watch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(old) = old {
            old.stop();
        }
    }
}

/// The core's [`AutoReason`] for the routing's reason.
fn auto_reason(why: MicRouteReason) -> AutoReason {
    match why {
        MicRouteReason::LeAudioHeadset => AutoReason::LeAudioHeadset,
        // Not the classic headset's call-quality mic: built in, or USB (the core's word covers
        // both, as the schema's does).
        MicRouteReason::NotBluetoothForBluetoothOutput => AutoReason::BuiltInForBluetoothOutput,
        MicRouteReason::HeadsetMicSetting => AutoReason::HeadsetMicSetting,
        MicRouteReason::OnlyBluetoothMics => AutoReason::NoBuiltInMic,
        MicRouteReason::FirstInput => AutoReason::FirstInput,
        // `Requested` is never the routing's answer (it routes only when nothing is named).
        MicRouteReason::DefaultInput | MicRouteReason::Requested => AutoReason::DefaultInput,
    }
}

/// A WASAPI capture stream: the mic, device loopback or process loopback.
///
/// A classic Bluetooth headset mic delivers 16 kHz call audio and **digital zeros while the user
/// is silent**, so a high share of exact zeros is normal on it; loopback delivers nothing at all
/// while nothing plays.
pub struct WasapiSource {
    channel: Channel,
    kind: StreamKind,
    name: String,
    format: StreamFormat,
    /// The speaker mask the stream is opened with (the device's own, or stereo's).
    mask: u32,
    clock: WinClock,
    guard: RealtimeGuard,
    counters: Arc<Counters>,
    running: Option<Running>,
}

impl WasapiSource {
    /// Reads the stream's format. Capture starts in `start`.
    fn open(
        channel: Channel,
        kind: StreamKind,
        name: String,
        clock: WinClock,
        guard: RealtimeGuard,
    ) -> Result<Self, PlatformError> {
        let (format, mask) = {
            let _com = ComScope::enter()?;
            stream::probe_format(&kind)?
        };
        Ok(Self {
            channel,
            kind,
            name,
            format,
            mask,
            clock,
            guard,
            counters: Arc::default(),
            running: None,
        })
    }

    /// The device's name as Windows shows it, or the executable for process loopback.
    pub fn device_name(&self) -> &str {
        &self.name
    }

    /// What it captures, for a log line: `mic`, `device loopback` or `process loopback (pid N)`.
    pub fn mode(&self) -> String {
        match &self.kind {
            StreamKind::Mic { .. } => "mic".into(),
            StreamKind::DeviceLoopback { .. } => "device loopback".into(),
            StreamKind::ProcessLoopback { pid } => format!("process loopback (pid {pid})"),
        }
    }

    /// Whether it hears one process tree alone (process loopback), rather than a whole device.
    pub fn is_process_loopback(&self) -> bool {
        matches!(self.kind, StreamKind::ProcessLoopback { .. })
    }

    /// The endpoint id it records (the mic's, or the output device loopback hears); `None` for
    /// process loopback.
    pub fn endpoint(&self) -> Option<&str> {
        match &self.kind {
            StreamKind::Mic { endpoint } | StreamKind::DeviceLoopback { endpoint } => {
                Some(endpoint)
            }
            StreamKind::ProcessLoopback { .. } => None,
        }
    }

    /// The current (or last) session's counters. **Any thread** that holds the source.
    pub fn stats(&self) -> IoStats {
        self.counters.snapshot()
    }

    fn what(&self) -> &'static str {
        match self.channel {
            Channel::Mic => "microphone",
            Channel::Far => "far end",
        }
    }
}

impl AudioSource for WasapiSource {
    fn channel(&self) -> Channel {
        self.channel
    }

    fn format(&self) -> StreamFormat {
        self.format
    }

    /// The mic refuses with `PermissionDenied(Microphone)` when Windows' privacy settings deny
    /// desktop apps the microphone, rather than starting a capture Windows would fill with
    /// silence.
    fn start(&mut self, sink: Box<dyn AudioSink>) -> Result<(), PlatformError> {
        if self.running.is_some() {
            return Err(PlatformError::Failed(format!(
                "the {} is already started",
                self.what()
            )));
        }
        if self.channel == Channel::Mic && permissions::microphone() == PermissionState::Denied {
            return Err(PlatformError::PermissionDenied(Permission::Microphone));
        }
        self.counters = Arc::default(); // each session is judged on its own
        self.running = Some(Running::start(
            self.kind.clone(),
            self.format,
            self.mask,
            sink,
            self.guard.clone(),
            self.clock,
            Arc::clone(&self.counters),
        )?);
        Ok(())
    }

    fn stop(&mut self) -> Result<SourceStats, PlatformError> {
        let Some(running) = self.running.take() else {
            return Ok(SourceStats::default());
        };
        running.stop();
        stream::session_result(self.counters.snapshot(), self.what())
    }

    /// The capture client failed (the device was removed or changed format) or the capture
    /// thread caught a panic: nothing more comes until it is opened again.
    fn ended(&self) -> bool {
        self.running.is_some() && self.counters.snapshot().ended()
    }
}

impl Drop for WasapiSource {
    fn drop(&mut self) {
        if let Some(running) = self.running.take() {
            running.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::ProcessEntry;

    /// The core decides from this list, with identities folded in ASCII, whether an Always app is
    /// heard alone: every entry must be ASCII, and found by its lowercased name.
    #[test]
    fn the_process_loopback_list_is_ascii_and_found_lowercased() {
        for exe in PROCESS_LOOPBACK_APPS {
            assert!(exe.is_ascii(), "{exe}");
            assert!(is_process_loopback_app(&exe.to_ascii_lowercase()), "{exe}");
        }
        assert!(!is_process_loopback_app("ms-teams.exe"));
    }

    fn app(id: &str, pid: Option<u32>) -> AppRef {
        AppRef {
            id: id.into(),
            pid,
            name: id.into(),
        }
    }

    fn table() -> ProcessTable {
        let e = |pid, parent, exe: &str| ProcessEntry {
            pid,
            parent,
            exe: exe.into(),
        };
        ProcessTable::from_entries([
            e(100, 4, "explorer.exe"),
            e(200, 100, "Zoom.exe"),
            e(210, 200, "Zoom.exe"),
            e(300, 100, "ms-teams.exe"),
            e(310, 300, "msedgewebview2.exe"),
            e(400, 100, "chrome.exe"),
            e(410, 400, "chrome.exe"),
        ])
    }

    fn session(pid: u32, active: bool, endpoint: &str) -> Session {
        Session {
            pid,
            active,
            endpoint: endpoint.into(),
        }
    }

    #[test]
    fn all_output_is_device_loopback_of_the_default() {
        let plan =
            plan_far_end(&FarEndTarget::AllOutput, Some("spk"), None, &[], &table()).unwrap();
        assert_eq!(
            plan,
            FarPlan::Device {
                endpoint: "spk".into(),
                reason: FarReason::AllOutput
            }
        );
        assert!(matches!(
            plan_far_end(&FarEndTarget::AllOutput, None, None, &[], &table()),
            Err(PlatformError::Device(_))
        ));
    }

    #[test]
    fn zoom_and_browsers_get_process_loopback_of_their_root() {
        let t = table();
        // The detector reports the child that holds the mic; loopback wants the tree's root.
        let zoom = FarEndTarget::Apps(vec![app("zoom.exe", Some(210))]);
        assert_eq!(
            plan_far_end(&zoom, Some("spk"), None, &[], &t).unwrap(),
            FarPlan::Process {
                pid: 200,
                exe: "Zoom.exe".into()
            }
        );
        // No pid: the first running tree of that name.
        let chrome = FarEndTarget::Apps(vec![app("Chrome.exe", None)]);
        assert_eq!(
            plan_far_end(&chrome, Some("spk"), None, &[], &t).unwrap(),
            FarPlan::Process {
                pid: 400,
                exe: "chrome.exe".into()
            }
        );
        // A stale pid now owned by another program is not trusted.
        let stale = FarEndTarget::Apps(vec![app("chrome.exe", Some(300))]);
        assert_eq!(
            plan_far_end(&stale, Some("spk"), None, &[], &t).unwrap(),
            FarPlan::Process {
                pid: 400,
                exe: "chrome.exe".into()
            }
        );
        let gone = FarEndTarget::Apps(vec![app("msedge.exe", None)]);
        assert!(matches!(
            plan_far_end(&gone, Some("spk"), None, &[], &t),
            Err(PlatformError::Device(m)) if m.contains("not running")
        ));
    }

    #[test]
    fn teams_gets_device_loopback_of_the_endpoint_it_plays_to() {
        let t = table();
        let teams = FarEndTarget::Apps(vec![app("ms-teams.exe", Some(300))]);
        let sessions = [
            session(400, true, "speakers"),
            session(300, false, "headset"),
            session(300, true, "usb-dac"),
        ];
        assert_eq!(
            plan_far_end(&teams, Some("speakers"), None, &sessions, &t).unwrap(),
            FarPlan::Device {
                endpoint: "usb-dac".into(),
                reason: FarReason::AppPlaysHere
            },
            "the active session wins over an idle one"
        );
        assert_eq!(
            plan_far_end(&teams, Some("speakers"), None, &sessions[..2], &t).unwrap(),
            FarPlan::Device {
                endpoint: "headset".into(),
                reason: FarReason::AppPlaysHere
            }
        );
        assert_eq!(
            plan_far_end(&teams, Some("speakers"), None, &[], &t).unwrap(),
            FarPlan::Device {
                endpoint: "speakers".into(),
                reason: FarReason::AppNotPlayingYet
            }
        );
    }

    #[test]
    fn several_apps_share_device_loopback() {
        let t = table();
        let both = FarEndTarget::Apps(vec![app("Zoom.exe", Some(200)), app("chrome.exe", None)]);
        let sessions = [session(410, true, "headset")];
        assert_eq!(
            plan_far_end(&both, Some("speakers"), None, &sessions, &t).unwrap(),
            FarPlan::Device {
                endpoint: "headset".into(),
                reason: FarReason::AppPlaysHere
            }
        );
        assert!(plan_far_end(&FarEndTarget::Apps(vec![]), Some("s"), None, &[], &t).is_err());
    }

    /// S3.5b: a device-loopback far end moves when its output goes, when "all output"'s default
    /// changes, and when its app plays on another output and not on this one; never while the app
    /// still plays here, or plays nowhere.
    #[test]
    fn a_device_loopback_far_end_moves_where_its_app_plays() {
        let t = table();
        let outputs = ["speakers".to_owned(), "headset".to_owned()];
        let all = FarEndTarget::AllOutput;
        let moved = |target: &FarEndTarget, current: &str, default: &str, sessions: &[Session]| {
            far_end_moved(target, current, &outputs, Some(default), None, sessions, &t)
        };
        assert!(!moved(&all, "speakers", "speakers", &[]));
        assert!(
            moved(&all, "speakers", "headset", &[]),
            "the default changed"
        );
        assert!(
            far_end_moved(&all, "usb-dac", &outputs, Some("speakers"), None, &[], &t),
            "its output was unplugged"
        );

        let teams = FarEndTarget::Apps(vec![app("ms-teams.exe", Some(300))]);
        // A headset plugged in: Teams now plays there, and no longer here.
        assert!(moved(
            &teams,
            "speakers",
            "headset",
            &[session(300, true, "headset")]
        ));
        assert!(moved(
            &teams,
            "speakers",
            "speakers",
            &[
                session(300, false, "speakers"),
                session(300, true, "headset")
            ]
        ));
        // It still plays here (a ringer elsewhere changes nothing), or plays nowhere: it stays.
        assert!(!moved(
            &teams,
            "speakers",
            "headset",
            &[
                session(300, true, "speakers"),
                session(300, true, "headset")
            ]
        ));
        assert!(!moved(
            &teams,
            "speakers",
            "headset",
            &[session(300, false, "headset")]
        ));
        // Another app playing elsewhere is not Teams.
        assert!(!moved(
            &teams,
            "speakers",
            "speakers",
            &[session(400, true, "headset")]
        ));
        // No session of Teams to be seen (it may play through a helper): the default, as planned.
        assert!(moved(
            &teams,
            "speakers",
            "headset",
            &[session(400, true, "speakers")]
        ));
        assert!(!moved(&teams, "speakers", "speakers", &[]));
        // No output at all: it must move, and opening it will say there is nowhere to go.
        assert!(far_end_moved(&teams, "speakers", &[], None, None, &[], &t));
    }

    /// The output the user chose is what all output records; an app keeps its own output.
    #[test]
    fn all_output_records_the_pinned_output_and_apps_keep_their_own() {
        let t = table();
        assert_eq!(
            plan_far_end(&FarEndTarget::AllOutput, Some("spk"), Some("dock"), &[], &t).unwrap(),
            FarPlan::Device {
                endpoint: "dock".into(),
                reason: FarReason::Pinned
            }
        );
        // Pinned, and no default at all: still the pinned output.
        assert!(plan_far_end(&FarEndTarget::AllOutput, None, Some("dock"), &[], &t).is_ok());
        let teams = FarEndTarget::Apps(vec![app("ms-teams.exe", Some(300))]);
        assert_eq!(
            plan_far_end(&teams, Some("spk"), Some("dock"), &[], &t).unwrap(),
            FarPlan::Device {
                endpoint: "spk".into(),
                reason: FarReason::AppNotPlayingYet
            },
            "an app not playing yet plays on the default, pin or not"
        );
        let zoom = FarEndTarget::Apps(vec![app("zoom.exe", Some(210))]);
        assert!(matches!(
            plan_far_end(&zoom, Some("spk"), Some("dock"), &[], &t).unwrap(),
            FarPlan::Process { .. }
        ));
    }

    /// Pinned all output stays on its output when the default changes, records the default while
    /// it is unplugged, and goes back to it when it returns. An app's far end ignores the pin.
    #[test]
    fn a_pinned_far_end_stays_falls_back_and_goes_back() {
        let t = table();
        let all = FarEndTarget::AllOutput;
        let with_dock = [
            "speakers".to_owned(),
            "dock".to_owned(),
            "headset".to_owned(),
        ];
        let without = ["speakers".to_owned(), "headset".to_owned()];
        let moved = |current: &str, outputs: &[String], default: &str| {
            far_end_moved(&all, current, outputs, Some(default), Some("dock"), &[], &t)
        };
        assert!(!moved("dock", &with_dock, "speakers"), "on its output");
        assert!(
            !moved("dock", &with_dock, "headset"),
            "a new default changes nothing"
        );
        assert!(
            moved("dock", &without, "speakers"),
            "unplugged: it must move"
        );
        assert!(
            !moved("speakers", &without, "speakers"),
            "on the default while the pin is away"
        );
        assert!(
            moved("speakers", &without, "headset"),
            "and follows the default meanwhile"
        );
        assert!(moved("speakers", &with_dock, "speakers"), "back to the pin");
        let teams = FarEndTarget::Apps(vec![app("ms-teams.exe", Some(300))]);
        assert!(!far_end_moved(
            &teams,
            "speakers",
            &with_dock,
            Some("speakers"),
            Some("dock"),
            &[session(300, true, "speakers")],
            &t
        ));
    }

    // Local only: these talk to the audio service. They open nothing and need no permission.

    #[test]
    #[ignore = "talks to the Windows audio service"]
    fn the_real_devices_list_and_route() {
        let capture = WinCapture::new(WinClock::new().unwrap());
        let inputs = capture.input_devices().expect("inputs");
        assert!(inputs.iter().filter(|d| d.is_default).count() <= 1);
        if let Some(first) = inputs.first() {
            assert!(first.is_default || inputs.iter().all(|d| !d.is_default));
        }
        let output = capture.default_output().expect("output");
        let route = capture.mic_route().expect("route");
        assert_eq!(route.is_some(), !inputs.is_empty(), "{output:?}");
        let plan = capture.far_end_plan(&FarEndTarget::AllOutput, None);
        assert_eq!(plan.is_ok(), output.is_some(), "{plan:?}");
    }

    #[test]
    #[ignore = "talks to the Windows audio service"]
    fn a_routed_mic_and_the_default_loopback_open_without_starting() {
        let capture = WinCapture::new(WinClock::new().unwrap());
        if !capture.input_devices().unwrap().is_empty() {
            let mic = capture.open_mic_source(None).expect("open mic");
            assert!(mic.format().sample_rate >= 8_000, "{:?}", mic.format());
            assert!(mic.format().channels >= 1);
            assert_eq!(mic.stats(), IoStats::default(), "nothing started");
        }
        if capture.default_output().unwrap().is_some() {
            let far = capture
                .open_far_end_source(&FarEndTarget::AllOutput, None)
                .expect("open loopback");
            assert!(far.format().sample_rate >= 8_000, "{:?}", far.format());
            assert_eq!(far.mode(), "device loopback");
            let endpoint = far.endpoint().expect("device loopback names its output");
            assert!(!far.ended(), "nothing started, nothing ended");
            // Just opened on the default output, which has not changed: it stays.
            assert!(
                !capture
                    .far_end_moved(&FarEndTarget::AllOutput, endpoint, None)
                    .expect("read the outputs")
            );
            assert!(
                capture
                    .far_end_moved(&FarEndTarget::AllOutput, "{0.0.0.00000000}.{gone}", None)
                    .expect("read the outputs"),
                "an output that is not there moves"
            );
        }
    }
}
