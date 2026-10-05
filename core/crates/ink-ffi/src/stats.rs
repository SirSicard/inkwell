//! The Stats screen: what the library says about the user's dictation and meetings, counted on
//! this computer from the store. Nothing is sent anywhere, nothing is compared with anyone else,
//! and nothing is drawn from what was said: every number is a count or a time from the record
//! digests ([`ink_core::stats`]) and the commitments' states. The one count that reads text is
//! the user's lines ending in a question mark, and it says so.
//!
//! | Command | Answer |
//! |---|---|
//! | `stats.get` | `stats.counted`: dictation, meetings, promises, milestones, the bests and last week's review, on the user's calendar |
//! | `milestones.check` | `milestones.reached`: the milestones reached since the last check, each reported once ever ([`celebrations`]), and a best the take just set ([`best_news`]) |
//! | `streak.pause`, `streak.resume` | `stats.counted`, with a pause of the streak started today or ended ([`pause`], [`resume`]) |
//!
//! # Where each number comes from
//!
//! - **Words dictated** (today, this week, all time): the words of every finished dictation's
//!   transcript, by the local day it started. Polished text is what was saved, so it is what is
//!   counted. File imports and meetings are not dictation.
//! - **Words per minute**: words over how long the key was held (a dictation's one line runs from
//!   0 to the release of the key), over every timed dictation in the window: this week, and the
//!   last 30 days. A window with less than [`WPM_MIN_SPOKEN_MS`] of speech has no figure: too
//!   little to say anything. Only ever against the user's own past.
//! - **Time saved**: typing the same words at the typing speed (`stats.typing_wpm`, 40 unless set)
//!   less the time the key was held, over timed dictations. It can be negative (slow, paused
//!   dictation); the shell says so rather than showing a negative time. What it is about, in
//!   things the user can picture, is [`about`]'s.
//! - **Streak**: local days with at least one dictation in a row, where a single missed day is
//!   forgiven and two in a row end it ([`streaks`]). A rest day (`stats.rest_days`) or a paused
//!   day ([`Pause`]) without a dictation is not missed: it neither ends the streak nor uses its
//!   forgiven day. One with a dictation counts like any other, so resting or pausing never makes
//!   a streak shorter. It is still running while no more than one day since the last active one
//!   was missed (today is never missed: it is not over). The longest is over all time; the
//!   latest is kept after it ends, so the shell shows an ended streak by what it reached, never
//!   as lost. Hidden (`stats.streak`), it is still counted and marked hidden, and its milestones
//!   are neither listed nor celebrated.
//! - **Heatmap**: words per local day, from the start of the week eleven weeks back to today.
//! - **Meetings** (this month and all time): meetings recorded here and finished. Imported
//!   meetings are left out: their channels came from elsewhere, and me versus them is stream
//!   identity here (architecture rule 5). Hours are end less start; talk time is each channel's
//!   lines (mic is the user, far everyone else); the longest monologue and the questions are the
//!   digests'.
//! - **Promises** (made this month, and all time): commitments not merged into another, by the
//!   local day their meeting started. Kept is done; overdue is open with a due day before today
//!   (as Owed shows it: due today is not late); open is the rest.
//! - **Milestones**: words dictated all time and the longest streak, against fixed thresholds
//!   ([`MILESTONES`]), each with a name the shells word.
//! - **Bests** ([`BestId`]): the longest and the fastest dictation, the most words in a day, the
//!   best week, the longest meeting and the longest monologue, from takes made here only. A record
//!   an import brought (Inkwell 0.2's dictations, another app's meetings) is history from
//!   elsewhere: it counts in words and the streak as before, never in a best. The fastest needs a
//!   take held [`FASTEST_MIN_HELD_MS`].
//! - **Last week** ([`WeekReview`]): its words, time saved, best day, speed against the four weeks
//!   before it, meetings and promises kept, until the user dismisses it for that week. Gains and
//!   plain facts only: nothing is said to be down.
//!
//! # The user's calendar
//!
//! Days are the user's local days. The core has no time-zone database, so the shell sends its
//! zone's UTC offsets over time (`utc_offsets`: each offset from the moment it took effect, oldest
//! first) and the first day of its week (`week_start`, ISO: 1 Monday to 7 Sunday). A record falls
//! on the local day of its start, with the offset in effect then, so a daylight-saving change
//! never moves an old record across midnight. A record made while travelling is placed by the zone
//! the user is in now: the store keeps no zone per record. The dates the core keeps (pauses, the
//! review dismissed, the day a best was last reported) are local dates.

use std::collections::{BTreeMap, BTreeSet};

use ink_core::stats::{CommitmentState, RecordDigest};
use ink_core::{RecordId, RecordKind, Store};
use ink_pipeline::civil::CivilTime;
use serde_json::{Map, Value, json};

use crate::events::event;
use crate::runtime::Shared;

/// The typing speed time saved is measured against while the user has set none.
pub const DEFAULT_TYPING_WPM: u32 = 40;
/// The slowest and fastest typing speeds the setting takes.
pub const TYPING_WPM_RANGE: std::ops::RangeInclusive<u32> = 10..=200;
/// The store setting holding the typing speed, as a whole number of words a minute.
pub const TYPING_WPM_KEY: &str = "stats.typing_wpm";
/// The store setting that turns milestone celebrations off (`off`); on unless set. Bests follow
/// it too.
pub const CELEBRATE_KEY: &str = "stats.celebrate";
/// The store setting noting the milestones already celebrated, or passed while celebrations were
/// off: a JSON array of ids. The core's own; no shell reads or writes it.
pub const MILESTONES_KEY: &str = "stats.milestones";
/// The store setting an import sets (`yes`) so the next milestone check notes what is reached
/// without reporting it: imported words are history, not a milestone reached now. The check
/// clears it (`no`) with its note, in one write. The core's own.
pub const MILESTONES_AFRESH_KEY: &str = "stats.milestones_afresh";
/// The store setting holding the weekdays the streak rests on ([`rest_days`]): `none`, or ISO
/// weekdays ascending and comma-separated (`6,7` for the weekend). None unless set.
pub const REST_DAYS_KEY: &str = "stats.rest_days";
/// The store setting that hides the streak (`hidden`) or shows it (`shown`, unless set).
pub const STREAK_KEY: &str = "stats.streak";
/// The store setting that lets the share card carry the heatmap (`on`). Off unless set: the grid
/// shows the days worked and the days away. The shells read it; the core does nothing with it.
pub const SHARE_HEATMAP_KEY: &str = "stats.share_heatmap";
/// The store setting noting the week whose review the user dismissed: its first day, as
/// `YYYY-MM-DD`.
pub const REVIEW_DISMISSED_KEY: &str = "stats.review_dismissed";
/// The store setting holding the streak's pauses ([`pauses_note`]). The core's own: the shells
/// pause and resume with `streak.pause` and `streak.resume`.
pub const PAUSES_KEY: &str = "stats.streak_pauses";
/// The store setting noting the bests as last seen, and the day one was last reported
/// ([`best_news`]). The core's own.
pub const BESTS_KEY: &str = "stats.bests";
/// The least speech a window needs before it has a words-per-minute figure.
pub const WPM_MIN_SPOKEN_MS: u64 = 60_000;
/// How far back the 30-day average reaches, today included.
pub const WPM_AVERAGE_DAYS: i64 = 30;
/// Whole weeks before this one in the heatmap.
pub const HEATMAP_WEEKS_BEFORE: i64 = 11;
/// The most offsets `utc_offsets` takes: two a year for two centuries.
pub const MAX_UTC_OFFSETS: usize = 400;
/// The longest a pause runs, in days, unless resumed sooner (as Apple pauses Activity rings).
pub const MAX_PAUSE_DAYS: i64 = 90;
/// The most pauses kept: about one a month for sixteen years.
pub const MAX_PAUSES: usize = 200;
/// The least a dictation is held before its speed can be a best. A take's speed is its words over
/// the time the key was held, and the press and the release are each a few hundred ms off the
/// speech: on a 3 s take that is a tenth of the figure, on 30 s a few in a hundred. Short takes
/// are also the quick replies ("sounds good, thanks"), whose speed says nothing about dictating.
/// Half the minute a window needs ([`WPM_MIN_SPOKEN_MS`]), because a best is one take, not a
/// window of them.
pub const FASTEST_MIN_HELD_MS: u64 = 30_000;
/// How many earlier entries of its kind a best must have beaten or tied before it is news: the
/// second take ever is always the longest yet, which is not worth a note.
pub const BEST_AFTER: u64 = 5;
/// The weeks before last week its speed is compared with.
pub const REVIEW_AVERAGE_WEEKS: i64 = 4;
/// The most of one picture [`about`] says ("about five coffee breaks"), but for the largest.
pub const ABOUT_MAX_COUNT: i64 = 5;

const DAY_MS: i64 = 86_400_000;
const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 60 * MINUTE_MS;

/// What time saved is about ([`about`]): a key the shells word ("about two feature films"), and
/// the time it stands for, largest first. Conventional lengths of everyday things, generic on
/// purpose: nothing whose length would need a source.
pub const TIME_EQUIVALENTS: &[(&str, i64)] = &[
    // A full-time week of 40 hours.
    ("working_week", 40 * HOUR_MS),
    ("working_day", 8 * HOUR_MS),
    // Feature films run about 90 to 150 minutes.
    ("feature_film", 2 * HOUR_MS),
    ("lunch_hour", HOUR_MS),
    ("coffee_break", 15 * MINUTE_MS),
];

/// What a milestone counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MilestoneKind {
    /// Words dictated, all time.
    Words,
    /// The longest streak, in days.
    Streak,
}

/// A milestone: an id the store remembers, what it counts and the count it needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Milestone {
    /// Its id, kept in the store once celebrated.
    pub id: &'static str,
    /// What it counts.
    pub kind: MilestoneKind,
    /// The count that reaches it.
    pub threshold: u64,
    /// Its name, a key the shells word ("First page"), so the chip, the celebration and the share
    /// card's seal say the same in both.
    pub name: &'static str,
}

/// Every milestone, smallest first within each kind.
pub const MILESTONES: &[Milestone] = &[
    Milestone {
        id: "words_1000",
        kind: MilestoneKind::Words,
        threshold: 1_000,
        name: "first_page",
    },
    Milestone {
        id: "words_10000",
        kind: MilestoneKind::Words,
        threshold: 10_000,
        name: "notebook",
    },
    Milestone {
        id: "words_50000",
        kind: MilestoneKind::Words,
        threshold: 50_000,
        name: "short_novel",
    },
    Milestone {
        id: "words_100000",
        kind: MilestoneKind::Words,
        threshold: 100_000,
        name: "novels_worth",
    },
    Milestone {
        id: "streak_7",
        kind: MilestoneKind::Streak,
        threshold: 7,
        name: "seven_days",
    },
    Milestone {
        id: "streak_30",
        kind: MilestoneKind::Streak,
        threshold: 30,
        name: "thirty_days",
    },
    Milestone {
        id: "streak_100",
        kind: MilestoneKind::Streak,
        threshold: 100,
        name: "hundred_days",
    },
];

/// The user's calendar: their zone's offsets over time and the first day of their week.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Calendar {
    /// `(from_unix_ms, minutes)`, oldest first; the first also covers everything before it.
    offsets: Vec<(i64, i32)>,
    /// ISO weekday the week starts on: 1 Monday to 7 Sunday.
    week_start: u32,
}

impl Calendar {
    /// A calendar from offsets (oldest first, each from the moment it took effect) and the week's
    /// first day; why not when either is out of range.
    pub fn new(offsets: Vec<(i64, i32)>, week_start: u32) -> Result<Self, String> {
        if offsets.is_empty() || offsets.len() > MAX_UTC_OFFSETS {
            return Err(format!(
                "\"utc_offsets\" holds 1 to {MAX_UTC_OFFSETS} offsets"
            ));
        }
        if offsets.windows(2).any(|w| w[0].0 >= w[1].0) {
            return Err("\"utc_offsets\" must be oldest first, each from a later moment".into());
        }
        if offsets
            .iter()
            .any(|(_, m)| !(-14 * 60..=14 * 60).contains(m))
        {
            return Err("\"utc_offsets\": each offset is minutes from -840 to 840".into());
        }
        if !(1..=7).contains(&week_start) {
            return Err("\"week_start\" is an ISO weekday, 1 (Monday) to 7 (Sunday)".into());
        }
        Ok(Self {
            offsets,
            week_start,
        })
    }

    /// One fixed offset all year, weeks from Monday: for tests.
    #[cfg(test)]
    fn fixed(minutes: i32) -> Self {
        Self {
            offsets: vec![(i64::MIN, minutes)],
            week_start: 1,
        }
    }

    fn offset_at(&self, unix_ms: i64) -> i32 {
        let after = self.offsets.partition_point(|(from, _)| *from <= unix_ms);
        self.offsets[after.saturating_sub(1)].1
    }

    /// The local day of `unix_ms`: days since 1970-01-01 on the user's calendar.
    pub fn day(&self, unix_ms: i64) -> i64 {
        let local = unix_ms.saturating_add(i64::from(self.offset_at(unix_ms)) * 60_000);
        local.div_euclid(DAY_MS)
    }

    /// The ISO weekday of a local day: 1 Monday to 7 Sunday (1970-01-01 was a Thursday).
    fn weekday(day: i64) -> u32 {
        ((day + 3).rem_euclid(7) + 1) as u32
    }

    /// The first day of the week holding `day`.
    pub fn week_start_of(&self, day: i64) -> i64 {
        day - i64::from((Self::weekday(day) + 7 - self.week_start) % 7)
    }

    /// The first day of the month holding `day`.
    pub fn month_start_of(day: i64) -> i64 {
        day - i64::from(CivilTime::at(day * DAY_MS, 0).day) + 1
    }

    /// `YYYY-MM-DD` of a local day.
    pub fn date(day: i64) -> String {
        CivilTime::at(day * DAY_MS, 0).date()
    }

    /// The local day a `YYYY-MM-DD` date names: `None` unless it is a date, written as
    /// [`Calendar::date`] writes it.
    pub fn parse_date(date: &str) -> Option<i64> {
        let digits = |at: std::ops::Range<usize>| -> Option<i64> {
            let part = date.get(at)?;
            // Digits only: `parse` would also take a sign.
            part.bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| part.parse().ok())?
        };
        let b = date.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
            return None;
        }
        let (year, month, day) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        let days = days_from_civil(year, month, day);
        // 30 February comes back as 2 March: not a date.
        (Self::date(days) == date).then_some(days)
    }
}

/// Days since 1970-01-01 of a date in the proleptic Gregorian calendar: the inverse of
/// [`CivilTime`]'s conversion (400-year eras of 146,097 days, years starting in March).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The rest days a [`REST_DAYS_KEY`] value names: `none`, or ISO weekdays (1 Monday to 7 Sunday)
/// ascending and comma-separated, each once, never all seven (a streak needs a day to count).
/// `None` for anything else, so each set has one spelling.
pub fn rest_days(value: &str) -> Option<BTreeSet<u32>> {
    if value == "none" {
        return Some(BTreeSet::new());
    }
    let mut days = BTreeSet::new();
    let mut last = 0;
    for part in value.split(',') {
        let day = match part.as_bytes() {
            [d @ b'1'..=b'7'] => u32::from(d - b'0'),
            _ => return None,
        };
        if day <= last {
            return None;
        }
        last = day;
        days.insert(day);
    }
    (days.len() < 7).then_some(days)
}

/// A pause of the streak: local days from `from` to `until` once resumed. One left running ends
/// by itself after [`MAX_PAUSE_DAYS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pause {
    /// Its first day.
    pub from: i64,
    /// Its last day, once resumed; `None` while running.
    pub until: Option<i64>,
}

impl Pause {
    /// Its last day: `until`, or [`MAX_PAUSE_DAYS`] from its first, whichever is sooner.
    pub fn last(&self) -> i64 {
        let longest = self.from.saturating_add(MAX_PAUSE_DAYS - 1);
        self.until.map_or(longest, |until| until.min(longest))
    }

    /// Whether `day` is paused.
    pub fn covers(&self, day: i64) -> bool {
        (self.from..=self.last()).contains(&day)
    }
}

/// The first day of the pause running `today`, if one is.
pub fn running_pause(pauses: &[Pause], today: i64) -> Option<i64> {
    pauses
        .iter()
        .find(|p| p.until.is_none() && p.covers(today))
        .map(|p| p.from)
}

/// `streak.pause`: the pauses with one started today, or unchanged while one runs. Each before it
/// is closed on its last day (one left running past its 90 days ends there). Refused when
/// [`MAX_PAUSES`] are kept.
pub fn pause(pauses: &[Pause], today: i64) -> Result<Vec<Pause>, String> {
    if running_pause(pauses, today).is_some() {
        return Ok(pauses.to_vec());
    }
    if pauses.len() >= MAX_PAUSES {
        return Err(format!(
            "streak.pause: at most {MAX_PAUSES} pauses are kept"
        ));
    }
    let mut out: Vec<Pause> = pauses
        .iter()
        .map(|p| Pause {
            from: p.from,
            until: Some(p.last()),
        })
        .collect();
    out.push(Pause {
        from: today,
        until: None,
    });
    Ok(out)
}

/// `streak.resume`: the pauses with the running one ended yesterday, so today counts as any day;
/// one started today goes, as if it never was. Unchanged when none runs.
pub fn resume(pauses: &[Pause], today: i64) -> Vec<Pause> {
    pauses
        .iter()
        .filter_map(|p| {
            if p.until.is_some() || !p.covers(today) {
                return Some(*p);
            }
            (p.from < today).then_some(Pause {
                from: p.from,
                until: Some(today - 1),
            })
        })
        .collect()
}

/// The pauses as [`PAUSES_KEY`] keeps them: local dates, oldest first, `until` left out while
/// running (`[{"from":"2026-10-01","until":"2026-10-04"},{"from":"2026-10-10"}]`).
pub fn pauses_note(pauses: &[Pause]) -> String {
    let rows = pauses
        .iter()
        .map(|p| {
            let mut row = Map::new();
            row.insert("from".into(), Calendar::date(p.from).into());
            if let Some(until) = p.until {
                row.insert("until".into(), Calendar::date(until).into());
            }
            Value::Object(row)
        })
        .collect();
    Value::Array(rows).to_string()
}

/// The pauses in [`PAUSES_KEY`]'s note: none without one, or (said in the log) when it cannot be
/// read. Only the core writes it.
pub fn pauses_from(note: Option<&str>) -> Vec<Pause> {
    let Some(note) = note else {
        return Vec::new();
    };
    let read = || -> Option<Vec<Pause>> {
        let rows = serde_json::from_str::<Value>(note).ok()?;
        rows.as_array()?
            .iter()
            .map(|row| {
                let day = |k: &str| {
                    row.get(k)
                        .map(|v| v.as_str().and_then(Calendar::parse_date))
                };
                Some(Pause {
                    from: day("from")??,
                    until: match day("until") {
                        None => None,
                        Some(until) => Some(until?),
                    },
                })
            })
            .collect()
    };
    read().unwrap_or_else(|| {
        log::warn!("stats: the streak's pauses could not be read; counted without them");
        Vec::new()
    })
}

/// A streak's three numbers, in active days.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Streaks {
    /// The streak running now, or 0.
    pub current: u64,
    /// The longest ever.
    pub longest: u64,
    /// The latest, running or ended: what an ended streak reached.
    pub latest: u64,
}

/// The streaks, from the local days with a dictation; `skip` says which days are off (rest days,
/// paused days).
///
/// Forgiving on purpose: one missed day between two active days keeps a streak going; two missed
/// days in a row end it. A day off is not missed, so it does neither, and an active day counts
/// whether it is off or not. The current streak is the last run while at most one day was missed
/// since its last day (today is never missed: it is not over), and 0 after that. Days after today
/// (a clock that was wrong) are not counted.
pub fn streaks(active: &BTreeSet<i64>, today: i64, skip: impl Fn(i64) -> bool) -> Streaks {
    // Days missed strictly between two days, counted to two: all the rule needs. A gap of days
    // off is as long as its pauses (90 days each) or a week of rest days, so this stays short.
    let missed = |after: i64, before: i64| {
        let (mut n, mut day) = (0, after + 1);
        while day < before && n < 2 {
            if !skip(day) {
                n += 1;
            }
            day += 1;
        }
        n
    };
    let (mut run, mut longest, mut last) = (0u64, 0u64, None::<i64>);
    for &day in active.range(..=today) {
        run = match last {
            Some(prev) if missed(prev, day) <= 1 => run + 1,
            _ => 1,
        };
        longest = longest.max(run);
        last = Some(day);
    }
    let current = match last {
        Some(day) if missed(day, today) <= 1 => run,
        _ => 0,
    };
    Streaks {
        current,
        longest,
        latest: run,
    }
}

/// Words per minute from words and speech, rounded.
fn rate(words: u64, spoken_ms: u64) -> u64 {
    let w = (u128::from(words) * 60_000 + u128::from(spoken_ms) / 2) / u128::from(spoken_ms.max(1));
    u64::try_from(w).unwrap_or(u64::MAX)
}

/// Words per minute from words and speech, rounded; `None` under [`WPM_MIN_SPOKEN_MS`].
fn wpm(words: u64, spoken_ms: u64) -> Option<u64> {
    (spoken_ms >= WPM_MIN_SPOKEN_MS).then(|| rate(words, spoken_ms))
}

/// Time saved, ms: `words` typed at `typing_wpm` less `spoken_ms`. Negative when speaking took
/// longer.
fn saved_ms(words: u64, spoken_ms: u64, typing_wpm: u32) -> i64 {
    let typing = i128::from(words) * 60_000 / i128::from(typing_wpm.max(1));
    let saved = typing - i128::from(spoken_ms);
    i64::try_from(saved).unwrap_or(if saved < 0 { i64::MIN } else { i64::MAX })
}

/// What `saved_ms` is about, as `(key, count)` from [`TIME_EQUIVALENTS`], largest first: the
/// nearest whole number of each, kept only when it is within a fifth of the time (so "about"
/// stays honest), and at most [`ABOUT_MAX_COUNT`] of any but the largest (so it stays something
/// to picture). Nothing for time lost, or under about 12 minutes; and some times between two
/// counts (20 minutes is neither one coffee break nor two) have nothing either.
pub fn about(saved_ms: i64) -> Vec<(&'static str, u64)> {
    if saved_ms <= 0 {
        return Vec::new();
    }
    let saved = i128::from(saved_ms);
    TIME_EQUIVALENTS
        .iter()
        .enumerate()
        .filter_map(|(i, &(key, unit))| {
            let unit = i128::from(unit);
            let count = (saved + unit / 2) / unit;
            let off = (count * unit - saved).abs();
            let fits =
                count >= 1 && (i == 0 || count <= i128::from(ABOUT_MAX_COUNT)) && off * 5 <= saved;
            fits.then(|| (key, u64::try_from(count).unwrap_or(u64::MAX)))
        })
        .collect()
}

/// A personal best, in the shelf's order ([`BestId::ALL`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BestId {
    /// The dictation held longest, ms.
    LongestDictation,
    /// The fastest dictation held at least [`FASTEST_MIN_HELD_MS`], words a minute.
    FastestDictation,
    /// The most words dictated in a local day.
    MostWordsDay,
    /// The most words dictated in a week (the user's weeks).
    BestWeek,
    /// The longest meeting recorded here, ms.
    LongestMeeting,
    /// The user's longest monologue in a meeting, ms (the digest's: no one else speaking, no pause
    /// over 3 s).
    LongestMonologue,
}

impl BestId {
    /// Every best, in the shelf's order: the order news is chosen in too.
    pub const ALL: [BestId; 6] = [
        BestId::LongestDictation,
        BestId::FastestDictation,
        BestId::MostWordsDay,
        BestId::BestWeek,
        BestId::LongestMeeting,
        BestId::LongestMonologue,
    ];

    /// Its id in events and in the store's note.
    pub fn id(self) -> &'static str {
        match self {
            BestId::LongestDictation => "longest_dictation",
            BestId::FastestDictation => "fastest_dictation",
            BestId::MostWordsDay => "most_words_day",
            BestId::BestWeek => "best_week",
            BestId::LongestMeeting => "longest_meeting",
            BestId::LongestMonologue => "longest_monologue",
        }
    }

    /// What its value counts: `ms`, `wpm` or `words`.
    pub fn unit(self) -> &'static str {
        match self {
            BestId::LongestDictation | BestId::LongestMeeting | BestId::LongestMonologue => "ms",
            BestId::FastestDictation => "wpm",
            BestId::MostWordsDay | BestId::BestWeek => "words",
        }
    }
}

/// A best on the shelf.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Best {
    /// Which.
    pub id: BestId,
    /// Its value, in its unit.
    pub value: u64,
    /// When: the take's local day, the day, or the week's first day.
    pub day: i64,
    /// The take holding it, for a take's best (not a day's or a week's).
    pub record: Option<RecordId>,
    /// Whether what just happened set it: the newest take of its kind, today, or this week. Only a
    /// fresh best is news.
    pub fresh: bool,
    /// How many entries of its kind it beat or tied: other takes, days or weeks.
    pub earlier: u64,
}

impl Best {
    /// What holds it, as the note keeps it: the take's id, or the day's date.
    fn holder(&self) -> String {
        self.record
            .as_ref()
            .map_or_else(|| Calendar::date(self.day), |r| r.0.clone())
    }
}

/// The leading take for one best while counting, and how many takes were in the running.
#[derive(Default)]
struct Leader<'a> {
    top: Option<(u64, i64, &'a RecordId)>,
    entries: u64,
}

impl<'a> Leader<'a> {
    /// A take's value: the higher leads, and the earlier on a tie (digests come in no order).
    fn offer(&mut self, value: u64, started: i64, record: &'a RecordId) {
        self.entries += 1;
        if self
            .top
            .is_none_or(|(v, s, r)| value > v || (value == v && (started, record) < (s, r)))
        {
            self.top = Some((value, started, record));
        }
    }

    fn best(&self, id: BestId, calendar: &Calendar, newest: Option<&RecordId>) -> Option<Best> {
        let (value, started, record) = self.top?;
        Some(Best {
            id,
            value,
            day: calendar.day(started),
            record: Some(record.clone()),
            fresh: newest == Some(record),
            earlier: self.entries - 1,
        })
    }
}

/// The highest of `totals` (by day or by week, ascending) and when, the earliest on a tie.
fn highest<'a>(totals: impl IntoIterator<Item = (&'a i64, &'a u64)>) -> Option<(i64, u64)> {
    totals
        .into_iter()
        .fold(None, |top, (&day, &value)| match top {
            Some((_, most)) if value <= most => top,
            _ => Some((day, value)),
        })
}

/// A day's or a week's best from the totals of each (none of them zero); fresh when `current`
/// holds it.
fn period_best(id: BestId, totals: &BTreeMap<i64, u64>, current: i64) -> Option<Best> {
    let (day, value) = highest(totals)?;
    Some(Best {
        id,
        value,
        day,
        record: None,
        fresh: day == current,
        earlier: totals.len() as u64 - 1,
    })
}

/// A best a take just set, for the Drop (a dictation's) or the meeting's end (a meeting's).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BestNews {
    /// Which.
    pub id: BestId,
    /// The best before.
    pub old: u64,
    /// The best now.
    pub new: u64,
    /// When, as [`Best::day`].
    pub day: i64,
    /// The take holding it, as [`Best::record`].
    pub record: Option<RecordId>,
}

/// Last week, reviewed: gains and plain facts, never a fall.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WeekReview {
    /// Its first day.
    pub week: i64,
    /// Words dictated.
    pub words: u64,
    /// Time saved, ms, when there was some.
    pub saved_ms: Option<i64>,
    /// The day with the most words, and its words.
    pub best_day: Option<(i64, u64)>,
    /// Words per minute, with a minute of speech.
    pub wpm: Option<u64>,
    /// How much faster than the four weeks before, when it was faster.
    pub wpm_gain: Option<u64>,
    /// Meetings recorded here.
    pub meetings: u64,
    /// Their length, ms.
    pub meeting_ms: u64,
    /// Of the promises made in its meetings, those done now (the store keeps no time a promise
    /// was done), when any are.
    pub promises_kept: Option<u64>,
}

/// Dictation, counted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dictation {
    /// Words today, this week, all time.
    pub words_today: u64,
    /// Words since the week started.
    pub words_week: u64,
    /// Words ever.
    pub words_all: u64,
    /// Finished dictations ever.
    pub dictations_all: u64,
    /// Words per minute this week, when there was enough speech.
    pub wpm_week: Option<u64>,
    /// Words per minute over the last 30 days, when there was enough speech.
    pub wpm_average: Option<u64>,
    /// Time saved this week and all time, ms (signed).
    pub saved_ms_week: i64,
    /// Time saved all time, ms (signed).
    pub saved_ms_all: i64,
    /// The current streak, in active days.
    pub streak_days: u64,
    /// The longest streak, in active days.
    pub longest_streak_days: u64,
    /// The latest streak, running or ended, in active days.
    pub latest_streak_days: u64,
    /// Local days with a dictation since the month started.
    pub active_days_month: u64,
    /// Whether the user hid the streak.
    pub streak_hidden: bool,
    /// The weekdays the streak rests on (ISO).
    pub rest_days: BTreeSet<u32>,
    /// The first day of the pause running today, if one is.
    pub streak_paused_since: Option<i64>,
    /// The heatmap's first local day (a week's first day).
    pub heatmap_first_day: i64,
    /// Words per local day from that day to today.
    pub heatmap_words: Vec<u64>,
}

/// Meetings in a window, counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Meetings {
    /// Meetings recorded here and finished.
    pub meetings: u64,
    /// Their length, ms.
    pub recorded_ms: u64,
    /// The user's talk time, ms.
    pub you_ms: u64,
    /// Everyone else's, ms.
    pub them_ms: u64,
    /// The user's longest monologue, ms.
    pub longest_monologue_ms: u64,
    /// The user's lines ending in a question mark.
    pub questions: u64,
}

/// Promises in a window, counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Promises {
    /// Made: kept, open and overdue together.
    pub made: u64,
    /// Done.
    pub kept: u64,
    /// Open, not yet late.
    pub open: u64,
    /// Open past their due day.
    pub overdue: u64,
}

/// Everything the Stats screen shows, counted at one moment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counted {
    /// Today's local day.
    pub today: i64,
    /// The typing speed time saved was measured against.
    pub typing_wpm: u32,
    /// Dictation.
    pub dictation: Dictation,
    /// Meetings since the month started.
    pub meetings_month: Meetings,
    /// Meetings ever.
    pub meetings_all: Meetings,
    /// Promises made since the month started.
    pub promises_month: Promises,
    /// Promises ever.
    pub promises_all: Promises,
    /// The bests held, in the shelf's order.
    pub bests: Vec<Best>,
    /// Last week, unless it had nothing in it or the user dismissed it.
    pub week_review: Option<WeekReview>,
}

impl Counted {
    /// Whether `m` is reached.
    pub fn reached(&self, m: &Milestone) -> bool {
        let count = match m.kind {
            MilestoneKind::Words => self.dictation.words_all,
            MilestoneKind::Streak => self.dictation.longest_streak_days,
        };
        count >= m.threshold
    }
}

/// The user's settings that shape the counts ([`settings`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The typing speed time saved is measured against.
    pub typing_wpm: u32,
    /// The weekdays the streak rests on (ISO).
    pub rest_days: BTreeSet<u32>,
    /// The streak's pauses, oldest first.
    pub pauses: Vec<Pause>,
    /// Whether the user hid the streak.
    pub streak_hidden: bool,
    /// The first day of the week whose review the user dismissed.
    pub review_dismissed: Option<i64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            typing_wpm: DEFAULT_TYPING_WPM,
            rest_days: BTreeSet::new(),
            pauses: Vec::new(),
            streak_hidden: false,
            review_dismissed: None,
        }
    }
}

/// Counts the library at `now_unix_ms` on `calendar`.
pub fn count(
    digests: &[RecordDigest],
    commitments: &[CommitmentState],
    now_unix_ms: i64,
    calendar: &Calendar,
    settings: &Settings,
) -> Counted {
    let today = calendar.day(now_unix_ms);
    let week = calendar.week_start_of(today);
    let month = Calendar::month_start_of(today);
    let average_from = today - (WPM_AVERAGE_DAYS - 1);
    let heatmap_first_day = week - HEATMAP_WEEKS_BEFORE * 7;
    let last_week = week - 7;
    let review_average_from = last_week - REVIEW_AVERAGE_WEEKS * 7;

    let mut d = Dictation {
        heatmap_first_day,
        heatmap_words: vec![0; usize::try_from(today - heatmap_first_day + 1).unwrap_or(0)],
        ..Dictation::default()
    };
    let mut per_day: BTreeMap<i64, u64> = BTreeMap::new();
    // (words, spoken ms) of timed dictations: this week, the last 30 days, all time; and for the
    // review, last week and the four weeks before it.
    let (mut timed_week, mut timed_average, mut timed_all) = ((0, 0), (0, 0), (0u64, 0u64));
    let (mut timed_last, mut timed_before) = ((0u64, 0u64), (0u64, 0u64));
    let (mut meetings_month, mut meetings_all) = (Meetings::default(), Meetings::default());
    let mut meetings_last = Meetings::default();
    // The bests count takes made here only.
    let (mut longest, mut fastest) = (Leader::default(), Leader::default());
    let (mut longest_meeting, mut longest_monologue) = (Leader::default(), Leader::default());
    let (mut own_days, mut own_weeks): (BTreeMap<i64, u64>, BTreeMap<i64, u64>) =
        (BTreeMap::new(), BTreeMap::new());
    // The newest take of each kind, by start and then id: the take a check follows.
    let (mut newest_dictation, mut newest_meeting) = (None::<(i64, &RecordId)>, None);

    for r in digests {
        // A record still live (or a dictation never finished) counts nothing yet.
        if r.ended_at_unix_ms.is_none() {
            continue;
        }
        let day = calendar.day(r.started_at_unix_ms);
        let take = (r.started_at_unix_ms, &r.record);
        match r.kind {
            RecordKind::Dictation => {
                let words = r.transcript.mic.words;
                let spoken = r.transcript.mic.speech_ms;
                d.dictations_all += 1;
                d.words_all += words;
                if day == today {
                    d.words_today += words;
                }
                if (week..=today).contains(&day) {
                    d.words_week += words;
                }
                if words > 0 {
                    *per_day.entry(day).or_default() += words;
                }
                // Only dictations that know how long the key was held go into speed and time saved,
                // on both sides of the sum.
                if spoken > 0 {
                    let add = |t: &mut (u64, u64)| {
                        t.0 += words;
                        t.1 += spoken;
                    };
                    add(&mut timed_all);
                    if (week..=today).contains(&day) {
                        add(&mut timed_week);
                    }
                    if (average_from..=today).contains(&day) {
                        add(&mut timed_average);
                    }
                    if (last_week..week).contains(&day) {
                        add(&mut timed_last);
                    }
                    if (review_average_from..last_week).contains(&day) {
                        add(&mut timed_before);
                    }
                }
                if !r.imported {
                    if newest_dictation.is_none_or(|n| take > n) {
                        newest_dictation = Some(take);
                    }
                    if spoken > 0 {
                        longest.offer(spoken, r.started_at_unix_ms, &r.record);
                    }
                    if spoken >= FASTEST_MIN_HELD_MS && words > 0 {
                        fastest.offer(rate(words, spoken), r.started_at_unix_ms, &r.record);
                    }
                    if words > 0 {
                        *own_days.entry(day).or_default() += words;
                        *own_weeks.entry(calendar.week_start_of(day)).or_default() += words;
                    }
                }
            }
            RecordKind::Meeting if !r.imported => {
                let length = r.ended_at_unix_ms.map_or(0, |end| {
                    end.saturating_sub(r.started_at_unix_ms).max(0) as u64
                });
                let add = |m: &mut Meetings| {
                    m.meetings += 1;
                    m.recorded_ms += length;
                    m.you_ms += r.transcript.mic.speech_ms;
                    m.them_ms += r.transcript.far.speech_ms;
                    m.longest_monologue_ms = m
                        .longest_monologue_ms
                        .max(r.transcript.longest_monologue_ms);
                    m.questions += r.transcript.mic_questions;
                };
                add(&mut meetings_all);
                if (month..=today).contains(&day) {
                    add(&mut meetings_month);
                }
                if (last_week..week).contains(&day) {
                    add(&mut meetings_last);
                }
                if newest_meeting.is_none_or(|n| take > n) {
                    newest_meeting = Some(take);
                }
                if length > 0 {
                    longest_meeting.offer(length, r.started_at_unix_ms, &r.record);
                }
                if r.transcript.longest_monologue_ms > 0 {
                    longest_monologue.offer(
                        r.transcript.longest_monologue_ms,
                        r.started_at_unix_ms,
                        &r.record,
                    );
                }
            }
            RecordKind::Meeting | RecordKind::FileImport => {}
        }
    }

    for (&day, &words) in per_day.range(heatmap_first_day..=today) {
        if let Ok(i) = usize::try_from(day - heatmap_first_day) {
            d.heatmap_words[i] = words;
        }
    }
    let active: BTreeSet<i64> = per_day.keys().copied().collect();
    let off = |day: i64| {
        settings.rest_days.contains(&Calendar::weekday(day))
            || settings.pauses.iter().any(|p| p.covers(day))
    };
    let s = streaks(&active, today, off);
    (d.streak_days, d.longest_streak_days, d.latest_streak_days) = (s.current, s.longest, s.latest);
    d.active_days_month = per_day.range(month..=today).count() as u64;
    d.streak_hidden = settings.streak_hidden;
    d.rest_days = settings.rest_days.clone();
    d.streak_paused_since = running_pause(&settings.pauses, today);
    d.wpm_week = wpm(timed_week.0, timed_week.1);
    d.wpm_average = wpm(timed_average.0, timed_average.1);
    d.saved_ms_week = saved_ms(timed_week.0, timed_week.1, settings.typing_wpm);
    d.saved_ms_all = saved_ms(timed_all.0, timed_all.1, settings.typing_wpm);

    let (newest_dictation, newest_meeting) = (
        newest_dictation.map(|(_, r)| r),
        newest_meeting.map(|(_, r)| r),
    );
    let bests = [
        longest.best(BestId::LongestDictation, calendar, newest_dictation),
        fastest.best(BestId::FastestDictation, calendar, newest_dictation),
        period_best(BestId::MostWordsDay, &own_days, today),
        period_best(BestId::BestWeek, &own_weeks, week),
        longest_meeting.best(BestId::LongestMeeting, calendar, newest_meeting),
        longest_monologue.best(BestId::LongestMonologue, calendar, newest_meeting),
    ]
    .into_iter()
    .flatten()
    .collect();

    let (mut promises_month, mut promises_all) = (Promises::default(), Promises::default());
    let mut kept_last = 0;
    for c in commitments.iter().filter(|c| !c.merged) {
        let add = |p: &mut Promises| {
            p.made += 1;
            if c.done {
                p.kept += 1;
            } else if c
                .due_at_unix_ms
                .is_some_and(|due| calendar.day(due) < today)
            {
                p.overdue += 1;
            } else {
                p.open += 1;
            }
        };
        add(&mut promises_all);
        let made = calendar.day(c.record_started_at_unix_ms);
        if (month..=today).contains(&made) {
            add(&mut promises_month);
        }
        if c.done && (last_week..week).contains(&made) {
            kept_last += 1;
        }
    }

    let words_last: u64 = per_day.range(last_week..week).map(|(_, w)| w).sum();
    let week_review = ((words_last > 0 || meetings_last.meetings > 0)
        && settings.review_dismissed != Some(last_week))
    .then(|| {
        let wpm_last = wpm(timed_last.0, timed_last.1);
        let saved = saved_ms(timed_last.0, timed_last.1, settings.typing_wpm);
        WeekReview {
            week: last_week,
            words: words_last,
            saved_ms: (saved > 0).then_some(saved),
            best_day: highest(per_day.range(last_week..week)),
            wpm: wpm_last,
            // A gain or nothing: a slower week is never set against the ones before.
            wpm_gain: wpm_last
                .zip(wpm(timed_before.0, timed_before.1))
                .and_then(|(last, before)| last.checked_sub(before))
                .filter(|gain| *gain > 0),
            meetings: meetings_last.meetings,
            meeting_ms: meetings_last.recorded_ms,
            promises_kept: (kept_last > 0).then_some(kept_last),
        }
    });

    Counted {
        today,
        typing_wpm: settings.typing_wpm,
        dictation: d,
        meetings_month,
        meetings_all,
        promises_month,
        promises_all,
        bests,
        week_review,
    }
}

/// A stats command, read. Each counts the library on the user's calendar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatsQuery {
    /// `stats.get`: everything the screen shows.
    Get {
        /// The user's calendar.
        calendar: Calendar,
    },
    /// `milestones.check`: what is newly reached, reported once, and a best just set.
    CheckMilestones {
        /// The user's calendar (a streak counts local days).
        calendar: Calendar,
    },
    /// `streak.pause`: a pause of the streak from today ([`pause`]).
    Pause {
        /// The user's calendar (today is theirs).
        calendar: Calendar,
    },
    /// `streak.resume`: the running pause ended ([`resume`]).
    Resume {
        /// The user's calendar (today is theirs).
        calendar: Calendar,
    },
}

impl StatsQuery {
    fn calendar(&self) -> &Calendar {
        match self {
            StatsQuery::Get { calendar }
            | StatsQuery::CheckMilestones { calendar }
            | StatsQuery::Pause { calendar }
            | StatsQuery::Resume { calendar } => calendar,
        }
    }
}

/// Reads `v` as a stats command when `name` is one: `None` for any other command.
pub fn parse(name: &str, v: &Value) -> Option<Result<StatsQuery, String>> {
    let query: fn(Calendar) -> StatsQuery = match name {
        "stats.get" => |calendar| StatsQuery::Get { calendar },
        "milestones.check" => |calendar| StatsQuery::CheckMilestones { calendar },
        "streak.pause" => |calendar| StatsQuery::Pause { calendar },
        "streak.resume" => |calendar| StatsQuery::Resume { calendar },
        _ => return None,
    };
    Some(parse_calendar(name, v).map(query))
}

fn parse_calendar(name: &str, v: &Value) -> Result<Calendar, String> {
    let obj = v.as_object().ok_or("command: not an object")?;
    if let Some(k) = obj
        .keys()
        .find(|k| !["cmd", "id", "utc_offsets", "week_start"].contains(&k.as_str()))
    {
        return Err(format!("{name}: unknown field \"{k}\""));
    }
    let bad = || {
        format!(
            "{name}: \"utc_offsets\" is [{{\"from_unix_ms\": <integer>, \"minutes\": <integer>}}, ...]"
        )
    };
    let offsets = obj
        .get("utc_offsets")
        .and_then(Value::as_array)
        .ok_or_else(bad)?
        .iter()
        .map(|o| {
            let o = o.as_object().filter(|o| o.len() == 2).ok_or_else(bad)?;
            let from = o
                .get("from_unix_ms")
                .and_then(Value::as_i64)
                .ok_or_else(bad)?;
            let minutes = o
                .get("minutes")
                .and_then(Value::as_i64)
                .and_then(|m| i32::try_from(m).ok())
                .ok_or_else(bad)?;
            Ok((from, minutes))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let week_start = obj
        .get("week_start")
        .and_then(Value::as_u64)
        .and_then(|w| u32::try_from(w).ok())
        .ok_or_else(|| format!("{name}: needs an integer \"week_start\""))?;
    Calendar::new(offsets, week_start).map_err(|e| format!("{name}: {e}"))
}

/// What a milestone check reports, and the note to store (`None`: unchanged), from the milestones
/// `reached` now and the stored note of those already handled.
///
/// - **Once ever.** A milestone is reported the first time a check finds it reached, and noted;
///   it stays noted if the count later falls back (records deleted), so it is never reported
///   twice.
/// - **A first check reports nothing.** With no note (a library from before milestones, or one
///   just imported into), what is reached already is noted silently: an old milestone is not
///   celebrated as new. A note that cannot be read is treated the same way, rather than
///   celebrating everything at once. So is the first check after an import (`afresh`), which keeps
///   what was noted before.
/// - **Off notes without reporting.** With celebrations off a milestone reached is noted, so
///   turning them back on later does not celebrate it late.
pub fn celebrations(
    reached: &[&'static str],
    noted: Option<&str>,
    celebrate: bool,
    afresh: bool,
) -> (Vec<&'static str>, Option<String>) {
    // An array keeps every id it holds that reads as one (an odd element is skipped, never the
    // whole note); anything else is no note at all.
    let previous: Option<BTreeSet<String>> = noted.and_then(|n| {
        let ids = serde_json::from_str::<Value>(n).ok()?;
        Some(
            ids.as_array()?
                .iter()
                .filter_map(|id| id.as_str().map(str::to_owned))
                .collect(),
        )
    });
    if noted.is_some() && previous.is_none() {
        log::warn!("stats: the note of milestones celebrated could not be read; noted afresh");
    }
    let first = previous.is_none() || afresh;
    let mut all = previous.unwrap_or_default();
    let new: Vec<&'static str> = reached
        .iter()
        .copied()
        .filter(|id| !all.contains(*id))
        .collect();
    if !first && new.is_empty() {
        return (Vec::new(), None);
    }
    all.extend(new.iter().map(|id| (*id).to_owned()));
    let note = Value::from(all.into_iter().collect::<Vec<String>>()).to_string();
    let report = if first || !celebrate { Vec::new() } else { new };
    (report, Some(note))
}

/// The typing speed in the store, or the default when unset or unreadable as one.
pub fn typing_wpm(store: &dyn Store) -> Result<u32, String> {
    Ok(store
        .setting(TYPING_WPM_KEY)
        .map_err(|e| e.to_string())?
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|w| TYPING_WPM_RANGE.contains(w))
        .unwrap_or(DEFAULT_TYPING_WPM))
}

/// What a milestone check reports of the bests, and the note to store (`None`: unchanged), from
/// the shelf now and [`BESTS_KEY`]'s note of the bests as last seen.
///
/// - **News is what just happened.** A best is news when it is fresh (the newest take, today or
///   this week holds it), beats the noted value, is held by something other than what was noted,
///   and beat at least [`BEST_AFTER`] earlier entries. A best held for the first time is the
///   baseline, not news. When several are news, the first in the shelf's order is reported.
/// - **One a day.** After a best is reported, the rest of the day's are noted silently: a take
///   that beats one just beaten is on the shelf, not in another note.
/// - **A first check reports nothing.** With no note (a library from before bests, or a note that
///   cannot be read) the shelf is noted silently.
/// - **Off notes without reporting.** With celebrations off a best is noted, so turning them back
///   on later does not report it late.
/// - **The note follows the shelf**, down as well (a best's take deleted): a best is news against
///   what the shelf showed before it.
pub fn best_news(
    shelf: &[Best],
    noted: Option<&str>,
    celebrate: bool,
    today: i64,
) -> (Option<BestNews>, Option<String>) {
    let previous = noted.and_then(read_best_note);
    if noted.is_some() && previous.is_none() {
        log::warn!("stats: the note of bests could not be read; noted afresh");
    }
    let first = previous.is_none();
    let (seen, reported_on) = previous.unwrap_or_default();
    let news = shelf
        .iter()
        .find_map(|b| {
            let (value, holder) = seen.get(b.id.id())?;
            (b.fresh && b.earlier >= BEST_AFTER && b.value > *value && b.holder() != *holder).then(
                || BestNews {
                    id: b.id,
                    old: *value,
                    new: b.value,
                    day: b.day,
                    record: b.record.clone(),
                },
            )
        })
        .filter(|_| !first && celebrate && reported_on != Some(today));
    let reported_on = if news.is_some() {
        Some(today)
    } else {
        reported_on
    };
    let note = best_note(shelf, reported_on);
    let changed = noted != Some(note.as_str());
    (news, changed.then_some(note))
}

/// [`BESTS_KEY`]'s note: each best's value and holder ([`Best::holder`]), and the day a best was
/// last reported (`{"bests":{"longest_dictation":{"holder":"<id>","value":192000}},
/// "reported_on":"2026-10-05"}`).
fn best_note(shelf: &[Best], reported_on: Option<i64>) -> String {
    let bests: Map<String, Value> = shelf
        .iter()
        .map(|b| {
            (
                b.id.id().to_owned(),
                json!({"value": b.value, "holder": b.holder()}),
            )
        })
        .collect();
    let mut note = Map::new();
    note.insert("bests".into(), Value::Object(bests));
    if let Some(day) = reported_on {
        note.insert("reported_on".into(), Calendar::date(day).into());
    }
    Value::Object(note).to_string()
}

/// The bests a note saw: each id's value and holder.
type Seen = BTreeMap<String, (u64, String)>;

/// The bests seen and the day one was last reported, from [`best_note`]'s note; `None` when it
/// cannot be read. An odd entry is skipped, never the note.
fn read_best_note(note: &str) -> Option<(Seen, Option<i64>)> {
    let note = serde_json::from_str::<Value>(note).ok()?;
    let seen = note
        .get("bests")?
        .as_object()?
        .iter()
        .filter_map(|(id, b)| {
            let value = b.get("value")?.as_u64()?;
            let holder = b.get("holder")?.as_str()?.to_owned();
            Some((id.clone(), (value, holder)))
        })
        .collect();
    let reported_on = note
        .get("reported_on")
        .and_then(Value::as_str)
        .and_then(Calendar::parse_date);
    Some((seen, reported_on))
}

/// The user's settings that shape the counts, from the store. A value that cannot be read (only
/// the core writes them, each checked) counts as unset.
pub fn settings(store: &dyn Store) -> Result<Settings, String> {
    let get = |key: &str| store.setting(key).map_err(|e| e.to_string());
    Ok(Settings {
        typing_wpm: typing_wpm(store)?,
        rest_days: get(REST_DAYS_KEY)?
            .as_deref()
            .and_then(rest_days)
            .unwrap_or_default(),
        pauses: pauses_from(get(PAUSES_KEY)?.as_deref()),
        streak_hidden: get(STREAK_KEY)?.as_deref() == Some("hidden"),
        review_dismissed: get(REVIEW_DISMISSED_KEY)?
            .as_deref()
            .and_then(Calendar::parse_date),
    })
}

/// **Worker** (the screens' thread). Counts the library and answers `stats.counted` (for
/// `stats.get`, `streak.pause` and `streak.resume`, after saving the pause) or
/// `milestones.reached`, echoing `id` as `ref`; or why it could not.
pub fn answer(shared: &Shared, query: StatsQuery, id: Option<&str>) -> Result<Value, String> {
    let store = shared.store.as_ref();
    let e = |err: ink_core::StoreError| err.to_string();
    let now = shared.clock.unix_ms();
    let calendar = query.calendar();
    let today = calendar.day(now);
    let mut settings = settings(store)?;
    let pauses = match &query {
        StatsQuery::Pause { .. } => Some(pause(&settings.pauses, today)?),
        StatsQuery::Resume { .. } => Some(resume(&settings.pauses, today)),
        StatsQuery::Get { .. } | StatsQuery::CheckMilestones { .. } => None,
    };
    if let Some(pauses) = pauses.filter(|p| *p != settings.pauses) {
        store
            .set_setting(PAUSES_KEY, &pauses_note(&pauses))
            .map_err(e)?;
        settings.pauses = pauses;
    }
    let counted = count(
        &store.digests().map_err(e)?,
        &store.commitment_states().map_err(e)?,
        now,
        calendar,
        &settings,
    );
    if !matches!(query, StatsQuery::CheckMilestones { .. }) {
        return Ok(counted_event(&counted, id));
    }
    let reached: Vec<&'static str> = MILESTONES
        .iter()
        .filter(|m| counted.reached(m))
        .map(|m| m.id)
        .collect();
    let noted = store.setting(MILESTONES_KEY).map_err(e)?;
    let celebrate = store.setting(CELEBRATE_KEY).map_err(e)?.as_deref() != Some("off");
    let afresh = store.setting(MILESTONES_AFRESH_KEY).map_err(e)?.as_deref() == Some("yes");
    let (mut report, note) = celebrations(&reached, noted.as_deref(), celebrate, afresh);
    if settings.streak_hidden {
        // A hidden streak celebrates nothing. Its milestones are noted all the same, so showing
        // it again later does not celebrate them late.
        report.retain(|id| {
            MILESTONES
                .iter()
                .any(|m| m.id == *id && m.kind == MilestoneKind::Words)
        });
    }
    let bests_noted = store.setting(BESTS_KEY).map_err(e)?;
    let (best, bests_note) = best_news(&counted.bests, bests_noted.as_deref(), celebrate, today);
    // Noted before it is reported, in one write: a note that fails to save fails the check, so
    // nothing is celebrated without being remembered.
    let mut writes: Vec<(&str, &str)> = Vec::new();
    if let Some(note) = &note {
        writes.push((MILESTONES_KEY, note));
    }
    if afresh {
        writes.push((MILESTONES_AFRESH_KEY, "no"));
    }
    if let Some(note) = &bests_note {
        writes.push((BESTS_KEY, note));
    }
    if !writes.is_empty() {
        store.set_settings(&writes).map_err(e)?;
    }
    let rows = MILESTONES
        .iter()
        .filter(|m| report.contains(&m.id))
        .map(|m| milestone(m, true))
        .collect();
    Ok(event(
        "milestones.reached",
        &[
            ("ref", id.map(Value::from)),
            ("milestones", Some(Value::Array(rows))),
            ("best", best.as_ref().map(best_news_value)),
        ],
    ))
}

fn meetings(m: &Meetings) -> Value {
    json!({
        "meetings": m.meetings,
        "recorded_ms": m.recorded_ms,
        "you_ms": m.you_ms,
        "them_ms": m.them_ms,
        "longest_monologue_ms": m.longest_monologue_ms,
        "questions": m.questions,
    })
}

fn promises(p: &Promises) -> Value {
    json!({"made": p.made, "kept": p.kept, "open": p.open, "overdue": p.overdue})
}

/// [`about`]'s answer as events carry it: `[{"key": "feature_film", "count": 2}, ...]`, or `None`
/// when there is nothing to picture.
fn about_value(saved_ms: i64) -> Option<Value> {
    let about = about(saved_ms);
    (!about.is_empty()).then(|| {
        about
            .iter()
            .map(|(key, count)| json!({"key": key, "count": count}))
            .collect()
    })
}

fn best_value(b: &Best) -> Value {
    let mut row = Map::new();
    row.insert("id".into(), b.id.id().into());
    row.insert("unit".into(), b.id.unit().into());
    row.insert("value".into(), b.value.into());
    row.insert("date".into(), Calendar::date(b.day).into());
    if let Some(record) = &b.record {
        row.insert("record".into(), record.0.clone().into());
    }
    Value::Object(row)
}

fn best_news_value(n: &BestNews) -> Value {
    let mut row = Map::new();
    row.insert("id".into(), n.id.id().into());
    row.insert("unit".into(), n.id.unit().into());
    row.insert("old".into(), n.old.into());
    row.insert("new".into(), n.new.into());
    row.insert("date".into(), Calendar::date(n.day).into());
    if let Some(record) = &n.record {
        row.insert("record".into(), record.0.clone().into());
    }
    Value::Object(row)
}

fn review_value(r: &WeekReview) -> Value {
    let mut row = Map::new();
    row.insert("week".into(), Calendar::date(r.week).into());
    row.insert("words".into(), r.words.into());
    row.insert("meetings".into(), r.meetings.into());
    row.insert("meeting_ms".into(), r.meeting_ms.into());
    if let Some(saved) = r.saved_ms {
        row.insert("saved_ms".into(), saved.into());
        if let Some(about) = about_value(saved) {
            row.insert("saved_about".into(), about);
        }
    }
    if let Some((day, words)) = r.best_day {
        row.insert("best_day".into(), Calendar::date(day).into());
        row.insert("best_day_words".into(), words.into());
    }
    if let Some(wpm) = r.wpm {
        row.insert("wpm".into(), wpm.into());
    }
    if let Some(gain) = r.wpm_gain {
        row.insert("wpm_gain".into(), gain.into());
    }
    if let Some(kept) = r.promises_kept {
        row.insert("promises_kept".into(), kept.into());
    }
    Value::Object(row)
}

/// `stats.counted`.
pub fn counted_event(c: &Counted, id: Option<&str>) -> Value {
    let d = &c.dictation;
    let mut dictation = Map::new();
    for (k, v) in [
        ("words_today", d.words_today),
        ("words_week", d.words_week),
        ("words_all", d.words_all),
        ("dictations_all", d.dictations_all),
        ("streak_days", d.streak_days),
        ("longest_streak_days", d.longest_streak_days),
        ("latest_streak_days", d.latest_streak_days),
        ("active_days_month", d.active_days_month),
    ] {
        dictation.insert(k.into(), v.into());
    }
    if let Some(w) = d.wpm_week {
        dictation.insert("wpm_week".into(), w.into());
    }
    if let Some(w) = d.wpm_average {
        dictation.insert("wpm_average".into(), w.into());
    }
    dictation.insert("saved_ms_week".into(), d.saved_ms_week.into());
    dictation.insert("saved_ms_all".into(), d.saved_ms_all.into());
    for (k, saved) in [
        ("saved_about_week", d.saved_ms_week),
        ("saved_about_all", d.saved_ms_all),
    ] {
        if let Some(about) = about_value(saved) {
            dictation.insert(k.into(), about);
        }
    }
    dictation.insert("streak_hidden".into(), d.streak_hidden.into());
    dictation.insert("rest_days".into(), json!(d.rest_days));
    if let Some(day) = d.streak_paused_since {
        dictation.insert("streak_paused_since".into(), Calendar::date(day).into());
    }
    dictation.insert(
        "heatmap_first_day".into(),
        Calendar::date(d.heatmap_first_day).into(),
    );
    dictation.insert("heatmap_words".into(), json!(d.heatmap_words));
    // A hidden streak lists no streak milestone: nowhere to show one.
    let milestones = MILESTONES
        .iter()
        .filter(|m| !(d.streak_hidden && m.kind == MilestoneKind::Streak))
        .map(|m| milestone(m, c.reached(m)))
        .collect();
    event(
        "stats.counted",
        &[
            ("ref", id.map(Value::from)),
            ("today", Some(Calendar::date(c.today).into())),
            ("typing_wpm", Some(c.typing_wpm.into())),
            ("dictation", Some(Value::Object(dictation))),
            ("meetings_month", Some(meetings(&c.meetings_month))),
            ("meetings_all", Some(meetings(&c.meetings_all))),
            ("promises_month", Some(promises(&c.promises_month))),
            ("promises_all", Some(promises(&c.promises_all))),
            ("milestones", Some(Value::Array(milestones))),
            (
                "bests",
                Some(Value::Array(c.bests.iter().map(best_value).collect())),
            ),
            ("week_review", c.week_review.as_ref().map(review_value)),
        ],
    )
}

fn milestone(m: &Milestone, reached: bool) -> Value {
    json!({
        "id": m.id,
        "kind": match m.kind {
            MilestoneKind::Words => "words",
            MilestoneKind::Streak => "streak",
        },
        "threshold": m.threshold,
        "reached": reached,
        "name": m.name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ink_core::RecordId;
    use ink_core::stats::{ChannelDigest, TranscriptDigest};

    /// 2026-10-03 (a Saturday) 12:00 UTC.
    const NOON: i64 = 1_791_028_800_000;
    const HOUR: i64 = 3_600_000;
    const DAY: i64 = 24 * HOUR;

    fn dictation(started: i64, words: u64, held_ms: u64) -> RecordDigest {
        RecordDigest {
            record: RecordId(format!("d{started}")),
            kind: RecordKind::Dictation,
            started_at_unix_ms: started,
            ended_at_unix_ms: Some(started + held_ms as i64 + 500),
            imported: false,
            transcript: TranscriptDigest {
                mic: ChannelDigest {
                    words,
                    speech_ms: held_ms,
                    lines: 1,
                },
                ..TranscriptDigest::default()
            },
        }
    }

    fn utc() -> Calendar {
        Calendar::fixed(0)
    }

    fn typing(typing_wpm: u32) -> Settings {
        Settings {
            typing_wpm,
            ..Settings::default()
        }
    }

    #[test]
    fn the_calendar_knows_its_days_weeks_and_months() {
        let cal = utc();
        let today = cal.day(NOON);
        assert_eq!(Calendar::date(today), "2026-10-03");
        assert_eq!(Calendar::weekday(today), 6, "a Saturday");
        assert_eq!(Calendar::date(cal.week_start_of(today)), "2026-09-28");
        let sunday = Calendar::new(vec![(0, 0)], 7).unwrap();
        assert_eq!(Calendar::date(sunday.week_start_of(today)), "2026-09-27");
        // A week that starts today starts today.
        let saturday = Calendar::new(vec![(0, 0)], 6).unwrap();
        assert_eq!(saturday.week_start_of(today), today);
        assert_eq!(
            Calendar::date(Calendar::month_start_of(today)),
            "2026-10-01"
        );
        assert_eq!(Calendar::date(cal.day(0)), "1970-01-01");
        assert_eq!(Calendar::date(cal.day(-1)), "1969-12-31");
    }

    /// Midnight is the user's: 23:30 and 00:30 local fall on different days, whatever UTC says.
    #[test]
    fn days_turn_at_local_midnight() {
        let plus2 = Calendar::fixed(120);
        // 21:59:59.999 UTC is 23:59:59.999 at UTC+2; a millisecond later it is tomorrow there.
        let local_midnight = NOON + 10 * HOUR;
        assert_eq!(Calendar::date(plus2.day(local_midnight - 1)), "2026-10-03");
        assert_eq!(Calendar::date(plus2.day(local_midnight)), "2026-10-04");
        // West of UTC the day is still yesterday's at 02:00 UTC.
        let minus5 = Calendar::fixed(-300);
        assert_eq!(Calendar::date(minus5.day(NOON - 10 * HOUR)), "2026-10-02");
    }

    /// An old record keeps the offset in effect when it was made: a daylight-saving change does
    /// not move it across midnight.
    #[test]
    fn each_moment_takes_the_offset_in_effect_then() {
        // UTC+2 in summer, UTC+1 from 2026-10-25 01:00 UTC.
        let change = 1_792_890_000_000;
        let cal = Calendar::new(vec![(i64::MIN, 120), (change, 60)], 1).unwrap();
        // 2026-10-03 22:30 UTC is 00:30 on the 4th in summer time.
        assert_eq!(
            Calendar::date(cal.day(NOON + 10 * HOUR + 30 * 60_000)),
            "2026-10-04"
        );
        // 2026-10-26 22:30 UTC is 23:30 on the 26th in winter time; summer's offset would have
        // put it on the 27th.
        let winter = change + DAY + 21 * HOUR + 30 * 60_000;
        assert_eq!(cal.offset_at(winter), 60);
        assert_eq!(cal.offset_at(change - 1), 120);
        assert_eq!(cal.offset_at(change), 60);
        assert_eq!(Calendar::date(cal.day(winter)), "2026-10-26");
    }

    #[test]
    fn a_bad_calendar_is_refused() {
        assert!(Calendar::new(vec![], 1).is_err());
        assert!(
            Calendar::new(vec![(5, 0), (5, 60)], 1).is_err(),
            "not ascending"
        );
        assert!(Calendar::new(vec![(5, 0), (4, 60)], 1).is_err());
        assert!(Calendar::new(vec![(0, 841)], 1).is_err());
        assert!(Calendar::new(vec![(0, 0)], 0).is_err());
        assert!(Calendar::new(vec![(0, 0)], 8).is_err());
        assert!(Calendar::new(vec![(0, -840)], 7).is_ok());
    }

    #[test]
    fn a_streak_forgives_one_missed_day_and_ends_at_two() {
        let s = |d: &[i64], today: i64| {
            let r = streaks(&d.iter().copied().collect(), today, |_| false);
            (r.current, r.longest)
        };
        assert_eq!(s(&[], 100), (0, 0));
        assert_eq!(s(&[100], 100), (1, 1));
        assert_eq!(s(&[99, 100], 100), (2, 2));
        // Yesterday counts while today is not over.
        assert_eq!(s(&[98, 99], 100), (2, 2));
        // One missed day (99) is forgiven: 97, 98 and 100 are one streak of three days.
        assert_eq!(s(&[97, 98, 100], 100), (3, 3));
        // Yesterday missed, the day before active: still running today.
        assert_eq!(s(&[97, 98], 100), (2, 2));
        // Two missed days end it: 96 and 97 were a streak of two, and nothing runs now.
        assert_eq!(s(&[96, 97], 100), (0, 2));
        // Two missed days between runs: a new streak; the longest is kept.
        assert_eq!(s(&[90, 91, 92, 93, 96, 97], 97), (2, 4));
        // Every other day is a streak: a single missed day at a time.
        assert_eq!(s(&[94, 96, 98, 100], 100), (4, 4));
        // A day after today (a wrong clock) is not counted.
        assert_eq!(s(&[99, 100, 101], 100), (2, 2));
    }

    #[test]
    fn words_are_counted_today_this_week_and_ever() {
        let cal = utc();
        let digests = [
            dictation(NOON, 30, 10_000),
            dictation(NOON - 2 * HOUR, 20, 10_000),
            // Monday this week.
            dictation(NOON - 5 * DAY, 100, 30_000),
            // Sunday last week.
            dictation(NOON - 6 * DAY, 1_000, 300_000),
        ];
        let c = count(&digests, &[], NOON, &cal, &Settings::default());
        assert_eq!(c.dictation.words_today, 50);
        assert_eq!(c.dictation.words_week, 150);
        assert_eq!(c.dictation.words_all, 1_150);
        assert_eq!(c.dictation.dictations_all, 4);
    }

    /// A live record, a meeting and a file import are not dictation.
    #[test]
    fn only_finished_dictations_are_dictation() {
        let mut live = dictation(NOON, 99, 1_000);
        live.ended_at_unix_ms = None;
        let mut meeting = dictation(NOON, 500, 1_000);
        meeting.kind = RecordKind::Meeting;
        let mut file = dictation(NOON, 700, 1_000);
        file.kind = RecordKind::FileImport;
        let c = count(
            &[live, meeting, file, dictation(NOON, 3, 1_000)],
            &[],
            NOON,
            &utc(),
            &Settings::default(),
        );
        assert_eq!(c.dictation.words_all, 3);
        assert_eq!(c.dictation.dictations_all, 1);
    }

    /// Words per minute is the user's own: words over the time the key was held, this week and
    /// over the last 30 days, and nothing until there is a minute of speech to go on.
    #[test]
    fn words_per_minute_is_against_your_own_past() {
        let cal = utc();
        // This week: 300 words in 2 minutes = 150 wpm.
        let mut digests = vec![
            dictation(NOON, 150, 60_000),
            dictation(NOON - DAY, 150, 60_000),
        ];
        // Earlier in the month: 600 words in 6 minutes.
        digests.push(dictation(NOON - 20 * DAY, 600, 360_000));
        // Older than 30 days: not in the average.
        digests.push(dictation(NOON - 30 * DAY, 10_000, 60_000));
        // A dictation without a duration (an old import) counts words, not speed.
        digests.push(dictation(NOON, 1_000, 0));
        let c = count(&digests, &[], NOON, &cal, &Settings::default());
        assert_eq!(c.dictation.wpm_week, Some(150));
        // 900 words in 8 minutes = 112.5, rounded.
        assert_eq!(c.dictation.wpm_average, Some(113));

        // Under a minute of speech: no figure.
        let short = count(
            &[dictation(NOON, 50, 59_999)],
            &[],
            NOON,
            &cal,
            &Settings::default(),
        );
        assert_eq!(short.dictation.wpm_week, None);
        assert_eq!(short.dictation.wpm_average, None);
    }

    /// Time saved: the same words typed at the typing speed, less the time spent speaking.
    #[test]
    fn time_saved_is_typing_less_speaking_at_the_speed_given() {
        let cal = utc();
        // 400 words in 2 minutes of speech.
        let digests = [dictation(NOON, 400, 120_000)];
        // At 40 wpm typing takes 10 minutes: 8 saved.
        let at40 = count(&digests, &[], NOON, &cal, &Settings::default());
        assert_eq!(at40.dictation.saved_ms_week, 8 * 60_000);
        assert_eq!(at40.dictation.saved_ms_all, 8 * 60_000);
        assert_eq!(at40.typing_wpm, 40);
        // At 80 wpm it takes 5: 3 saved.
        let at80 = count(&digests, &[], NOON, &cal, &typing(80));
        assert_eq!(at80.dictation.saved_ms_all, 3 * 60_000);
        // Slow dictation saves nothing: it is negative, and said so by the shell.
        let slow = count(
            &[dictation(NOON, 20, 120_000)],
            &[],
            NOON,
            &cal,
            &Settings::default(),
        );
        assert_eq!(slow.dictation.saved_ms_all, 30_000 - 120_000);
        // Last week's words are saved time all time, not this week.
        let old = count(
            &[dictation(NOON - 7 * DAY, 400, 120_000)],
            &[],
            NOON,
            &cal,
            &Settings::default(),
        );
        assert_eq!(old.dictation.saved_ms_week, 0);
        assert_eq!(old.dictation.saved_ms_all, 8 * 60_000);
    }

    #[test]
    fn the_heatmap_runs_from_eleven_weeks_back_to_today() {
        let cal = utc();
        let first = cal.week_start_of(cal.day(NOON)) - 77;
        let digests = [
            dictation(NOON, 5, 1_000),
            dictation(NOON - HOUR, 7, 1_000),
            dictation(first * DAY + HOUR, 9, 1_000),
            // The day before the first: not in the map.
            dictation(first * DAY - HOUR, 11, 1_000),
        ];
        let c = count(&digests, &[], NOON, &cal, &Settings::default());
        assert_eq!(c.dictation.heatmap_first_day, first);
        assert_eq!(Calendar::date(first), "2026-07-13");
        // Monday 13 July to Saturday 3 October: 11 weeks and 6 days.
        assert_eq!(c.dictation.heatmap_words.len(), 83);
        assert_eq!(c.dictation.heatmap_words[0], 9);
        assert_eq!(*c.dictation.heatmap_words.last().unwrap(), 12);
        assert_eq!(c.dictation.heatmap_words.iter().sum::<u64>(), 21);
    }

    /// The streak and the heatmap follow the local day: a dictation at 00:30 local belongs to the
    /// day it was made on there, though UTC still says yesterday.
    #[test]
    fn the_streak_follows_local_days() {
        let plus2 = Calendar::fixed(120);
        // 22:30 UTC on the 2nd and 22:30 UTC on the 1st: the 3rd and the 2nd at UTC+2.
        let digests = [
            dictation(NOON - 14 * HOUR + 30 * 60_000, 5, 1_000),
            dictation(NOON - DAY - 14 * HOUR + 30 * 60_000, 5, 1_000),
        ];
        let c = count(&digests, &[], NOON, &plus2, &Settings::default());
        assert_eq!(c.dictation.words_today, 5);
        assert_eq!(c.dictation.streak_days, 2);
        // In UTC the same two are the 2nd and the 1st: still a streak (yesterday and before),
        // and nothing today.
        let u = count(&digests, &[], NOON, &utc(), &Settings::default());
        assert_eq!(u.dictation.words_today, 0);
        assert_eq!(u.dictation.streak_days, 2);
    }

    fn meeting(started: i64, minutes: i64, transcript: TranscriptDigest) -> RecordDigest {
        RecordDigest {
            record: RecordId(format!("m{started}")),
            kind: RecordKind::Meeting,
            started_at_unix_ms: started,
            ended_at_unix_ms: Some(started + minutes * 60_000),
            imported: false,
            transcript,
        }
    }

    /// Talk time is stream identity: the mic is the user, the far end everyone else.
    #[test]
    fn meetings_are_counted_this_month_and_ever() {
        let talk = |you_ms, them_ms, monologue, questions| TranscriptDigest {
            mic: ChannelDigest {
                words: 10,
                speech_ms: you_ms,
                lines: 3,
            },
            far: ChannelDigest {
                words: 10,
                speech_ms: them_ms,
                lines: 3,
            },
            longest_monologue_ms: monologue,
            mic_questions: questions,
        };
        let mut imported = meeting(NOON, 60, talk(1, 1, 1, 9));
        imported.imported = true;
        let mut live = meeting(NOON, 60, talk(1, 1, 1, 9));
        live.ended_at_unix_ms = None;
        let digests = [
            meeting(NOON - HOUR, 30, talk(600_000, 900_000, 120_000, 4)),
            meeting(NOON - 2 * DAY, 60, talk(1_200_000, 1_800_000, 90_000, 2)),
            // September: all time only.
            meeting(NOON - 3 * DAY, 45, talk(300_000, 300_000, 300_000, 1)),
            imported,
            live,
        ];
        let c = count(&digests, &[], NOON, &utc(), &Settings::default());
        assert_eq!(
            c.meetings_month,
            Meetings {
                meetings: 2,
                recorded_ms: 90 * 60_000,
                you_ms: 1_800_000,
                them_ms: 2_700_000,
                longest_monologue_ms: 120_000,
                questions: 6,
            }
        );
        assert_eq!(c.meetings_all.meetings, 3);
        assert_eq!(c.meetings_all.recorded_ms, 135 * 60_000);
        assert_eq!(c.meetings_all.longest_monologue_ms, 300_000);
        assert_eq!(c.meetings_all.questions, 7);
    }

    /// Promises take Owed's states: done is kept, open past its due day is overdue (due today is
    /// not), the rest are open; one merged into another is that other one.
    #[test]
    fn promises_kept_open_and_overdue_follow_owed() {
        let cal = Calendar::fixed(120);
        let state =
            |n: u8, started: i64, due: Option<i64>, done: bool, merged: bool| CommitmentState {
                commitment: ink_core::CommitmentId(format!("c{n}")),
                record: RecordId("m".into()),
                record_started_at_unix_ms: started,
                due_at_unix_ms: due,
                done,
                merged,
            };
        let this_month = NOON - DAY;
        let states = [
            state(1, this_month, Some(NOON - 5 * DAY), true, false),
            // Due yesterday, local: overdue.
            state(2, this_month, Some(NOON - DAY), false, false),
            // Due earlier today: not late.
            state(3, this_month, Some(NOON - 2 * HOUR), false, false),
            state(4, this_month, None, false, false),
            // Said twice: folded into another, not a promise of its own.
            state(5, this_month, None, false, true),
            // Made in September.
            state(6, NOON - 10 * DAY, Some(NOON - 9 * DAY), true, false),
            state(7, NOON - 10 * DAY, Some(NOON - 9 * DAY), false, false),
        ];
        let c = count(&[], &states, NOON, &cal, &Settings::default());
        assert_eq!(
            c.promises_month,
            Promises {
                made: 4,
                kept: 1,
                open: 2,
                overdue: 1
            }
        );
        assert_eq!(
            c.promises_all,
            Promises {
                made: 6,
                kept: 2,
                open: 2,
                overdue: 2
            }
        );
    }

    #[test]
    fn milestones_are_reached_by_words_and_the_longest_streak() {
        let cal = utc();
        let mut digests: Vec<RecordDigest> = (0..7)
            .map(|i| dictation(NOON - i * DAY, 150, 1_000))
            .collect();
        let c = count(&digests, &[], NOON, &cal, &Settings::default());
        let reached: Vec<&str> = MILESTONES
            .iter()
            .filter(|m| c.reached(m))
            .map(|m| m.id)
            .collect();
        assert_eq!(reached, ["words_1000", "streak_7"]);
        digests.push(dictation(NOON - 400 * DAY, 99_000, 1_000));
        let c = count(&digests, &[], NOON, &cal, &Settings::default());
        assert!(MILESTONES.iter().take(4).all(|m| c.reached(m)));
    }

    /// The once-only rule, in full: a first check takes note silently; after that a milestone is
    /// reported the first time it is reached and never again, kept even if the count falls back;
    /// with celebrations off it is noted and not reported; an unreadable note is a first check.
    #[test]
    fn milestones_are_reported_once_and_noted_whatever_happens() {
        let ids = |v: &[&'static str]| v.to_vec();
        // First check: nothing reported, what is reached noted.
        assert_eq!(
            celebrations(&ids(&["words_1000"]), None, true, false),
            (vec![], Some(r#"["words_1000"]"#.to_string()))
        );
        assert_eq!(
            celebrations(&[], None, true, false),
            (vec![], Some("[]".to_string()))
        );
        // A new one is reported and noted with the old.
        assert_eq!(
            celebrations(
                &ids(&["words_1000", "streak_7"]),
                Some(r#"["words_1000"]"#),
                true,
                false
            ),
            (
                vec!["streak_7"],
                Some(r#"["streak_7","words_1000"]"#.to_string())
            )
        );
        // Nothing new: nothing reported, nothing written.
        assert_eq!(
            celebrations(
                &ids(&["words_1000"]),
                Some(r#"["words_1000"]"#),
                true,
                false
            ),
            (vec![], None)
        );
        // Reached once, then not (records deleted): still noted, and not reported again later.
        assert_eq!(
            celebrations(&[], Some(r#"["words_1000"]"#), true, false),
            (vec![], None)
        );
        assert_eq!(
            celebrations(
                &ids(&["words_1000"]),
                Some(r#"["words_1000"]"#),
                true,
                false
            ),
            (vec![], None)
        );
        // Off: noted, not reported.
        assert_eq!(
            celebrations(&ids(&["streak_7"]), Some("[]"), false, false),
            (vec![], Some(r#"["streak_7"]"#.to_string()))
        );
        // After an import: noted silently, what was noted before kept.
        assert_eq!(
            celebrations(
                &ids(&["words_1000", "words_10000"]),
                Some(r#"["streak_7"]"#),
                true,
                true
            ),
            (
                vec![],
                Some(r#"["streak_7","words_1000","words_10000"]"#.to_string())
            )
        );
        // A note with an odd element keeps the ids it can read.
        assert_eq!(
            celebrations(
                &ids(&["words_1000", "streak_7"]),
                Some(r#"["words_1000", 7]"#),
                true,
                false
            ),
            (
                vec!["streak_7"],
                Some(r#"["streak_7","words_1000"]"#.to_string())
            )
        );
        // An unreadable note is a first check, rather than a flood of old milestones.
        assert_eq!(
            celebrations(&ids(&["words_1000"]), Some("not json"), true, false),
            (vec![], Some(r#"["words_1000"]"#.to_string()))
        );
    }

    #[test]
    fn stats_get_parses_or_says_why_not() {
        let p = |json: &str| {
            let v: Value = serde_json::from_str(json).unwrap();
            parse(v["cmd"].as_str().unwrap(), &v)
        };
        assert!(p(r#"{"cmd":"library.stats","since_unix_ms":0}"#).is_none());
        assert_eq!(
            p(r#"{"cmd":"stats.get","id":"s1","week_start":7,
                  "utc_offsets":[{"from_unix_ms":0,"minutes":60},{"from_unix_ms":9,"minutes":120}]}"#),
            Some(Ok(StatsQuery::Get {
                calendar: Calendar::new(vec![(0, 60), (9, 120)], 7).unwrap()
            }))
        );
        for bad in [
            r#"{"cmd":"stats.get","week_start":1}"#,
            r#"{"cmd":"stats.get","week_start":1,"utc_offsets":[]}"#,
            r#"{"cmd":"stats.get","week_start":1,"utc_offsets":[{"from_unix_ms":0}]}"#,
            r#"{"cmd":"stats.get","week_start":1,"utc_offsets":[{"from_unix_ms":0,"minutes":60,"x":1}]}"#,
            r#"{"cmd":"stats.get","week_start":1,"utc_offsets":[{"from_unix_ms":0,"minutes":900}]}"#,
            r#"{"cmd":"stats.get","week_start":0,"utc_offsets":[{"from_unix_ms":0,"minutes":0}]}"#,
            r#"{"cmd":"stats.get","utc_offsets":[{"from_unix_ms":0,"minutes":0}]}"#,
            r#"{"cmd":"stats.get","week_start":1,"utc_offsets":[{"from_unix_ms":0,"minutes":0}],"range":"week"}"#,
        ] {
            assert!(matches!(p(bad), Some(Err(_))), "{bad}");
        }
    }

    #[test]
    fn a_date_reads_back_as_its_day_or_not_at_all() {
        assert_eq!(Calendar::parse_date("1970-01-01"), Some(0));
        assert_eq!(Calendar::parse_date("1969-12-31"), Some(-1));
        assert_eq!(Calendar::parse_date("2026-10-03"), Some(utc().day(NOON)));
        assert!(Calendar::parse_date("2024-02-29").is_some(), "a leap day");
        for bad in [
            "2025-02-29",
            "2026-02-30",
            "2026-13-01",
            "2026-00-10",
            "2026-1-01",
            "26-10-03",
            "+026-10-03",
            "2026-10-03 ",
            "2026/10/03",
            "",
        ] {
            assert_eq!(Calendar::parse_date(bad), None, "{bad}");
        }
    }

    /// One spelling per set of rest days, and never all seven: a streak needs a day to count.
    #[test]
    fn rest_days_are_weekdays_in_one_spelling() {
        assert_eq!(rest_days("none"), Some(BTreeSet::new()));
        assert_eq!(rest_days("6,7"), Some(BTreeSet::from([6, 7])));
        assert_eq!(rest_days("3"), Some(BTreeSet::from([3])));
        assert_eq!(
            rest_days("1,2,3,4,5,6"),
            Some(BTreeSet::from([1, 2, 3, 4, 5, 6]))
        );
        for bad in [
            "",
            "7,6",
            "6,6",
            "0",
            "8",
            "6, 7",
            "6,7,",
            "06",
            "sat",
            "1,2,3,4,5,6,7",
        ] {
            assert_eq!(rest_days(bad), None, "{bad}");
        }
    }

    /// Rest days and pauses are days off: one without a dictation neither breaks the streak nor
    /// uses its forgiven day, and one with a dictation counts like any other active day.
    #[test]
    fn rest_days_and_pauses_neither_break_nor_count_against_a_streak() {
        // Day 95 is a Monday, 100 and 101 the weekend after.
        assert_eq!(Calendar::weekday(95), 1);
        let days = |d: &[i64]| d.iter().copied().collect::<BTreeSet<i64>>();
        let weekend = |day: i64| Calendar::weekday(day) >= 6;
        let two_weeks: Vec<i64> = (95..=99).chain(102..=106).collect();
        // Without rest days the weekend is two missed days: a new streak on Monday.
        assert_eq!(streaks(&days(&two_weeks), 106, |_| false).current, 5);
        let rested = streaks(&days(&two_weeks), 106, weekend);
        assert_eq!((rested.current, rested.longest), (10, 10));
        // The forgiven day still applies beside them: Monday missed after the weekend.
        assert_eq!(
            streaks(&days(&[95, 96, 97, 98, 99, 103]), 103, weekend).current,
            6
        );
        // Monday and Tuesday missed after it: two missed days end it, rest days or not.
        let ended = streaks(&days(&[95, 96, 97, 98, 99, 104]), 104, weekend);
        assert_eq!((ended.current, ended.longest), (1, 5));
        // A dictation on a rest day counts.
        assert_eq!(streaks(&days(&[98, 99, 100, 101]), 101, weekend).current, 4);
        // Running over a weekend still to come: Friday, with Saturday today.
        assert_eq!(streaks(&days(&[98, 99]), 101, weekend).current, 2);

        // A pause: ten days away keep a streak of three, and today's dictation adds to it.
        let away = [Pause {
            from: 93,
            until: Some(102),
        }];
        let paused = |day: i64| away.iter().any(|p| p.covers(day));
        assert_eq!(streaks(&days(&[90, 91, 92, 103]), 103, paused).current, 4);
        // Still running while the pause runs.
        let open = [Pause {
            from: 93,
            until: None,
        }];
        let running = |day: i64| open.iter().any(|p| p.covers(day));
        assert_eq!(streaks(&days(&[90, 91, 92]), 120, running).current, 3);
        // A pause left running ends by itself after 90 days (93 to 182): then days are missed again.
        assert_eq!(streaks(&days(&[90, 91, 92]), 184, running).current, 3);
        assert_eq!(streaks(&days(&[90, 91, 92]), 185, running).current, 0);
    }

    /// The latest streak is the last run, running or not: what the shell shows once it has ended
    /// (with the longest), so an ended streak is never shown as a loss.
    #[test]
    fn the_latest_streak_is_kept_after_it_ends() {
        let days = |d: &[i64]| d.iter().copied().collect::<BTreeSet<i64>>();
        let s = streaks(&days(&[90, 91, 92]), 100, |_| false);
        assert_eq!((s.current, s.longest, s.latest), (0, 3, 3));
        let s = streaks(
            &days(&(80..=89).chain([95, 96]).collect::<Vec<_>>()),
            100,
            |_| false,
        );
        assert_eq!((s.current, s.longest, s.latest), (0, 10, 2));
        let s = streaks(&days(&[99, 100]), 100, |_| false);
        assert_eq!((s.current, s.latest), (2, 2));
        assert_eq!(
            streaks(&BTreeSet::new(), 100, |_| false),
            Streaks::default()
        );
    }

    #[test]
    fn a_pause_runs_from_today_until_resumed_or_ninety_days() {
        let p = pause(&[], 100).unwrap();
        assert_eq!(
            p,
            [Pause {
                from: 100,
                until: None
            }]
        );
        assert_eq!(running_pause(&p, 100), Some(100));
        // Paused already: unchanged.
        assert_eq!(pause(&p, 105).unwrap(), p);
        // Resumed: the pause ends yesterday, and today counts as any day.
        let closed = resume(&p, 105);
        assert_eq!(
            closed,
            [Pause {
                from: 100,
                until: Some(104)
            }]
        );
        assert_eq!(running_pause(&closed, 105), None);
        // Resumed the day it began: as if it never was.
        assert_eq!(resume(&p, 100), []);
        // Not paused: resuming changes nothing.
        assert_eq!(resume(&closed, 106), closed);
        // Left running, it ends by itself after 90 days; a new one starts after it.
        assert_eq!(running_pause(&p, 189), Some(100));
        assert_eq!(running_pause(&p, 190), None);
        assert_eq!(
            pause(&p, 200).unwrap(),
            [
                Pause {
                    from: 100,
                    until: Some(189)
                },
                Pause {
                    from: 200,
                    until: None
                }
            ]
        );
        // At most MAX_PAUSES are kept: one more is refused, with why.
        let many: Vec<Pause> = (0..MAX_PAUSES as i64)
            .map(|i| Pause {
                from: i * 2,
                until: Some(i * 2),
            })
            .collect();
        assert!(pause(&many, 10_000).unwrap_err().contains("pauses"));
    }

    /// Pauses are kept as the user's local dates; a note that cannot be read is no pauses.
    #[test]
    fn pauses_are_kept_as_local_dates() {
        let day = |d: &str| Calendar::parse_date(d).unwrap();
        let p = vec![
            Pause {
                from: day("2026-10-01"),
                until: Some(day("2026-10-04")),
            },
            Pause {
                from: day("2026-10-10"),
                until: None,
            },
        ];
        let note = pauses_note(&p);
        assert_eq!(
            note,
            r#"[{"from":"2026-10-01","until":"2026-10-04"},{"from":"2026-10-10"}]"#
        );
        assert_eq!(pauses_from(Some(&note)), p);
        assert_eq!(pauses_from(None), []);
        assert_eq!(pauses_from(Some("nonsense")), []);
        assert_eq!(pauses_from(Some(r#"[{"from":"2026-02-30"}]"#)), []);
    }

    /// The settings shape the counts: rest days and a running pause go into the streak, and the
    /// summary says which are in force.
    #[test]
    fn the_streak_follows_the_rest_days_and_the_pause() {
        let cal = utc();
        let today = cal.day(NOON);
        // Sunday 27 and Monday 28 September, then Thursday 1 and Friday 2 October: Tuesday and
        // Wednesday missed between them, which ends a streak.
        let digests: Vec<RecordDigest> = [6, 5, 2, 1]
            .iter()
            .map(|d| dictation(NOON - d * DAY, 10, 1_000))
            .collect();
        let plain = count(&digests, &[], NOON, &cal, &Settings::default());
        assert_eq!(plain.dictation.streak_days, 2);
        assert_eq!(plain.dictation.rest_days, BTreeSet::new());
        assert_eq!(plain.dictation.streak_paused_since, None);
        // Wednesday a rest day: Tuesday is the one missed day, forgiven. A pause running since
        // yesterday (a day with a dictation, which still counts) changes nothing yet.
        let rules = Settings {
            rest_days: BTreeSet::from([3]),
            pauses: vec![Pause {
                from: today - 1,
                until: None,
            }],
            ..Settings::default()
        };
        let c = count(&digests, &[], NOON, &cal, &rules);
        assert_eq!(c.dictation.streak_days, 4);
        assert_eq!(c.dictation.rest_days, BTreeSet::from([3]));
        assert_eq!(c.dictation.streak_paused_since, Some(today - 1));
        assert_eq!(c.dictation.active_days_month, 2, "the 1st and the 2nd");
        assert_eq!(c.dictation.latest_streak_days, 4);
    }

    /// The shelf: each best is the user's own, from takes made here (an import is history from
    /// elsewhere), the earliest holding a tie; an empty library holds none (never a zero).
    #[test]
    fn bests_are_the_users_own_from_takes_made_here() {
        let cal = utc();
        let mut imported = dictation(NOON - 3 * DAY, 5_000, 600_000);
        imported.imported = true;
        let mut imported_call = meeting(NOON - 3 * DAY, 600, TranscriptDigest::default());
        imported_call.imported = true;
        let monologue = |ms| TranscriptDigest {
            longest_monologue_ms: ms,
            ..TranscriptDigest::default()
        };
        let digests = [
            // 150 wpm over 40 s.
            dictation(NOON, 100, 40_000),
            // The longest: 2 minutes, 150 wpm.
            dictation(NOON - DAY, 300, 120_000),
            // 270 wpm, but 20 s: under the floor for speed.
            dictation(NOON - 2 * DAY, 90, 20_000),
            // 160 wpm over a minute, last week: the fastest.
            dictation(NOON - 9 * DAY, 160, 60_000),
            imported,
            imported_call,
            meeting(NOON - HOUR, 30, monologue(90_000)),
            meeting(NOON - 2 * DAY, 75, monologue(45_000)),
        ];
        let c = count(&digests, &[], NOON, &cal, &Settings::default());
        let best = |id: BestId| {
            c.bests
                .iter()
                .find(|b| b.id == id)
                .map(|b| (b.value, Calendar::date(b.day), b.record.clone()))
        };
        let take = |started: i64| Some(RecordId(format!("d{started}")));
        assert_eq!(
            best(BestId::LongestDictation),
            Some((120_000, "2026-10-02".into(), take(NOON - DAY)))
        );
        assert_eq!(
            best(BestId::FastestDictation),
            Some((160, "2026-09-24".into(), take(NOON - 9 * DAY)))
        );
        // Most words in a day: the 2nd's 300 (the import's 5,000 on the 30th not counted).
        assert_eq!(
            best(BestId::MostWordsDay),
            Some((300, "2026-10-02".into(), None))
        );
        // The best week: this one, from Monday 28 September.
        assert_eq!(
            best(BestId::BestWeek),
            Some((490, "2026-09-28".into(), None))
        );
        assert_eq!(
            best(BestId::LongestMeeting),
            Some((
                75 * 60_000,
                "2026-10-01".into(),
                Some(RecordId(format!("m{}", NOON - 2 * DAY)))
            ))
        );
        assert_eq!(
            best(BestId::LongestMonologue).map(|b| (b.0, b.1)),
            Some((90_000, "2026-10-03".into()))
        );
        // In the shelf's fixed order.
        let order: Vec<BestId> = c.bests.iter().map(|b| b.id).collect();
        assert_eq!(order, BestId::ALL);

        assert!(
            count(&[], &[], NOON, &cal, &Settings::default())
                .bests
                .is_empty()
        );
        // Only short takes: no speed best at all, rather than one from a two-word take.
        let short = count(
            &[dictation(NOON, 2, 600), dictation(NOON - DAY, 40, 29_999)],
            &[],
            NOON,
            &cal,
            &Settings::default(),
        );
        assert!(!short.bests.iter().any(|b| b.id == BestId::FastestDictation));
    }

    /// News is for what just happened: a take best held by the newest take, a day best by today,
    /// a week best by this week. A tie leaves the earlier holding it.
    #[test]
    fn a_best_is_fresh_when_the_newest_take_today_or_this_week_set_it() {
        let cal = utc();
        let find = |c: &Counted, id: BestId| c.bests.iter().find(|b| b.id == id).cloned().unwrap();
        let mut digests = vec![
            dictation(NOON - DAY, 10, 50_000),
            dictation(NOON - 2 * HOUR, 10, 50_000),
        ];
        let c = count(&digests, &[], NOON, &cal, &Settings::default());
        let longest = find(&c, BestId::LongestDictation);
        assert_eq!(longest.day, cal.day(NOON - DAY), "the earlier of a tie");
        assert!(!longest.fresh);
        assert_eq!(longest.earlier, 1);
        digests.push(dictation(NOON - HOUR, 10, 60_000));
        let c = count(&digests, &[], NOON, &cal, &Settings::default());
        let longest = find(&c, BestId::LongestDictation);
        assert!(longest.fresh);
        assert_eq!(longest.earlier, 2);
        // Today's 20 words beat yesterday's 10; this week is the only week.
        let day = find(&c, BestId::MostWordsDay);
        assert_eq!((day.value, day.fresh, day.earlier), (20, true, 1));
        let week = find(&c, BestId::BestWeek);
        assert_eq!((week.value, week.fresh, week.earlier), (30, true, 0));
    }

    fn shelf_best(id: BestId, value: u64, holder: &str, fresh: bool) -> Best {
        Best {
            id,
            value,
            day: 100,
            record: Some(RecordId(holder.into())),
            fresh,
            earlier: BEST_AFTER,
        }
    }

    /// The once-a-day rule for bests, in full: a first check takes note silently; a best is news
    /// when it beats the noted value, was set by what just happened, and has at least
    /// BEST_AFTER earlier entries to beat; at most one note a day, the rest noted silently; with
    /// celebrations off it is noted and not reported; a best that fell (records deleted) is
    /// followed down.
    #[test]
    fn a_best_is_news_once_a_day_and_noted_whatever_happens() {
        let today = 100;
        let longest =
            |value, holder, fresh| shelf_best(BestId::LongestDictation, value, holder, fresh);
        let note = |shelf: &[Best], reported: Option<i64>| best_note(shelf, reported);

        // First check: nothing reported, the shelf noted.
        let (news, written) = best_news(&[longest(100, "a", true)], None, true, today);
        assert_eq!(news, None);
        assert_eq!(written, Some(note(&[longest(100, "a", true)], None)));
        // An unreadable note is a first check too.
        assert_eq!(
            best_news(&[longest(100, "a", true)], Some("{"), true, today).0,
            None
        );

        let noted = note(&[longest(100, "a", true)], None);
        // Beaten by the newest take: news, with the old value and the new.
        let (news, written) = best_news(&[longest(120, "b", true)], Some(&noted), true, today);
        assert_eq!(
            news,
            Some(BestNews {
                id: BestId::LongestDictation,
                old: 100,
                new: 120,
                day: 100,
                record: Some(RecordId("b".into())),
            })
        );
        assert_eq!(written, Some(note(&[longest(120, "b", true)], Some(today))));
        // Nothing changed: nothing reported, nothing written.
        let noted = written.unwrap();
        assert_eq!(
            best_news(&[longest(120, "b", true)], Some(&noted), true, today),
            (None, None)
        );
        // Beaten again the same day: noted, not reported (one note a day).
        let (news, written) = best_news(&[longest(130, "c", true)], Some(&noted), true, today);
        assert_eq!(news, None);
        assert_eq!(written, Some(note(&[longest(130, "c", true)], Some(today))));
        // The next day it is news again.
        let noted = written.unwrap();
        assert_eq!(
            best_news(&[longest(140, "d", true)], Some(&noted), true, today + 1)
                .0
                .map(|n| (n.old, n.new)),
            Some((130, 140))
        );

        let noted = note(&[longest(100, "a", true)], None);
        // Not set by what just happened (a calendar change, say): noted silently.
        assert_eq!(
            best_news(&[longest(120, "b", false)], Some(&noted), true, today).0,
            None
        );
        // Too few earlier takes to beat: noted silently.
        let mut early = longest(120, "b", true);
        early.earlier = BEST_AFTER - 1;
        assert_eq!(best_news(&[early], Some(&noted), true, today).0, None);
        // Celebrations off: noted, not reported.
        let (news, written) = best_news(&[longest(120, "b", true)], Some(&noted), false, today);
        assert_eq!(news, None);
        assert_eq!(written, Some(note(&[longest(120, "b", true)], None)));
        // A best held for the first time (the first meeting) is the baseline, not news.
        let meeting = shelf_best(BestId::LongestMeeting, 60_000, "m", true);
        assert_eq!(
            best_news(
                &[longest(100, "a", true), meeting.clone()],
                Some(&noted),
                true,
                today
            )
            .0,
            None
        );
        // The same holder beating itself (today's words growing after today's note): silent.
        let day_noted = note(&[shelf_best(BestId::MostWordsDay, 300, "x", true)], None);
        assert_eq!(
            best_news(
                &[shelf_best(BestId::MostWordsDay, 400, "x", true)],
                Some(&day_noted),
                true,
                today
            )
            .0,
            None
        );

        // A best that fell (its take deleted) is followed down, silently; the next to beat the
        // lower value is news against it.
        let (news, written) = best_news(&[longest(80, "z", false)], Some(&noted), true, today);
        assert_eq!(news, None);
        let noted = written.unwrap();
        assert_eq!(
            best_news(&[longest(90, "y", true)], Some(&noted), true, today)
                .0
                .map(|n| (n.old, n.new)),
            Some((80, 90))
        );
        // A best no longer held at all is dropped from the note.
        assert_eq!(
            best_news(&[], Some(&noted), true, today).1,
            Some(note(&[], None))
        );

        // Two bests at once: the first in the shelf's order is the note, the other noted.
        let both = note(
            &[
                longest(100, "a", true),
                shelf_best(BestId::FastestDictation, 150, "a", true),
            ],
            None,
        );
        let now = [
            longest(120, "b", true),
            shelf_best(BestId::FastestDictation, 170, "b", true),
        ];
        let (news, written) = best_news(&now, Some(&both), true, today);
        assert_eq!(news.map(|n| n.id), Some(BestId::LongestDictation));
        assert_eq!(written, Some(note(&now, Some(today))));
    }

    /// Last week, reviewed: gains and plain facts only. Speed is compared with the four weeks
    /// before it and given only as a gain; time saved only when there was some.
    #[test]
    fn last_weeks_review_carries_gains_and_plain_facts() {
        let cal = utc();
        // Today is Saturday 3 October: last week is 21 to 27 September, the four before it
        // 24 August to 20 September.
        let last_week = cal.day(NOON - 12 * DAY);
        assert_eq!(Calendar::date(last_week), "2026-09-21");
        let mut digests = vec![
            // Saturday 26th: 300 words in 2 minutes.
            dictation(NOON - 7 * DAY, 300, 120_000),
            // Wednesday 23rd: 200 in 1 minute. Last week: 500 in 3 minutes, 167 wpm.
            dictation(NOON - 10 * DAY, 200, 60_000),
            // Saturday 19th, in the four weeks before: 120 wpm.
            dictation(NOON - 14 * DAY, 240, 120_000),
            // This week: not last week's.
            dictation(NOON, 1_000, 60_000),
            meeting(NOON - 8 * DAY, 45, TranscriptDigest::default()),
        ];
        let state = |n: u8, done: bool| CommitmentState {
            commitment: ink_core::CommitmentId(format!("c{n}")),
            record: RecordId("m".into()),
            record_started_at_unix_ms: NOON - 8 * DAY,
            due_at_unix_ms: None,
            done,
            merged: false,
        };
        let promises = [state(1, true), state(2, false)];
        let c = count(&digests, &promises, NOON, &cal, &Settings::default());
        assert_eq!(
            c.week_review,
            Some(WeekReview {
                week: last_week,
                words: 500,
                // 500 words at 40 wpm is 12.5 minutes; 3 were spoken.
                saved_ms: Some(570_000),
                best_day: Some((cal.day(NOON - 7 * DAY), 300)),
                wpm: Some(167),
                wpm_gain: Some(47),
                meetings: 1,
                meeting_ms: 45 * 60_000,
                promises_kept: Some(1),
            })
        );

        // Slower than the four weeks before: the speed is a plain fact, and there is no gain.
        digests.push(dictation(NOON - 15 * DAY, 2_000, 120_000));
        let slower = count(&digests, &[], NOON, &cal, &Settings::default())
            .week_review
            .unwrap();
        assert_eq!((slower.wpm, slower.wpm_gain), (Some(167), None));
        assert_eq!(slower.promises_kept, None, "none kept: not said");

        // Speaking took longer than typing would have: no time saved is given, never a loss.
        let slow = count(
            &[dictation(NOON - 7 * DAY, 20, 120_000)],
            &[],
            NOON,
            &cal,
            &Settings::default(),
        )
        .week_review
        .unwrap();
        assert_eq!(
            (slow.saved_ms, slow.wpm, slow.wpm_gain),
            (None, Some(10), None)
        );

        // Dismissed for that week: gone. Dismissed for an older one: still there.
        let dismissed = |day| Settings {
            review_dismissed: Some(day),
            ..Settings::default()
        };
        assert_eq!(
            count(&digests, &[], NOON, &cal, &dismissed(last_week)).week_review,
            None
        );
        assert!(
            count(&digests, &[], NOON, &cal, &dismissed(last_week - 7))
                .week_review
                .is_some()
        );
        // A week with nothing in it has no review.
        assert_eq!(
            count(
                &[dictation(NOON, 10, 1_000)],
                &[],
                NOON,
                &cal,
                &Settings::default()
            )
            .week_review,
            None
        );
    }

    /// Time saved is about something the user can picture: the nearest whole number of a unit,
    /// only when within a fifth of the time, at most five of any but the largest; nothing for
    /// under about 12 minutes, or for time lost.
    #[test]
    fn time_saved_is_about_something_within_a_fifth() {
        let min = 60_000;
        assert_eq!(about(0), []);
        assert_eq!(about(-30 * min), []);
        assert_eq!(about(10 * min), []);
        assert_eq!(about(15 * min), [("coffee_break", 1)]);
        assert_eq!(about(60 * min), [("lunch_hour", 1), ("coffee_break", 4)]);
        assert_eq!(about(260 * min), [("feature_film", 2), ("lunch_hour", 4)]);
        assert_eq!(about(16 * 60 * min), [("working_day", 2)]);
        assert_eq!(about(400 * 60 * min), [("working_week", 10)]);
        // Every answer, from a minute to a year of hours, is within a fifth of the time saved.
        let unit = |key: &str| TIME_EQUIVALENTS.iter().find(|(k, _)| *k == key).unwrap().1;
        let mut ms = min;
        while ms < 8_760 * 60 * min {
            for (key, n) in about(ms) {
                let off = (i128::from(n) * i128::from(unit(key)) - i128::from(ms)).abs();
                assert!(off * 5 <= i128::from(ms), "{ms} ms: {n} {key}");
                assert!(n >= 1);
            }
            ms += 7 * min + 13_000;
        }
        // Saturates rather than overflows at the far end.
        assert_eq!(about(i64::MAX)[0].0, "working_week");
    }

    #[test]
    fn every_milestone_has_its_own_name() {
        let names: BTreeSet<&str> = MILESTONES.iter().map(|m| m.name).collect();
        assert_eq!(names.len(), MILESTONES.len());
        assert_eq!(
            MILESTONES.iter().map(|m| m.name).collect::<Vec<_>>(),
            [
                "first_page",
                "notebook",
                "short_novel",
                "novels_worth",
                "seven_days",
                "thirty_days",
                "hundred_days"
            ]
        );
    }

    /// With the streak hidden, stats.counted lists no streak milestone (the numbers stay, marked
    /// hidden, so a count is never shown as zero); the bests and the review are in the event.
    #[test]
    fn the_summary_carries_the_shelf_the_review_and_the_hidden_streak() {
        let cal = utc();
        let digests = [
            dictation(NOON - 7 * DAY, 400, 120_000),
            dictation(NOON, 100, 40_000),
        ];
        let hidden = Settings {
            streak_hidden: true,
            rest_days: BTreeSet::from([6, 7]),
            ..Settings::default()
        };
        let c = count(&digests, &[], NOON, &cal, &hidden);
        let v = counted_event(&c, Some("s1"));
        assert_eq!(v["dictation"]["streak_hidden"], true);
        assert_eq!(v["dictation"]["rest_days"], json!([6, 7]));
        assert!(v["dictation"].get("streak_paused_since").is_none());
        assert!(
            v["milestones"]
                .as_array()
                .unwrap()
                .iter()
                .all(|m| m["kind"] == "words" && m["name"].is_string())
        );
        assert_eq!(v["bests"][0]["id"], "longest_dictation");
        assert_eq!(v["bests"][0]["unit"], "ms");
        assert_eq!(v["bests"][0]["value"], 120_000);
        assert_eq!(v["bests"][0]["date"], "2026-09-26");
        assert_eq!(v["week_review"]["week"], "2026-09-21");
        assert_eq!(v["week_review"]["best_day"], "2026-09-26");
        assert_eq!(v["week_review"]["best_day_words"], 400);
        // 400 words at 40 wpm is 10 minutes, 2 spoken: 8 minutes saved, nothing to picture yet.
        assert!(v["week_review"].get("saved_about").is_none());
        let shown = counted_event(
            &count(&digests, &[], NOON, &cal, &Settings::default()),
            None,
        );
        assert_eq!(shown["dictation"]["streak_hidden"], false);
        assert_eq!(
            shown["milestones"].as_array().unwrap().len(),
            MILESTONES.len()
        );
    }
}
