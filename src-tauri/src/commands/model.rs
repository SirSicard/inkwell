//! Model catalogue, switching, download and removal.
//!
//! All model metadata comes from `crate::models`; nothing here hardcodes a
//! model id, path or URL.

use crate::{models, AppState};
use tauri::{Emitter, Manager};

#[tauri::command]
pub fn get_model_name(state: tauri::State<AppState>) -> String {
    state.engine.name()
}

/// The model catalogue with installed state, so the UI does not keep its own
/// copy of the list (it used to, and the two drifted).
#[tauri::command]
pub fn list_models(state: tauri::State<AppState>) -> Vec<models::ModelInfo> {
    let models_dir = state.models_dir.lock().unwrap().clone();
    models::catalog(std::path::Path::new(&models_dir))
}

/// Loading an encoder takes seconds and hundreds of megabytes, so this is
/// async and does the work on a blocking task. As a sync command it ran on the
/// main thread and froze the whole UI until the model was ready.
#[tauri::command]
pub async fn switch_model(
    app: tauri::AppHandle,
    model: String,
) -> Result<String, String> {
    let spec = models::find(&model).ok_or_else(|| format!("Unknown model: {}", model))?;
    let models_dir = {
        let state = app.state::<AppState>();
        let dir = state.models_dir.lock().unwrap().clone();
        std::path::PathBuf::from(dir)
    };

    let handle = app.clone();
    let name_str = tauri::async_runtime::spawn_blocking(move || {
        handle.state::<AppState>().engine.load(spec, models_dir)
    })
    .await
    .map_err(|e| format!("Model load task failed: {}", e))??;

    let _ = app.emit("model-loaded", &name_str);
    log::info!("Switched to model: {}", name_str);

    let state = app.state::<AppState>();
    let mut settings = state.settings.lock().unwrap();
    settings.model = model;
    let path = state.settings_path.lock().unwrap().clone();
    let _ = settings.save(std::path::Path::new(&path));

    Ok(name_str)
}

/// Generic model downloader. Downloads files from HuggingFace to the models directory.
#[tauri::command]
pub async fn download_model(app: tauri::AppHandle, model_id: String) -> Result<(), String> {
    use futures_util::StreamExt;
    use std::io::Write;

    let spec = models::find_any(&model_id).ok_or_else(|| format!("Unknown model: {}", model_id))?;
    let dir_name = spec.dir;
    let files = spec.files;
    let hf_base = spec.hf_base;

    let models_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("App data dir error: {}", e))?
        .join("models")
        .join(dir_name);

    std::fs::create_dir_all(&models_dir)
        .map_err(|e| format!("Failed to create model dir: {}", e))?;

    let total_bytes: u64 = spec.total_bytes();
    let mut downloaded: u64 = 0;
    let client = reqwest::Client::new();

    for (filename, _) in files {
        let dest = models_dir.join(filename);
        if dest.exists() {
            let file_size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
            downloaded += file_size;
            let pct = (downloaded * 100 / total_bytes).min(99) as u32;
            let _ = app.emit(
                "model-download-progress",
                serde_json::json!({
                    "percent": pct, "file": filename, "model": &model_id
                }),
            );
            continue;
        }

        let url = format!("{}/{}", hf_base, filename);
        log::info!("Downloading {} -> {}", url, dest.display());

        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Download failed {}: {}", filename, e))?;

        if !resp.status().is_success() {
            return Err(format!("HTTP {} for {}", resp.status(), filename));
        }

        let mut file =
            std::fs::File::create(&dest).map_err(|e| format!("Cannot create {}: {}", filename, e))?;

        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("Stream error: {}", e))?;
            file.write_all(&chunk)
                .map_err(|e| format!("Write error: {}", e))?;
            downloaded += chunk.len() as u64;
            let pct = (downloaded * 100 / total_bytes).min(99) as u32;
            let _ = app.emit(
                "model-download-progress",
                serde_json::json!({
                    "percent": pct, "file": filename, "model": &model_id
                }),
            );
        }

        log::info!("Downloaded {}", filename);
    }

    let _ = app.emit(
        "model-download-progress",
        serde_json::json!({ "percent": 100, "file": "done", "model": &model_id }),
    );
    log::info!("Model {} download complete", model_id);
    Ok(())
}

/// Remove a downloaded model's files from disk.
#[tauri::command]
pub fn remove_model(state: tauri::State<AppState>, model_id: String) -> Result<(), String> {
    let models_dir = state.models_dir.lock().unwrap().clone();
    let models_path = std::path::Path::new(&models_dir);

    let spec = models::find_any(&model_id).ok_or_else(|| format!("Unknown model: {}", model_id))?;
    let dir_name = spec.dir;

    // Don't allow removing the currently active model. The engine reports the
    // spec's display string, so this is a direct comparison rather than the
    // per-family string rebuilding it used to do.
    let current = state.engine.name();
    if current == spec.display {
        return Err(
            "Cannot remove the currently active model. Switch to another model first.".to_string(),
        );
    }

    // The streaming model is never the *transcription* engine, so the check
    // above can never catch it, and `find_any` is what made it reachable here
    // at all. Deleting files a loaded recognizer still has open fails outright
    // on Windows and is quietly undefined everywhere else.
    if spec.id == models::STREAMING_MODEL.id && state.streaming.is_ready() {
        return Err("Turn Live Preview off before removing its model.".to_string());
    }

    let target = models_path.join(dir_name);
    if target.exists() {
        std::fs::remove_dir_all(&target)
            .map_err(|e| format!("Failed to remove {}: {}", dir_name, e))?;
        log::info!("Removed model: {} ({})", model_id, target.display());
    }

    Ok(())
}

/// Load the streaming model, or unload it and say why if it cannot be loaded.
///
/// Shared by startup and by the settings toggle so the two cannot disagree
/// about what "on" means. Blocking: callers on the main thread need a thread.
///
/// A failure here is reported and then dropped. Partials are feedback on a
/// dictation, not part of it, so a missing or half-downloaded streaming model
/// must leave a fully working app behind.
pub fn load_streaming_model(app: &tauri::AppHandle, models_dir: &std::path::Path) {
    let state = app.state::<AppState>();
    if !models::STREAMING_MODEL.is_installed(models_dir) {
        // Nothing downloaded yet. Not an error: the toggle is allowed to be on
        // before the download finishes, and the UI is what asks for it.
        log::info!("Live preview is on but its model is not downloaded yet");
        return;
    }
    match state.streaming.load(models::STREAMING_MODEL.dir_in(models_dir)) {
        Ok(()) => {
            let _ = app.emit("partials-ready", true);
        }
        Err(e) => {
            log::warn!("Live preview unavailable: {}", e);
            state.streaming.unload();
            let _ = app.emit("partials-error", e);
        }
    }
}

/// What the Live Preview setting can currently do.
///
/// The Models tab deliberately does not list the streaming model, so nothing
/// else in the UI can answer "is it downloaded". Without this the toggle would
/// have to be optimistic, and turning it on with nothing on disk would look
/// like a feature that silently does not work.
#[derive(serde::Serialize)]
pub struct PartialsStatus {
    /// Files are on disk.
    pub installed: bool,
    /// Loaded into memory and able to produce partials right now.
    pub ready: bool,
    pub model_id: &'static str,
    pub size: &'static str,
}

#[tauri::command]
pub fn get_partials_status(state: tauri::State<AppState>) -> PartialsStatus {
    let models_dir = state.models_dir.lock().unwrap().clone();
    PartialsStatus {
        installed: models::STREAMING_MODEL.is_installed(std::path::Path::new(&models_dir)),
        ready: state.streaming.is_ready(),
        model_id: models::STREAMING_MODEL.id,
        size: models::STREAMING_MODEL.size,
    }
}
