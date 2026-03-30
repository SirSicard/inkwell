use crate::llm;
use crate::usage;
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
    log::info!("API key saved for provider: {}", provider);
    Ok(())
}

#[tauri::command]
pub fn get_api_key_status() -> serde_json::Value {
    let providers = ["openai", "groq", "anthropic", "openrouter", "custom"];
    let mut status = serde_json::Map::new();
    for p in &providers {
        let configured = keyring::Entry::new("inkwell", p)
            .ok()
            .and_then(|e| e.get_password().ok())
            .map(|k| !k.is_empty())
            .unwrap_or(false);
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
pub fn get_usage(state: tauri::State<AppState>) -> serde_json::Value {
    let usage = state.usage.lock().unwrap();
    serde_json::json!({
        "words_used": usage.words_used,
        "free_tier": usage::FREE_TIER_WORDS,
        "remaining": usage.remaining(),
        "over_limit": usage.over_limit(),
        "week_start": usage.week_start,
    })
}

#[tauri::command]
pub async fn run_ai_polish(
    state: tauri::State<'_, AppState>,
    text: String,
    provider: Option<String>,
    model: Option<String>,
) -> Result<serde_json::Value, String> {
    let prompt = state.polish_prompt.lock().unwrap().clone();
    let install_id = state.settings.lock().unwrap().install_id.clone();

    let provider = provider.unwrap_or_else(|| "proxy".to_string());

    if provider == "proxy" {
        let result = llm::call_proxy(&install_id, &text, &prompt).await?;
        let polished = result.text.unwrap_or(text);

        let mut usage = state.usage.lock().unwrap();
        usage.ensure_current();
        if let (Some(used), Some(limit)) = (result.words_used, result.limit) {
            usage.words_used = used;
            let path = state.usage_path.lock().unwrap().clone();
            let _ = usage.save(std::path::Path::new(&path));
            let remaining = limit.saturating_sub(used);
            log::info!(
                "AI Polish (proxy): {}/{} words used this week",
                used,
                limit
            );
            return Ok(
                serde_json::json!({ "text": polished, "words_used": used, "remaining": remaining }),
            );
        } else {
            usage.add_words(&polished);
            let path = state.usage_path.lock().unwrap().clone();
            let _ = usage.save(std::path::Path::new(&path));
            let words_used = usage.words_used;
            let remaining = usage.remaining();
            log::info!(
                "AI Polish (proxy, local count): {}/{} words this week",
                words_used,
                usage::FREE_TIER_WORDS
            );
            return Ok(
                serde_json::json!({ "text": polished, "words_used": words_used, "remaining": remaining }),
            );
        }
    }

    // BYOK path
    let api_key = keyring::Entry::new("inkwell", &provider)
        .map_err(|e| format!("Keyring error: {}", e))?
        .get_password()
        .map_err(|_| {
            format!(
                "No API key configured for {}. Add one in Settings → AI.",
                provider
            )
        })?;

    if api_key.is_empty() {
        return Err(format!(
            "No API key configured for {}. Add one in Settings → AI.",
            provider
        ));
    }

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
