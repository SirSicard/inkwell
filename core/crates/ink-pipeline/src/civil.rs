//! Calendar arithmetic for the few places that print a date: snippet variables and export stamps.
//!
//! The core gets time as Unix milliseconds from the platform clock and the user's UTC offset from
//! the shell (as `ink-llm`'s due-date resolution does), so a date is pure arithmetic: no time-zone
//! database, no `chrono`. `ink-llm` has the same day conversion, private to its due dates; it is
//! a dozen lines, so it is repeated here rather than widening that crate's public surface.

const DAY_MS: i64 = 86_400_000;

/// A moment as the user's wall clock shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CivilTime {
    /// The year.
    pub year: i64,
    /// 1–12.
    pub month: u32,
    /// 1–31.
    pub day: u32,
    /// 0–23.
    pub hour: u32,
    /// 0–59.
    pub minute: u32,
    /// 0–59.
    pub second: u32,
}

impl CivilTime {
    /// `unix_ms` shifted by `utc_offset_minutes` (`120` for UTC+2; `0` for UTC).
    pub fn at(unix_ms: i64, utc_offset_minutes: i32) -> Self {
        let local = unix_ms.saturating_add(i64::from(utc_offset_minutes) * 60_000);
        let days = local.div_euclid(DAY_MS);
        let ms_of_day = local.rem_euclid(DAY_MS);
        let (year, month, day) = civil_from_days(days);
        let seconds = ms_of_day / 1_000;
        // In range by construction: a day holds fewer than 86,400 seconds.
        Self {
            year,
            month,
            day,
            hour: (seconds / 3_600) as u32,
            minute: (seconds / 60 % 60) as u32,
            second: (seconds % 60) as u32,
        }
    }

    /// `YYYY-MM-DD`.
    pub fn date(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// `HH:MM`, 24-hour.
    pub fn time(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    /// `YYYY-MM-DDTHH:MM:SSZ`. Meaningful for a time built with a zero offset.
    pub fn iso_utc(&self) -> String {
        format!(
            "{}T{:02}:{:02}:{:02}Z",
            self.date(),
            self.hour,
            self.minute,
            self.second
        )
    }
}

/// Year, month and day of `days` since 1970-01-01 in the proleptic Gregorian calendar (the
/// standard era-based conversion: 400-year eras of 146,097 days, years starting in March).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_dates_convert() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        // 2000-02-29: a leap day in a year divisible by 400.
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        let t = CivilTime::at(1_774_872_000_000, 0);
        assert_eq!(t.iso_utc(), "2026-03-30T12:00:00Z");
    }

    #[test]
    fn the_offset_moves_the_date_and_the_time() {
        // 23:30 UTC is 01:30 the next day at UTC+2, and 18:30 the same day at UTC−5.
        let late = 1_774_872_000_000 + 11 * 3_600_000 + 30 * 60_000;
        let east = CivilTime::at(late, 120);
        assert_eq!(
            (east.date().as_str(), east.time().as_str()),
            ("2026-03-31", "01:30")
        );
        let west = CivilTime::at(late, -300);
        assert_eq!(
            (west.date().as_str(), west.time().as_str()),
            ("2026-03-30", "18:30")
        );
    }
}
