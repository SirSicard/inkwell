//! The global hotkey: a `CGEventTap` on the session, inserted at the head.
//!
//! **Active, not listen-only (decided).** The tap is created with `kCGEventTapOptionDefault`, so
//! it can swallow the hotkey's own events: a chord never types into the focused app, and Fn never
//! reaches it either. The price is the permission: an active tap needs **Accessibility**, where a
//! listen-only tap would need Input Monitoring. Insertion needs Accessibility anyway, so the app
//! asks for one permission instead of two. Without it, `start` fails with
//! `PermissionDenied(Accessibility)`; it never prompts.
//!
//! **Threads.** The tap runs on its own thread with its own run loop. Its callback reads the
//! event, updates a `Copy` state machine held in a `Cell`, calls the core's sink (which only
//! enqueues) and returns: no locks, no allocation, no blocking, because a slow tap stalls every
//! keystroke on the machine and the OS then disables it. When the OS does disable it
//! (`kCGEventTapDisabledByTimeout` or `ByUserInput`), the callback switches it back on and
//! reports `Cancelled` for any hold in progress. Events this crate posts itself (the paste
//! keystroke, typed text) carry a marker and pass through untouched.
//!
//! **Timestamps** are host time on the [`MacClock`] timebase. `CGEventGetTimestamp` is documented
//! as nanoseconds but carries mach ticks on Apple silicon, so the event's stamp is taken in
//! whichever reading lands in the last second, and the tap's own `now_ns` otherwise.
#![cfg(target_os = "macos")]

pub(crate) mod binding;
mod machine;
mod tap;

use std::sync::{Mutex, PoisonError};

use ink_core::{EventSink, HotkeyBinding, HotkeyEvent, HotkeySource, PlatformError};

use crate::clock::{MacClock, Timebase};
use binding::Binding;
use tap::Tap;

/// The marker in `kCGEventSourceUserData` on every event this crate posts, so its own tap lets
/// them through. Arbitrary; ASCII for "inkw".
pub(crate) const SYNTHETIC_EVENT_MARK: i64 = 0x696E_6B77;

/// [`HotkeySource`] on a session event tap.
pub struct MacHotkeySource {
    clock: MacClock,
    tap: Mutex<Option<Tap>>,
}

impl MacHotkeySource {
    /// A source that stamps events on `clock`'s timebase. No tap exists until
    /// [`start`](HotkeySource::start), so an app that never binds a hotkey never holds one.
    pub fn new(clock: MacClock) -> Self {
        Self {
            clock,
            tap: Mutex::new(None),
        }
    }
}

impl HotkeySource for MacHotkeySource {
    /// Parses the token first, so an unsupported one leaves the current binding running. Then
    /// stops the old tap (reporting `Cancelled` to the old sink if a hold was in progress) and
    /// starts a new one. It waits for the new tap to exist, so a refused permission is an error
    /// here rather than a hotkey that never fires.
    fn start(
        &self,
        binding: &HotkeyBinding,
        on_event: EventSink<HotkeyEvent>,
    ) -> Result<(), PlatformError> {
        let parsed = Binding::parse(&binding.0)?;
        let mut slot = self.tap.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(old) = slot.take() {
            old.shutdown();
        }
        *slot = Some(Tap::spawn(parsed, on_event, self.clock)?);
        Ok(())
    }

    /// Stops and joins the tap thread. A hold in progress is reported as `Cancelled`, from this
    /// thread, before it returns; after that the sink is never called again. Never call it from
    /// the sink itself: the join would wait for the callback that is calling it.
    fn stop(&self) {
        let old = self
            .tap
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(old) = old {
            old.shutdown();
        }
    }
}

impl Drop for MacHotkeySource {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The host time of an event, in nanoseconds.
///
/// `stamp` is `CGEventGetTimestamp`: "roughly nanoseconds since startup" per the header, mach
/// ticks in practice on Apple silicon. Both readings are tried, and the one that falls within the
/// second before `now_ns` wins; if neither does, the event is stamped `now_ns`, which is late by
/// only the tap's own latency.
pub(crate) fn event_time_ns(stamp: u64, timebase: Timebase, now_ns: u64) -> u64 {
    const WINDOW_NS: u64 = 1_000_000_000;
    let plausible = |t: u64| t != 0 && t <= now_ns && now_ns - t <= WINDOW_NS;
    [timebase.ticks_to_ns(stamp), stamp]
        .into_iter()
        .find(|&t| plausible(t))
        .unwrap_or(now_ns)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: u64 = 1_000_000_000;

    fn apple_silicon() -> Timebase {
        Timebase::new(125, 3).expect("valid")
    }

    #[test]
    fn a_stamp_in_mach_ticks_is_converted() {
        let now = 5_000 * SECOND;
        let event_ns = now - 3_000_000; // 3 ms ago
        let ticks = event_ns / 125 * 3;
        let age = now - event_time_ns(ticks, apple_silicon(), now);
        // One tick is about 42 ns, so the round trip lands within a tick of 3 ms.
        assert!((2_999_900..=3_000_100).contains(&age), "{age}");
    }

    #[test]
    fn a_stamp_already_in_nanoseconds_is_kept() {
        let now = 5_000 * SECOND;
        assert_eq!(
            event_time_ns(now - 1_000, apple_silicon(), now),
            now - 1_000
        );
    }

    #[test]
    fn an_implausible_stamp_falls_back_to_now() {
        let now = 5_000 * SECOND;
        assert_eq!(event_time_ns(0, apple_silicon(), now), now);
        assert_eq!(event_time_ns(u64::MAX, apple_silicon(), now), now);
        // In the future on both readings.
        assert_eq!(event_time_ns(now + SECOND, apple_silicon(), now), now);
        // Older than a second on both readings.
        assert_eq!(event_time_ns(now / 1000, apple_silicon(), now), now);
    }

    #[test]
    fn on_intel_both_readings_agree() {
        let intel = Timebase::new(1, 1).expect("valid");
        let now = 42 * SECOND;
        assert_eq!(event_time_ns(now - 7, intel, now), now - 7);
    }

    #[test]
    fn our_marker_is_recognisable_ascii() {
        assert_eq!(SYNTHETIC_EVENT_MARK.to_be_bytes()[4..], *b"inkw");
    }

    /// Needs Accessibility for the terminal that runs it, so it cannot run on CI or from an
    /// agent's shell. `INPUT-CHECKLIST.md` covers the same ground by hand.
    #[test]
    #[ignore = "needs the Accessibility permission (TCC)"]
    fn a_tap_starts_and_stops() {
        use std::sync::Arc;
        let source = MacHotkeySource::new(MacClock::new().expect("clock"));
        let sink: EventSink<HotkeyEvent> = Arc::new(|_| {});
        source
            .start(&HotkeyBinding("fn".into()), sink.clone())
            .expect("tap with Accessibility granted");
        source
            .start(&HotkeyBinding("ctrl+shift+space".into()), sink)
            .expect("rebinding replaces the tap");
        source.stop();
        source.stop();
    }

    #[test]
    fn an_unsupported_token_is_refused_before_any_tap_exists() {
        use std::sync::Arc;
        let source = MacHotkeySource::new(MacClock::new().expect("clock"));
        let sink: EventSink<HotkeyEvent> = Arc::new(|_| {});
        assert!(matches!(
            source.start(&HotkeyBinding("left_option".into()), sink),
            Err(PlatformError::Unsupported(_))
        ));
        assert!(source.tap.lock().expect("unpoisoned").is_none());
    }
}
