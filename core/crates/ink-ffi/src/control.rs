//! Meetings, controlled: `meeting.start`, `meeting.stop`, `meeting.dismiss`, detection, and
//! recovery after a crash, on their own thread, `ink-meetings`.
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
//! | `meeting.dismiss {app}` | "Not this one": the offer goes, and that app is not offered again until it releases the mic |
//! | `meetings.recover` | finishes the meetings a crash interrupted ([`recovery`](crate::recovery)) |
//! | a platform signal | [`Detection`] decides: `meeting.detected` (the consent Drop), `meeting.detection_ended`, or the recorded app's meeting ends |
//! | the `meetings.detect` setting | starts or stops detection: `meeting.detection {listening}`; the first state is always said, off included, and a setting that could not be read is off with a message |
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

use crate::capture::MeetingCapture;
use crate::detection::{Action, Detection};
use crate::events::{self, event};
use crate::meeting::{Ending, MeetingInfo};
use crate::runtime::{Runs, Shared, lock, start_meeting};

/// The setting that turns detection on or off (`on`, the default, or `off`).
pub const DETECT_KEY: &str = "meetings.detect";

/// The setting: record the Bluetooth headset's own mic, not the built-in one (`on` or `off`, the
/// default).
pub const HEADSET_MIC_KEY: &str = "meetings.headset_mic";

/// A message to the meetings thread.
pub enum Msg {
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
    /// The detection setting changed (or the core started): on or off. `why_off`: why it is off
    /// against the user's wish (the setting could not be read), said with the state.
    Detect {
        /// Listen, or not.
        on: bool,
        /// Why detection is off when the user did not turn it off.
        why_off: Option<String>,
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
    recovery: Arc<Mutex<Option<JoinHandle<()>>>>,
}

/// What the thread owns.
struct State {
    shared: Arc<Shared>,
    runs: Arc<Mutex<Runs>>,
    capture: Arc<dyn MeetingCapture>,
    detector: Option<Arc<dyn MeetingDetector>>,
    detection: Detection,
    listening: bool,
    /// Whether the shell has been told if detection listens: the first state is always said,
    /// off included, so the shell never guesses.
    announced: bool,
    /// Whether the current meeting is being ended by the user rather than its app.
    by_hand: bool,
    tx: Sender<Msg>,
    recovery: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl Control {
    /// Starts `ink-meetings`.
    pub fn start(
        shared: Arc<Shared>,
        runs: Arc<Mutex<Runs>>,
        capture: Arc<dyn MeetingCapture>,
        detector: Option<Arc<dyn MeetingDetector>>,
    ) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let recovery: Arc<Mutex<Option<JoinHandle<()>>>> = Arc::default();
        let state = State {
            shared,
            runs,
            capture,
            detector,
            detection: Detection::new(),
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
        if let Some(recovery) = lock(&self.recovery).take()
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
        self.listen(false, None);
    }

    fn failed(&self, command: &str, id: Option<&str>, message: &str) {
        log::warn!("command {command} failed: {message}");
        self.shared
            .events
            .emit(events::command_failed(command, id, message));
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Start { id, app, title } => self.start(id.as_deref(), app, title),
            Msg::Stop { id } => self.stop(id.as_deref()),
            Msg::Dismiss { id, app } => {
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
            Msg::Detect { on, why_off } => self.listen(on, why_off),
            Msg::Signal(signal) => {
                let now = self.shared.clock.now_ns();
                let actions = self.detection.signal(signal, now);
                self.act(actions);
            }
            Msg::CaptureEnded { by_hand } => {
                self.detection.ended(by_hand || self.by_hand);
                self.by_hand = false;
            }
            Msg::Quit => {}
        }
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
                        ],
                    ));
                }
                Action::Withdraw(app) => self.shared.events.emit(event(
                    "meeting.detection_ended",
                    &[
                        ("app", Some(app.id.into())),
                        ("dismissed", Some(false.into())),
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
        const NAME: &str = "meeting.start";
        if lock(&self.runs)
            .meeting
            .as_ref()
            .is_some_and(|m| !m.is_over())
        {
            return self.failed(NAME, id, "a meeting is already running");
        }
        // The app as detection knows it (its name for the shell), or as the command names it.
        let app: Option<AppRef> = app.map(|app_id| match self.detection.offered() {
            Some(offered) if offered.id == app_id => offered.clone(),
            _ => AppRef {
                name: app_id.clone(),
                id: app_id,
                pid: None,
            },
        });
        let headset_mic = self
            .shared
            .store
            .setting(HEADSET_MIC_KEY)
            .ok()
            .flatten()
            .as_deref()
            == Some("on");
        let opened = match self.capture.open(app.as_ref(), headset_mic) {
            Ok(opened) => opened,
            Err(e) => return self.failed(NAME, id, &e),
        };
        let info = MeetingInfo {
            title,
            app: app.as_ref().map(|a| (a.id.clone(), a.name.clone())),
            routing: opened.routing,
            mic: opened.mic,
            far: opened.far,
        };
        let tx = Mutex::new(self.tx.clone());
        let ended = Box::new(move || {
            let _ = lock(&tx).send(Msg::CaptureEnded { by_hand: false });
        });
        match start_meeting(&self.shared, &self.runs, opened.sides, info, Some(ended)) {
            Ok(()) => {
                self.by_hand = false;
                let now = self.shared.clock.now_ns();
                self.detection
                    .started(app.as_ref().map(|a| a.id.as_str()), now);
            }
            Err(e) => self.failed(NAME, id, &e),
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

    fn recover(&mut self, id: Option<&str>) {
        let mut slot = lock(&self.recovery);
        if slot.as_ref().is_some_and(|h| !h.is_finished()) {
            drop(slot);
            return self.failed("meetings.recover", id, "recovery is already running");
        }
        if let Some(done) = slot.take() {
            let _ = done.join();
        }
        let shared = self.shared.clone();
        let live: Option<RecordId> = lock(&self.runs)
            .meeting
            .as_ref()
            .and_then(|m| m.record().cloned());
        let spawned = thread::Builder::new()
            .name("ink-recovery".into())
            .spawn(move || {
                let found = match crate::recovery::interrupted(&shared.data_dir) {
                    Ok(found) => found,
                    Err(e) => {
                        // Said once, here: a meeting a crash interrupted may be waiting, and
                        // the next launch looks again.
                        log::warn!("meeting recovery: the meetings could not be listed: {e}");
                        shared.events.emit(event(
                            "meetings.recovered",
                            &[
                                ("meetings", Some(0.into())),
                                (
                                    "message",
                                    Some(
                                        format!(
                                            "couldn't look for meetings a crash interrupted: {e}"
                                        )
                                        .into(),
                                    ),
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
                    if live.as_ref() == Some(&record) {
                        continue;
                    }
                    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        crate::recovery::recover(&shared, &dir, &record, &shared.shutdown);
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
            });
        match spawned {
            Ok(handle) => *slot = Some(handle),
            Err(e) => {
                drop(slot);
                self.failed(
                    "meetings.recover",
                    id,
                    &format!("the recovery thread did not start: {e}"),
                );
            }
        }
    }
}
