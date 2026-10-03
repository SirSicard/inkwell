//! The Stats screen: what the library says about the user's dictation and meetings, counted on
//! this computer from the store. Nothing is sent anywhere, nothing is compared with anyone else,
//! and nothing is drawn from what was said: every number is a count or a time from the record
//! digests ([`ink_core::stats`]) and the commitments' states. The one count that reads text is
//! the user's lines ending in a question mark, and it says so.
//!
//! | Command | Answer |
//! |---|---|
//! | `stats.get` | `stats.counted`: dictation, meetings, promises and milestones, on the user's calendar |
//! | `milestones.check` | `milestones.reached`: the milestones reached since the last check, each reported once ever ([`celebrations`]) |
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
//!   dictation); the shell says so rather than showing a negative time.
//! - **Streak**: local days with at least one dictation in a row, where a single missed day is
//!   forgiven and two in a row end it ([`streaks`]). It is still running while the last active
//!   day is today, yesterday, or (yesterday missed) the day before. The longest is over all time.
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
//!   ([`MILESTONES`]).
//!
//! # The user's calendar
//!
//! Days are the user's local days. The core has no time-zone database, so the shell sends its
//! zone's UTC offsets over time (`utc_offsets`: each offset from the moment it took effect, oldest
//! first) and the first day of its week (`week_start`, ISO: 1 Monday to 7 Sunday). A record falls
//! on the local day of its start, with the offset in effect then, so a daylight-saving change
//! never moves an old record across midnight. A record made while travelling is placed by the zone
//! the user is in now: the store keeps no zone per record.

use std::collections::{BTreeMap, BTreeSet};

use ink_core::stats::{CommitmentState, RecordDigest};
use ink_core::{RecordKind, Store};
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
/// The store setting that turns milestone celebrations off (`off`); on unless set.
pub const CELEBRATE_KEY: &str = "stats.celebrate";
/// The store setting noting the milestones already celebrated, or passed while celebrations were
/// off: a JSON array of ids. The core's own; no shell reads or writes it.
pub const MILESTONES_KEY: &str = "stats.milestones";
/// The store setting an import sets (`yes`) so the next milestone check notes what is reached
/// without reporting it: imported words are history, not a milestone reached now. The check
/// clears it (`no`) with its note, in one write. The core's own.
pub const MILESTONES_AFRESH_KEY: &str = "stats.milestones_afresh";
/// The least speech a window needs before it has a words-per-minute figure.
pub const WPM_MIN_SPOKEN_MS: u64 = 60_000;
/// How far back the 30-day average reaches, today included.
pub const WPM_AVERAGE_DAYS: i64 = 30;
/// Whole weeks before this one in the heatmap.
pub const HEATMAP_WEEKS_BEFORE: i64 = 11;
/// The most offsets `utc_offsets` takes: two a year for two centuries.
pub const MAX_UTC_OFFSETS: usize = 400;

const DAY_MS: i64 = 86_400_000;

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
}

/// Every milestone, smallest first within each kind.
pub const MILESTONES: &[Milestone] = &[
    Milestone {
        id: "words_1000",
        kind: MilestoneKind::Words,
        threshold: 1_000,
    },
    Milestone {
        id: "words_10000",
        kind: MilestoneKind::Words,
        threshold: 10_000,
    },
    Milestone {
        id: "words_50000",
        kind: MilestoneKind::Words,
        threshold: 50_000,
    },
    Milestone {
        id: "words_100000",
        kind: MilestoneKind::Words,
        threshold: 100_000,
    },
    Milestone {
        id: "streak_7",
        kind: MilestoneKind::Streak,
        threshold: 7,
    },
    Milestone {
        id: "streak_30",
        kind: MilestoneKind::Streak,
        threshold: 30,
    },
    Milestone {
        id: "streak_100",
        kind: MilestoneKind::Streak,
        threshold: 100,
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
}

/// The current streak and the longest, in active days, from the local days with a dictation.
///
/// Forgiving on purpose: one missed day between two active days keeps a streak going; two missed
/// days in a row end it. The current streak is the last run while its last day is today,
/// yesterday, or the day before (yesterday then being the one missed day), and 0 after that.
/// Days after today (a clock that was wrong) are not counted.
pub fn streaks(active: &BTreeSet<i64>, today: i64) -> (u64, u64) {
    let (mut run, mut longest, mut last) = (0u64, 0u64, None::<i64>);
    for &day in active.range(..=today) {
        run = match last {
            Some(prev) if day - prev <= 2 => run + 1,
            _ => 1,
        };
        longest = longest.max(run);
        last = Some(day);
    }
    let current = match last {
        Some(day) if today - day <= 2 => run,
        _ => 0,
    };
    (current, longest)
}

/// Words per minute from words and speech, rounded; `None` under [`WPM_MIN_SPOKEN_MS`].
fn wpm(words: u64, spoken_ms: u64) -> Option<u64> {
    (spoken_ms >= WPM_MIN_SPOKEN_MS)
        .then(|| (u128::from(words) * 60_000 + u128::from(spoken_ms) / 2) / u128::from(spoken_ms))
        .map(|w| u64::try_from(w).unwrap_or(u64::MAX))
}

/// Time saved, ms: `words` typed at `typing_wpm` less `spoken_ms`. Negative when speaking took
/// longer.
fn saved_ms(words: u64, spoken_ms: u64, typing_wpm: u32) -> i64 {
    let typing = i128::from(words) * 60_000 / i128::from(typing_wpm.max(1));
    let saved = typing - i128::from(spoken_ms);
    i64::try_from(saved).unwrap_or(if saved < 0 { i64::MIN } else { i64::MAX })
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

/// Counts the library at `now_unix_ms` on `calendar`.
pub fn count(
    digests: &[RecordDigest],
    commitments: &[CommitmentState],
    now_unix_ms: i64,
    calendar: &Calendar,
    typing_wpm: u32,
) -> Counted {
    let today = calendar.day(now_unix_ms);
    let week = calendar.week_start_of(today);
    let month = Calendar::month_start_of(today);
    let average_from = today - (WPM_AVERAGE_DAYS - 1);
    let heatmap_first_day = week - HEATMAP_WEEKS_BEFORE * 7;

    let mut d = Dictation {
        heatmap_first_day,
        heatmap_words: vec![0; usize::try_from(today - heatmap_first_day + 1).unwrap_or(0)],
        ..Dictation::default()
    };
    let mut per_day: BTreeMap<i64, u64> = BTreeMap::new();
    // (words, spoken ms) of timed dictations: this week, the last 30 days, all time.
    let (mut timed_week, mut timed_average, mut timed_all) = ((0, 0), (0, 0), (0u64, 0u64));
    let (mut meetings_month, mut meetings_all) = (Meetings::default(), Meetings::default());

    for r in digests {
        // A record still live (or a dictation never finished) counts nothing yet.
        if r.ended_at_unix_ms.is_none() {
            continue;
        }
        let day = calendar.day(r.started_at_unix_ms);
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
                }
            }
            RecordKind::Meeting if !r.imported => {
                let add = |m: &mut Meetings| {
                    m.meetings += 1;
                    m.recorded_ms += r.ended_at_unix_ms.map_or(0, |end| {
                        end.saturating_sub(r.started_at_unix_ms).max(0) as u64
                    });
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
    (d.streak_days, d.longest_streak_days) = streaks(&active, today);
    d.wpm_week = wpm(timed_week.0, timed_week.1);
    d.wpm_average = wpm(timed_average.0, timed_average.1);
    d.saved_ms_week = saved_ms(timed_week.0, timed_week.1, typing_wpm);
    d.saved_ms_all = saved_ms(timed_all.0, timed_all.1, typing_wpm);

    let (mut promises_month, mut promises_all) = (Promises::default(), Promises::default());
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
        if (month..=today).contains(&calendar.day(c.record_started_at_unix_ms)) {
            add(&mut promises_month);
        }
    }

    Counted {
        today,
        typing_wpm,
        dictation: d,
        meetings_month,
        meetings_all,
        promises_month,
        promises_all,
    }
}

/// A stats command, read. Both count the library on the user's calendar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatsQuery {
    /// `stats.get`: everything the screen shows.
    Get {
        /// The user's calendar.
        calendar: Calendar,
    },
    /// `milestones.check`: what is newly reached, reported once.
    CheckMilestones {
        /// The user's calendar (a streak counts local days).
        calendar: Calendar,
    },
}

/// Reads `v` as a stats command when `name` is one: `None` for any other command.
pub fn parse(name: &str, v: &Value) -> Option<Result<StatsQuery, String>> {
    match name {
        "stats.get" => Some(parse_calendar(name, v).map(|calendar| StatsQuery::Get { calendar })),
        "milestones.check" => {
            Some(parse_calendar(name, v).map(|calendar| StatsQuery::CheckMilestones { calendar }))
        }
        _ => None,
    }
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

/// **Worker** (the screens' thread). Counts the library and answers `stats.counted` or
/// `milestones.reached`, echoing `id` as `ref`; or why it could not.
pub fn answer(shared: &Shared, query: StatsQuery, id: Option<&str>) -> Result<Value, String> {
    let store = shared.store.as_ref();
    let e = |err: ink_core::StoreError| err.to_string();
    let calendar = match &query {
        StatsQuery::Get { calendar } | StatsQuery::CheckMilestones { calendar } => calendar,
    };
    let counted = count(
        &store.digests().map_err(e)?,
        &store.commitment_states().map_err(e)?,
        shared.clock.unix_ms(),
        calendar,
        typing_wpm(store)?,
    );
    if let StatsQuery::Get { .. } = query {
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
    let (report, note) = celebrations(&reached, noted.as_deref(), celebrate, afresh);
    // Noted before it is reported: a note that fails to save fails the check, so a milestone is
    // never celebrated without being remembered.
    match (note, afresh) {
        (Some(note), true) => store
            .set_settings(&[(MILESTONES_KEY, &note), (MILESTONES_AFRESH_KEY, "no")])
            .map_err(e)?,
        (Some(note), false) => store.set_setting(MILESTONES_KEY, &note).map_err(e)?,
        (None, true) => store.set_setting(MILESTONES_AFRESH_KEY, "no").map_err(e)?,
        (None, false) => {}
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
    dictation.insert(
        "heatmap_first_day".into(),
        Calendar::date(d.heatmap_first_day).into(),
    );
    dictation.insert("heatmap_words".into(), json!(d.heatmap_words));
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
            (
                "milestones",
                Some(Value::Array(
                    MILESTONES
                        .iter()
                        .map(|m| milestone(m, c.reached(m)))
                        .collect(),
                )),
            ),
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
        let days = |d: &[i64]| d.iter().copied().collect::<BTreeSet<i64>>();
        assert_eq!(streaks(&days(&[]), 100), (0, 0));
        assert_eq!(streaks(&days(&[100]), 100), (1, 1));
        assert_eq!(streaks(&days(&[99, 100]), 100), (2, 2));
        // Yesterday counts while today is not over.
        assert_eq!(streaks(&days(&[98, 99]), 100), (2, 2));
        // One missed day (99) is forgiven: 97, 98 and 100 are one streak of three days.
        assert_eq!(streaks(&days(&[97, 98, 100]), 100), (3, 3));
        // Yesterday missed, the day before active: still running today.
        assert_eq!(streaks(&days(&[97, 98]), 100), (2, 2));
        // Two missed days end it: 96 and 97 were a streak of two, and nothing runs now.
        assert_eq!(streaks(&days(&[96, 97]), 100), (0, 2));
        // Two missed days between runs: a new streak; the longest is kept.
        assert_eq!(streaks(&days(&[90, 91, 92, 93, 96, 97]), 97), (2, 4));
        // Every other day is a streak: a single missed day at a time.
        assert_eq!(streaks(&days(&[94, 96, 98, 100]), 100), (4, 4));
        // A day after today (a wrong clock) is not counted.
        assert_eq!(streaks(&days(&[99, 100, 101]), 100), (2, 2));
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
        let c = count(&digests, &[], NOON, &cal, 40);
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
            40,
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
        let c = count(&digests, &[], NOON, &cal, 40);
        assert_eq!(c.dictation.wpm_week, Some(150));
        // 900 words in 8 minutes = 112.5, rounded.
        assert_eq!(c.dictation.wpm_average, Some(113));

        // Under a minute of speech: no figure.
        let short = count(&[dictation(NOON, 50, 59_999)], &[], NOON, &cal, 40);
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
        let at40 = count(&digests, &[], NOON, &cal, 40);
        assert_eq!(at40.dictation.saved_ms_week, 8 * 60_000);
        assert_eq!(at40.dictation.saved_ms_all, 8 * 60_000);
        assert_eq!(at40.typing_wpm, 40);
        // At 80 wpm it takes 5: 3 saved.
        let at80 = count(&digests, &[], NOON, &cal, 80);
        assert_eq!(at80.dictation.saved_ms_all, 3 * 60_000);
        // Slow dictation saves nothing: it is negative, and said so by the shell.
        let slow = count(&[dictation(NOON, 20, 120_000)], &[], NOON, &cal, 40);
        assert_eq!(slow.dictation.saved_ms_all, 30_000 - 120_000);
        // Last week's words are saved time all time, not this week.
        let old = count(
            &[dictation(NOON - 7 * DAY, 400, 120_000)],
            &[],
            NOON,
            &cal,
            40,
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
        let c = count(&digests, &[], NOON, &cal, 40);
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
        let c = count(&digests, &[], NOON, &plus2, 40);
        assert_eq!(c.dictation.words_today, 5);
        assert_eq!(c.dictation.streak_days, 2);
        // In UTC the same two are the 2nd and the 1st: still a streak (yesterday and before),
        // and nothing today.
        let u = count(&digests, &[], NOON, &utc(), 40);
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
        let c = count(&digests, &[], NOON, &utc(), 40);
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
        let c = count(&[], &states, NOON, &cal, 40);
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
        let c = count(&digests, &[], NOON, &cal, 40);
        let reached: Vec<&str> = MILESTONES
            .iter()
            .filter(|m| c.reached(m))
            .map(|m| m.id)
            .collect();
        assert_eq!(reached, ["words_1000", "streak_7"]);
        digests.push(dictation(NOON - 400 * DAY, 99_000, 1_000));
        let c = count(&digests, &[], NOON, &cal, 40);
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
}
