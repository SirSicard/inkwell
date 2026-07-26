use crate::llm;
use crate::AppState;

#[tauri::command]
pub fn save_api_key(provider: String, key: String) -> Result<(), String> {
    let entry = keyring::Entry::new("inkwell", &provider)
        .map_err(|e| format!("Keyring error: {}", e))?;
    if key.is_empty() {
        let _ = entry.delete_credential();
    } else {
        entry
            .set_password(&key)
            .map_err(|e| format!("Failed to save key: {}", e))?;
    }
    // Keyring only: an API key is never written to settings.json.
    log::info!("API key saved for provider: {}", provider);
    Ok(())
}

#[tauri::command]
pub fn get_api_key_status() -> serde_json::Value {
    let mut status = serde_json::Map::new();
    for p in llm::PROVIDERS {
        let configured = llm::api_key_for(p).is_some();
        status.insert(p.to_string(), serde_json::Value::Bool(configured));
    }
    serde_json::Value::Object(status)
}

#[tauri::command]
pub fn get_polish_settings(state: tauri::State<AppState>) -> serde_json::Value {
    let enabled = *state.polish_enabled.lock().unwrap();
    let prompt = state.polish_prompt.lock().unwrap().clone();
    serde_json::json!({ "enabled": enabled, "prompt": prompt })
}

#[tauri::command]
pub fn set_polish_settings(state: tauri::State<AppState>, enabled: bool, prompt: String) {
    *state.polish_enabled.lock().unwrap() = enabled;
    *state.polish_prompt.lock().unwrap() = prompt.clone();

    let mut settings = state.settings.lock().unwrap();
    settings.polish_enabled = enabled;
    settings.polish_prompt = prompt;
    let path = state.settings_path.lock().unwrap().clone();
    let _ = settings.save(std::path::Path::new(&path));
    log::info!("Polish settings saved: enabled={}", enabled);
}

#[tauri::command]
pub async fn run_ai_polish(
    state: tauri::State<'_, AppState>,
    text: String,
    provider: Option<String>,
    model: Option<String>,
) -> Result<serde_json::Value, String> {
    let prompt = state.polish_prompt.lock().unwrap().clone();

    let provider = provider
        .or_else(llm::first_configured_provider)
        .ok_or("No API key configured. Add one in Settings → AI.")?;

    let api_key = llm::api_key_for(&provider).ok_or_else(|| {
        format!(
            "No API key configured for {}. Add one in Settings → AI.",
            provider
        )
    })?;

    let cfg = llm::ProviderConfig {
        provider: provider.clone(),
        api_key,
        custom_url: None,
        model,
    };

    let llm_provider = llm::build_provider(cfg);
    let result = llm_provider.complete(&prompt, &text).await?;

    log::info!(
        "AI Polish (BYOK {}) {} chars -> {} chars",
        provider,
        text.len(),
        result.text.len()
    );
    Ok(serde_json::json!({ "text": result.text }))
}
