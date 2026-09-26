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
//! **Never silently blind.** The whole poll runs under `catch_unwind`. A panic in it, or
//! [`LOST_AFTER_FAILED_POLLS`] failed polls in a row, ends the watcher with one
//! [`MeetingSignal::Lost`]. A poll fails when the process list cannot be read, or when every
//! process's read in it fails; one unreadable process is counted but does not blind detection.
//! Stopping a watcher checks how its thread ended, and reports `Lost` if it died unreported.
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

/// Failed polls in a row after which detection reports [`MeetingSignal::Lost`] and stops: ten
/// seconds of an audio server that does not answer, at [`POLL_INTERVAL`]. Long enough to ride out
/// `coreaudiod` restarting; short enough that a meeting is not missed unannounced.
pub const LOST_AFTER_FAILED_POLLS: u32 = 10;

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

/// One poll: what it saw, and how many of its per-process reads failed.
struct Poll {
    scan: Scan,
    reads: usize,
    failed: usize,
}

impl Poll {
    /// Every process's read failed: the poll saw nothing it can trust. (No processes at all is not
    /// blind; it is an idle Mac.)
    fn blind(&self) -> bool {
        self.reads > 0 && self.failed == self.reads
    }
}

/// Reads which processes hold input. A process that exits between the list and its reads is left
/// out. A process still listed whose flag cannot be read keeps its previous answer (it held the mic
/// last time, so it still does), so a read failure never ends a meeting; each is counted in
/// `errors`. A failure to list at all is an error: the caller keeps its state rather than reporting
/// everyone released.
fn scan(
    hal: &dyn ProcessHal,
    own_pid: i32,
    previous: &Scan,
    errors: &AtomicU64,
) -> Result<Poll, HalError> {
    let mut poll = Poll {
        scan: Scan::new(),
        reads: 0,
        failed: 0,
    };
    let keep_previous = |poll: &mut Poll, object: ObjectId| {
        errors.fetch_add(1, Ordering::Relaxed);
        poll.failed += 1;
        if let Some(held) = previous.get(&object) {
            poll.scan.insert(object, held.clone());
        }
    };
    for object in hal.process_objects()? {
        let running = match hal.is_running_input(object) {
            Ok(running) => running,
            Err(e) if e.is_gone() => continue,
            Err(_) => {
                poll.reads += 1;
                keep_previous(&mut poll, object);
                continue;
            }
        };
        poll.reads += 1;
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
                keep_previous(&mut poll, object);
                continue;
            }
        };
        if counts_as_candidate(&bundle_id, pid, own_pid) {
            poll.scan.insert(object, Holder { pid, bundle_id });
        }
    }
    Ok(poll)
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

/// What a watcher and its detector share.
#[derive(Debug, Default)]
struct WatchState {
    /// Why the watcher ended on its own, once it has reported [`MeetingSignal::Lost`].
    lost: Mutex<Option<String>>,
}

/// A running watcher thread.
struct Watcher {
    stop: mpsc::Sender<()>,
    thread: JoinHandle<()>,
    sink: EventSink<MeetingSignal>,
    state: Arc<WatchState>,
}

/// The detector's counters, shared with its watchers.
#[derive(Debug, Default)]
struct Counters {
    errors: AtomicU64,
    callback_panics: AtomicU64,
    watcher_panics: AtomicU64,
}

/// Calls the core's sink; a panic in it is caught and counted, and loses that one signal.
fn deliver(sink: &EventSink<MeetingSignal>, signal: MeetingSignal, counters: &Counters) {
    if catch_unwind(AssertUnwindSafe(|| sink(signal))).is_err() {
        counters.callback_panics.fetch_add(1, Ordering::Relaxed);
    }
}

/// Reports [`MeetingSignal::Lost`], at most once per watcher.
fn report_lost(
    sink: &EventSink<MeetingSignal>,
    state: &WatchState,
    counters: &Counters,
    reason: String,
) {
    {
        let mut lost = state.lost.lock().unwrap_or_else(PoisonError::into_inner);
        if lost.is_some() {
            return;
        }
        *lost = Some(reason.clone());
    }
    deliver(sink, MeetingSignal::Lost { reason }, counters);
}

/// What the watcher does after a poll.
enum Next {
    Wait,
    Lost(String),
}

/// The watcher thread: poll, report, wait; until stopped, or until it reports `Lost`.
fn watch(
    config: (Arc<dyn ProcessHal>, Namer, Duration, i32, u32),
    sink: &EventSink<MeetingSignal>,
    state: &WatchState,
    counters: &Counters,
    stopped: &mpsc::Receiver<()>,
) {
    let (hal, namer, interval, own_pid, limit) = config;
    let mut scanned = Scan::new();
    let mut failed_polls = 0u32;
    loop {
        // The whole poll is guarded: a bug in it ends detection with `Lost`, never silently.
        let next = catch_unwind(AssertUnwindSafe(|| {
            match scan(hal.as_ref(), own_pid, &scanned, &counters.errors) {
                Ok(poll) => {
                    for signal in changes(&scanned, &poll.scan, namer.as_ref()) {
                        deliver(sink, signal, counters);
                    }
                    failed_polls = if poll.blind() { failed_polls + 1 } else { 0 };
                    scanned = poll.scan;
                }
                Err(_) => {
                    counters.errors.fetch_add(1, Ordering::Relaxed);
                    failed_polls += 1;
                }
            }
            if failed_polls >= limit {
                Next::Lost(format!(
                    "the audio server did not answer {limit} polls in a row; meeting detection \
                     stopped"
                ))
            } else {
                Next::Wait
            }
        }));
        match next {
            Ok(Next::Wait) => {}
            Ok(Next::Lost(reason)) => return report_lost(sink, state, counters, reason),
            Err(_) => {
                counters.watcher_panics.fetch_add(1, Ordering::Relaxed);
                let reason = "the meeting detector panicked (a bug); detection stopped".to_owned();
                return report_lost(sink, state, counters, reason);
            }
        }
        match stopped.recv_timeout(interval) {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// [`MeetingDetector`] for macOS: a thread that polls the audio server's process objects.
pub struct MacMeetingDetector {
    hal: Arc<dyn ProcessHal>,
    namer: Namer,
    interval: Duration,
    own_pid: i32,
    lost_after: u32,
    watcher: Mutex<Option<Watcher>>,
    counters: Arc<Counters>,
}

impl MacMeetingDetector {
    /// A detector on the real audio server, polling every [`POLL_INTERVAL`] and reporting `Lost`
    /// after [`LOST_AFTER_FAILED_POLLS`]. Nothing runs until [`start`](MeetingDetector::start).
    /// Needs no permission.
    pub fn new() -> Self {
        Self::with_hal(
            Arc::new(SystemHal),
            Arc::new(display_name),
            POLL_INTERVAL,
            own_pid(),
            LOST_AFTER_FAILED_POLLS,
        )
    }

    fn with_hal(
        hal: Arc<dyn ProcessHal>,
        namer: Namer,
        interval: Duration,
        own_pid: i32,
        lost_after: u32,
    ) -> Self {
        Self {
            hal,
            namer,
            interval,
            own_pid,
            lost_after: lost_after.max(1),
            watcher: Mutex::new(None),
            counters: Arc::default(),
        }
    }

    /// Reads that failed since this detector was made: a whole process list (that poll keeps the
    /// previous state) or one process's flags (its previous answer is kept). A count that keeps
    /// rising means detection is running blind; past [`LOST_AFTER_FAILED_POLLS`] failed polls in
    /// a row it reports `Lost`.
    pub fn read_errors(&self) -> u64 {
        self.counters.errors.load(Ordering::Relaxed)
    }

    /// Panics caught from the core's callback. Each one lost that signal; a non-zero count is a
    /// bug to report.
    pub fn callback_panics(&self) -> u64 {
        self.counters.callback_panics.load(Ordering::Relaxed)
    }

    /// Panics in the watcher itself: each one ended detection with `Lost`. A bug to report.
    pub fn watcher_panics(&self) -> u64 {
        self.counters.watcher_panics.load(Ordering::Relaxed)
    }

    /// Why the current watcher ended on its own, if it did (the reason `Lost` carried).
    pub fn lost_reason(&self) -> Option<String> {
        let slot = self.watcher.lock().unwrap_or_else(PoisonError::into_inner);
        slot.as_ref().and_then(|w| {
            w.state
                .lost
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        })
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
            halt(old, &self.counters);
        }
        let (stop, stopped) = mpsc::channel();
        let state = Arc::new(WatchState::default());
        let config = (
            Arc::clone(&self.hal),
            Arc::clone(&self.namer),
            self.interval,
            self.own_pid,
            self.lost_after,
        );
        let thread = thread::Builder::new()
            .name("ink-meeting-detector".into())
            .spawn({
                let (sink, state) = (on_signal.clone(), Arc::clone(&state));
                let counters = Arc::clone(&self.counters);
                move || watch(config, &sink, &state, &counters, &stopped)
            })
            .map_err(|e| PlatformError::Failed(format!("could not start the detector: {e}")))?;
        *slot = Some(Watcher {
            stop,
            thread,
            sink: on_signal,
            state,
        });
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
            halt(old, &self.counters);
        }
    }
}

impl Drop for MacMeetingDetector {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Stops a watcher and reads how its thread ended. `true` when it ended cleanly. A thread that
/// died from a panic that escaped its guard is counted and, unless it already
/// reported `Lost`, reported `Lost` now, from this thread, before `stop` returns.
fn halt(watcher: Watcher, counters: &Counters) -> bool {
    // A send fails only if the thread already ended (it reported `Lost`, or it died); the join
    // below tells which.
    let _ = watcher.stop.send(());
    if watcher.thread.join().is_ok() {
        return true;
    }
    counters.watcher_panics.fetch_add(1, Ordering::Relaxed);
    let reason = "the meeting detector thread died (a bug); detection stopped".to_owned();
    report_lost(&watcher.sink, &watcher.state, counters, reason);
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::hal::fake::FakeHal;
    use objc2_core_audio::kAudioHardwareIllegalOperationError;

    const OWN: i32 = 4_242;

    fn scan_of(hal: &FakeHal, previous: &Scan) -> Scan {
        scan(hal, OWN, previous, &AtomicU64::new(0)).unwrap().scan
    }

    /// A detector on `hal` polling every `interval`, reporting `Lost` after `limit` failed polls.
    fn detector(hal: &Arc<FakeHal>, interval_ms: u64, limit: u32) -> MacMeetingDetector {
        MacMeetingDetector::with_hal(
            hal.clone(),
            Arc::new(name),
            Duration::from_millis(interval_ms),
            OWN,
            limit,
        )
    }

    fn recording() -> (EventSink<MeetingSignal>, mpsc::Receiver<MeetingSignal>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sink: EventSink<MeetingSignal> = Arc::new(move |s| {
            let _ = tx.lock().unwrap().send(s);
        });
        (sink, rx)
    }

    fn next(rx: &mpsc::Receiver<MeetingSignal>) -> MeetingSignal {
        rx.recv_timeout(Duration::from_secs(2)).expect("a signal")
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
                MeetingSignal::Lost { .. } => "lost".into(),
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
        let after = scan(&hal, OWN, &before, &errors).unwrap().scan;
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
        let detector = detector(&hal, 5, 1_000);
        let (sink, rx) = recording();
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
        let detector = detector(&hal, 5, 1_000);
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

    /// A bug in the scan must not end detection silently: the watcher reports `Lost` once, with
    /// the reason, and stops; nothing arrives after it.
    #[test]
    fn a_panicking_scan_reports_lost_once_and_stops() {
        let hal = Arc::new(FakeHal::with(&[(70, 1_300, "com.example.Call", true)]));
        let detector = detector(&hal, 5, 3);
        let (sink, rx) = recording();
        detector.start(sink).unwrap();
        assert_eq!(ids(&[next(&rx)]), ["+com.example.Call"]);

        hal.list_panics.store(true, Ordering::SeqCst);
        match next(&rx) {
            MeetingSignal::Lost { reason } => assert!(reason.contains("panicked"), "{reason}"),
            other => panic!("{other:?}"),
        }
        hal.list_panics.store(false, Ordering::SeqCst);
        hal.set(70, 1_300, "com.example.Call", false);
        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "nothing after Lost"
        );
        assert_eq!(detector.watcher_panics(), 1);
        assert!(detector.lost_reason().is_some());
        detector.stop();
    }

    /// An audio server that stops answering is `Lost` after the stated run of failed polls, once.
    #[test]
    fn persistent_read_failures_report_lost_once_and_stop() {
        let hal = Arc::new(FakeHal::with(&[(80, 1_400, "com.example.Call", true)]));
        let detector = detector(&hal, 5, 3);
        let (sink, rx) = recording();
        detector.start(sink).unwrap();
        assert_eq!(ids(&[next(&rx)]), ["+com.example.Call"]);

        *hal.list_fails.lock().unwrap() = Some(kAudioHardwareIllegalOperationError);
        match next(&rx) {
            MeetingSignal::Lost { reason } => assert!(reason.contains("3 polls"), "{reason}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(detector.read_errors(), 3);
        *hal.list_fails.lock().unwrap() = None;
        hal.set(80, 1_400, "com.example.Call", false);
        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "stopped"
        );
        detector.stop();
    }

    /// A failed poll short of the limit is counted, keeps the state, and is not `Lost`.
    #[test]
    fn a_transient_read_failure_is_counted_and_is_not_lost() {
        let hal = Arc::new(FakeHal::with(&[(90, 1_500, "com.example.Call", true)]));
        let detector = detector(&hal, 5, 3);
        let (sink, rx) = recording();
        detector.start(sink).unwrap();
        assert_eq!(ids(&[next(&rx)]), ["+com.example.Call"]);

        hal.fail_next_lists.store(2, Ordering::SeqCst); // one short of the limit
        let start = std::time::Instant::now();
        while detector.read_errors() < 2 && start.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(2));
        }
        hal.set(90, 1_500, "com.example.Call", false);
        assert_eq!(ids(&[next(&rx)]), ["-com.example.Call"], "still watching");
        assert_eq!(detector.read_errors(), 2);
        assert!(detector.lost_reason().is_none());
        detector.stop();
    }

    /// One unreadable process does not blind detection; every read failing does.
    #[test]
    fn a_poll_in_which_every_read_fails_is_a_failed_poll() {
        let hal = FakeHal::with(&[
            (1, 10, "com.example.A", true),
            (2, 11, "com.example.B", true),
        ]);
        hal.processes
            .lock()
            .unwrap()
            .get_mut(&1)
            .unwrap()
            .running_input = None;
        let one = scan(&hal, OWN, &Scan::new(), &AtomicU64::new(0)).unwrap();
        assert!(!one.blind());
        hal.processes
            .lock()
            .unwrap()
            .get_mut(&2)
            .unwrap()
            .running_input = None;
        let all = scan(&hal, OWN, &Scan::new(), &AtomicU64::new(0)).unwrap();
        assert!(all.blind());
        assert!(
            !scan(&FakeHal::default(), OWN, &Scan::new(), &AtomicU64::new(0))
                .unwrap()
                .blind(),
            "no processes is not blind"
        );
    }

    /// `halt` reads the join: a watcher thread that died without reporting is reported `Lost`
    /// while stopping, never swallowed.
    #[test]
    fn stopping_a_watcher_whose_thread_died_reports_lost() {
        let (sink, rx) = recording();
        let (stop, _stopped) = mpsc::channel();
        let watcher = Watcher {
            stop,
            thread: thread::spawn(|| panic!("escaped the guard")),
            sink,
            state: Arc::default(),
        };
        let counters = Counters::default();
        assert!(!halt(watcher, &counters));
        assert!(matches!(rx.try_recv(), Ok(MeetingSignal::Lost { .. })));
        assert_eq!(counters.watcher_panics.load(Ordering::Relaxed), 1);

        // A watcher that already reported `Lost` is not reported twice.
        let (sink, rx) = recording();
        let (stop, _stopped) = mpsc::channel();
        let state = Arc::new(WatchState::default());
        *state.lost.lock().unwrap() = Some("earlier".into());
        let watcher = Watcher {
            stop,
            thread: thread::spawn(|| panic!("escaped the guard")),
            sink,
            state,
        };
        assert!(!halt(watcher, &counters));
        assert!(rx.try_recv().is_err());
    }

    /// Needs no permission, only the audio server: whatever holds the mic here, Apple's daemons
    /// are filtered and this process is not reported.
    #[test]
    #[ignore = "talks to the local audio server"]
    fn the_real_server_scans_without_apple_daemons() {
        let now = scan(&SystemHal, own_pid(), &Scan::new(), &AtomicU64::new(0))
            .expect("scan")
            .scan;
        for holder in now.values() {
            assert!(!is_apple_daemon(&holder.bundle_id), "{holder:?}");
            assert_ne!(holder.pid, own_pid());
        }
        let set: std::collections::BTreeSet<&str> =
            now.values().map(|h| h.bundle_id.as_str()).collect();
        eprintln!("apps holding the mic now: {}", set.len());
    }
}
