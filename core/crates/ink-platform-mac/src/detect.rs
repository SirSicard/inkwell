//! Meeting detection: which apps hold the microphone, from the audio server.
//!
//! **The signal** is `kAudioProcessPropertyIsRunningInput` on each HAL process object: the app has
//! an input stream running. A process holding the mic open is what a call is, mechanically, so
//! this works for any meeting app, a browser tab or a phone call routed through the Mac, with no
//! per-app integration.
//!
//! **Polled, not listened for (decided).** The process-object list only changes when a process
//! appears or disappears; an app that was already running when it joins a call only flips its
//! running-input flag. A listener on the list misses exactly that common case, so a thread polls
//! every second while watching. This is detection's own cadence, on its own thread; nothing is
//! drawn.
//!
//! **Apple's daemons do not count.** `com.apple.CoreSpeech` (Siri's listener) holds an input stream
//! permanently: counted, every Mac would be in a meeting from boot. The rule is an allowlist, not
//! a denylist: any `com.apple.*` process is ignored unless it is one of Apple's own call apps
//! ([`APPLE_CALL_APPS`]). Apple adds daemons; a denylist would rot, and a rotted one means
//! detection that never turns off. Also ignored: this process (it holds the mic while it records)
//! and processes without a bundle id (command-line tools).
//!
//! The core debounces the signals and applies its own allowlist of meeting apps; this reports what
//! the OS says, minus the above.
#![cfg(target_os = "macos")]

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ink_core::{AppRef, EventSink, MeetingDetector, MeetingSignal, PlatformError};
use objc2_app_kit::NSRunningApplication;

use crate::capture::{HalError, ObjectId, ProcessHal, SystemHal, own_pid};

/// How often the watcher polls.
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Apple's own apps whose microphone use is a call. Every other `com.apple.*` process is a system
/// daemon for detection's purposes.
pub const APPLE_CALL_APPS: [&str; 3] = [
    "com.apple.FaceTime",
    "com.apple.Safari",
    "com.apple.QuickTimePlayerX",
];

/// Whether a process holding the mic may be reported: not this process, not a tool without a
/// bundle id, and not one of Apple's system daemons.
pub fn counts_as_candidate(bundle_id: &str, pid: i32, own_pid: i32) -> bool {
    pid != own_pid && !bundle_id.is_empty() && !is_apple_daemon(bundle_id)
}

/// `com.apple.*` and not one of [`APPLE_CALL_APPS`].
pub fn is_apple_daemon(bundle_id: &str) -> bool {
    bundle_id.starts_with("com.apple.") && !APPLE_CALL_APPS.contains(&bundle_id)
}

/// One process holding the mic.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Holder {
    pid: i32,
    bundle_id: String,
}

/// What one poll saw: the process objects holding input, and each one's identity.
type Scan = BTreeMap<ObjectId, Holder>;

/// Reads which processes hold input. A process that exits between the list and its reads is left
/// out. A process still listed whose flag cannot be read keeps its previous answer (it held the mic
/// last time, so it still does), so a read failure never ends a meeting. A failure to list at all
/// is an error: the caller keeps its state rather than reporting everyone released.
fn scan(
    hal: &dyn ProcessHal,
    own_pid: i32,
    previous: &Scan,
    errors: &AtomicU64,
) -> Result<Scan, PlatformError> {
    let mut now = Scan::new();
    for object in hal.process_objects()? {
        let running = match hal.is_running_input(object) {
            Ok(running) => running,
            Err(e) if e.is_gone() => continue,
            Err(_) => {
                errors.fetch_add(1, Ordering::Relaxed);
                if let Some(held) = previous.get(&object) {
                    now.insert(object, held.clone());
                }
                continue;
            }
        };
        if !running {
            continue;
        }
        let identity = || -> Result<(i32, String), HalError> {
            Ok((hal.pid(object)?, hal.bundle_id(object)?))
        };
        let (pid, bundle_id) = match identity() {
            Ok(identity) => identity,
            Err(e) if e.is_gone() => continue,
            Err(_) => {
                errors.fetch_add(1, Ordering::Relaxed);
                if let Some(held) = previous.get(&object) {
                    now.insert(object, held.clone());
                }
                continue;
            }
        };
        if counts_as_candidate(&bundle_id, pid, own_pid) {
            now.insert(object, Holder { pid, bundle_id });
        }
    }
    Ok(now)
}

/// The apps (by bundle id) in a scan, each with one of its pids.
fn apps(scan: &Scan) -> BTreeMap<&str, i32> {
    let mut apps = BTreeMap::new();
    for holder in scan.values() {
        apps.entry(holder.bundle_id.as_str()).or_insert(holder.pid);
    }
    apps
}

/// The signals between two scans: releases first, then new holders, each in bundle-id order. An
/// app is one signal however many of its processes hold the mic.
fn changes(before: &Scan, after: &Scan, name: &dyn Fn(&str, i32) -> String) -> Vec<MeetingSignal> {
    let (was, is) = (apps(before), apps(after));
    let app = |bundle: &str, pid: i32| AppRef {
        id: bundle.to_owned(),
        pid: u32::try_from(pid).ok(),
        name: name(bundle, pid),
    };
    let released = was
        .iter()
        .filter(|(bundle, _)| !is.contains_key(*bundle))
        .map(|(bundle, &pid)| MeetingSignal::MicReleased {
            app: app(bundle, pid),
        });
    let started = is
        .iter()
        .filter(|(bundle, _)| !was.contains_key(*bundle))
        .map(|(bundle, &pid)| MeetingSignal::MicInUse {
            app: app(bundle, pid),
        });
    released.chain(started).collect()
}

/// The name macOS shows for the app with `pid`, else the last part of its bundle id
/// (`com.example.VideoCall` → `VideoCall`).
fn display_name(bundle_id: &str, pid: i32) -> String {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .and_then(|app| app.localizedName())
        .map(|name| name.to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| fallback_name(bundle_id))
}

fn fallback_name(bundle_id: &str) -> String {
    bundle_id
        .rsplit('.')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(bundle_id)
        .to_owned()
}

type Namer = Arc<dyn Fn(&str, i32) -> String + Send + Sync>;

/// A running watcher thread.
struct Watcher {
    stop: mpsc::Sender<()>,
    thread: JoinHandle<()>,
}

/// [`MeetingDetector`] for macOS: a thread that polls the audio server's process objects.
pub struct MacMeetingDetector {
    hal: Arc<dyn ProcessHal>,
    namer: Namer,
    interval: Duration,
    own_pid: i32,
    watcher: Mutex<Option<Watcher>>,
    errors: Arc<AtomicU64>,
    panics: Arc<AtomicU64>,
}

impl MacMeetingDetector {
    /// A detector on the real audio server, polling every [`POLL_INTERVAL`]. Nothing runs until
    /// [`start`](MeetingDetector::start). Needs no permission.
    pub fn new() -> Self {
        Self::with_hal(
            Arc::new(SystemHal),
            Arc::new(display_name),
            POLL_INTERVAL,
            own_pid(),
        )
    }

    fn with_hal(hal: Arc<dyn ProcessHal>, namer: Namer, interval: Duration, own_pid: i32) -> Self {
        Self {
            hal,
            namer,
            interval,
            own_pid,
            watcher: Mutex::new(None),
            errors: Arc::default(),
            panics: Arc::default(),
        }
    }

    /// Reads that failed since this detector was made: the process list (that poll is skipped and
    /// the previous state kept) or one process's flags (its previous answer is kept). A count that
    /// keeps rising means detection is running blind; the UI should say so.
    pub fn read_errors(&self) -> u64 {
        self.errors.load(Ordering::Relaxed)
    }

    /// Panics caught from the core's callback. Each one lost that signal; a non-zero count is a
    /// bug to report.
    pub fn callback_panics(&self) -> u64 {
        self.panics.load(Ordering::Relaxed)
    }
}

impl Default for MacMeetingDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl MeetingDetector for MacMeetingDetector {
    /// Starts a fresh watcher, replacing (and joining) any running one. Its first poll happens at
    /// once and reports every app already holding the mic as `MicInUse`, so launching during a
    /// call still catches the call.
    fn start(&self, on_signal: EventSink<MeetingSignal>) -> Result<(), PlatformError> {
        let mut slot = self.watcher.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(old) = slot.take() {
            halt(old);
        }
        let (stop, stopped) = mpsc::channel();
        let hal = Arc::clone(&self.hal);
        let namer = Arc::clone(&self.namer);
        let (interval, own_pid) = (self.interval, self.own_pid);
        let (errors, panics) = (Arc::clone(&self.errors), Arc::clone(&self.panics));
        let thread = thread::Builder::new()
            .name("ink-meeting-detector".into())
            .spawn(move || {
                let mut state = Scan::new();
                loop {
                    match scan(hal.as_ref(), own_pid, &state, &errors) {
                        Ok(now) => {
                            for signal in changes(&state, &now, namer.as_ref()) {
                                let sink = &on_signal;
                                if catch_unwind(AssertUnwindSafe(|| sink(signal))).is_err() {
                                    panics.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            state = now;
                        }
                        Err(_) => {
                            errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    match stopped.recv_timeout(interval) {
                        Err(RecvTimeoutError::Timeout) => {}
                        Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            })
            .map_err(|e| PlatformError::Failed(format!("could not start the detector: {e}")))?;
        *slot = Some(Watcher { stop, thread });
        Ok(())
    }

    /// Stops and joins the watcher: when it returns, the callback will not run again. Never call
    /// it from the callback itself (the join would wait for the call that is making it).
    fn stop(&self) {
        let old = self
            .watcher
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(old) = old {
            halt(old);
        }
    }
}

impl Drop for MacMeetingDetector {
    fn drop(&mut self) {
        self.stop();
    }
}

fn halt(watcher: Watcher) {
    // A send fails only if the thread already ended; either way it is ending.
    let _ = watcher.stop.send(());
    // The thread catches the callback's panics, so a join error is a panic in the scan itself,
    // which leaves nothing to clean up.
    let _ = watcher.thread.join();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::hal::fake::FakeHal;
    use objc2_core_audio::kAudioHardwareIllegalOperationError;

    const OWN: i32 = 4_242;

    fn scan_of(hal: &FakeHal, previous: &Scan) -> Scan {
        scan(hal, OWN, previous, &AtomicU64::new(0)).unwrap()
    }

    fn name(bundle: &str, _pid: i32) -> String {
        fallback_name(bundle)
    }

    fn ids(signals: &[MeetingSignal]) -> Vec<String> {
        signals
            .iter()
            .map(|s| match s {
                MeetingSignal::MicInUse { app } => format!("+{}", app.id),
                MeetingSignal::MicReleased { app } => format!("-{}", app.id),
            })
            .collect()
    }

    #[test]
    fn core_speech_is_not_a_meeting() {
        assert!(!counts_as_candidate("com.apple.CoreSpeech", 88, OWN));
        let hal = FakeHal::with(&[(10, 88, "com.apple.CoreSpeech", true)]);
        assert!(scan_of(&hal, &Scan::new()).is_empty());
    }

    #[test]
    fn unknown_apple_daemons_are_not_meetings() {
        for daemon in [
            "com.apple.siri.daemon",
            "com.apple.corespeechd",
            "com.apple.NewDaemon",
        ] {
            assert!(is_apple_daemon(daemon), "{daemon}");
            assert!(!counts_as_candidate(daemon, 88, OWN));
        }
    }

    #[test]
    fn apple_call_apps_count() {
        for app in APPLE_CALL_APPS {
            assert!(counts_as_candidate(app, 88, OWN), "{app}");
        }
        assert!(counts_as_candidate("us.zoom.xos", 88, OWN));
    }

    #[test]
    fn this_process_and_tools_without_a_bundle_never_count() {
        assert!(!counts_as_candidate("com.example.Inkwell", OWN, OWN));
        assert!(!counts_as_candidate("", 88, OWN));
    }

    /// Detection polls `kAudioProcessPropertyIsRunningInput`: an app that was already running
    /// flips only its flag, and each flip is one signal.
    #[test]
    fn a_running_app_taking_and_releasing_the_mic_is_reported_once_each() {
        let hal = FakeHal::with(&[
            (10, 88, "com.apple.CoreSpeech", true),
            (11, 501, "us.zoom.xos", false),
        ]);
        let first = scan_of(&hal, &Scan::new());
        assert!(changes(&Scan::new(), &first, &name).is_empty());

        hal.set(11, 501, "us.zoom.xos", true); // joins a call
        let second = scan_of(&hal, &first);
        let signals = changes(&first, &second, &name);
        assert_eq!(ids(&signals), ["+us.zoom.xos"]);
        match &signals[0] {
            MeetingSignal::MicInUse { app } => {
                assert_eq!(app.pid, Some(501));
                assert_eq!(app.name, "xos");
            }
            other => panic!("{other:?}"),
        }
        assert!(
            changes(&second, &scan_of(&hal, &second), &name).is_empty(),
            "no repeat"
        );

        hal.set(11, 501, "us.zoom.xos", false); // leaves
        let third = scan_of(&hal, &second);
        assert_eq!(ids(&changes(&second, &third, &name)), ["-us.zoom.xos"]);
    }

    #[test]
    fn an_app_with_several_processes_on_the_mic_is_one_signal() {
        let hal = FakeHal::with(&[
            (20, 700, "com.example.Browser", true),
            (21, 701, "com.example.Browser", true),
        ]);
        let now = scan_of(&hal, &Scan::new());
        assert_eq!(
            ids(&changes(&Scan::new(), &now, &name)),
            ["+com.example.Browser"]
        );
        hal.set(21, 701, "com.example.Browser", false);
        let later = scan_of(&hal, &now);
        assert!(
            changes(&now, &later, &name).is_empty(),
            "one process still holds it"
        );
    }

    #[test]
    fn a_process_that_exits_is_released() {
        let hal = FakeHal::with(&[(30, 900, "com.example.Call", true)]);
        let before = scan_of(&hal, &Scan::new());
        hal.remove(30);
        let after = scan_of(&hal, &before);
        assert_eq!(ids(&changes(&before, &after, &name)), ["-com.example.Call"]);
    }

    #[test]
    fn a_failed_flag_read_keeps_the_previous_answer_and_is_counted() {
        let hal = FakeHal::with(&[(40, 1_000, "com.example.Call", true)]);
        let before = scan_of(&hal, &Scan::new());
        hal.processes
            .lock()
            .unwrap()
            .get_mut(&40)
            .unwrap()
            .running_input = None;
        let errors = AtomicU64::new(0);
        let after = scan(&hal, OWN, &before, &errors).unwrap();
        assert!(
            changes(&before, &after, &name).is_empty(),
            "no false release"
        );
        assert_eq!(errors.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_failed_process_list_is_an_error_not_everyone_released() {
        let hal = FakeHal::with(&[(40, 1_000, "com.example.Call", true)]);
        *hal.list_fails.lock().unwrap() = Some(kAudioHardwareIllegalOperationError);
        assert!(scan(&hal, OWN, &Scan::new(), &AtomicU64::new(0)).is_err());
    }

    #[test]
    fn fallback_names_come_from_the_bundle_id() {
        assert_eq!(fallback_name("com.example.VideoCall"), "VideoCall");
        assert_eq!(fallback_name("tool"), "tool");
        assert_eq!(fallback_name("trailing."), "trailing.");
    }

    /// The watcher thread end to end on the fake server: the call already in progress is reported
    /// at start, changes arrive, errors are counted, and after `stop` nothing more arrives.
    #[test]
    fn the_watcher_reports_changes_and_is_silent_after_stop() {
        let hal = Arc::new(FakeHal::with(&[(50, 1_100, "com.example.Call", true)]));
        let detector = MacMeetingDetector::with_hal(
            hal.clone(),
            Arc::new(name),
            Duration::from_millis(5),
            OWN,
        );
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sink: EventSink<MeetingSignal> = Arc::new(move |s| {
            let _ = tx.lock().unwrap().send(s);
        });
        detector.start(sink).unwrap();
        let first = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(ids(&[first]), ["+com.example.Call"], "already in progress");

        hal.set(50, 1_100, "com.example.Call", false);
        let second = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(ids(&[second]), ["-com.example.Call"]);

        *hal.list_fails.lock().unwrap() = Some(kAudioHardwareIllegalOperationError);
        let start = std::time::Instant::now();
        while detector.read_errors() == 0 && start.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(detector.read_errors() > 0, "a failed poll is counted");
        *hal.list_fails.lock().unwrap() = None;

        detector.stop();
        hal.set(50, 1_100, "com.example.Call", true);
        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "nothing after stop"
        );
    }

    #[test]
    fn a_panicking_callback_is_counted_and_the_watcher_goes_on() {
        let hal = Arc::new(FakeHal::with(&[(60, 1_200, "com.example.Call", true)]));
        let detector = MacMeetingDetector::with_hal(
            hal.clone(),
            Arc::new(name),
            Duration::from_millis(5),
            OWN,
        );
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sink: EventSink<MeetingSignal> = Arc::new(move |s| {
            if matches!(s, MeetingSignal::MicInUse { .. }) {
                panic!("core bug");
            }
            let _ = tx.lock().unwrap().send(s);
        });
        detector.start(sink).unwrap();
        // The first poll reports the call (and the sink panics) before the call ends.
        let start = std::time::Instant::now();
        while detector.callback_panics() == 0 && start.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(5));
        }
        hal.set(60, 1_200, "com.example.Call", false);
        let released = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(ids(&[released]), ["-com.example.Call"]);
        assert_eq!(detector.callback_panics(), 1);
        detector.stop();
    }

    /// Needs no permission, only the audio server: whatever holds the mic here, Apple's daemons
    /// are filtered and this process is not reported.
    #[test]
    #[ignore = "talks to the local audio server"]
    fn the_real_server_scans_without_apple_daemons() {
        let now = scan(&SystemHal, own_pid(), &Scan::new(), &AtomicU64::new(0)).expect("scan");
        for holder in now.values() {
            assert!(!is_apple_daemon(&holder.bundle_id), "{holder:?}");
            assert_ne!(holder.pid, own_pid());
        }
        let set: std::collections::BTreeSet<&str> =
            now.values().map(|h| h.bundle_id.as_str()).collect();
        eprintln!("apps holding the mic now: {}", set.len());
    }
}
