use rusqlite::{Connection, params};
use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize)]
pub struct Transcript {
    pub id: i64,
    pub text: String,
    pub raw_text: String,
    pub style: String,
    pub model: String,
    pub audio_duration_ms: i64,
    pub created_at: String,
}

pub struct TranscriptDb {
    conn: Mutex<Connection>,
}

impl TranscriptDb {
    /// Open or create the transcript database.
    pub fn open(db_path: &Path) -> Result<Self, String> {
        let conn = Connection::open(db_path)
            .map_err(|e| format!("Failed to open DB: {}", e))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS transcripts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                raw_text TEXT NOT NULL,
                style TEXT NOT NULL DEFAULT 'formal',
                model TEXT NOT NULL DEFAULT 'unknown',
                audio_duration_ms INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
            );
            CREATE INDEX IF NOT EXISTS idx_created ON transcripts(created_at DESC);"
        ).map_err(|e| format!("Failed to create table: {}", e))?;

        log::info!("Transcript DB opened: {}", db_path.display());
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Insert a new transcript. Returns the row ID.
    pub fn insert(
        &self,
        text: &str,
        raw_text: &str,
        style: &str,
        model: &str,
        audio_duration_ms: i64,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO transcripts (text, raw_text, style, model, audio_duration_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![text, raw_text, style, model, audio_duration_ms],
        ).map_err(|e| format!("Insert failed: {}", e))?;

        let id = conn.last_insert_rowid();
        log::info!("Transcript saved (id={})", id);
        Ok(id)
    }

    /// Get recent transcripts, newest first.
    pub fn recent(&self, limit: usize) -> Result<Vec<Transcript>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, text, raw_text, style, model, audio_duration_ms, created_at
             FROM transcripts ORDER BY id DESC LIMIT ?1"
        ).map_err(|e| format!("Query failed: {}", e))?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(Transcript {
                id: row.get(0)?,
                text: row.get(1)?,
                raw_text: row.get(2)?,
                style: row.get(3)?,
                model: row.get(4)?,
                audio_duration_ms: row.get(5)?,
                created_at: row.get(6)?,
            })
        }).map_err(|e| format!("Query map failed: {}", e))?;

        let mut transcripts = Vec::new();
        for row in rows {
            transcripts.push(row.map_err(|e| format!("Row read failed: {}", e))?);
        }
        Ok(transcripts)
    }

    /// Search transcripts by text content.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Transcript>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, text, raw_text, style, model, audio_duration_ms, created_at
             FROM transcripts WHERE text LIKE ?1 ORDER BY id DESC LIMIT ?2"
        ).map_err(|e| format!("Search query failed: {}", e))?;

        let pattern = format!("%{}%", query);
        let rows = stmt.query_map(params![pattern, limit as i64], |row| {
            Ok(Transcript {
                id: row.get(0)?,
                text: row.get(1)?,
                raw_text: row.get(2)?,
                style: row.get(3)?,
                model: row.get(4)?,
                audio_duration_ms: row.get(5)?,
                created_at: row.get(6)?,
            })
        }).map_err(|e| format!("Search map failed: {}", e))?;

        let mut transcripts = Vec::new();
        for row in rows {
            transcripts.push(row.map_err(|e| format!("Row read failed: {}", e))?);
        }
        Ok(transcripts)
    }

    /// Delete a transcript by ID.
    pub fn delete(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM transcripts WHERE id = ?1", params![id])
            .map_err(|e| format!("Delete failed: {}", e))?;
        log::info!("Transcript deleted (id={})", id);
        Ok(())
    }
}

/// Aggregates for the Stats view.
///
/// Everything here is derived from the transcripts table at ask time; nothing
/// is counted separately or stored elsewhere. That is a privacy decision as
/// much as a simplicity one: the delete button in the history is authoritative,
/// so deleting transcripts genuinely deletes them from the statistics too,
/// rather than leaving a running total that remembers what the rows said goodbye to.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Stats {
    pub total_count: i64,
    pub total_words: i64,
    /// Sum of recorded speech, in milliseconds of key-held time.
    pub total_speaking_ms: i64,
    /// Distinct calendar days with at least one dictation.
    pub days_active: i64,
    /// Consecutive days ending today (or yesterday, so an unopened morning
    /// does not read as a broken streak).
    pub streak_days: i64,
    /// (date, count) for the busiest single day.
    pub best_day: Option<(String, i64)>,
    /// Last 14 calendar days, oldest first, zero-filled: (date, count, words).
    pub recent_days: Vec<(String, i64, i64)>,
    /// (model, count), most used first.
    pub per_model: Vec<(String, i64)>,
}

fn word_count(text: &str) -> i64 {
    text.split_whitespace().count() as i64
}

impl TranscriptDb {
    /// Compute all stats in one table scan. Row counts here are thousands at
    /// most, so a fold in Rust beats pushing word-splitting into SQL.
    pub fn stats(&self, today: chrono::NaiveDate) -> Result<Stats, String> {
        use std::collections::BTreeMap;

        let conn = self.conn.lock().map_err(|_| "DB lock poisoned".to_string())?;
        let mut stmt = conn
            .prepare("SELECT text, model, audio_duration_ms, substr(created_at, 1, 10) FROM transcripts")
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;

        let mut stats = Stats::default();
        let mut by_day: BTreeMap<String, (i64, i64)> = BTreeMap::new(); // date -> (count, words)
        let mut by_model: BTreeMap<String, i64> = BTreeMap::new();

        for row in rows {
            let (text, model, duration_ms, date) = row.map_err(|e| e.to_string())?;
            let words = word_count(&text);
            stats.total_count += 1;
            stats.total_words += words;
            stats.total_speaking_ms += duration_ms.max(0);
            let d = by_day.entry(date).or_insert((0, 0));
            d.0 += 1;
            d.1 += words;
            *by_model.entry(model).or_insert(0) += 1;
        }

        stats.days_active = by_day.len() as i64;
        stats.best_day = by_day
            .iter()
            .max_by_key(|(date, (count, _))| (*count, std::cmp::Reverse((*date).clone())))
            .map(|(date, (count, _))| (date.clone(), *count));

        // Streak: walk backward from today; a quiet today defers to yesterday
        // before conceding, so the streak survives until a full day is missed.
        let mut cursor = today;
        if !by_day.contains_key(&cursor.to_string()) {
            cursor = cursor - chrono::Days::new(1);
        }
        while by_day.contains_key(&cursor.to_string()) {
            stats.streak_days += 1;
            cursor = cursor - chrono::Days::new(1);
        }

        // Last 14 days, zero-filled so the strip has a bar slot per day.
        for offset in (0..14).rev() {
            let date = (today - chrono::Days::new(offset)).to_string();
            let (count, words) = by_day.get(&date).copied().unwrap_or((0, 0));
            stats.recent_days.push((date, count, words));
        }

        let mut models: Vec<(String, i64)> = by_model.into_iter().collect();
        models.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        stats.per_model = models;

        Ok(stats)
    }
}

#[cfg(test)]
mod stats_tests {
    use super::*;

    fn db_with(rows: &[(&str, &str, i64, &str)]) -> TranscriptDb {
        let db = TranscriptDb::open(Path::new(":memory:")).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            for (text, model, ms, created) in rows {
                conn.execute(
                    "INSERT INTO transcripts (text, raw_text, style, model, audio_duration_ms, created_at)
                     VALUES (?1, ?1, 'formal', ?2, ?3, ?4)",
                    params![text, model, ms, created],
                )
                .unwrap();
            }
        }
        db
    }

    fn day(s: &str) -> chrono::NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn empty_db_is_all_zeroes_with_a_full_zero_strip() {
        let s = db_with(&[]).stats(day("2026-08-09")).unwrap();
        assert_eq!(s.total_count, 0);
        assert_eq!(s.streak_days, 0);
        assert_eq!(s.best_day, None);
        assert_eq!(s.recent_days.len(), 14);
        assert!(s.recent_days.iter().all(|(_, c, w)| *c == 0 && *w == 0));
        assert_eq!(s.recent_days[13].0, "2026-08-09");
        assert_eq!(s.recent_days[0].0, "2026-07-27");
    }

    #[test]
    fn totals_and_model_split() {
        let s = db_with(&[
            ("one two three", "Parakeet V2", 2000, "2026-08-09 10:00:00"),
            ("four five", "Qwen3 ASR", 3000, "2026-08-09 11:00:00"),
            ("six", "Parakeet V2", 1000, "2026-08-08 09:00:00"),
        ])
        .stats(day("2026-08-09"))
        .unwrap();
        assert_eq!(s.total_count, 3);
        assert_eq!(s.total_words, 6);
        assert_eq!(s.total_speaking_ms, 6000);
        assert_eq!(s.days_active, 2);
        assert_eq!(s.per_model, vec![("Parakeet V2".into(), 2), ("Qwen3 ASR".into(), 1)]);
    }

    #[test]
    fn streak_counts_back_from_today() {
        let s = db_with(&[
            ("a", "m", 0, "2026-08-09 10:00:00"),
            ("b", "m", 0, "2026-08-08 10:00:00"),
            ("c", "m", 0, "2026-08-07 10:00:00"),
            // gap on the 6th
            ("d", "m", 0, "2026-08-05 10:00:00"),
        ])
        .stats(day("2026-08-09"))
        .unwrap();
        assert_eq!(s.streak_days, 3);
    }

    #[test]
    fn quiet_morning_defers_to_yesterday_instead_of_breaking() {
        let s = db_with(&[
            ("a", "m", 0, "2026-08-08 10:00:00"),
            ("b", "m", 0, "2026-08-07 10:00:00"),
        ])
        .stats(day("2026-08-09"))
        .unwrap();
        assert_eq!(s.streak_days, 2);
    }

    #[test]
    fn full_missed_day_breaks_the_streak() {
        let s = db_with(&[("a", "m", 0, "2026-08-07 10:00:00")])
            .stats(day("2026-08-09"))
            .unwrap();
        assert_eq!(s.streak_days, 0);
    }

    #[test]
    fn best_day_ties_go_to_the_earlier_date() {
        let s = db_with(&[
            ("a", "m", 0, "2026-08-08 10:00:00"),
            ("b", "m", 0, "2026-08-08 11:00:00"),
            ("c", "m", 0, "2026-08-09 10:00:00"),
            ("d", "m", 0, "2026-08-09 11:00:00"),
        ])
        .stats(day("2026-08-09"))
        .unwrap();
        assert_eq!(s.best_day, Some(("2026-08-08".into(), 2)));
    }
}
