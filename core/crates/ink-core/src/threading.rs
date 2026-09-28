//! Threading contracts, and the two primitives every trait uses to cross threads.
//!
//! Every trait method in this crate names one of these threads. A method may only be called
//! from the thread it names, and an implementation may assume it.
//!
//! - **Realtime:** an OS audio callback (a Core Audio IOProc, a WASAPI event thread). No
//!   allocation, no locks, no blocking system calls, no logging. Only [`AudioSink::push`] and
//!   [`Clock::now_ns`] run here. I4 enforces this from S1.2a with a thread-scoped no-alloc guard.
//! - **Pump:** the core thread that drains the capture rings into the chunk store and wakes the
//!   pipeline. It does bounded work per wake and never waits on an engine, the store or the
//!   network.
//! - **Worker:** core-owned threads for engines, the store, platform calls and language models.
//!   Calls here may block for as long as the work takes; long ones take a [`CancelToken`].
//! - **Callback:** a thread the platform or an external engine owns (an event tap, a detector's
//!   notification, a Swift engine's queue). Anything the core hands over as an [`EventSink`] runs
//!   here. It must return promptly and never block: it enqueues and returns.
//! - **Main:** the shell's UI thread. The core never calls into it and never waits on it.
//!   Platform code that needs it (AppKit, accessibility, the pasteboard) hops there itself and
//!   hides the hop behind a worker-thread method.
//!
//! Trait objects that cross threads are `Send + Sync` unless their docs say otherwise.
//!
//! [`AudioSink::push`]: crate::audio::AudioSink::push
//! [`Clock::now_ns`]: crate::clock::Clock::now_ns

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// A callback the core hands to a platform service or an engine.
///
/// **Callback thread.** It may be invoked from any thread, in order for a given stream. It must
/// not block: the core's implementations only enqueue.
pub type EventSink<T> = Arc<dyn Fn(T) + Send + Sync>;

/// Cooperative cancellation for long worker calls: offline transcription, diarization, language
/// model requests.
///
/// Cloning shares the flag. An implementation checks [`is_cancelled`](Self::is_cancelled) at
/// convenient points and returns its `Cancelled` error. Checking is lock-free (a token with a
/// deadline also reads the monotonic clock), so it is safe anywhere, including a realtime thread.
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
    deadline: Option<Instant>,
}

impl CancelToken {
    /// A token that is not cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// A token that also reads as cancelled once `deadline` has passed. Nothing fires it: a
    /// holder sees the deadline at its next check, exactly as it would see [`cancel`](Self::cancel),
    /// so a budget costs no thread or timer.
    pub fn with_deadline(deadline: Instant) -> Self {
        Self {
            flag: Arc::default(),
            deadline: Some(deadline),
        }
    }

    /// Asks every holder of this token to stop. It cannot be undone.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
    }

    /// The deadline this token carries, if any: when a holder will first see it cancelled.
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Whether [`cancel`](Self::cancel) has been called on any clone, or the deadline has passed.
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire) || self.deadline.is_some_and(|d| Instant::now() >= d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelling_one_clone_cancels_all() {
        let a = CancelToken::new();
        let b = a.clone();
        assert!(!b.is_cancelled());
        a.cancel();
        assert!(b.is_cancelled());
    }

    #[test]
    fn a_deadline_cancels_every_clone_once_it_passes_with_no_timer() {
        let now = Instant::now();
        let past = CancelToken::with_deadline(now);
        assert!(past.clone().is_cancelled(), "a deadline already reached");

        let future = CancelToken::with_deadline(now + std::time::Duration::from_secs(3_600));
        let clone = future.clone();
        assert!(!clone.is_cancelled());
        assert_eq!(
            clone.deadline(),
            Some(now + std::time::Duration::from_secs(3_600)),
            "clones carry the deadline"
        );
        assert_eq!(CancelToken::new().deadline(), None);
        // `cancel` still works before the deadline, on any clone.
        future.cancel();
        assert!(clone.is_cancelled());
    }
}
