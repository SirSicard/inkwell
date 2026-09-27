//! Meeting detection, decided: which app that holds the microphone the shell should offer to
//! record, and when a recorded call has ended.
//!
//! The platform reports what the OS says (`MeetingSignal`: an app took or released the mic,
//! minus the OS's own daemons and this app); this turns it into the consent Drop's two moments
//! and the meeting's end:
//!
//! | Rule | Why |
//! |---|---|
//! | An app must hold the mic for [`DETECT_HOLD`] before it is offered | a voice message or a mic check in a browser is not a meeting (Blotter's open question) |
//! | One offer at a time, the app that took the mic first | the Drop has one line to say it in |
//! | "Not this one" dismisses the app until it releases the mic | the next call is a new question; this one was answered |
//! | Stopping a recording by hand dismisses its app the same way | or the Drop would ask again at once, mid-call |
//! | A recorded app that releases the mic ends the meeting after [`STOP_GRACE`], unless it takes it back | a mute that closes the stream, or a device switch, must not end the meeting |
//! | A meeting started without an app ("Record now") never ends by itself | there is nothing to watch |
//!
//! Pure and deterministic: the caller passes the time (host ns) and runs the actions. The owner
//! wakes at [`Detection::deadline_ns`] only while something is pending; idle, nothing ticks.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use ink_core::{AppRef, MeetingSignal};

/// How long an app must hold the mic before the Drop offers to record it: 3 s.
pub const DETECT_HOLD: Duration = Duration::from_secs(3);

/// How long a recorded app may be without the mic before its meeting ends: 15 s.
pub const STOP_GRACE: Duration = Duration::from_secs(15);

/// What the owner should do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Offer to record `app` (the consent Drop).
    Offer(AppRef),
    /// The offered app released the mic before the user chose: take the offer back.
    Withdraw(AppRef),
    /// The recorded app has been without the mic for [`STOP_GRACE`]: end the meeting.
    StopMeeting,
    /// Detection stopped on its own (the platform's `Lost`); what it says, never content.
    Lost(String),
}

#[derive(Clone, Debug)]
struct Held {
    app: AppRef,
    since_ns: u64,
}

/// The meeting being recorded, as detection sees it.
#[derive(Clone, Debug)]
struct Recording {
    /// The app it records, when it was started for one.
    app: Option<String>,
    /// When that app released the mic, while it is without it.
    released_ns: Option<u64>,
}

/// See the module docs.
#[derive(Debug, Default)]
pub struct Detection {
    /// Apps holding the mic now, by id, in the order they took it (ties by id).
    held: BTreeMap<String, Held>,
    /// Apps the user said "not this one" to, until they release the mic.
    dismissed: BTreeSet<String>,
    /// The app offered now.
    offered: Option<AppRef>,
    recording: Option<Recording>,
}

fn ns(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

impl Detection {
    /// Nothing held, nothing recording.
    pub fn new() -> Self {
        Self::default()
    }

    /// The app offered now, if any.
    pub fn offered(&self) -> Option<&AppRef> {
        self.offered.as_ref()
    }

    /// Whether a meeting is being recorded.
    pub fn recording(&self) -> bool {
        self.recording.is_some()
    }

    /// A platform signal at `now_ns`.
    pub fn signal(&mut self, signal: MeetingSignal, now_ns: u64) -> Vec<Action> {
        let mut actions = Vec::new();
        match signal {
            MeetingSignal::MicInUse { app } => {
                if let Some(r) = &mut self.recording
                    && r.app.as_deref() == Some(app.id.as_str())
                {
                    // Back within the grace: the meeting goes on.
                    r.released_ns = None;
                }
                self.held.entry(app.id.clone()).or_insert(Held {
                    app,
                    since_ns: now_ns,
                });
            }
            MeetingSignal::MicReleased { app } => {
                self.held.remove(&app.id);
                self.dismissed.remove(&app.id);
                if self.offered.as_ref().is_some_and(|o| o.id == app.id) {
                    self.offered = None;
                    actions.push(Action::Withdraw(app.clone()));
                }
                if let Some(r) = &mut self.recording
                    && r.app.as_deref() == Some(app.id.as_str())
                    && r.released_ns.is_none()
                {
                    r.released_ns = Some(now_ns);
                }
            }
            MeetingSignal::Lost { reason } => {
                // Which apps hold the mic is unknown from here: nothing is offered on a guess,
                // and a recorded meeting is left for the user to stop.
                self.held.clear();
                self.dismissed.clear();
                if let Some(app) = self.offered.take() {
                    actions.push(Action::Withdraw(app));
                }
                if let Some(r) = &mut self.recording {
                    r.released_ns = None;
                }
                actions.push(Action::Lost(reason));
                return actions;
            }
        }
        actions.extend(self.tick(now_ns));
        actions
    }

    /// Judges what is pending at `now_ns`: an offer whose hold has passed, a recorded app whose
    /// grace has run out.
    pub fn tick(&mut self, now_ns: u64) -> Vec<Action> {
        let mut actions = Vec::new();
        if let Some(r) = &mut self.recording
            && let Some(released) = r.released_ns
            && now_ns >= released.saturating_add(ns(STOP_GRACE))
        {
            r.released_ns = None;
            actions.push(Action::StopMeeting);
        }
        if self.recording.is_none()
            && self.offered.is_none()
            && let Some(held) = self.next_offer()
            && now_ns >= held.since_ns.saturating_add(ns(DETECT_HOLD))
        {
            let app = held.app.clone();
            self.offered = Some(app.clone());
            actions.push(Action::Offer(app));
        }
        actions
    }

    /// The first app, by when it took the mic, that could be offered.
    fn next_offer(&self) -> Option<&Held> {
        self.held
            .values()
            .filter(|h| !self.dismissed.contains(&h.app.id))
            .min_by_key(|h| h.since_ns)
    }

    /// When [`tick`](Self::tick) has something to judge next, if anything is pending.
    pub fn deadline_ns(&self) -> Option<u64> {
        let stop = self
            .recording
            .as_ref()
            .and_then(|r| r.released_ns)
            .map(|t| t.saturating_add(ns(STOP_GRACE)));
        let offer = (self.recording.is_none() && self.offered.is_none())
            .then(|| self.next_offer())
            .flatten()
            .map(|h| h.since_ns.saturating_add(ns(DETECT_HOLD)));
        match (stop, offer) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// The user said "not this one" to `app`: it is not offered again until it releases the mic.
    pub fn dismiss(&mut self, app: &str) {
        self.dismissed.insert(app.to_owned());
        if self.offered.as_ref().is_some_and(|o| o.id == app) {
            self.offered = None;
        }
    }

    /// A meeting started, for `app` (the offer taken) or none ("Record now"). An app that does not
    /// hold the mic now starts its grace at once: a meeting for an app that never takes the mic
    /// ends by itself.
    pub fn started(&mut self, app: Option<&str>, now_ns: u64) {
        self.offered = None;
        let released_ns = app
            .filter(|id| !self.held.contains_key(*id))
            .map(|_| now_ns);
        self.recording = Some(Recording {
            app: app.map(str::to_owned),
            released_ns,
        });
    }

    /// The meeting's capture ended. `by_hand`: the user stopped it while its app still held the
    /// mic, so that app is dismissed until it releases the mic (the Drop does not ask again
    /// mid-call).
    pub fn ended(&mut self, by_hand: bool) {
        if let Some(r) = self.recording.take()
            && by_hand
            && let Some(app) = r.app
            && self.held.contains_key(&app)
        {
            self.dismissed.insert(app);
        }
    }

    /// Detection stopped (turned off, or the platform lost it): nothing is held or offered.
    pub fn reset(&mut self) {
        self.held.clear();
        self.dismissed.clear();
        self.offered = None;
        if let Some(r) = &mut self.recording {
            r.released_ns = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u64 = 1_000_000_000;

    fn app(id: &str) -> AppRef {
        AppRef {
            id: id.into(),
            pid: None,
            name: id.to_uppercase(),
        }
    }

    fn uses(id: &str) -> MeetingSignal {
        MeetingSignal::MicInUse { app: app(id) }
    }

    fn releases(id: &str) -> MeetingSignal {
        MeetingSignal::MicReleased { app: app(id) }
    }

    #[test]
    fn an_app_is_offered_once_it_has_held_the_mic_for_the_hold() {
        let mut d = Detection::new();
        assert!(d.signal(uses("zoom"), 10 * S).is_empty());
        assert_eq!(d.deadline_ns(), Some(13 * S));
        assert!(
            d.tick(12 * S).is_empty(),
            "a voice message is not a meeting"
        );
        assert_eq!(d.tick(13 * S), vec![Action::Offer(app("zoom"))]);
        assert_eq!(
            d.deadline_ns(),
            None,
            "nothing pending while an offer shows"
        );
        assert!(d.tick(20 * S).is_empty(), "offered once");
    }

    #[test]
    fn a_short_hold_is_never_offered_and_a_release_withdraws_an_offer() {
        let mut d = Detection::new();
        d.signal(uses("browser"), 0);
        assert!(d.signal(releases("browser"), 2 * S).is_empty());
        assert_eq!(d.deadline_ns(), None, "idle: nothing to wake for");
        d.signal(uses("zoom"), 10 * S);
        assert_eq!(d.tick(13 * S), vec![Action::Offer(app("zoom"))]);
        assert_eq!(
            d.signal(releases("zoom"), 14 * S),
            vec![Action::Withdraw(app("zoom"))]
        );
        assert_eq!(d.offered(), None);
    }

    #[test]
    fn not_this_one_lasts_until_the_app_releases_the_mic() {
        let mut d = Detection::new();
        d.signal(uses("zoom"), 0);
        d.tick(3 * S);
        d.dismiss("zoom");
        assert!(d.tick(60 * S).is_empty(), "not asked again mid-call");
        assert_eq!(d.deadline_ns(), None);
        d.signal(releases("zoom"), 70 * S);
        d.signal(uses("zoom"), 80 * S);
        assert_eq!(
            d.tick(83 * S),
            vec![Action::Offer(app("zoom"))],
            "a new call"
        );
    }

    #[test]
    fn with_two_apps_the_first_is_offered_and_the_other_after_it_is_answered() {
        let mut d = Detection::new();
        d.signal(uses("meet"), 0);
        d.signal(uses("zoom"), S);
        assert_eq!(d.tick(4 * S), vec![Action::Offer(app("meet"))]);
        d.dismiss("meet");
        assert_eq!(d.tick(4 * S), vec![Action::Offer(app("zoom"))]);
    }

    #[test]
    fn a_recorded_app_ends_its_meeting_after_the_grace_unless_it_comes_back() {
        let mut d = Detection::new();
        d.signal(uses("zoom"), 0);
        d.tick(3 * S);
        d.started(Some("zoom"), 4 * S);
        assert!(d.recording() && d.offered().is_none());
        assert!(d.signal(releases("zoom"), 100 * S).is_empty());
        assert_eq!(d.deadline_ns(), Some(115 * S));
        // A mute that closed the stream: back within the grace.
        d.signal(uses("zoom"), 105 * S);
        assert!(d.tick(200 * S).is_empty());
        d.signal(releases("zoom"), 300 * S);
        assert!(d.tick(314 * S).is_empty());
        assert_eq!(d.tick(315 * S), vec![Action::StopMeeting]);
        assert!(d.tick(400 * S).is_empty(), "said once");
        d.ended(false);
        assert!(!d.recording());
    }

    #[test]
    fn record_now_never_ends_by_itself_and_offers_nothing_while_it_records() {
        let mut d = Detection::new();
        d.started(None, 0);
        d.signal(uses("zoom"), S);
        assert!(d.tick(100 * S).is_empty(), "no offer while recording");
        d.signal(releases("zoom"), 200 * S);
        assert!(d.tick(1_000 * S).is_empty(), "nothing to watch");
    }

    #[test]
    fn stopping_by_hand_mid_call_does_not_ask_again_until_the_call_ends() {
        let mut d = Detection::new();
        d.signal(uses("zoom"), 0);
        d.tick(3 * S);
        d.started(Some("zoom"), 4 * S);
        d.ended(true);
        assert!(d.tick(60 * S).is_empty(), "the call goes on, unrecorded");
        d.signal(releases("zoom"), 70 * S);
        d.signal(uses("zoom"), 80 * S);
        assert_eq!(d.tick(83 * S), vec![Action::Offer(app("zoom"))]);
    }

    #[test]
    fn a_meeting_for_an_app_that_never_takes_the_mic_ends_by_itself() {
        let mut d = Detection::new();
        d.started(Some("zoom"), 10 * S);
        assert_eq!(d.deadline_ns(), Some(25 * S));
        assert_eq!(d.tick(25 * S), vec![Action::StopMeeting]);
    }

    #[test]
    fn lost_detection_withdraws_the_offer_and_leaves_a_meeting_to_the_user() {
        let mut d = Detection::new();
        d.signal(uses("zoom"), 0);
        d.tick(3 * S);
        let actions = d.signal(
            MeetingSignal::Lost {
                reason: "the audio server stopped answering".into(),
            },
            5 * S,
        );
        assert_eq!(
            actions,
            vec![
                Action::Withdraw(app("zoom")),
                Action::Lost("the audio server stopped answering".into())
            ]
        );
        let mut d = Detection::new();
        d.signal(uses("zoom"), 0);
        d.started(Some("zoom"), S);
        d.signal(releases("zoom"), 2 * S);
        d.signal(MeetingSignal::Lost { reason: "x".into() }, 3 * S);
        assert_eq!(d.deadline_ns(), None, "no end on a guess");
        assert!(d.recording());
    }
}
