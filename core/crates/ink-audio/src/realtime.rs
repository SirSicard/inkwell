//! The seam that lets tests prove realtime work allocation-free (I4).
//!
//! A realtime callback must not allocate, lock, block or log. Code review alone does not hold that
//! line, so every source in the core runs each callback's work through a [`RealtimeGuard`]. In
//! production the guard just runs the work; tests pass one that forbids allocation for the length
//! of the call (`assert_no_alloc`) and count what it catches.

use std::sync::Arc;

/// Runs one callback's worth of realtime work.
///
/// **Realtime.** It is called on the realtime thread, once per delivered block, so it must not
/// allocate, lock or block itself. It must call the work exactly once.
pub type RealtimeGuard = Arc<dyn Fn(&mut dyn FnMut()) + Send + Sync>;

/// The production guard: runs the work and nothing else.
pub fn unguarded() -> RealtimeGuard {
    Arc::new(|work: &mut dyn FnMut()| work())
}
