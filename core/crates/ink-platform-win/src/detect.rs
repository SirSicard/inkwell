//! Meeting detection: which apps are recording from a microphone, from the audio session manager.
//!
//! **The signal** is an `Active` audio session on a capture endpoint (`IAudioSessionManager2`): the
//! process has a capture stream running. As on the Mac, that is what a call is, mechanically, so it
//! works for any meeting app or browser tab with no per-app integration.
//!
//! **Notifications plus a poll (decided).** A session-created notification
//! (`IAudioSessionNotification`) on each capture endpoint wakes the watcher at once when an app
//! opens the mic. It does not cover the common case of an app whose session already exists going
//! from inactive to active (a call app that joins a second call), so the watcher also polls every
//! [`POLL_INTERVAL`]. Nothing is drawn; this is detection's own thread.
//!
//! **Debounced.** An app is reported as taking or releasing the mic only once it has been seen
//! that way in [`DEBOUNCE_POLLS`] polls in a row, so a stream reopened on a device switch, or a
//! notification's poll catching a session mid-start, is not a meeting ending or starting.
//!
//! **Never counted:** this process, sessions without a process (the system's), and the apps in
//! [`DENYLIST`], which hold a microphone open for their own reasons all the time: the Copilot
//! wake word, Voice Access, Discord and NVIDIA Broadcast.
//!
//! **Never silently blind.** A panic in a poll, or [`LOST_AFTER_FAILED_POLLS`] failed polls in a
//! row, ends the watcher with one [`MeetingSignal::Lost`]. A read that fails for some sessions but
//! not all is counted and can add holders but never release one, so a read failure never ends a
//! meeting. Stopping a watcher checks how its thread ended, and reports `Lost` if it died
//! unreported.
//!
//! The core debounces again (the consent Drop waits 3 s) and applies its own rules; this reports
//! what Windows says, minus the above. `AppRef.id` is the executable name (`Zoom.exe`), and `pid`
//! the process that holds the mic, which may be a child of the app's main process.
#![cfg(windows)]

use std::collections::{BTreeMap, HashMap};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ink_core::{AppRef, EventSink, MeetingDetector, MeetingSignal, PlatformError};
use windows::Win32::Media::Audio::{
    IAudioSessionControl, IAudioSessionManager2, IAudioSessionNotification,
    IAudioSessionNotification_Impl, IMMDeviceEnumerator,
};
use windows::Win32::System::Threading::WaitForMultipleObjects;
use windows::core::{Ref, implement};

use crate::capture::devices::{self, Flow};
use crate::capture::stream::{Event, Woke, woke};
use crate::com::{ComScope, display_stem};
use crate::process::ProcessTable;
use crate::sessions::{self, SessionScan};

/// How often the watcher polls, besides waking on a new session.
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Polls in a row a change must show before it is reported.
pub const DEBOUNCE_POLLS: u32 = 2;

/// Failed polls in a row after which detection reports [`MeetingSignal::Lost`] and stops: ten
/// seconds of an audio service that does not answer, at [`POLL_INTERVAL`].
pub const LOST_AFTER_FAILED_POLLS: u32 = 10;

/// Executables that hold a microphone open by design, so their use is never a meeting: the Copilot
/// app (its wake word), Voice Access, Discord, and NVIDIA Broadcast (its app and its UI).
/// Case-insensitive. The executable names are to be confirmed on a machine that runs each
/// (`windows/S3.1-CHECKLIST.md`).
pub const DENYLIST: [&str; 5] = [
    "Copilot.exe",
    "VoiceAccess.exe",
    "Discord.exe",
    "NVIDIA Broadcast.exe",
    "NVIDIA Broadcast UI.exe",
];

/// Whether a process holding the mic may be reported: not this process, not the system's, and
/// not a denylisted app.
pub fn counts_as_candidate(exe: &str, pid: u32, own_pid: u32) -> bool {
    pid != 0 && pid != own_pid && !exe.is_empty() && !is_denylisted(exe)
}

/// One of [`DENYLIST`].
pub fn is_denylisted(exe: &str) -> bool {
    DENYLIST.iter().any(|d| d.eq_ignore_ascii_case(exe))
}

/// Apps holding the mic, by executable name (lower-cased key), each with one pid and the name as
/// Windows spells it.
pub(crate) type Holders = BTreeMap<String, (u32, String)>;

/// What one poll saw.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Poll {
    pub(crate) holders: Holders,
    /// Some reads failed: holders may be missing, so none may be released on this poll's word.
    pub(crate) partial: bool,
    /// Every read failed: the poll saw nothing it can trust.
    pub(crate) blind: bool,
}

/// The apps in a session scan that count.
pub(crate) fn holders(scan: &SessionScan, processes: &ProcessTable, own_pid: u32) -> Holders {
    let mut out = Holders::new();
    for session in scan.sessions.iter().filter(|s| s.active) {
        let Some(exe) = processes.exe(session.pid) else {
            continue; // exited since the scan
        };
        if counts_as_candidate(exe, session.pid, own_pid) {
            out.entry(exe.to_ascii_lowercase())
                .or_insert((session.pid, exe.to_owned()));
        }
    }
    out
}

/// Turns polls into debounced signals. Pure.
#[derive(Debug, Default)]
pub(crate) struct Debouncer {
    reported: Holders,
    /// Apps whose state differs from the reported one, with how many polls in a row it has.
    streak: BTreeMap<String, u32>,
}

impl Debouncer {
    /// The signals this poll completes: releases first, then new holders, each in name order.
    pub(crate) fn observe(&mut self, poll: &Poll) -> Vec<MeetingSignal> {
        let mut seen = poll.holders.clone();
        if poll.partial {
            // A partial read may miss a holder: it can add, never release.
            for (key, held) in &self.reported {
                seen.entry(key.clone()).or_insert_with(|| held.clone());
            }
        }
        let keys: Vec<String> = self.reported.keys().chain(seen.keys()).cloned().collect();
        let mut released = Vec::new();
        let mut started = Vec::new();
        for key in keys {
            if self.reported.contains_key(&key) == seen.contains_key(&key) {
                self.streak.remove(&key);
                continue;
            }
            let streak = self.streak.entry(key.clone()).or_insert(0);
            *streak += 1;
            if *streak < DEBOUNCE_POLLS {
                continue;
            }
            self.streak.remove(&key);
            if let Some(held) = seen.get(&key) {
                self.reported.insert(key.clone(), held.clone());
                started.push(held.clone());
            } else if let Some(held) = self.reported.remove(&key) {
                released.push(held);
            }
        }
        released.sort_by_key(|h| h.1.to_ascii_lowercase());
        started.sort_by_key(|h| h.1.to_ascii_lowercase());
        released.dedup();
        started.dedup();
        let app = |(pid, exe): (u32, String)| AppRef {
            name: display_stem(&exe),
            id: exe,
            pid: Some(pid),
        };
        released
            .into_iter()
            .map(|h| MeetingSignal::MicReleased { app: app(h) })
            .chain(
                started
                    .into_iter()
                    .map(|h| MeetingSignal::MicInUse { app: app(h) }),
            )
            .collect()
    }
}

/// Where polls come from: the audio service, or a script in the tests. Made and used on the
/// watcher thread (its COM objects stay there), so it need not be `Send`.
pub(crate) trait PollSource {
    /// One poll. An error is a failed poll (the endpoints could not be listed at all).
    fn poll(&mut self) -> Result<Poll, PlatformError>;
}

/// Wakes the watcher when a session is created on an endpoint it watches.
#[implement(IAudioSessionNotification)]
struct Wake(Arc<Event>);

impl IAudioSessionNotification_Impl for Wake_Impl {
    fn OnSessionCreated(&self, _session: Ref<IAudioSessionControl>) -> windows::core::Result<()> {
        self.0.set();
        Ok(())
    }
}

/// One watched capture endpoint: its session manager and our notification on it.
struct Watched {
    manager: IAudioSessionManager2,
    notification: Option<IAudioSessionNotification>,
}

impl Drop for Watched {
    fn drop(&mut self) {
        if let Some(notification) = self.notification.take() {
            // SAFETY: registered on this manager by this thread; never from inside the callback.
            let _ = unsafe { self.manager.UnregisterSessionNotification(&notification) };
        }
    }
}

/// [`PollSource`] on the audio service. Lives on the watcher thread (COM objects stay there).
struct SystemPolls {
    devices: IMMDeviceEnumerator,
    watched: HashMap<String, Watched>,
    wake: Arc<Event>,
    own_pid: u32,
    errors: Arc<AtomicU64>,
}

impl PollSource for SystemPolls {
    fn poll(&mut self) -> Result<Poll, PlatformError> {
        let endpoints = devices::endpoints(&self.devices, Flow::Capture)?;
        self.watched
            .retain(|id, _| endpoints.iter().any(|e| e.info.id.0 == *id));
        let mut scan = SessionScan::default();
        for endpoint in &endpoints {
            let id = &endpoint.info.id.0;
            if !self.watched.contains_key(id) {
                let Ok(watched) = self.watch(id) else {
                    scan.reads += 1;
                    scan.failed += 1;
                    continue;
                };
                self.watched.insert(id.clone(), watched);
            }
            match sessions::read(&self.watched[id].manager, id) {
                Ok(one) => {
                    scan.sessions.extend(one.sessions);
                    scan.reads += one.reads;
                    scan.failed += one.failed;
                }
                Err(_) => {
                    scan.reads += 1;
                    scan.failed += 1;
                }
            }
        }
        if scan.failed > 0 {
            self.errors.fetch_add(scan.failed as u64, Ordering::Relaxed);
        }
        let processes = if scan.sessions.iter().any(|s| s.active) {
            ProcessTable::snapshot()?
        } else {
            ProcessTable::default()
        };
        Ok(Poll {
            holders: holders(&scan, &processes, self.own_pid),
            partial: scan.failed > 0,
            blind: scan.blind(),
        })
    }
}

impl SystemPolls {
    fn watch(&self, id: &str) -> Result<Watched, PlatformError> {
        let device = devices::endpoint_by_id(&self.devices, id)?;
        let manager = sessions::manager(&device)?;
        // The session enumerator must exist before notifications are registered; `poll` reads it
        // right after, but it is asked for here first so the order is guaranteed.
        // SAFETY: a live manager.
        let _ = unsafe { manager.GetSessionEnumerator() };
        let notification: IAudioSessionNotification = Wake(Arc::clone(&self.wake)).into();
        // SAFETY: a live manager and COM object, in this thread's MTA.
        let registered = unsafe { manager.RegisterSessionNotification(&notification) }.is_ok();
        // Without the notification the poll still sees every session, a second later.
        Ok(Watched {
            manager,
            notification: registered.then_some(notification),
        })
    }
}

/// What a watcher and its detector share.
#[derive(Debug, Default)]
struct WatchState {
    lost: Mutex<Option<String>>,
}

/// The detector's counters, shared with its watchers.
#[derive(Debug, Default)]
struct Counters {
    errors: Arc<AtomicU64>,
    callback_panics: AtomicU64,
    watcher_panics: AtomicU64,
}

fn deliver(sink: &EventSink<MeetingSignal>, signal: MeetingSignal, counters: &Counters) {
    if catch_unwind(AssertUnwindSafe(|| sink(signal))).is_err() {
        counters.callback_panics.fetch_add(1, Ordering::Relaxed);
    }
}

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

/// How the watcher gets its polls: a factory run on the watcher thread (COM objects are made
/// there), and the event that wakes it early.
type MakePolls = Box<dyn FnOnce(Arc<Event>) -> Result<Box<dyn PollSource>, PlatformError> + Send>;

struct WatchConfig {
    make: MakePolls,
    interval: Duration,
    limit: u32,
}

/// The watcher thread: poll, report, wait (for the interval, a new session, or stop); until
/// stopped, or until it reports `Lost`.
fn watch(
    config: WatchConfig,
    sink: &EventSink<MeetingSignal>,
    state: &WatchState,
    counters: &Counters,
    stop: &Event,
) {
    let _com = match ComScope::enter() {
        Ok(com) => com,
        Err(e) => return report_lost(sink, state, counters, format!("meeting detection: {e}")),
    };
    let wake = match Event::new() {
        Ok(event) => Arc::new(event),
        Err(e) => return report_lost(sink, state, counters, format!("meeting detection: {e}")),
    };
    let mut polls = match (config.make)(Arc::clone(&wake)) {
        Ok(polls) => polls,
        Err(e) => return report_lost(sink, state, counters, format!("meeting detection: {e}")),
    };
    let mut debouncer = Debouncer::default();
    let mut failed_polls = 0u32;
    let timeout = u32::try_from(config.interval.as_millis()).unwrap_or(u32::MAX);
    let handles = [stop.handle(), wake.handle()];
    loop {
        let next = catch_unwind(AssertUnwindSafe(|| {
            match polls.poll() {
                Ok(poll) => {
                    if !poll.blind {
                        for signal in debouncer.observe(&poll) {
                            deliver(sink, signal, counters);
                        }
                    }
                    failed_polls = if poll.blind { failed_polls + 1 } else { 0 };
                }
                Err(_) => {
                    counters.errors.fetch_add(1, Ordering::Relaxed);
                    failed_polls += 1;
                }
            }
            (failed_polls >= config.limit).then(|| {
                format!(
                    "the audio service did not answer {} polls in a row; meeting detection \
                     stopped",
                    config.limit
                )
            })
        }));
        match next {
            Ok(None) => {}
            Ok(Some(reason)) => return report_lost(sink, state, counters, reason),
            Err(_) => {
                counters.watcher_panics.fetch_add(1, Ordering::Relaxed);
                let reason = "the meeting detector panicked (a bug); detection stopped".to_owned();
                return report_lost(sink, state, counters, reason);
            }
        }
        // SAFETY: two live event handles.
        match woke(unsafe { WaitForMultipleObjects(&handles, false, timeout) }) {
            Woke::Stop => return,
            Woke::Look => {}
            Woke::Failed => {
                let reason = "waiting for the next poll failed; meeting detection stopped".into();
                return report_lost(sink, state, counters, reason);
            }
        }
    }
}

struct Watcher {
    stop: Arc<Event>,
    thread: JoinHandle<()>,
    sink: EventSink<MeetingSignal>,
    state: Arc<WatchState>,
}

type PollFactory = Arc<dyn Fn() -> MakePolls + Send + Sync>;

/// [`MeetingDetector`] for Windows: a thread on the audio session manager.
pub struct WinMeetingDetector {
    factory: PollFactory,
    interval: Duration,
    lost_after: u32,
    watcher: Mutex<Option<Watcher>>,
    counters: Arc<Counters>,
}

impl WinMeetingDetector {
    /// A detector on the audio service, polling every [`POLL_INTERVAL`] and reporting `Lost` after
    /// [`LOST_AFTER_FAILED_POLLS`]. Nothing runs until [`start`](MeetingDetector::start). Needs no
    /// permission.
    pub fn new() -> Self {
        let counters = Arc::new(Counters::default());
        let errors = Arc::clone(&counters.errors);
        let factory: PollFactory = Arc::new(move || {
            let errors = Arc::clone(&errors);
            Box::new(move |wake: Arc<Event>| {
                Ok(Box::new(SystemPolls {
                    devices: devices::enumerator()?,
                    watched: HashMap::new(),
                    wake,
                    own_pid: std::process::id(),
                    errors,
                }) as Box<dyn PollSource>)
            })
        });
        Self::with_factory(factory, POLL_INTERVAL, LOST_AFTER_FAILED_POLLS, counters)
    }

    fn with_factory(
        factory: PollFactory,
        interval: Duration,
        lost_after: u32,
        counters: Arc<Counters>,
    ) -> Self {
        Self {
            factory,
            interval,
            lost_after: lost_after.max(1),
            watcher: Mutex::new(None),
            counters,
        }
    }

    /// Reads that failed since this detector was made. A count that keeps rising means detection
    /// is running partly blind; past [`LOST_AFTER_FAILED_POLLS`] blind polls it reports `Lost`.
    pub fn read_errors(&self) -> u64 {
        self.counters.errors.load(Ordering::Relaxed)
    }

    /// Panics caught from the core's callback. Each lost that signal; a bug to report.
    pub fn callback_panics(&self) -> u64 {
        self.counters.callback_panics.load(Ordering::Relaxed)
    }

    /// Panics in the watcher itself: each ended detection with `Lost`. A bug to report.
    pub fn watcher_panics(&self) -> u64 {
        self.counters.watcher_panics.load(Ordering::Relaxed)
    }

    /// Why the current watcher ended on its own, if it did.
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

impl Default for WinMeetingDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl MeetingDetector for WinMeetingDetector {
    /// Starts a fresh watcher, replacing (and joining) any running one. Apps already holding the
    /// mic are reported as `MicInUse` once the debounce has seen them (about a second), so
    /// launching during a call still catches the call.
    fn start(&self, on_signal: EventSink<MeetingSignal>) -> Result<(), PlatformError> {
        let mut slot = self.watcher.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(old) = slot.take() {
            halt(old, &self.counters);
        }
        let stop = Arc::new(Event::new()?);
        let state = Arc::new(WatchState::default());
        let config = WatchConfig {
            make: (self.factory)(),
            interval: self.interval,
            limit: self.lost_after,
        };
        let thread = thread::Builder::new()
            .name("ink-meeting-detector".into())
            .spawn({
                let (sink, state) = (on_signal.clone(), Arc::clone(&state));
                let (counters, stop) = (Arc::clone(&self.counters), Arc::clone(&stop));
                move || watch(config, &sink, &state, &counters, &stop)
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
    /// it from the callback itself.
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

impl Drop for WinMeetingDetector {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Stops a watcher and reads how its thread ended. A thread that died from a panic that escaped
/// its guard is counted and, unless it already reported `Lost`, reported `Lost` now.
fn halt(watcher: Watcher, counters: &Counters) {
    watcher.stop.set();
    if watcher.thread.join().is_ok() {
        return;
    }
    counters.watcher_panics.fetch_add(1, Ordering::Relaxed);
    let reason = "the meeting detector thread died (a bug); detection stopped".to_owned();
    report_lost(&watcher.sink, &watcher.state, counters, reason);
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::mpsc;

    use super::*;
    use crate::process::ProcessEntry;
    use crate::sessions::Session;

    const OWN: u32 = 4_242;

    fn poll(apps: &[(&str, u32)]) -> Poll {
        Poll {
            holders: apps
                .iter()
                .map(|&(exe, pid)| (exe.to_ascii_lowercase(), (pid, exe.to_owned())))
                .collect(),
            partial: false,
            blind: false,
        }
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
    fn the_denylist_and_this_process_never_count() {
        assert!(!counts_as_candidate("discord.exe", 10, OWN));
        assert!(!counts_as_candidate("VoiceAccess.exe", 10, OWN));
        assert!(!counts_as_candidate("NVIDIA Broadcast.exe", 10, OWN));
        assert!(!counts_as_candidate("Copilot.exe", 10, OWN));
        assert!(!counts_as_candidate("inkwell.exe", OWN, OWN));
        assert!(!counts_as_candidate("", 10, OWN));
        assert!(!counts_as_candidate("Zoom.exe", 0, OWN));
        assert!(counts_as_candidate("Zoom.exe", 10, OWN));
        assert!(counts_as_candidate("ms-teams.exe", 10, OWN));
    }

    #[test]
    fn holders_are_active_capture_sessions_by_executable() {
        let processes = ProcessTable::from_entries([
            ProcessEntry {
                pid: 10,
                parent: 1,
                exe: "Zoom.exe".into(),
            },
            ProcessEntry {
                pid: 11,
                parent: 10,
                exe: "zoom.exe".into(),
            },
            ProcessEntry {
                pid: 20,
                parent: 1,
                exe: "Discord.exe".into(),
            },
            ProcessEntry {
                pid: 30,
                parent: 1,
                exe: "chrome.exe".into(),
            },
        ]);
        let session = |pid, active| Session {
            pid,
            active,
            endpoint: "mic".into(),
        };
        let scan = SessionScan {
            sessions: vec![
                session(10, true),
                session(11, true),
                session(20, true),
                session(30, false),
                session(99, true), // exited
            ],
            reads: 5,
            failed: 0,
        };
        let held = holders(&scan, &processes, OWN);
        assert_eq!(held.len(), 1, "{held:?}");
        assert_eq!(held["zoom.exe"], (10, "Zoom.exe".into()));
    }

    #[test]
    fn a_change_is_reported_after_two_polls_and_once() {
        let mut d = Debouncer::default();
        assert!(
            d.observe(&poll(&[("Zoom.exe", 7)])).is_empty(),
            "one poll is a blip"
        );
        let started = d.observe(&poll(&[("Zoom.exe", 7)]));
        assert_eq!(ids(&started), ["+Zoom.exe"]);
        let MeetingSignal::MicInUse { app } = &started[0] else {
            unreachable!()
        };
        assert_eq!(app.name, "Zoom");
        assert_eq!(app.pid, Some(7));
        assert!(d.observe(&poll(&[("Zoom.exe", 7)])).is_empty(), "no repeat");
        assert!(d.observe(&poll(&[])).is_empty());
        assert_eq!(ids(&d.observe(&poll(&[]))), ["-Zoom.exe"]);
        assert!(d.observe(&poll(&[])).is_empty());
    }

    #[test]
    fn a_one_poll_gap_is_not_a_release() {
        let mut d = Debouncer::default();
        d.observe(&poll(&[("ms-teams.exe", 3)]));
        d.observe(&poll(&[("ms-teams.exe", 3)]));
        assert!(
            d.observe(&poll(&[])).is_empty(),
            "stream reopened on a device switch"
        );
        assert!(d.observe(&poll(&[("ms-teams.exe", 3)])).is_empty());
        assert!(d.observe(&poll(&[])).is_empty(), "the streak started over");
    }

    #[test]
    fn a_partial_poll_can_add_but_never_release() {
        let mut d = Debouncer::default();
        d.observe(&poll(&[("Zoom.exe", 1)]));
        d.observe(&poll(&[("Zoom.exe", 1)]));
        let mut partial = poll(&[("chrome.exe", 2)]);
        partial.partial = true;
        assert!(d.observe(&partial).is_empty());
        assert_eq!(
            ids(&d.observe(&partial)),
            ["+chrome.exe"],
            "Zoom not released"
        );
    }

    #[test]
    fn releases_come_before_new_holders() {
        let mut d = Debouncer::default();
        d.observe(&poll(&[("b.exe", 1)]));
        d.observe(&poll(&[("b.exe", 1)]));
        d.observe(&poll(&[("a.exe", 2)]));
        assert_eq!(
            ids(&d.observe(&poll(&[("a.exe", 2)]))),
            ["-b.exe", "+a.exe"]
        );
    }

    /// A scripted poll source for the watcher.
    struct Scripted {
        polls: VecDeque<Result<Poll, PlatformError>>,
        panic_when_empty: bool,
    }

    impl PollSource for Scripted {
        fn poll(&mut self) -> Result<Poll, PlatformError> {
            match self.polls.pop_front() {
                Some(poll) => poll,
                None if self.panic_when_empty => panic!("scripted panic"),
                None => Ok(Poll::default()),
            }
        }
    }

    fn detector(
        script: Vec<Result<Poll, PlatformError>>,
        panic_when_empty: bool,
    ) -> WinMeetingDetector {
        let script = Arc::new(Mutex::new(Some(script)));
        let factory: PollFactory = Arc::new(move || {
            let polls = script.lock().unwrap().take().unwrap_or_default();
            Box::new(move |_wake: Arc<Event>| {
                Ok(Box::new(Scripted {
                    polls: polls.into(),
                    panic_when_empty,
                }) as Box<dyn PollSource>)
            })
        });
        WinMeetingDetector::with_factory(
            factory,
            Duration::from_millis(5),
            3,
            Arc::new(Counters::default()),
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
        rx.recv_timeout(Duration::from_secs(5)).expect("a signal")
    }

    #[test]
    fn the_watcher_reports_changes_and_is_silent_after_stop() {
        let zoom = || Ok(poll(&[("Zoom.exe", 5)]));
        let d = detector(
            vec![zoom(), zoom(), zoom(), Ok(poll(&[])), Ok(poll(&[]))],
            false,
        );
        let (sink, rx) = recording();
        d.start(sink).unwrap();
        assert_eq!(ids(&[next(&rx)]), ["+Zoom.exe"]);
        assert_eq!(ids(&[next(&rx)]), ["-Zoom.exe"]);
        d.stop();
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
        assert_eq!(d.lost_reason(), None);
    }

    #[test]
    fn failed_polls_report_lost_once_and_stop() {
        let fail = || Err(PlatformError::Device("no service".into()));
        let d = detector(vec![fail(), fail(), fail(), fail()], false);
        let (sink, rx) = recording();
        d.start(sink).unwrap();
        assert!(matches!(next(&rx), MeetingSignal::Lost { reason } if reason.contains("3 polls")));
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err(), "once");
        assert!(d.read_errors() >= 3);
        assert!(d.lost_reason().is_some());
        d.stop();
    }

    #[test]
    fn blind_polls_count_as_failed_and_a_good_one_resets() {
        let blind = || {
            Ok(Poll {
                blind: true,
                ..Poll::default()
            })
        };
        let d = detector(
            vec![blind(), blind(), Ok(poll(&[])), blind(), blind()],
            false,
        );
        let (sink, rx) = recording();
        d.start(sink).unwrap();
        assert!(
            rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "never three in a row"
        );
        d.stop();
    }

    #[test]
    fn a_panicking_poll_reports_lost_once_and_stops() {
        let d = detector(vec![], true);
        let (sink, rx) = recording();
        d.start(sink).unwrap();
        assert!(matches!(next(&rx), MeetingSignal::Lost { reason } if reason.contains("panicked")));
        assert_eq!(d.watcher_panics(), 1);
        d.stop();
    }

    #[test]
    fn a_panicking_callback_is_counted_and_the_watcher_goes_on() {
        let zoom = || Ok(poll(&[("Zoom.exe", 5)]));
        let d = detector(vec![zoom(), zoom(), Ok(poll(&[])), Ok(poll(&[]))], false);
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sink: EventSink<MeetingSignal> = Arc::new(move |s| {
            if matches!(s, MeetingSignal::MicInUse { .. }) {
                panic!("core bug");
            }
            let _ = tx.lock().unwrap().send(s);
        });
        d.start(sink).unwrap();
        assert!(matches!(next(&rx), MeetingSignal::MicReleased { .. }));
        assert_eq!(d.callback_panics(), 1);
        d.stop();
    }

    /// Talks to the audio service; run on the PC with `-- --ignored`. It watches for two seconds
    /// and must not go blind.
    #[test]
    #[ignore = "talks to the Windows audio service"]
    fn the_real_service_is_watched_without_losing_it() {
        let d = WinMeetingDetector::new();
        let (sink, rx) = recording();
        d.start(sink).unwrap();
        std::thread::sleep(Duration::from_secs(2));
        d.stop();
        let signals: Vec<_> = rx.try_iter().collect();
        assert!(
            !signals
                .iter()
                .any(|s| matches!(s, MeetingSignal::Lost { .. })),
            "{signals:?}"
        );
        assert_eq!(d.read_errors(), 0);
    }
}
