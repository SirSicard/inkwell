//! Is the hook still there? Windows removes a low-level hook whose callback runs past its timeout
//! and tells nobody, so the hook thread checks: now and then it injects one marked no-op key, which
//! a live hook sees (and swallows) within [`DEADLINE_MS`]. A missed heartbeat means the hook is
//! gone, and it is installed again.
//!
//! **Only while the user is active.** Injected input counts as user input to Windows, so a
//! heartbeat every few seconds would keep the machine from ever going idle (no screen lock, no
//! sleep, "away" never shown). A heartbeat is sent only when real input arrived in the last
//! [`ACTIVE_MS`] and that input was not the previous heartbeat. A hook can only be removed while
//! it is handling input, so an idle machine has nothing to check.
//!
//! **A miss can be false.** Another low-level hook installed after ours (AutoHotkey, a key
//! remapper) runs first and may swallow the heartbeat key, so our live hook never sees it. So a
//! miss counts only if our callback saw no key event at all since the heartbeat went out, and
//! reinstalls are capped ([`ReinstallBudget`]: three in ten minutes); past the cap the hotkey is
//! reported lost once, rather than churn. A reinstall puts our hook first again, which ends
//! the false misses of that kind. A reinstall never cancels a hold: if its release was really
//! missed, the core's stuck-hold watchdog ends it.
//!
//! Pure over tick-counter milliseconds (which wrap every 49.7 days, hence the wrapping maths).
#![cfg(windows)]

/// How often the hook thread considers a heartbeat.
pub(crate) const INTERVAL_MS: u32 = 5_000;
/// How long a live hook may take to see the heartbeat.
pub(crate) const DEADLINE_MS: u32 = 100;
/// Real input within this long counts as an active user.
pub(crate) const ACTIVE_MS: u32 = 2_000;
/// Input stamped up to this long after the heartbeat was sent is taken as the heartbeat itself.
const OURS_MARGIN_MS: u32 = 50;
/// At most this many reinstalls...
pub(crate) const MAX_REINSTALLS: usize = 3;
/// ...within this window.
pub(crate) const REINSTALL_WINDOW_MS: u32 = 10 * 60 * 1_000;

/// Whether tick `a` is later than tick `b`, across the wrap.
fn later(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) > 0
}

/// The heartbeat's state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Heartbeat {
    pending: bool,
    /// The last input stamp that may be our own heartbeat.
    ours_until: Option<u32>,
    /// Our hook callback's count when the pending heartbeat went out.
    callbacks_at_send: u64,
}

impl Heartbeat {
    /// Whether to send one now: none is pending, and the last input (`GetLastInputInfo`) is recent
    /// and not our own.
    pub(crate) fn should_send(&self, now_ms: u32, last_input_ms: u32) -> bool {
        let recent = now_ms.wrapping_sub(last_input_ms) <= ACTIVE_MS;
        let not_ours = self
            .ours_until
            .is_none_or(|ours| later(last_input_ms, ours));
        !self.pending && recent && not_ours
    }

    /// One was injected; `after_ms` is the tick just after `SendInput` returned, `callbacks` our
    /// hook callback's count then.
    pub(crate) fn sent(&mut self, after_ms: u32, callbacks: u64) {
        self.pending = true;
        self.callbacks_at_send = callbacks;
        self.ours_until = Some(after_ms.wrapping_add(OURS_MARGIN_MS));
    }

    /// The hook saw it.
    pub(crate) fn seen(&mut self) {
        self.pending = false;
    }

    /// At the deadline: whether the hook missed the heartbeat and saw nothing else either (it is
    /// gone). `callbacks` is our callback's count now. Clears the heartbeat either way.
    pub(crate) fn missed(&mut self, callbacks: u64) -> bool {
        std::mem::take(&mut self.pending) && callbacks == self.callbacks_at_send
    }
}

/// Caps reinstalls at [`MAX_REINSTALLS`] per [`REINSTALL_WINDOW_MS`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ReinstallBudget {
    /// Ticks of the reinstalls inside the window.
    recent: Vec<u32>,
}

impl ReinstallBudget {
    /// Whether one more reinstall is allowed at `now_ms`; if so, it is counted.
    pub(crate) fn allow(&mut self, now_ms: u32) -> bool {
        self.recent
            .retain(|&at| now_ms.wrapping_sub(at) < REINSTALL_WINDOW_MS);
        if self.recent.len() >= MAX_REINSTALLS {
            return false;
        }
        self.recent.push(now_ms);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_user_gets_no_heartbeat() {
        let hb = Heartbeat::default();
        assert!(!hb.should_send(10_000, 10_000 - ACTIVE_MS - 1));
        assert!(hb.should_send(10_000, 9_000), "typed a second ago");
    }

    #[test]
    fn a_heartbeat_does_not_count_as_the_user_being_active() {
        let mut hb = Heartbeat::default();
        hb.sent(10_000, 0);
        hb.seen();
        // Five seconds on, the last input is the heartbeat itself (stamped a few ms later).
        assert!(!hb.should_send(15_000, 10_020));
        // The user typed after it.
        assert!(hb.should_send(15_000, 14_500));
    }

    #[test]
    fn a_seen_heartbeat_is_not_missed_and_an_unseen_one_is() {
        let mut hb = Heartbeat::default();
        hb.sent(1_000, 0);
        assert!(!hb.should_send(1_050, 1_040), "one at a time");
        hb.seen();
        assert!(!hb.missed(0));
        hb.sent(6_000, 0);
        assert!(hb.missed(0), "the hook never saw it");
        assert!(!hb.missed(0), "reported once");
    }

    #[test]
    fn a_miss_while_our_hook_saw_other_keys_is_not_a_miss() {
        // Another hook swallowed the heartbeat, but ours saw the user's keys meanwhile: alive.
        let mut hb = Heartbeat::default();
        hb.sent(1_000, 40);
        assert!(!hb.missed(43));
        // Nothing at all reached our callback: gone.
        hb.sent(6_000, 43);
        assert!(hb.missed(43));
    }

    #[test]
    fn reinstalls_are_capped_at_three_in_ten_minutes() {
        let mut budget = ReinstallBudget::default();
        assert!(budget.allow(1_000));
        assert!(budget.allow(2_000));
        assert!(budget.allow(3_000));
        assert!(
            !budget.allow(4_000),
            "a fourth within ten minutes: lost instead"
        );
        // Ten minutes after the first, one slot is free again.
        assert!(budget.allow(1_000 + REINSTALL_WINDOW_MS));
        assert!(!budget.allow(1_000 + REINSTALL_WINDOW_MS + 1));
        // Across the tick counter's wrap.
        let mut wrap = ReinstallBudget::default();
        assert!(wrap.allow(u32::MAX - 5));
        assert!(wrap.allow(10));
        assert!(wrap.allow(20));
        assert!(!wrap.allow(30));
    }

    #[test]
    fn ticks_wrap() {
        let mut hb = Heartbeat::default();
        hb.sent(u32::MAX - 10, 0);
        hb.seen();
        // The margin wraps past zero; input at 100 is after it.
        assert!(hb.should_send(200, 100));
        assert!(!hb.should_send(30, 20), "inside the margin: ours");
    }
}
