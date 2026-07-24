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
