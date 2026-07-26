//! Transcript history: listing, search, deletion and export.

use crate::{export, history, AppState};

#[tauri::command]
pub fn get_transcripts(
    state: tauri::State<AppState>,
    limit: Option<usize>,
) -> Result<Vec<history::Transcript>, String> {
    let db_guard = state.db.lock().unwrap();
    match db_guard.as_ref() {
        Some(db) => db.recent(limit.unwrap_or(50)),
        None => Ok(vec![]),
    }
}

#[tauri::command]
pub fn search_transcripts(
    state: tauri::State<AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<history::Transcript>, String> {
    let db_guard = state.db.lock().unwrap();
    match db_guard.as_ref() {
        Some(db) => db.search(&query, limit.unwrap_or(50)),
        None => Ok(vec![]),
    }
}

#[tauri::command]
pub fn delete_transcript(state: tauri::State<AppState>, id: i64) -> Result<(), String> {
    let db_guard = state.db.lock().unwrap();
    match db_guard.as_ref() {
        Some(db) => db.delete(id),
        None => Err("No database".to_string()),
    }
}

#[tauri::command]
pub fn export_transcripts(
    state: tauri::State<AppState>,
    format: String,
    ids: Vec<i64>,
) -> Result<String, String> {
    let db_guard = state.db.lock().unwrap();
    let db = db_guard.as_ref().ok_or("Database not initialized")?;
    let transcripts = if ids.is_empty() {
        db.recent(10_000)?
    } else {
        let all = db.recent(10_000)?;
        all.into_iter().filter(|t| ids.contains(&t.id)).collect()
    };

    let content = match format.as_str() {
        "txt" => export::to_txt(&transcripts),
        "srt" => export::to_srt(&transcripts),
        "json" => export::to_json(&transcripts),
        "csv" => export::to_csv(&transcripts),
        other => return Err(format!("Unknown format: {}", other)),
    };

    log::info!(
        "Exported {} transcripts as {}",
        transcripts.len(),
        format
    );
    Ok(content)
}
