//! The host clock: `mach_absolute_time` in nanoseconds.
//!
//! Core Audio stamps capture buffers with `mach_absolute_time` ticks (`AudioTimeStamp.mHostTime`),
//! so this is the one timebase the mic, the far end and the hotkey can share. Ticks are not
//! nanoseconds: on Apple silicon one tick is 125/3 ns (a 24 MHz counter), on Intel it is 1 ns.
//! [`MacClock::ticks_to_ns`] is the conversion; S2.1a uses it for `mHostTime`.
#![cfg(target_os = "macos")]

use std::time::{SystemTime, UNIX_EPOCH};

use ink_core::{Clock, PlatformError};

/// The ratio that turns mach ticks into nanoseconds, as `mach_timebase_info` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Timebase {
    numer: u32,
    denom: u32,
}

impl Timebase {
    /// A timebase of `numer / denom` nanoseconds per tick. `None` when `denom` is zero.
    pub(crate) const fn new(numer: u32, denom: u32) -> Option<Self> {
        if denom == 0 {
            None
        } else {
            Some(Self { numer, denom })
        }
    }

    /// Ticks to nanoseconds, saturating at `u64::MAX`. No allocation, no locks: realtime-safe.
    pub(crate) fn ticks_to_ns(self, ticks: u64) -> u64 {
        // u128 cannot overflow here (both factors are at most 64 and 32 bits), and it costs no
        // allocation, so the conversion stays realtime-safe.
        let ns = u128::from(ticks) * u128::from(self.numer) / u128::from(self.denom);
        u64::try_from(ns).unwrap_or(u64::MAX)
    }
}

/// `mach_timebase_info_data_t`.
#[repr(C)]
struct MachTimebaseInfo {
    numer: u32,
    denom: u32,
}

// SAFETY: both signatures match `<mach/mach_time.h>`; libSystem is always linked.
unsafe extern "C" {
    /// `uint64_t mach_absolute_time(void)`, from libSystem. No arguments, no allocation, no
    /// locks: callable from any thread, a realtime audio callback included.
    safe fn mach_absolute_time() -> u64;
    /// `kern_return_t mach_timebase_info(mach_timebase_info_t info)`, from libSystem.
    fn mach_timebase_info(info: *mut MachTimebaseInfo) -> i32;
}

/// [`Clock`] on the mach timebase.
///
/// Cheap to copy: it holds only the timebase, read once at construction, so
/// [`now_ns`](Clock::now_ns) is one `mach_absolute_time` call and integer arithmetic.
#[derive(Clone, Copy, Debug)]
pub struct MacClock {
    timebase: Timebase,
}

impl MacClock {
    /// Reads the host timebase. It fails only if the kernel refuses `mach_timebase_info`.
    pub fn new() -> Result<Self, PlatformError> {
        let mut info = MachTimebaseInfo { numer: 0, denom: 0 };
        // SAFETY: `info` is a valid, writable struct with the layout of
        // `mach_timebase_info_data_t`, which the call fills in.
        let status = unsafe { mach_timebase_info(&mut info) };
        match Timebase::new(info.numer, info.denom) {
            Some(timebase) if status == 0 && info.numer != 0 => Ok(Self { timebase }),
            _ => Err(PlatformError::Failed(format!(
                "mach_timebase_info failed (status {status})"
            ))),
        }
    }

    /// Converts a `mach_absolute_time` value (a Core Audio `mHostTime`, say) to nanoseconds on
    /// this clock's timebase. **Any thread, realtime included.**
    pub fn ticks_to_ns(&self, ticks: u64) -> u64 {
        self.timebase.ticks_to_ns(ticks)
    }

    pub(crate) fn timebase(&self) -> Timebase {
        self.timebase
    }
}

impl Clock for MacClock {
    fn now_ns(&self) -> u64 {
        self.timebase.ticks_to_ns(mach_absolute_time())
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

    const APPLE_SILICON: Timebase = match Timebase::new(125, 3) {
        Some(t) => t,
        None => panic!("125/3 is a valid timebase"),
    };

    #[test]
    fn apple_silicon_ticks_are_125_thirds_of_a_nanosecond() {
        // 24 MHz: one second of ticks is one second of nanoseconds.
        assert_eq!(APPLE_SILICON.ticks_to_ns(24_000_000), 1_000_000_000);
        assert_eq!(APPLE_SILICON.ticks_to_ns(3), 125);
        assert_eq!(APPLE_SILICON.ticks_to_ns(0), 0);
    }

    #[test]
    fn intel_ticks_are_nanoseconds() {
        let intel = Timebase::new(1, 1).expect("1/1 is valid");
        assert_eq!(intel.ticks_to_ns(123_456_789), 123_456_789);
    }

    #[test]
    fn conversion_saturates_instead_of_overflowing() {
        assert_eq!(APPLE_SILICON.ticks_to_ns(u64::MAX), u64::MAX);
        // A year of uptime is nowhere near saturation.
        let year_ticks = 24_000_000u64 * 3600 * 24 * 365;
        assert_eq!(
            APPLE_SILICON.ticks_to_ns(year_ticks),
            1_000_000_000 * 3600 * 24 * 365
        );
    }

    #[test]
    fn a_zero_denominator_is_refused() {
        assert_eq!(Timebase::new(1, 0), None);
    }

    /// Runs on CI: the mach clock needs no permission.
    #[test]
    fn host_clock_is_monotonic_and_wall_clock_is_plausible() {
        let clock = MacClock::new().expect("mach_timebase_info");
        let a = clock.now_ns();
        let b = clock.now_ns();
        assert!(b >= a, "host time went backwards");
        assert!(a > 0);
        // 2024-01-01T00:00:00Z. Anything earlier means the conversion is wrong, not the machine.
        assert!(clock.unix_ms() > 1_704_067_200_000);
    }

    #[test]
    fn host_clock_advances_at_nanosecond_scale() {
        let clock = MacClock::new().expect("mach_timebase_info");
        let a = clock.now_ns();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let elapsed = clock.now_ns() - a;
        // A ticks-as-nanoseconds bug on Apple silicon would read about 0.5 ms here.
        assert!(elapsed >= 20_000_000, "elapsed {elapsed} ns");
        assert!(elapsed < 5_000_000_000, "elapsed {elapsed} ns");
    }
}
