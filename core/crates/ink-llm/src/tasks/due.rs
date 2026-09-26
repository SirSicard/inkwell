//! Resolving a spoken due date ("by Friday") against the record's start.
//!
//! Models are asked for the date **as spoken** and never do the arithmetic: "Friday" needs the
//! meeting's date to mean anything, and a model asked to compute dates occasionally invents one.
//! The resolution here is deliberately small and fails to `None` whenever it is unsure, because
//! a wrong "overdue" is worse than no date:
//!
//! | Said | Resolves to the end of |
//! |---|---|
//! | `today`, `tonight`, `end of (the) day`, `EOD` | the record's day |
//! | `tomorrow` | the next day |
//! | `Friday`, `by Friday`, `on Friday`, `this Friday`, `end of Friday` | the next Friday within six days |
//! | `2026-10-02` (ISO) | that day |
//!
//! Unresolved, on purpose: the record's own weekday ("by Wednesday" on a Wednesday: today, or a
//! week today?), `next Friday` (this coming one, or the one after?), `before Friday`, parts of a
//! day (`Friday morning`), weeks and months (`end of the week`, `next month`), and anything else.
//!
//! Days are the user's local days: the record's start plus its UTC offset. "End of" a day is its
//! last millisecond, so a commitment is overdue once that day is over.

/// When a record started, for resolving dates spoken in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordTime {
    /// The record's start, Unix ms.
    pub started_at_unix_ms: i64,
    /// The local UTC offset at the start, in minutes (`120` for UTC+2). The shell knows it.
    pub utc_offset_minutes: i32,
}

const DAY_MS: i64 = 86_400_000;
const WEEKDAYS: [&str; 7] = [
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
];

impl RecordTime {
    fn offset_ms(self) -> i64 {
        i64::from(self.utc_offset_minutes) * 60_000
    }

    /// The local day the record started on, as days since 1970-01-01.
    fn local_day(self) -> i64 {
        self.started_at_unix_ms
            .saturating_add(self.offset_ms())
            .div_euclid(DAY_MS)
    }

    /// The local date, `YYYY-MM-DD (Weekday)`, for prompts.
    pub fn local_date(self) -> String {
        let day = self.local_day();
        let (y, m, d) = civil_from_days(day);
        let weekday = WEEKDAYS[weekday(day)];
        let mut name = weekday.to_owned();
        if let Some(first) = name.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        format!("{y:04}-{m:02}-{d:02} ({name})")
    }

    /// The last millisecond of local day `day`, Unix ms.
    fn end_of(self, day: i64) -> i64 {
        (day + 1)
            .saturating_mul(DAY_MS)
            .saturating_sub(1)
            .saturating_sub(self.offset_ms())
    }
}

/// The due time for `due` as spoken, or `None` when it does not resolve unambiguously (see the
/// module docs for exactly what does).
pub fn resolve_due(due: &str, at: RecordTime) -> Option<i64> {
    let normalized = due
        .trim()
        .trim_end_matches(['.', '!', ','])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let mut rest = normalized.as_str();
    // Strip leading filler until none applies: "by the end of the day" -> "day".
    const PREFIXES: [&str; 10] = [
        "by ",
        "on ",
        "due ",
        "until ",
        "till ",
        "the end of ",
        "end of ",
        "the ",
        "this ",
        "at ",
    ];
    while let Some(stripped) = PREFIXES.iter().find_map(|p| rest.strip_prefix(p)) {
        rest = stripped;
    }

    let today = at.local_day();
    let day = match rest {
        "today" | "tonight" | "day" | "eod" => today,
        "tomorrow" => today + 1,
        _ => {
            if let Some(target) = WEEKDAYS.iter().position(|w| *w == rest) {
                let ahead = (target + 7 - weekday(today)) % 7;
                if ahead == 0 {
                    // Today, or a week today: unsure.
                    return None;
                }
                today + i64::try_from(ahead).ok()?
            } else {
                iso_date(rest)?
            }
        }
    };
    Some(at.end_of(day))
}

/// Monday = 0. Day 0 (1970-01-01) was a Thursday.
fn weekday(day: i64) -> usize {
    // rem_euclid(7) is in 0..7, so the cast cannot truncate.
    (day + 3).rem_euclid(7) as usize
}

/// `YYYY-MM-DD` as days since 1970-01-01, if it is a real date.
fn iso_date(text: &str) -> Option<i64> {
    let mut parts = text.splitn(3, '-');
    let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if !(all_digits(y) && all_digits(m) && all_digits(d)) {
        return None;
    }
    let (y, m, d): (i64, u32, u32) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    if !(1..=12).contains(&m) || d == 0 {
        return None;
    }
    let day = days_from_civil(y, m, d);
    // A round trip rejects 2026-02-30 and friends.
    (civil_from_days(day) == (y, m, d)).then_some(day)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (H. Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day since 1970-01-01 (H. Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    // d is in 1..=31 and m in 1..=12 by construction.
    (y, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wednesday 2026-09-23 09:00 at UTC+2 (checked with an independent date library).
    const WEDNESDAY_MORNING: RecordTime = RecordTime {
        started_at_unix_ms: 1_790_146_800_000,
        utc_offset_minutes: 120,
    };
    /// Friday 2026-09-25 23:59:59.999 at UTC+2.
    const END_OF_FRIDAY: i64 = 1_790_373_599_999;

    #[test]
    fn by_friday_resolves_to_the_end_of_the_coming_friday() {
        for said in [
            "by Friday",
            "Friday",
            "on Friday",
            "this Friday",
            "by the end of Friday",
            "friday.",
            "  By   FRIDAY ",
        ] {
            assert_eq!(
                resolve_due(said, WEDNESDAY_MORNING),
                Some(END_OF_FRIDAY),
                "{said}"
            );
        }
    }

    #[test]
    fn today_tomorrow_and_iso_dates_resolve() {
        let end_of_wednesday = 1_790_200_799_999;
        for said in [
            "today",
            "tonight",
            "by end of day",
            "by the end of the day",
            "EOD",
        ] {
            assert_eq!(
                resolve_due(said, WEDNESDAY_MORNING),
                Some(end_of_wednesday),
                "{said}"
            );
        }
        assert_eq!(
            resolve_due("tomorrow", WEDNESDAY_MORNING),
            Some(1_790_287_199_999)
        );
        assert_eq!(
            resolve_due("by 2026-10-02", WEDNESDAY_MORNING),
            Some(1_790_978_399_999)
        );
        assert_eq!(
            resolve_due("Tuesday", WEDNESDAY_MORNING),
            Some(END_OF_FRIDAY + 4 * DAY_MS),
            "a weekday already past this week is next week's"
        );
    }

    #[test]
    fn unsure_dates_stay_unresolved() {
        for said in [
            "by Wednesday",
            "next Friday",
            "before Friday",
            "Friday morning",
            "end of the week",
            "next week",
            "soon",
            "ASAP",
            "2026-02-30",
            "2026-13-01",
            "26-10-02",
            "",
        ] {
            assert_eq!(resolve_due(said, WEDNESDAY_MORNING), None, "{said}");
        }
    }

    #[test]
    fn days_are_the_users_local_days() {
        // Thursday 02:00 UTC is Wednesday 21:00 at UTC-5: "tomorrow" is Thursday there.
        let evening = RecordTime {
            started_at_unix_ms: 1_790_215_200_000,
            utc_offset_minutes: -300,
        };
        assert_eq!(resolve_due("tomorrow", evening), Some(1_790_312_399_999));
        assert_eq!(evening.local_date(), "2026-09-23 (Wednesday)");
        assert_eq!(WEDNESDAY_MORNING.local_date(), "2026-09-23 (Wednesday)");
    }

    #[test]
    fn the_calendar_helpers_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(weekday(0), 3, "1970-01-01 was a Thursday");
        for day in [-800_000, -1, 0, 59, 60, 11_016, 20_719, 2_000_000] {
            let (y, m, d) = civil_from_days(day);
            assert_eq!(days_from_civil(y, m, d), day);
        }
        assert_eq!(iso_date("2024-02-29"), Some(days_from_civil(2024, 2, 29)));
        assert_eq!(iso_date("2026-02-29"), None);
    }
}
