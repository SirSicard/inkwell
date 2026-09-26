//! Test doubles for every trait, behind the `mock` feature.
//!
//! - [`MemStore`]: an in-memory [`Store`](crate::store::Store) that applies the real supersede
//!   guard.
//! - [`MockEngine`]: an offline and streaming engine that answers from fixtures keyed by
//!   [`fixture_hash`] and records the level of every input it receives, so a test can assert what
//!   the gain stage delivered.
//! - [`MockDiarizer`] and [`MockLlm`]: canned answers, counted calls.
//! - [`MockPlatform`]: every platform trait, driven from the test (feed audio, press the hotkey,
//!   emit meeting signals, deny a permission), with a [`MockClock`].
//!
//! These lock and allocate freely. None is realtime-safe except [`MockClock`].

mod engine;
mod platform;
mod store;

use std::sync::{Mutex, MutexGuard, PoisonError};

pub use engine::{MockCall, MockDiarizer, MockEngine, MockLlm, fixture_hash};
pub use platform::{MockClock, MockPlatform};
pub use store::MemStore;

/// Locks, ignoring poison: a test that panicked while holding a mock's lock must not cascade into
/// every later assertion on the same mock.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
