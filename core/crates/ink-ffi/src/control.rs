//! Meetings, controlled: `meeting.start`, `meeting.stop`, `meeting.discard`, `meeting.dismiss`,
//! detection with each app's call policy ([`calls`](crate::calls)), and recovery after a crash, on
//! their own thread, `ink-meetings`.
//!
//! Apart from the command thread on purpose: that thread can be held for minutes by a model
//! download, and "Record this call" must start the recording now. Each of these is quick
//! (opening two devices, starting two threads); the final pass runs on the meeting's own worker,
//! recovery on its own thread.
//!
//! | Message | Does |
//! |---|---|
//! | `meeting.start {app?, title?}` | opens the mic and the far end ([`capture`](crate::capture)) and starts the meeting; `meeting.started` |
//! | `meeting.stop` | ends capture; the final pass follows (`meeting.stopped` ... `meeting.finished`) |
//! | `meeting.discard` | "Stop and delete", within [`DELETE_WINDOW`] of a start made here: capture ends, no final pass runs, and the record and its audio are deleted (`meeting.stopped`, `meeting.discarded`); refused after that (`delete_window_over`) |
//! | `meeting.dismiss {app}` | "Not this one": the offer goes, and that app is not offered again until it releases the mic |
//! | `meetings.recover` | finishes the meetings a crash interrupted ([`recovery`](crate::recovery)) |
//! | `meetings.calls.list` | `meetings.calls`: the default and every app seen or chosen for, with its policy |
//! | `meetings.calls.set {app, policy}` | the user's choice for one app (`always`, `ask`, `never`, or `default` to clear it), saved, applied at once ([`Detection::set_policies`]); `meetings.calls` |
//! | a platform signal | [`Detection`] decides: `meeting.detected` (the consent Drop), `meeting.detection_ended`, an Always app's meeting started as by `meeting.start` (`meeting.started` with `auto`), or the recorded app's meeting ends; apps seen past the hold join the list |
//! | the default (`meetings.calls.default`, or the old `meetings.detect`) | read again: detection listens while any app could be offered or recorded, `meeting.detection {listening}`; the first state is always said, off included, and a default that could not be read is off with a message |
//!
//! It sleeps until a message arrives, or until [`Detection::deadline_ns`] while something is
//! pending (an offer waiting out its hold, a recorded app's grace); idle, nothing ticks. The
//! platform's own detector polls the audio server once a second while detection is on.

use std::io;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ink_core::{AppRef, EventSink, MeetingDetector, MeetingSignal, RecordId};
use serde_json::Value;

use crate::calls::{CallPolicies, CallPolicy, Seen};
use crate::capture::{FarScope, MeetingCapture};
use crate::detection::{Action, DELETE_WINDOW, Detection};
use crate::events::{self, event};
use crate::meeting::{Ending, MeetingInfo};
use crate::runtime::{Runs, Shared, lock, start_meeting};

/// The switch "Offer to record calls" (`on`, the default, or `off`), which the default call policy
/// replaces: off is Never ([`crate::calls`], which migrates it and answers for it).
pub const DETECT_KEY: &str = "meetings.detect";

/// Why an Always app is offered rather than recorded when its own sound cannot be recorded alone
/// (the Mac's fallback to everything it plays, or Windows' device loopback): `meeting.detected`'s
/// message. A recording that takes in other apps' sound, and other people's, starts only on a tap.
pub const NOT_ALONE: &str =
    "Inkwell can only record everything this computer plays for this app, so it asks first";

/// Why [`State::open_and_start`] started nothing.
enum NotStarted {
    /// An Always app's far end would not be the app alone ([`NOT_ALONE`]): nothing was started,
    /// and capture, if it was opened to learn so, is closed again.
    NotAlone,
    /// The start failed: why (the platform's words, never audio).
    Failed(String),
}

impl From<String> for NotStarted {
    fn from(why: String) -> Self {
        Self::Failed(why)
    }
}

/// `signal` with its app as the core keeps it ([`kept`]), so detection, the policies and every
/// event compare and say it one way.
fn identified(signal: MeetingSignal) -> MeetingSignal {
    match signal {
        MeetingSignal::MicInUse { app } => MeetingSignal::MicInUse { app: kept(app) },
        MeetingSignal::MicReleased { app } => MeetingSignal::MicReleased { app: kept(app) },
        MeetingSignal::Lost { reason } => MeetingSignal::Lost { reason },
    }
}

/// What an app is called when neither its name nor its identity shows anything.
const NAMELESS: &str = "an app";

/// `app` as the core keeps it: its identity as [`crate::calls::identity`] says (lowercased on
/// Windows), its name as the shell may show it ([`crate::calls::clean_name`]: no invisible
/// characters, one line, cut to length), else its identity cleaned the same way, else
/// [`NAMELESS`]: the consent Drop never shows what the cleaning would drop.
fn kept(app: AppRef) -> AppRef {
    let id = crate::calls::identity(&app.id);
    let name = crate::calls::clean_name(&app.name)
        .or_else(|| crate::calls::clean_name(&id))
        .unwrap_or_else(|| NAMELESS.to_owned());
    AppRef { id, name, ..app }
}

/// The app `meeting.start` names: as detection offered it (its name for the shell), or as the
/// command names it, kept as a signal's app is.
fn start_app(app_id: &str, offered: Option<&AppRef>) -> AppRef {
    let id = crate::calls::identity(app_id);
    match offered {
        Some(offered) if offered.id == id => offered.clone(),
        _ => kept(AppRef {
            name: app_id.to_owned(),
            id,
            pid: None,
        }),
    }
}

/// A message to the meetings thread.
pub enum Msg {
    /// Bind after startup or a confirmed shortcut-setting change.
    ShortcutReload,
    /// Suspend all meeting key actions while the shell records a new shortcut.
    ShortcutSuspend {
        /// The command id, acknowledged after the hook stops or resumes.
        id: Option<String>,
        /// Whether the shell is capturing a shortcut.
        suspended: bool,
    },
    /// Answer a settings screen without resetting a held key.
    ShortcutState {
        /// The query id.
        id: Option<String>,
    },
    /// A global shortcut callback; its binding identity is checked on this thread.
    Shortcut(crate::meeting_keys::KeyEvent),
    /// `meeting.start`.
    Start {
        /// The command's id.
        id: Option<String>,
        /// The app to record, by id, when the start answers an offer.
        app: Option<String>,
        /// A title, when the shell knows one (a calendar event).
        title: Option<String>,
    },
    /// `meeting.stop`.
    Stop {
        /// The command's id.
        id: Option<String>,
    },
    /// `meeting.discard`: "Stop and delete".
    Discard {
        /// The command's id.
        id: Option<String>,
    },
    /// `meeting.dismiss`.
    Dismiss {
        /// The command's id.
        id: Option<String>,
        /// The app.
        app: String,
    },
    /// `meetings.recover`.
    Recover {
        /// The command's id.
        id: Option<String>,
    },
    /// The core started, or the default call policy changed: the policies are read again, and
    /// detection listens or not by them.
    Calls {
        /// Say `meetings.calls` once read (a change the Settings list shows).
        announce: bool,
    },
    /// `meetings.calls.list`.
    CallsList {
        /// The command's id.
        id: Option<String>,
    },
    /// `meetings.calls.set`.
    CallsSet {
        /// The command's id.
        id: Option<String>,
        /// The app, by identity (checked: [`crate::calls::check_app`]).
        app: String,
        /// Its policy; `None` follows the default.
        policy: Option<CallPolicy>,
        /// Start over a stored list that cannot be read (refused without it).
        replace_unreadable: bool,
    },
    /// Forget one app's choice and seen history, leaving recordings intact.
    CallsRemove {
        /// The command's id.
        id: Option<String>,
        /// The app identity validated by the command reader.
        app: String,
        /// The default explained in the shell's removal confirmation.
        expected_default: CallPolicy,
    },
    /// A platform signal, from its callback thread.
    Signal(MeetingSignal),
    /// A meeting's capture ended.
    CaptureEnded {
        /// Whether the user (or the shutdown) ended it, not its app.
        by_hand: bool,
    },
    /// Stop the thread.
    Quit,
}

/// The meetings thread.
pub struct Control {
    tx: Sender<Msg>,
    thread: JoinHandle<()>,
    recovery: Arc<Mutex<Recovery>>,
}

/// Recovery's thread and whether it is (or must go on) running, under one lock, so a
/// `meetings.recover` is never refused for a run that is ending: `running` clears under the lock
/// only once no ask is waiting, and an ask while it is set is one more round ([`State::recover`]).
#[derive(Default)]
struct Recovery {
    handle: Option<JoinHandle<()>>,
    /// A round is running, or about to.
    running: bool,
    /// A `meetings.recover` arrived during the round: run one more (asks coalesce into it).
    again: bool,
}

/// What the thread owns.
struct State {
    keys: crate::meeting_keys::MeetingKeys,
    shared: Arc<Shared>,
    runs: Arc<Mutex<Runs>>,
    capture: Arc<dyn MeetingCapture>,
    detector: Option<Arc<dyn MeetingDetector>>,
    detection: Detection,
    /// The call policies as saved; detection holds a copy.
    policies: CallPolicies,
    /// Why an Always app's start failed, said with the offer that follows it.
    auto_failure: Option<String>,
    /// Whether `policies` was read from the store; until then (or after a read that failed) a
    /// choice is refused, so it never overwrites a list the core could not read.
    policies_read: bool,
    listening: bool,
    /// Whether the shell has been told if detection listens: the first state is always said,
    /// off included, so the shell never guesses.
    announced: bool,
    /// Whether the current meeting is being ended by the user rather than its app.
    by_hand: bool,
    tx: Sender<Msg>,
    recovery: Arc<Mutex<Recovery>>,
}

impl Control {
    /// Starts `ink-meetings`.
    pub fn start(
        shared: Arc<Shared>,
        runs: Arc<Mutex<Runs>>,
        capture: Arc<dyn MeetingCapture>,
        detector: Option<Arc<dyn MeetingDetector>>,
        keys: Option<Arc<dyn ink_core::HotkeySource>>,
    ) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let recovery: Arc<Mutex<Recovery>> = Arc::default();
        let state = State {
            keys: crate::meeting_keys::MeetingKeys::new(keys),
            shared,
            runs,
            capture,
            detector,
            detection: Detection::new(),
            // Until the store is read (the first message): nothing offered or recorded.
            policies: CallPolicies::new(CallPolicy::Never),
            auto_failure: None,
            policies_read: false,
            listening: false,
            announced: false,
            by_hand: false,
            tx: tx.clone(),
            recovery: recovery.clone(),
        };
        let thread = thread::Builder::new()
            .name("ink-meetings".into())
            .spawn(move || state.run(&rx))?;
        Ok(Self {
            tx,
            thread,
            recovery,
        })
    }

    /// A sender for the thread's messages (the screens' settings reach it through one).
    pub(crate) fn sender(&self) -> Sender<Msg> {
        self.tx.clone()
    }

    /// Queues a message. Errors mean the thread has stopped.
    pub fn send(&self, msg: Msg) -> Result<(), String> {
        self.tx
            .send(msg)
            .map_err(|_| "the meetings thread has stopped".to_owned())
    }

    /// Stops detection and the thread, and waits for a recovery in progress (its final pass sees
    /// the shutdown's cancel and stops, leaving its marker for the next launch).
    pub fn stop(self) {
        let _ = self.tx.send(Msg::Quit);
        if self.thread.join().is_err() {
            log::error!("the meetings thread panicked outside its per-message boundary");
        }
        let handle = lock(&self.recovery).handle.take();
        if let Some(recovery) = handle
            && recovery.join().is_err()
        {
            log::error!("the recovery thread panicked");
        }
    }
}

impl State {
    fn run(mut self, rx: &Receiver<Msg>) {
        loop {
            let deadline = self
                .detection
                .deadline_ns()
                .map(|d| Duration::from_nanos(d.saturating_sub(self.shared.clock.now_ns())));
            let msg = match deadline {
                Some(wait) => match rx.recv_timeout(wait) {
                    Ok(msg) => Some(msg),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return self.quit(),
                },
                None => match rx.recv() {
                    Ok(msg) => Some(msg),
                    Err(_) => return self.quit(),
                },
            };
            match msg {
                Some(Msg::Quit) => return self.quit(),
                Some(msg) => {
                    let ran =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.handle(msg)));
                    if ran.is_err() {
                        // The payload is not logged: it could hold what was said (I5).
                        log::error!("a meetings message panicked; the next one still runs");
                    }
                }
                None => {
                    let now = self.shared.clock.now_ns();
                    let actions = self.detection.tick(now);
                    self.act(actions);
                }
            }
        }
    }

    fn quit(mut self) {
        self.keys.stop();
        self.listen(false, None);
    }

    fn failed(&self, command: &str, id: Option<&str>, message: &str) {
        self.failed_coded(command, id, message, None);
    }

    fn failed_coded(&self, command: &str, id: Option<&str>, message: &str, code: Option<&str>) {
        log::warn!("command {command} failed: {message}");
        self.shared
            .events
            .emit(events::command_failed_coded(command, id, message, code));
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::ShortcutReload => self.bind_shortcut(None),
            Msg::ShortcutState { id } => self.shared.events.emit(self.keys.state(id.as_deref())),
            Msg::ShortcutSuspend { id, suspended } => {
                self.keys.suspend(suspended);
                self.bind_shortcut(id.as_deref());
            }
            Msg::Shortcut(crate::meeting_keys::KeyEvent::Pressed(generation)) => {
                if self.keys.accepts(generation) {
                    self.toggle_shortcut();
                }
            }
            Msg::Shortcut(crate::meeting_keys::KeyEvent::Lost(generation)) => {
                if self.keys.lost(generation) {
                    self.shared.events.emit(self.keys.state(None));
                }
            }
            Msg::Start { id, app, title } => self.start(id.as_deref(), app, title),
            Msg::Stop { id } => self.stop(id.as_deref()),
            Msg::Discard { id } => self.discard(id.as_deref()),
            Msg::Dismiss { id, app } => {
                let app = crate::calls::identity(&app);
                let offered = self.detection.offered().is_some_and(|o| o.id == app);
                self.detection.dismiss(&app);
                if offered {
                    self.shared.events.emit(event(
                        "meeting.detection_ended",
                        &[("app", Some(app.into())), ("dismissed", Some(true.into()))],
                    ));
                } else {
                    self.failed(
                        "meeting.dismiss",
                        id.as_deref(),
                        "that app is not being offered",
                    );
                }
            }
            Msg::Recover { id } => self.recover(id.as_deref()),
            Msg::Calls { announce } => self.reload(announce),
            Msg::CallsList { id } => self
                .shared
                .events
                .emit(crate::calls::listing(&self.policies, id.as_deref())),
            Msg::CallsSet {
                id,
                app,
                policy,
                replace_unreadable,
            } => self.choose(id.as_deref(), &app, policy, replace_unreadable),
            Msg::CallsRemove {
                id,
                app,
                expected_default,
            } => self.remove_app(id.as_deref(), &app, expected_default),
            Msg::Signal(signal) => {
                let now = self.shared.clock.now_ns();
                let actions = self.detection.signal(identified(signal), now);
                self.act(actions);
            }
            Msg::CaptureEnded { by_hand } => {
                self.detection.ended(by_hand || self.by_hand);
                self.by_hand = false;
            }
            Msg::Quit => {}
        }
    }

    fn bind_shortcut(&mut self, reference: Option<&str>) {
        let tx = self.tx.clone();
        self.keys.bind(
            self.shared.store.as_ref(),
            Arc::new(move |event| {
                // A disconnected receiver means shutdown; no audio or model work on the hook thread.
                if tx.send(Msg::Shortcut(event)).is_err() {
                    log::debug!("meeting shortcut: meetings thread stopped");
                }
            }),
        );
        self.shared.events.emit(self.keys.state(reference));
    }

    fn toggle_shortcut(&mut self) {
        let (capturing, over) = lock(&self.runs)
            .meeting
            .as_ref()
            .map_or((false, true), |m| (m.is_capturing(), m.is_over()));
        if capturing {
            self.stop(None);
        } else if over {
            self.start(None, None, None);
        }
        // While capture is stopping or its final pass runs, another press starts nothing.
    }

    /// Runs what detection decided.
    fn act(&mut self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Offer(app) => {
                    log::info!("meeting detection: an app holds the microphone; offering it");
                    self.shared.events.emit(event(
                        "meeting.detected",
                        &[
                            ("app", Some(app.id.into())),
                            ("app_name", Some(app.name.into())),
                            ("message", self.auto_failure.take().map(Into::into)),
                        ],
                    ));
                }
                Action::Record(app) => self.record(app),
                Action::Withdraw(app) => self.shared.events.emit(event(
                    "meeting.detection_ended",
                    &[
                        ("app", Some(app.id.into())),
                        ("dismissed", Some(false.into())),
                    ],
                )),
                Action::Declined(app) => self.shared.events.emit(event(
                    "meeting.detection_ended",
                    &[
                        ("app", Some(app.id.into())),
                        ("dismissed", Some(true.into())),
                    ],
                )),
                Action::StopMeeting => {
                    log::info!("meeting detection: the recorded app released the microphone");
                    let ended = lock(&self.runs)
                        .meeting
                        .as_ref()
                        .is_some_and(|m| m.end() == Ending::Ended);
                    if !ended {
                        self.detection.ended(false);
                    }
                }
                Action::Lost(reason) => {
                    log::warn!("meeting detection stopped: {reason}");
                    self.listening = false;
                    if let Some(detector) = &self.detector {
                        // It stopped itself; this only joins its thread.
                        detector.stop();
                    }
                    self.shared.events.emit(event(
                        "meeting.detection",
                        &[
                            ("listening", Some(false.into())),
                            ("message", Some(reason.into())),
                        ],
                    ));
                }
            }
        }
        self.note_seen();
    }

    /// Adds the apps detection saw to the list and saves it; a new app or a new name is said
    /// (`meetings.calls`), so Settings lists it. A save that fails is logged: the list is kept in
    /// memory, and the next save writes it.
    fn note_seen(&mut self) {
        let seen = self.detection.take_seen();
        if seen.is_empty() {
            return;
        }
        let now = self.shared.clock.unix_ms();
        let mut changed = false;
        let mut save = false;
        for app in &seen {
            match self.policies.seen(app, now) {
                Seen::New | Seen::Renamed => {
                    changed = true;
                    save = true;
                }
                Seen::Again => save = true,
                Seen::NotKept => {}
            }
        }
        if save
            && let Err(e) = self
                .shared
                .store
                .set_setting(crate::calls::APPS_KEY, &self.policies.to_json())
        {
            log::warn!("call policies: the apps seen could not be saved: {e}");
        }
        if changed {
            self.shared
                .events
                .emit(crate::calls::listing(&self.policies, None));
        }
    }

    /// Reads the call policies again (at launch, and when the default changed), applies them, and
    /// listens while any app could be offered or recorded. A default that cannot be read leaves
    /// detection off, with why.
    fn reload(&mut self, announce: bool) {
        match crate::calls::load(self.shared.store.as_ref()) {
            Ok(policies) => {
                self.policies = policies;
                self.policies_read = true;
                let now = self.shared.clock.now_ns();
                let actions = self.detection.set_policies(self.policies.clone(), now);
                self.listen(self.policies.listens(), None);
                if self.listening {
                    self.act(actions);
                }
                if announce {
                    self.shared
                        .events
                        .emit(crate::calls::listing(&self.policies, None));
                }
            }
            Err(e) => {
                log::warn!("the detection setting could not be read ({e}); detection stays off");
                self.policies = CallPolicies::new(CallPolicy::Never);
                self.policies_read = false;
                let now = self.shared.clock.now_ns();
                let actions = self.detection.set_policies(self.policies.clone(), now);
                self.act(actions);
                self.listen(
                    false,
                    Some(format!("couldn't read the detection setting: {e}")),
                );
            }
        }
    }

    /// Save before applying, on the same worker that observes apps and changes their policies.
    /// A failed save leaves both the remembered row and the running detector untouched.
    fn remove_app(&mut self, id: Option<&str>, app: &str, expected_default: CallPolicy) {
        const NAME: &str = "meetings.calls.remove";
        if !self.policies_read {
            return self.failed(
                NAME,
                id,
                "the call policies could not be read; nothing was removed",
            );
        }
        // The shell may show an optimistic default whose save failed. Compare with the store
        // too: a confirmed Never must never remove an exception under a stored Always.
        let stored_default = match crate::calls::read_default(self.shared.store.as_ref()) {
            Ok(policy) => policy,
            Err(e) => return self.failed(NAME, id, &format!("couldn't read the default: {e}")),
        };
        if stored_default != expected_default || self.policies.default_policy() != expected_default
        {
            return self.failed(
                NAME,
                id,
                "the default changed; review the current default and remove the app again",
            );
        }
        let mut next = self.policies.clone();
        if let Err(e) = next.remove(app) {
            return self.failed(NAME, id, &e);
        }
        if let Err(e) = self
            .shared
            .store
            .set_settings(&[(crate::calls::APPS_KEY, &next.to_json())])
        {
            return self.failed(NAME, id, &format!("couldn't remove the app: {e}"));
        }
        self.policies = next;
        let now = self.shared.clock.now_ns();
        let actions = self.detection.set_policies(self.policies.clone(), now);
        self.listen(self.policies.listens(), None);
        if self.listening {
            self.act(actions);
        }
        self.shared
            .events
            .emit(crate::calls::listing(&self.policies, id));
    }

    /// `meetings.calls.set`: saved first, then applied; a save that fails changes nothing. Refused
    /// while the policies could not be read, and over a stored list set aside unless the command
    /// says to start it over (`list_unreadable`).
    fn choose(
        &mut self,
        id: Option<&str>,
        app: &str,
        policy: Option<CallPolicy>,
        replace_unreadable: bool,
    ) {
        const NAME: &str = "meetings.calls.set";
        if !self.policies_read {
            return self.failed(
                NAME,
                id,
                "the call policies could not be read; change the default to try again",
            );
        }
        if self.policies.unreadable().is_some() && !replace_unreadable {
            return self.failed_coded(
                NAME,
                id,
                "the apps' stored choices cannot be read: send it again with replace_unreadable to start the list over",
                Some("list_unreadable"),
            );
        }
        let mut next = self.policies.clone();
        let started_over = match next.choose(app, policy) {
            Ok(started_over) => started_over,
            Err(e) => return self.failed(NAME, id, &e),
        };
        // Starting over under Always lowers the default to Ask, decided from the store alone
        // (the default is the queries thread's: what this thread last read may be stale). Lowered
        // unless the store positively says Ask or Never; a read that fails lowers too. No
        // compare-and-set: a default the user sets between this read and the write below is lost
        // to Ask (a Never just set becomes Ask: offered, never recorded). Its reload follows.
        let stored = started_over.then(|| crate::calls::read_default(self.shared.store.as_ref()));
        let lowered = match stored {
            None => false,
            // What the store says stands, here at once (this thread's may be a stale Always).
            Some(Ok(stored_default @ (CallPolicy::Ask | CallPolicy::Never))) => {
                next.set_default(stored_default);
                false
            }
            Some(_) => {
                next.set_default(CallPolicy::Ask);
                true
            }
        };
        let list = next.to_json();
        // The list and, when starting over lowered it, the default: both or neither.
        let mut writes = vec![(crate::calls::APPS_KEY, list.as_str())];
        if lowered {
            writes.push((crate::calls::DEFAULT_KEY, CallPolicy::Ask.name()));
        }
        if let Err(e) = self.shared.store.set_settings(&writes) {
            return self.failed(NAME, id, &format!("couldn't save the choice: {e}"));
        }
        self.policies = next;
        let now = self.shared.clock.now_ns();
        let actions = self.detection.set_policies(self.policies.clone(), now);
        // Listening first: a choice that turns detection on (an Always app, over a default of
        // Never) starts the platform's detector before anything is decided.
        self.listen(self.policies.listens(), None);
        if self.listening {
            self.act(actions);
        }
        let mut answer = crate::calls::listing(&self.policies, id);
        if lowered {
            log::info!("call policies: started over under Always; the default is Ask now");
            answer["message"] = "the stored choices could not be read and were started over; the \
                default is Ask now (it was Always, or could not be read): set it to Always again \
                to record every app"
                .into();
            // For a screen that shows the default (the queries thread answers it the same way).
            self.shared.events.emit(event(
                "setting.value",
                &[
                    ("key", Some(crate::calls::DEFAULT_KEY.into())),
                    ("value", Some(CallPolicy::Ask.name().into())),
                ],
            ));
        }
        self.shared.events.emit(answer);
    }

    /// Starts or stops detection, and says so: every change, the first state (off included), and
    /// an off with a reason.
    fn listen(&mut self, on: bool, why_off: Option<String>) {
        let first = !std::mem::replace(&mut self.announced, true);
        let Some(detector) = self.detector.clone() else {
            let message = match on {
                true => Some("this platform cannot detect meetings yet".to_owned()),
                false => why_off,
            };
            if (on || first || message.is_some()) && !self.shared.shutdown.is_cancelled() {
                self.shared.events.emit(event(
                    "meeting.detection",
                    &[
                        ("listening", Some(false.into())),
                        ("message", message.map(Into::into)),
                    ],
                ));
            }
            return;
        };
        if on == self.listening && !first && why_off.is_none() {
            return;
        }
        let message: Option<Value> = if on == self.listening {
            // Nothing to start or stop: the state is said (the first time, or with its reason).
            why_off.map(Into::into)
        } else if on {
            let tx = Mutex::new(self.tx.clone());
            // Callback thread: it only enqueues.
            let sink: EventSink<MeetingSignal> = Arc::new(move |signal| {
                let _ = lock(&tx).send(Msg::Signal(signal));
            });
            match detector.start(sink) {
                Ok(()) => {
                    self.listening = true;
                    None
                }
                Err(e) => {
                    log::warn!("meeting detection did not start: {e}");
                    Some(e.to_string().into())
                }
            }
        } else {
            detector.stop();
            self.listening = false;
            self.detection.reset();
            why_off.map(Into::into)
        };
        if self.shared.shutdown.is_cancelled() {
            return;
        }
        self.shared.events.emit(event(
            "meeting.detection",
            &[
                ("listening", Some(self.listening.into())),
                ("message", message),
            ],
        ));
    }

    fn start(&mut self, id: Option<&str>, app: Option<String>, title: Option<String>) {
        let app = app.map(|app_id| start_app(&app_id, self.detection.offered()));
        match self.open_and_start(app, title, false) {
            Ok(()) => {}
            // Never for a start the user made: only a policy's start is refused for its scope.
            Err(NotStarted::NotAlone) => self.failed("meeting.start", id, NOT_ALONE),
            Err(NotStarted::Failed(e)) => self.failed("meeting.start", id, &e),
        }
    }

    /// [`Action::Record`]: an Always app's call, started exactly as `meeting.start` starts one
    /// (the same capture, record, events and end), with `auto` in `meeting.started`. A start that
    /// fails, or would record more than the app's own sound, is offered instead, with why.
    fn record(&mut self, app: AppRef) {
        log::info!("meeting detection: an app the user always records holds the microphone");
        if let Err(e) = self.open_and_start(Some(app.clone()), None, true) {
            self.auto_failure = Some(match e {
                NotStarted::NotAlone => {
                    log::info!(
                        "meeting detection: an Always app's sound cannot be recorded alone; asking"
                    );
                    NOT_ALONE.to_owned()
                }
                NotStarted::Failed(e) => {
                    log::warn!("meeting detection: the recording did not start by itself: {e}");
                    format!("couldn't start recording by itself: {e}")
                }
            });
            let actions = self.detection.start_failed(&app);
            self.act(actions);
            // Said with the offer only; none came (the app let go meanwhile).
            self.auto_failure = None;
        }
    }

    /// Opens capture and starts the meeting, for `app` or none; `auto` when its policy started
    /// it, which it does only when the far end is the app alone. Tells detection it started.
    fn open_and_start(
        &mut self,
        app: Option<AppRef>,
        title: Option<String>,
        auto: bool,
    ) -> Result<(), NotStarted> {
        if lock(&self.runs)
            .meeting
            .as_ref()
            .is_some_and(|m| !m.is_over())
        {
            return Err(NotStarted::Failed("a meeting is already running".into()));
        }
        // Decided before anything opens where the platform knows (Windows' plan: device loopback
        // for every app but Zoom and the browsers).
        if auto
            && let Some(app) = &app
            && self
                .capture
                .planned_far(app)
                .is_some_and(|far| far != FarScope::App)
        {
            return Err(NotStarted::NotAlone);
        }
        let choices = crate::devices::Choices::new(self.shared.store.clone());
        let opened = self.capture.open(app.as_ref(), &choices)?;
        // Otherwise known only once opened (the Mac's tap may find no process for the app; Zoom
        // or a browser on Windows may not be running), and decided before anything starts: the
        // sides are opened, not started, and dropping them closes them. No meeting.started is
        // said.
        if auto && opened.far != FarScope::App {
            drop(opened);
            return Err(NotStarted::NotAlone);
        }
        let mic = opened.mic.clone();
        let window_ms = i64::try_from(DELETE_WINDOW.as_millis()).unwrap_or(i64::MAX);
        let info = MeetingInfo {
            title,
            app: app.as_ref().map(|a| (a.id.clone(), a.name.clone())),
            routing: opened.routing,
            mic: opened.mic,
            far: opened.far,
            auto,
            delete_until_unix_ms: Some(self.shared.clock.unix_ms().saturating_add(window_ms)),
        };
        let tx = Mutex::new(self.tx.clone());
        let ended = Box::new(move || {
            let _ = lock(&tx).send(Msg::CaptureEnded { by_hand: false });
        });
        start_meeting(&self.shared, &self.runs, opened.sides, info, Some(ended))?;
        // After the start, and after `delete_until_unix_ms` was read: the core's window closes a
        // moment after the time the shell was given, never before it.
        let now = self.shared.clock.now_ns();
        self.by_hand = false;
        self.detection
            .started(app.as_ref().map(|a| a.id.as_str()), now);
        if let Some(mic) = &mic {
            // A stand-in for a chosen mic that is not connected, said once per spell.
            self.shared.sound.opened_on(&self.shared.events, mic);
        }
        Ok(())
    }

    /// `meeting.discard`: "Stop and delete", while [`Detection::deletable`] (the first
    /// [`DELETE_WINDOW`] of a meeting started here, however it started). Capture ends as by
    /// `meeting.stop`, and the meeting's worker then deletes it instead of running its final pass
    /// ([`crate::meeting::MeetingRun::discard`]).
    fn discard(&mut self, id: Option<&str>) {
        const NAME: &str = "meeting.discard";
        if !self.detection.recording() {
            return self.failed(NAME, id, "no meeting is being recorded");
        }
        if !self.detection.deletable(self.shared.clock.now_ns()) {
            return self.failed_coded(
                NAME,
                id,
                "the first minute is over: stop the meeting, then delete it from the library",
                Some("delete_window_over"),
            );
        }
        // The answer is what happens: deleted, or (its worker already finishing it, or over)
        // kept. Asked under the runs lock, which `discard` holds while it writes and syncs the
        // crash marker (a write, two syncs and a rename: milliseconds). No deadlock: the worker
        // takes the marker's lock but never the runs lock, so the two are only ever taken in
        // this order. The cost is a short stall for whatever waits on the runs lock meanwhile (a
        // start, a replay, recovery's look at the live record).
        let deleted = lock(&self.runs).meeting.as_ref().map(|m| m.discard());
        match deleted {
            Some(true) => {
                log::info!("meeting: stopped to be deleted, by the user");
                // However its capture ends now, its app is not recorded again this call.
                self.by_hand = true;
            }
            Some(false) => self.failed(
                NAME,
                id,
                "the meeting had already stopped, or failed: delete it from the library once it is there",
            ),
            None => self.failed(NAME, id, "no meeting is being recorded"),
        }
    }

    fn stop(&mut self, id: Option<&str>) {
        let ended = lock(&self.runs).meeting.as_ref().map(|m| m.end());
        match ended {
            Some(Ending::Ended) => self.by_hand = true,
            Some(Ending::AlreadyEnding) => {
                self.failed("meeting.stop", id, "the meeting is already stopping");
            }
            Some(Ending::NotCapturing) | None => {
                self.failed("meeting.stop", id, "no meeting is being recorded");
            }
        }
    }

    /// `meetings.recover`. One run at a time, on `ink-recovery`; one that arrives during a run is
    /// never refused: it makes the run go one more round (asks during a round coalesce), which
    /// finds what is still marked then. Each round ends with `meetings.recovered`.
    fn recover(&mut self, id: Option<&str>) {
        let mut state = lock(&self.recovery);
        if state.running {
            state.again = true;
            return;
        }
        // The last run has cleared `running` and is at most returning: wait it out.
        if let Some(done) = state.handle.take()
            && done.join().is_err()
        {
            log::error!("the recovery thread panicked");
        }
        state.running = true;
        let shared = self.shared.clone();
        let runs = self.runs.clone();
        let recovery = self.recovery.clone();
        let spawned = thread::Builder::new()
            .name("ink-recovery".into())
            .spawn(move || {
                loop {
                    // The meeting live in this process now, read at each round.
                    let live: Option<RecordId> = lock(&runs)
                        .meeting
                        .as_ref()
                        .and_then(|m| m.record().cloned());
                    recovery_round(&shared, live.as_ref());
                    let mut state = lock(&recovery);
                    if state.again && !shared.shutdown.is_cancelled() {
                        state.again = false;
                        continue;
                    }
                    state.again = false;
                    state.running = false;
                    break;
                }
            });
        match spawned {
            Ok(handle) => state.handle = Some(handle),
            Err(e) => {
                state.running = false;
                drop(state);
                self.failed(
                    "meetings.recover",
                    id,
                    &format!("the recovery thread did not start: {e}"),
                );
            }
        }
    }
}

/// **Worker** (`ink-recovery`). One round: finds every meeting a crash interrupted and recovers
/// each (never `live`, the meeting recording in this process), then `meetings.recovered`.
fn recovery_round(shared: &Arc<Shared>, live: Option<&RecordId>) {
    let found = match crate::recovery::interrupted(&shared.data_dir) {
        Ok(found) => found,
        Err(e) => {
            // Said once, here: a meeting a crash interrupted may be waiting, and the next launch
            // looks again.
            log::warn!("meeting recovery: the meetings could not be listed: {e}");
            shared.events.emit(event(
                "meetings.recovered",
                &[
                    ("meetings", Some(0.into())),
                    (
                        "message",
                        Some(format!("couldn't look for meetings a crash interrupted: {e}").into()),
                    ),
                ],
            ));
            return;
        }
    };
    let mut recovered = 0usize;
    for (dir, record) in found {
        if shared.shutdown.is_cancelled() {
            break;
        }
        // Never the meeting live in this process (its marker is its own).
        if live == Some(&record) {
            continue;
        }
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::recovery::recover(shared, &dir, &record, &shared.shutdown);
        }));
        if ran.is_err() {
            log::error!("meeting recovery panicked; the next meeting still runs");
        }
        recovered += 1;
    }
    shared.events.emit(event(
        "meetings.recovered",
        &[("meetings", Some(recovered.into()))],
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where an identity enters the core it is kept one way: lowercased on Windows, as Windows
    /// compares executables, and as given elsewhere; the name as given, but what the shell must
    /// never show.
    #[test]
    fn a_signals_identity_is_kept_as_the_core_compares_it() {
        let app = AppRef {
            id: "Zoom.EXE".into(),
            name: "Zoom Workplace".into(),
            pid: Some(7),
        };
        let MeetingSignal::MicInUse { app: held } = identified(MeetingSignal::MicInUse { app })
        else {
            panic!("the same signal");
        };
        let id = if cfg!(windows) {
            "zoom.exe"
        } else {
            "Zoom.EXE"
        };
        assert_eq!(held.id, id);
        assert_eq!(held.name, "Zoom Workplace");
        assert_eq!(held.pid, Some(7));
        let lost = MeetingSignal::Lost {
            reason: "gone".into(),
        };
        assert_eq!(identified(lost.clone()), lost);
        // A name the shell would show spoofed, or blank: cleaned, else the identity.
        let MeetingSignal::MicReleased { app: spoofed } = identified(MeetingSignal::MicReleased {
            app: AppRef {
                id: "chat.exe".into(),
                name: "Zo\u{202E}om\u{200B}\nMeetings\u{00A0}".into(),
                pid: None,
            },
        }) else {
            panic!("the same signal");
        };
        assert_eq!(spoofed.name, "ZoomMeetings");
        let blank = kept(AppRef {
            id: "chat.exe".into(),
            name: "\u{200B}\u{3164}".into(),
            pid: None,
        });
        assert_eq!(blank.name, "chat.exe");
        // A blank name and an identity that would spoof one: the identity cleaned, never raw.
        let hostile = kept(AppRef {
            id: "us.zoom.xos\u{202E}\u{200B}".into(),
            name: String::new(),
            pid: None,
        });
        assert_eq!(hostile.name, "us.zoom.xos");
        assert_eq!(
            hostile.id, "us.zoom.xos\u{202E}\u{200B}",
            "the identity as given"
        );
        let nothing = kept(AppRef {
            id: "\u{200B}".into(),
            name: "\u{3164}".into(),
            pid: None,
        });
        assert_eq!(nothing.name, NAMELESS);
    }

    /// `meeting.start`'s app, folded as a signal's is, so it finds the offer it answers (and
    /// takes its name) whatever case the shell sent; one never offered is named by its identity.
    #[test]
    fn a_started_app_is_found_by_its_identity_as_kept() {
        let offered = kept(AppRef {
            id: "Zoom.exe".into(),
            name: "Zoom".into(),
            pid: Some(3),
        });
        let started = start_app("ZOOM.EXE", Some(&offered));
        if cfg!(windows) {
            assert_eq!(started, offered, "the offer, by its identity in lowercase");
        } else {
            assert_eq!(started.id, "ZOOM.EXE", "bundle ids keep their case");
            assert_eq!(started.pid, None, "not the offer");
        }
        let unoffered = start_app("chat\u{200B}.exe", None);
        assert_eq!(unoffered.name, "chat.exe", "the name cleaned");
        assert_eq!(unoffered.pid, None);
    }
}
