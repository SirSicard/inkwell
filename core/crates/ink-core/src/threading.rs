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

/// A callback the core hands to a platform service or an engine.
///
/// **Callback thread.** It may be invoked from any thread, in order for a given stream. It must
/// not block: the core's implementations only enqueue.
pub type EventSink<T> = Arc<dyn Fn(T) + Send + Sync>;

/// Cooperative cancellation for long worker calls: offline transcription, diarization, language
/// model requests.
///
/// Cloning shares the flag. An implementation checks [`is_cancelled`](Self::is_cancelled) at
/// convenient points and returns its `Cancelled` error. Checking is lock-free, so it is safe
/// anywhere, including a realtime thread.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// A token that is not cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks every holder of this token to stop. It cannot be undone.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Whether [`cancel`](Self::cancel) has been called on any clone.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
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
}
