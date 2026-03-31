use crate::AppState;
use tauri::Manager;

/// Send transcribed text to OpenClaw gateway as a chat completion.
/// Fire-and-forget: the response shows up in the user's OpenClaw session.
pub async fn send_to_openclaw(
    url: &str,
    token: &str,
    agent_id: &str,
    text: &str,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", url))
        .bearer_auth(token)
        .header("Content-Type", "application/json")
        .header("x-openclaw-agent-id", agent_id)
        .json(&serde_json::json!({
            "model": "openclaw",
            "user": "inkwell-voice",
            "messages": [{"role": "user", "content": text}]
        }))
        .send()
        .await
        .map_err(|e| format!("OpenClaw request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("OpenClaw error {}: {}", status, body));
    }

    Ok(())
}

/// Process a recording for agent mode: transcribe and send to OpenClaw.
pub fn process_agent_recording(handle: &tauri::AppHandle, samples: Vec<f32>, source_rate: usize) {
    use crate::{recording, vad};

    let app_state = handle.state::<AppState>();

    // 1. Resample
    let resampled = match recording::resample_to_16k(&samples, source_rate) {
        Ok(r) => {
            log::info!("Agent: resampled {} -> {} samples", samples.len(), r.len());
            r
        }
        Err(e) => {
            log::error!("Agent: resampling failed: {}", e);
            return;
        }
    };

    // 2. VAD
    let vad_path = app_state.vad_model_path.lock().unwrap().clone();
    let vad_threshold = app_state.settings.lock().unwrap().vad_threshold;
    let speech = if !vad_path.is_empty() && std::path::Path::new(&vad_path).exists() {
        vad::remove_silence(&resampled, &vad_path, vad_threshold).unwrap_or(resampled)
    } else {
        resampled
    };

    if speech.is_empty() {
        log::warn!("Agent: no speech detected");
        return;
    }

    // 3. Transcribe
    let text = {
        let engine_guard = app_state.engine.lock().unwrap();
        if let Some(engine) = engine_guard.as_ref() {
            match engine.transcribe(&speech) {
                Ok(t) => t,
                Err(e) => {
                    log::error!("Agent: transcription failed: {}", e);
                    return;
                }
            }
        } else {
            log::warn!("Agent: no engine loaded");
            return;
        }
    };

    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        log::info!("Agent: empty transcription, skipping");
        return;
    }

    log::info!("Agent: \"{}\"", trimmed);

    // 4. Send to OpenClaw
    let settings = app_state.settings.lock().unwrap();
    let url = settings.agent_url.clone();
    let agent_id = settings.agent_id.clone();
    drop(settings);

    // Get token: try keyring first, then settings
    let token = keyring::Entry::new("inkwell", "openclaw")
        .ok()
        .and_then(|e| e.get_password().ok())
        .filter(|k| !k.is_empty())
        .unwrap_or_else(|| {
            app_state.settings.lock().unwrap().agent_token.clone()
        });

    if token.is_empty() {
        log::error!("Agent: no OpenClaw token configured");
        let _ = tauri::Emitter::emit(handle, "agent-error", "No OpenClaw token configured. Add it in Settings > Agent.");
        return;
    }

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            log::error!("Agent: failed to create runtime: {}", e);
            return;
        }
    };

    let result = std::thread::spawn(move || {
        rt.block_on(send_to_openclaw(&url, &token, &agent_id, &trimmed))
    }).join();

    match result {
        Ok(Ok(())) => {
            log::info!("Agent: sent to OpenClaw successfully");
            let _ = tauri::Emitter::emit(handle, "agent-sent", "Message sent to OpenClaw");
        }
        Ok(Err(e)) => {
            log::error!("Agent: {}", e);
            let _ = tauri::Emitter::emit(handle, "agent-error", e.as_str());
        }
        Err(_) => {
            log::error!("Agent: send thread panicked");
        }
    }
}
