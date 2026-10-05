//! Meeting detection, decided: which app that holds the microphone the shell should offer to
//! record, or record at once because the user chose to always record it, and when a recorded call
//! has ended.
//!
//! The platform reports what the OS says (`MeetingSignal`: an app took or released the mic,
//! minus the OS's own daemons and this app); this turns it, with each app's call policy
//! ([`CallPolicies`]: Always, Ask or Never), into the consent Drop's two moments, the start of
//! a call recorded always, and the meeting's end:
//!
//! | Rule | Why |
//! |---|---|
//! | An app must hold the mic for [`DETECT_HOLD`] before it is offered | a voice message or a mic check in a browser is not a meeting (Blotter's open question) |
//! | One offer at a time, the app that took the mic first | the Drop has one line to say it in |
//! | "Not this one" dismisses the app until it releases the mic | the next call is a new question; this one was answered |
//! | Stopping a recording by hand dismisses its app the same way | or the Drop would ask again at once, mid-call |
//! | A recorded app that releases the mic ends the meeting after [`STOP_GRACE`], unless it takes it back | a mute that closes the stream, or a device switch, must not end the meeting |
//! | A meeting started without an app ("Record now") never ends by itself | there is nothing to watch |
//! | Past the hold, an app's policy decides: Always records it at once ([`Action::Record`]), Ask offers it, Never does neither | the user chose for this app, in the Drop or in Settings |
//! | An Always start goes through the same start as Record | so it shows, records and ends as a meeting the user started: nothing records unseen |
//! | An Always app is offered instead, until it releases the mic, after the user stopped a recording by hand while it held the mic, when its start failed (or would record more than its own sound), when its recording ended by itself while it still held the mic, or when a policy change made it Always after its hold had passed | recording again by itself would undo the user's stop, or start over and over; a failed start is said, and Record can try again; a click in Settings never starts a recording |
//! | An Always app past its hold is recorded even while another app is offered: that offer is withdrawn first | an unanswered question never keeps a call the user chose to record from being recorded |
//! | A policy changed applies at once: the offered app set to Never is withdrawn as if dismissed; one set to Always stays offered; a held app that was Never is judged afresh (offered, never recorded) | the choice takes effect on this call; an offer being answered is the user's |
//! | A policy never stops a recording | only the user, or its app letting go, does |
//! | Every app that holds the mic for [`DETECT_HOLD`] is reported seen ([`Detection::take_seen`]), once per hold, whatever its policy | Settings lists the apps there are to choose for |
//! | A meeting started here may be stopped and deleted for [`DELETE_WINDOW`] after its start, however it started | a call recorded by mistake (an Always the user forgot, a wrong tap) goes as if never made; after a minute it is a meeting, deleted from the library like any other |
//!
//! Pure and deterministic: the caller passes the time (host ns) and runs the actions. The owner
//! wakes at [`Detection::deadline_ns`] only while something is pending; idle, nothing ticks.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use ink_core::{AppRef, MeetingSignal};

use crate::calls::{CallPolicies, CallPolicy};

/// How long an app must hold the mic before the Drop offers to record it: 3 s.
pub const DETECT_HOLD: Duration = Duration::from_secs(3);

/// How long a recorded app may be without the mic before its meeting ends: 15 s.
pub const STOP_GRACE: Duration = Duration::from_secs(15);

/// How long after its start a meeting may be stopped and deleted (`meeting.discard`): 60 s.
pub const DELETE_WINDOW: Duration = Duration::from_secs(60);

/// What the owner should do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Offer to record `app` (the consent Drop).
    Offer(AppRef),
    /// Record `app` now, without asking: its policy is Always. The owner starts the meeting as
    /// Record does and answers with [`Detection::started`], or [`Detection::start_failed`],
    /// before it calls anything else here.
    Record(AppRef),
    /// The offered app released the mic before the user chose, or gives way to an Always app's
    /// recording: take the offer back.
    Withdraw(AppRef),
    /// The offered app's policy became Never: the offer goes, as by "Not this one".
    Declined(AppRef),
    /// The recorded app has been without the mic for [`STOP_GRACE`]: end the meeting.
    StopMeeting,
    /// Detection stopped on its own (the platform's `Lost`); what it says, never content.
    Lost(String),
}

#[derive(Clone, Debug)]
struct Held {
    app: AppRef,
    since_ns: u64,
    /// Reported seen (it held the mic for the hold).
    seen: bool,
}

/// The meeting being recorded, as detection sees it.
#[derive(Clone, Debug)]
struct Recording {
    /// The app it records, when it was started for one.
    app: Option<String>,
    /// When that app released the mic, while it is without it.
    released_ns: Option<u64>,
    /// When it started (host ns): it may be stopped and deleted for [`DELETE_WINDOW`].
    started_ns: u64,
}

/// See the module docs.
#[derive(Debug, Default)]
pub struct Detection {
    /// Apps holding the mic now, by id, in the order they took it (ties by id).
    held: BTreeMap<String, Held>,
    /// Apps the user said "not this one" to, until they release the mic.
    dismissed: BTreeSet<String>,
    /// Apps offered rather than recorded by an Always policy, until they release the mic (see the
    /// module docs). Kept when detection stops or is lost: a stale entry only asks.
    asked_instead: BTreeSet<String>,
    /// The app offered now.
    offered: Option<AppRef>,
    recording: Option<Recording>,
    policies: CallPolicies,
    /// Apps reported seen and not yet taken.
    seen: Vec<AppRef>,
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

    /// Whether the meeting being recorded may still be stopped and deleted at `now_ns`: one
    /// started here less than [`DELETE_WINDOW`] ago.
    pub fn deletable(&self, now_ns: u64) -> bool {
        self.recording
            .as_ref()
            .is_some_and(|r| now_ns < r.started_ns.saturating_add(ns(DELETE_WINDOW)))
    }

    /// The apps that have held the mic for the hold since the last call, each once per hold.
    pub fn take_seen(&mut self) -> Vec<AppRef> {
        std::mem::take(&mut self.seen)
    }

    /// The policies changed at `now_ns` (see the module docs: they apply at once, and never stop
    /// a recording).
    pub fn set_policies(&mut self, policies: CallPolicies, now_ns: u64) -> Vec<Action> {
        // An app the change makes Always, already past its hold by the clock: its call goes on
        // now, so it is asked about, never recorded by the change. An app that was Always before
        // keeps its place (one queued behind another recording is still recorded after it).
        let became_always: Vec<String> = self
            .held
            .values()
            .filter(|h| {
                now_ns >= h.since_ns.saturating_add(ns(DETECT_HOLD))
                    && self.policies.policy(&h.app.id) != CallPolicy::Always
                    && policies.policy(&h.app.id) == CallPolicy::Always
            })
            .map(|h| h.app.id.clone())
            .collect();
        self.asked_instead.extend(became_always);
        self.policies = policies;
        let mut actions = Vec::new();
        if let Some(offered) = &self.offered
            && self.policies.policy(&offered.id) == CallPolicy::Never
        {
            actions.push(Action::Declined(offered.clone()));
            self.offered = None;
        }
        actions.extend(self.tick(now_ns));
        actions
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
                    seen: false,
                });
            }
            MeetingSignal::MicReleased { app } => {
                self.held.remove(&app.id);
                self.dismissed.remove(&app.id);
                self.asked_instead.remove(&app.id);
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

    /// Judges what is pending at `now_ns`: apps whose hold has passed (seen, then offered or
    /// recorded by their policy), a recorded app whose grace has run out.
    pub fn tick(&mut self, now_ns: u64) -> Vec<Action> {
        let mut actions = Vec::new();
        if let Some(r) = &mut self.recording
            && let Some(released) = r.released_ns
            && now_ns >= released.saturating_add(ns(STOP_GRACE))
        {
            r.released_ns = None;
            actions.push(Action::StopMeeting);
        }
        for held in self.held.values_mut() {
            if !held.seen && now_ns >= held.since_ns.saturating_add(ns(DETECT_HOLD)) {
                held.seen = true;
                self.seen.push(held.app.clone());
            }
        }
        if self.recording.is_some() {
            return actions;
        }
        if let Some(held) = self.next_record(now_ns) {
            let app = held.app.clone();
            if let Some(offered) = self.offered.take() {
                actions.push(Action::Withdraw(offered));
            }
            // Taken as starting at once, so nothing else is offered or recorded meanwhile; the
            // owner's `started` or `start_failed` answers it.
            self.recording = Some(Recording {
                app: Some(app.id.clone()),
                released_ns: None,
                started_ns: now_ns,
            });
            actions.push(Action::Record(app));
        } else if self.offered.is_none()
            && let Some(held) = self.next_offer()
            && now_ns >= held.since_ns.saturating_add(ns(DETECT_HOLD))
        {
            let app = held.app.clone();
            self.offered = Some(app.clone());
            actions.push(Action::Offer(app));
        }
        actions
    }

    /// Whether `held` would be recorded by its policy, not offered.
    fn records(&self, held: &Held) -> bool {
        self.policies.policy(&held.app.id) == CallPolicy::Always
            && !self.asked_instead.contains(&held.app.id)
            && !self.dismissed.contains(&held.app.id)
    }

    /// The first Always app, by when it took the mic, whose hold has passed at `now_ns`.
    fn next_record(&self, now_ns: u64) -> Option<&Held> {
        self.held
            .values()
            .filter(|h| self.records(h) && now_ns >= h.since_ns.saturating_add(ns(DETECT_HOLD)))
            .min_by_key(|h| h.since_ns)
    }

    /// The first app, by when it took the mic, that could be offered or recorded.
    fn next_offer(&self) -> Option<&Held> {
        self.held
            .values()
            .filter(|h| {
                !self.dismissed.contains(&h.app.id)
                    && self.policies.policy(&h.app.id) != CallPolicy::Never
            })
            .min_by_key(|h| h.since_ns)
    }

    /// When [`tick`](Self::tick) has something to judge next, if anything is pending.
    pub fn deadline_ns(&self) -> Option<u64> {
        let stop = self
            .recording
            .as_ref()
            .and_then(|r| r.released_ns)
            .map(|t| t.saturating_add(ns(STOP_GRACE)));
        // Nothing offered: the next app to offer or record; one offered: only an Always app can
        // take its place.
        let offer = self
            .recording
            .is_none()
            .then(|| match self.offered {
                None => self.next_offer(),
                Some(_) => self
                    .held
                    .values()
                    .filter(|h| self.records(h))
                    .min_by_key(|h| h.since_ns),
            })
            .flatten()
            .map(|h| h.since_ns.saturating_add(ns(DETECT_HOLD)));
        let seen = self
            .held
            .values()
            .filter(|h| !h.seen)
            .map(|h| h.since_ns.saturating_add(ns(DETECT_HOLD)))
            .min();
        [stop, offer, seen].into_iter().flatten().min()
    }

    /// The user said "not this one" to `app`: it is not offered again until it releases the mic.
    pub fn dismiss(&mut self, app: &str) {
        self.dismissed.insert(app.to_owned());
        if self.offered.as_ref().is_some_and(|o| o.id == app) {
            self.offered = None;
        }
    }

    /// A meeting started, for `app` (the offer taken, or [`Action::Record`]) or none ("Record
    /// now"). An app that does not hold the mic now starts its grace at once: a meeting for an app
    /// that never takes the mic ends by itself.
    pub fn started(&mut self, app: Option<&str>, now_ns: u64) {
        self.offered = None;
        let released_ns = app
            .filter(|id| !self.held.contains_key(*id))
            .map(|_| now_ns);
        self.recording = Some(Recording {
            app: app.map(str::to_owned),
            released_ns,
            started_ns: now_ns,
        });
    }

    /// The meeting [`Action::Record`] asked for could not start: `app` is offered instead, until
    /// it releases the mic, if it still holds it (the owner says why with the offer).
    pub fn start_failed(&mut self, app: &AppRef) -> Vec<Action> {
        if self
            .recording
            .as_ref()
            .is_some_and(|r| r.app.as_deref() == Some(app.id.as_str()))
        {
            self.recording = None;
        }
        if !self.held.contains_key(&app.id) {
            return Vec::new();
        }
        self.asked_instead.insert(app.id.clone());
        if self.offered.is_none() && !self.dismissed.contains(&app.id) {
            self.offered = Some(app.clone());
            return vec![Action::Offer(app.clone())];
        }
        Vec::new()
    }

    /// The meeting's capture ended. `by_hand`: the user stopped it, so its app, if it still holds
    /// the mic, is dismissed until it releases the mic (the Drop does not ask again mid-call), and
    /// every app holding the mic then is asked about rather than recorded by itself. Not by hand
    /// (its app let go, or its capture ended by itself), an app that still holds the mic is asked
    /// about rather than recorded again: a capture that keeps failing never loops.
    pub fn ended(&mut self, by_hand: bool) {
        let Some(r) = self.recording.take() else {
            return;
        };
        if !by_hand {
            if let Some(app) = r.app
                && self.held.contains_key(&app)
            {
                self.asked_instead.insert(app);
            }
            return;
        }
        self.asked_instead.extend(self.held.keys().cloned());
        if let Some(app) = r.app
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

    /// `default` for every app but those `chosen`.
    fn policies(default: CallPolicy, chosen: &[(&str, CallPolicy)]) -> CallPolicies {
        let mut p = CallPolicies::new(default);
        for (id, policy) in chosen {
            p.choose(id, Some(*policy)).unwrap();
        }
        p
    }

    /// Detection with `default` and `chosen`.
    fn with(default: CallPolicy, chosen: &[(&str, CallPolicy)]) -> Detection {
        let mut d = Detection::new();
        assert!(d.set_policies(policies(default, chosen), 0).is_empty());
        d
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

    #[test]
    fn an_always_app_is_recorded_at_once_past_the_hold_and_a_never_app_is_left_alone() {
        let mut d = with(
            CallPolicy::Ask,
            &[("zoom", CallPolicy::Always), ("chat", CallPolicy::Never)],
        );
        d.signal(uses("chat"), 0);
        assert!(d.tick(3 * S).is_empty(), "Never: no offer");
        assert_eq!(d.deadline_ns(), None, "nothing pending for a Never app");
        d.signal(uses("zoom"), 10 * S);
        assert!(d.tick(12 * S).is_empty(), "the hold applies to Always too");
        assert_eq!(d.tick(13 * S), vec![Action::Record(app("zoom"))]);
        assert!(d.recording() && d.offered().is_none(), "taken as starting");
        assert!(d.tick(14 * S).is_empty(), "said once");
        d.started(Some("zoom"), 13 * S);
        // It ends as a recording the user started does: its app lets go for the grace.
        d.signal(releases("zoom"), 100 * S);
        assert_eq!(d.tick(115 * S), vec![Action::StopMeeting]);
        d.ended(false);
        // The next call is recorded again: no stop by hand came between.
        d.signal(uses("zoom"), 200 * S);
        assert_eq!(d.tick(203 * S), vec![Action::Record(app("zoom"))]);
    }

    #[test]
    fn the_default_decides_for_apps_not_chosen_for() {
        let mut d = with(CallPolicy::Always, &[]);
        d.signal(uses("meet"), 0);
        assert_eq!(d.tick(3 * S), vec![Action::Record(app("meet"))]);

        let mut d = with(CallPolicy::Never, &[("zoom", CallPolicy::Ask)]);
        d.signal(uses("meet"), 0);
        d.signal(uses("zoom"), S);
        assert_eq!(
            d.tick(4 * S),
            vec![Action::Offer(app("zoom"))],
            "only the app chosen for"
        );

        let mut d = with(CallPolicy::Ask, &[]);
        d.signal(uses("meet"), 0);
        assert_eq!(d.tick(3 * S), vec![Action::Offer(app("meet"))], "as before");
    }

    #[test]
    fn stopping_an_always_recording_by_hand_asks_instead_until_the_app_lets_go() {
        let mut d = with(CallPolicy::Ask, &[("zoom", CallPolicy::Always)]);
        d.signal(uses("zoom"), 0);
        assert_eq!(d.tick(3 * S), vec![Action::Record(app("zoom"))]);
        d.started(Some("zoom"), 3 * S);
        d.ended(true);
        assert!(d.tick(60 * S).is_empty(), "its own call is dismissed");
        d.signal(releases("zoom"), 70 * S);
        d.signal(uses("zoom"), 80 * S);
        assert_eq!(
            d.tick(83 * S),
            vec![Action::Record(app("zoom"))],
            "a new call"
        );

        // Record now stopped by hand while an Always app held the mic: that app is offered,
        // never recorded by itself right after the stop.
        let mut d = with(CallPolicy::Ask, &[("zoom", CallPolicy::Always)]);
        d.started(None, 0);
        d.signal(uses("zoom"), S);
        assert!(d.tick(10 * S).is_empty(), "one meeting at a time");
        d.ended(true);
        assert_eq!(d.tick(10 * S), vec![Action::Offer(app("zoom"))]);
    }

    #[test]
    fn an_always_start_that_fails_is_offered_instead() {
        let mut d = with(CallPolicy::Always, &[]);
        d.signal(uses("zoom"), 0);
        assert_eq!(d.tick(3 * S), vec![Action::Record(app("zoom"))]);
        assert_eq!(
            d.start_failed(&app("zoom")),
            vec![Action::Offer(app("zoom"))]
        );
        assert!(!d.recording());
        assert_eq!(d.offered(), Some(&app("zoom")));
        // Not this one: it is not recorded by itself while it holds the mic either.
        d.dismiss("zoom");
        assert!(d.tick(60 * S).is_empty());
        // A failed start for an app that let go meanwhile offers nothing.
        let mut d = with(CallPolicy::Always, &[]);
        d.signal(uses("meet"), 0);
        d.tick(3 * S);
        d.signal(releases("meet"), 3 * S);
        assert!(d.start_failed(&app("meet")).is_empty());
        assert!(!d.recording() && d.offered().is_none());
    }

    #[test]
    fn a_never_app_that_becomes_ask_is_offered_and_one_that_becomes_always_is_recorded() {
        let mut d = with(CallPolicy::Never, &[]);
        d.signal(uses("zoom"), 0);
        assert!(d.tick(30 * S).is_empty());
        assert_eq!(
            d.set_policies(
                policies(CallPolicy::Never, &[("zoom", CallPolicy::Ask)]),
                31 * S
            ),
            vec![Action::Offer(app("zoom"))],
            "judged afresh, its hold long passed"
        );

        // Always, chosen while the call goes on: asked about, never recorded by the click.
        let mut d = with(CallPolicy::Never, &[]);
        d.signal(uses("zoom"), 0);
        d.tick(30 * S);
        assert_eq!(
            d.set_policies(policies(CallPolicy::Always, &[]), 31 * S),
            vec![Action::Offer(app("zoom"))]
        );
        // An app still inside its hold is the policy's first decision: recorded.
        let mut d = with(CallPolicy::Never, &[]);
        d.signal(uses("meet"), 0);
        assert!(
            d.set_policies(policies(CallPolicy::Always, &[]), S)
                .is_empty()
        );
        assert_eq!(d.tick(3 * S), vec![Action::Record(app("meet"))]);
    }

    #[test]
    fn an_always_app_queued_behind_a_recording_is_still_recorded_after_an_unrelated_change() {
        let mut d = with(CallPolicy::Ask, &[("zoom", CallPolicy::Always)]);
        d.started(None, 0);
        d.signal(uses("zoom"), S);
        assert!(d.tick(10 * S).is_empty(), "one meeting at a time");
        assert!(
            d.set_policies(
                policies(
                    CallPolicy::Ask,
                    &[("zoom", CallPolicy::Always), ("chat", CallPolicy::Never)]
                ),
                20 * S
            )
            .is_empty()
        );
        // The first ends by itself (not by hand): Zoom, Always all along, is recorded.
        d.ended(false);
        assert_eq!(d.tick(30 * S), vec![Action::Record(app("zoom"))]);
    }

    #[test]
    fn a_recording_that_ends_by_itself_while_its_app_holds_the_mic_is_not_started_again() {
        let mut d = with(CallPolicy::Always, &[]);
        d.signal(uses("zoom"), 0);
        assert_eq!(d.tick(3 * S), vec![Action::Record(app("zoom"))]);
        d.started(Some("zoom"), 3 * S);
        // Its capture ended by itself (a device gone), the app still in its call.
        d.ended(false);
        assert_eq!(
            d.tick(4 * S),
            vec![Action::Offer(app("zoom"))],
            "asked, never a loop of starts"
        );
    }

    #[test]
    fn an_always_app_takes_the_place_of_an_unanswered_offer() {
        let mut d = with(CallPolicy::Ask, &[("zoom", CallPolicy::Always)]);
        d.signal(uses("voice"), 0);
        assert_eq!(d.tick(3 * S), vec![Action::Offer(app("voice"))]);
        d.signal(uses("zoom"), 10 * S);
        assert_eq!(d.deadline_ns(), Some(13 * S), "woken for the Always app");
        assert_eq!(
            d.tick(13 * S),
            vec![Action::Withdraw(app("voice")), Action::Record(app("zoom"))]
        );
        assert_eq!(d.offered(), None);
    }

    #[test]
    fn a_stop_by_hand_is_remembered_when_detection_restarts_mid_call() {
        let mut d = with(CallPolicy::Ask, &[("zoom", CallPolicy::Always)]);
        d.signal(uses("zoom"), 0);
        d.tick(3 * S);
        d.started(Some("zoom"), 3 * S);
        d.ended(true);
        d.reset();
        d.signal(uses("zoom"), 10 * S);
        assert_eq!(
            d.tick(13 * S),
            vec![Action::Offer(app("zoom"))],
            "the stop stands: asked, not recorded"
        );
    }

    #[test]
    fn never_withdraws_the_offer_and_always_leaves_it_to_the_user() {
        let mut d = with(CallPolicy::Ask, &[]);
        d.signal(uses("zoom"), 0);
        assert_eq!(d.tick(3 * S), vec![Action::Offer(app("zoom"))]);
        assert!(
            d.set_policies(
                policies(CallPolicy::Ask, &[("zoom", CallPolicy::Always)]),
                4 * S
            )
            .is_empty(),
            "the offer stands: the user is answering it (meeting.start follows)"
        );
        assert_eq!(d.offered(), Some(&app("zoom")));
        assert_eq!(
            d.set_policies(
                policies(CallPolicy::Ask, &[("zoom", CallPolicy::Never)]),
                5 * S
            ),
            vec![Action::Declined(app("zoom"))]
        );
        assert_eq!(d.offered(), None);
        assert!(d.tick(60 * S).is_empty());
    }

    #[test]
    fn a_policy_never_stops_a_recording() {
        let mut d = with(CallPolicy::Ask, &[("zoom", CallPolicy::Always)]);
        d.signal(uses("zoom"), 0);
        d.tick(3 * S);
        d.started(Some("zoom"), 3 * S);
        assert!(
            d.set_policies(
                policies(CallPolicy::Never, &[("zoom", CallPolicy::Never)]),
                10 * S
            )
            .is_empty()
        );
        assert!(d.recording());
        assert_eq!(d.deadline_ns(), None, "and its end is still its app's");
    }

    #[test]
    fn every_app_held_past_the_hold_is_seen_once_per_hold_whatever_its_policy() {
        let mut d = with(CallPolicy::Ask, &[("chat", CallPolicy::Never)]);
        d.signal(uses("chat"), 0);
        d.signal(uses("short"), 0);
        assert_eq!(d.deadline_ns(), Some(3 * S), "woken to see them");
        d.signal(releases("short"), 2 * S);
        d.tick(3 * S);
        assert_eq!(d.take_seen(), vec![app("chat")], "a short hold is not seen");
        assert!(d.take_seen().is_empty(), "taken once");
        d.tick(30 * S);
        assert!(d.take_seen().is_empty(), "once per hold");
        // While a meeting records, an app taking the mic is still seen.
        d.started(None, 30 * S);
        d.signal(uses("zoom"), 31 * S);
        assert_eq!(d.deadline_ns(), Some(34 * S));
        d.tick(34 * S);
        assert_eq!(d.take_seen(), vec![app("zoom")]);
        d.signal(releases("chat"), 40 * S);
        d.signal(uses("chat"), 50 * S);
        d.tick(53 * S);
        assert_eq!(d.take_seen(), vec![app("chat")], "a new hold");
    }

    #[test]
    fn a_meeting_may_be_deleted_for_its_first_minute_however_it_started() {
        let mut d = Detection::new();
        assert!(!d.deletable(0), "nothing records");
        d.started(None, 10 * S);
        assert!(d.deletable(10 * S));
        assert!(d.deletable(69 * S + S - 1));
        assert!(!d.deletable(70 * S), "a minute after its start");
        d.ended(true);
        assert!(!d.deletable(11 * S));

        let mut d = with(CallPolicy::Always, &[]);
        d.signal(uses("zoom"), 0);
        d.tick(3 * S);
        d.started(Some("zoom"), 4 * S);
        assert!(d.deletable(63 * S) && !d.deletable(64 * S));
    }
}
