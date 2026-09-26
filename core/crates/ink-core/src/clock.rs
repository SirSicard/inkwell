//! The host clock.

/// Monotonic host time plus wall-clock time.
///
/// The platform crate provides the implementation, because capture timestamps come from the OS
/// (`mach_absolute_time` on macOS, `QueryPerformanceCounter` on Windows) and must share a timebase
/// with [`now_ns`](Self::now_ns). A clock built on something else, such as a `std::time::Instant`
/// anchor, would silently misalign the mic and the far end.
pub trait Clock: Send + Sync {
    /// **Any thread, realtime included.** Monotonic host time in nanoseconds, on the same timebase
    /// as [`AudioBlock::host_time_ns`](crate::audio::AudioBlock::host_time_ns). Must not
    /// allocate or lock.
    fn now_ns(&self) -> u64;

    /// **Any thread except realtime.** Wall-clock time as Unix milliseconds, for record
    /// timestamps. Not monotonic: never use it to measure durations.
    fn unix_ms(&self) -> i64;
}
