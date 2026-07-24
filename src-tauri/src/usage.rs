use serde::{Deserialize, Serialize};
use std::path::Path;

pub const FREE_TIER_WORDS: u32 = 4000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageData {
    pub week_start: String,  // ISO date of Monday, e.g. "2026-03-23"
    pub words_used: u32,
}

impl UsageData {
    pub fn load(path: &Path) -> Self {
        if let Ok(data) = std::fs::read_to_string(path) {
            if let Ok(usage) = serde_json::from_str::<UsageData>(&data) {
                return usage;
            }
        }
        Self::new_week()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Serialize error: {}", e))?;
        std::fs::write(path, json).map_err(|e| format!("Write error: {}", e))?;
        Ok(())
    }

    pub fn new_week() -> Self {
        Self {
            week_start: current_monday(),
            words_used: 0,
        }
    }

    /// Check if the stored week is current. If not, reset.
    pub fn ensure_current(&mut self) {
        let monday = current_monday();
        if self.week_start != monday {
            self.week_start = monday;
            self.words_used = 0;
        }
    }

    /// Count words in text and add to usage. Returns new total.
    pub fn add_words(&mut self, text: &str) -> u32 {
        let count = text.split_whitespace().count() as u32;
        self.words_used += count;
        self.words_used
    }

    /// Whether the user has exceeded the free tier.
    pub fn over_limit(&self) -> bool {
        self.words_used >= FREE_TIER_WORDS
    }

    pub fn remaining(&self) -> u32 {
        FREE_TIER_WORDS.saturating_sub(self.words_used)
    }
}

fn current_monday() -> String {
    // Calculate Monday of current ISO week from unix timestamp
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let days_since_epoch = secs / 86400;
    // 1970-01-01 was a Thursday (day 3, 0=Sunday). Adjust so Monday=0.
    // Days since Monday 1969-12-29 = days_since_epoch + 3
    let days_since_monday_epoch = days_since_epoch + 3;
    let week_day = days_since_monday_epoch % 7; // 0=Monday
    let monday_days = days_since_epoch - week_day;

    let (y, m, d) = days_to_ymd(monday_days);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    let mut remaining = days;
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if remaining < days_in_year { break; }
        remaining -= days_in_year;
        year += 1;
    }
    let months = [31u64, if is_leap(year) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u64;
    for &dim in &months {
        if remaining < dim { break; }
        remaining -= dim;
        month += 1;
    }
    (year, month, remaining + 1)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
