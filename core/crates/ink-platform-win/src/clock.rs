//! The host clock: `QueryPerformanceCounter` in nanoseconds.
//!
//! WASAPI stamps every capture packet with the performance counter at the moment the device
//! recorded its first frame (`IAudioCaptureClient::GetBuffer`'s `pu64QPCPosition`), already
//! converted to 100-nanosecond units. So the mic, the far end and the hotkey share one timebase:
//! this clock reads the same counter and converts it to nanoseconds, and [`qpc_position_to_ns`]
//! converts a packet's stamp.
#![cfg(windows)]

use std::time::{SystemTime, UNIX_EPOCH};

use ink_core::{Clock, PlatformError};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

/// Counter ticks to nanoseconds at `frequency` ticks per second, saturating at `u64::MAX`.
/// No allocation, no locks: realtime-safe.
pub(crate) fn ticks_to_ns(ticks: u64, frequency: u64) -> u64 {
    if frequency == 0 {
        return 0;
    }
    // u128 cannot overflow (64-bit factors), and costs no allocation.
    let ns = u128::from(ticks) * 1_000_000_000 / u128::from(frequency);
    u64::try_from(ns).unwrap_or(u64::MAX)
}

/// A WASAPI QPC position (100-ns units) in nanoseconds on this clock's timebase, saturating.
/// **Any thread, realtime included.**
pub fn qpc_position_to_ns(position_100ns: u64) -> u64 {
    position_100ns.saturating_mul(100)
}

/// [`Clock`] on the performance counter.
///
/// Cheap to copy: it holds the counter's frequency, read once at construction (Windows fixes it
/// at boot), so [`now_ns`](Clock::now_ns) is one `QueryPerformanceCounter` call and arithmetic.
#[derive(Clone, Copy, Debug)]
pub struct WinClock {
    frequency: u64,
}

impl WinClock {
    /// Reads the counter's frequency. It never fails on Windows XP or later; the error is there so
    /// a zero frequency is refused rather than divided by.
    pub fn new() -> Result<Self, PlatformError> {
        let mut frequency = 0i64;
        // SAFETY: `frequency` is a valid, writable i64 for the call.
        let read = unsafe { QueryPerformanceFrequency(&mut frequency) };
        match (read, u64::try_from(frequency)) {
            (Ok(()), Ok(frequency)) if frequency > 0 => Ok(Self { frequency }),
            _ => Err(PlatformError::Failed(
                "QueryPerformanceFrequency reported no counter".into(),
            )),
        }
    }

    /// Ticks per second.
    pub fn frequency(&self) -> u64 {
        self.frequency
    }
}

impl Clock for WinClock {
    fn now_ns(&self) -> u64 {
        let mut ticks = 0i64;
        // SAFETY: `ticks` is a valid, writable i64 for the call. It cannot fail on XP or later;
        // on failure `ticks` stays 0, which reads as the start of the timebase, never a panic.
        let _ = unsafe { QueryPerformanceCounter(&mut ticks) };
        ticks_to_ns(u64::try_from(ticks).unwrap_or(0), self.frequency)
    }

    fn unix_ms(&self) -> i64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(since) => i64::try_from(since.as_millis()).unwrap_or(i64::MAX),
            // A clock set before 1970: negative, as the type allows.
            Err(before) => i64::try_from(before.duration().as_millis()).map_or(i64::MIN, |ms| -ms),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_megahertz_ticks_are_100_ns() {
        // The usual QPC frequency on Windows 10 and later.
        assert_eq!(ticks_to_ns(10_000_000, 10_000_000), 1_000_000_000);
        assert_eq!(ticks_to_ns(1, 10_000_000), 100);
        assert_eq!(ticks_to_ns(0, 10_000_000), 0);
    }

    #[test]
    fn an_odd_frequency_converts_exactly_at_whole_seconds() {
        assert_eq!(ticks_to_ns(3_579_545, 3_579_545), 1_000_000_000);
    }

    #[test]
    fn conversions_saturate_and_a_zero_frequency_is_not_divided_by() {
        assert_eq!(ticks_to_ns(u64::MAX, 1), u64::MAX);
        assert_eq!(ticks_to_ns(5, 0), 0);
        assert_eq!(qpc_position_to_ns(u64::MAX), u64::MAX);
    }

    /// The stamp WASAPI puts on a packet and `now_ns` must agree: both come from the counter, one
    /// as 100-ns units, the other as ticks.
    #[test]
    fn a_qpc_position_and_now_share_a_timebase() {
        let clock = WinClock::new().expect("QueryPerformanceFrequency");
        let mut ticks = 0i64;
        // SAFETY: a valid, writable i64.
        unsafe { QueryPerformanceCounter(&mut ticks) }.expect("QueryPerformanceCounter");
        let position_100ns = ticks_to_ns(ticks as u64, clock.frequency()) / 100;
        let now = clock.now_ns();
        let stamp = qpc_position_to_ns(position_100ns);
        assert!(stamp <= now, "{stamp} after {now}");
        assert!(now - stamp < 1_000_000_000, "{} ns apart", now - stamp);
    }

    /// Runs on CI: the counter needs no permission.
    #[test]
    fn host_clock_is_monotonic_and_advances_at_nanosecond_scale() {
        let clock = WinClock::new().expect("QueryPerformanceFrequency");
        let a = clock.now_ns();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let b = clock.now_ns();
        assert!(b >= a + 15_000_000, "elapsed {} ns", b - a);
        assert!(b - a < 5_000_000_000, "elapsed {} ns", b - a);
        // 2024-01-01T00:00:00Z.
        assert!(clock.unix_ms() > 1_704_067_200_000);
    }
}
