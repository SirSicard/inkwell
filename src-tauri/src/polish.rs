use crate::llm;
use crate::AppState;

/// First provider with a key, in `llm::PROVIDERS` preference order rather than
/// the order the user happened to save them in.
///
/// Asks the keychain each time rather than caching. A cache existed here briefly
/// and was wrong within minutes: a lookup that failed for any reason other than
/// absence, such as the user dismissing the authorization dialog, was recorded
/// as "no key configured" and never revisited. Since the existence check no
/// longer costs a prompt, there is nothing left to cache and nothing to go stale.
pub fn preferred_provider() -> Option<String> {
    preferred_from(&llm::configured_providers())
}

/// Split out so the ordering rule can be tested without touching a keychain.
fn preferred_from(configured: &[String]) -> Option<String> {
    llm::PROVIDERS
        .iter()
        .find(|p| configured.iter().any(|c| c == *p))
        .map(|p| p.to_string())
}

#[tauri::command]
pub fn save_api_key(provider: String, key: String) -> Result<(), String> {
    let entry = keyring::Entry::new(llm::KEYRING_SERVICE, &provider)
        .map_err(|e| format!("Keyring error: {}", e))?;
    if key.is_empty() {
        let _ = entry.delete_credential();
    } else {
        entry
            .set_password(&key)
            .map_err(|e| format!("Failed to save key: {}", e))?;
    }
    // Keyring only: an API key is never written to settings.json.
    log::info!(
        "API key {} for provider: {}",
        if key.is_empty() { "cleared" } else { "saved" },
        provider
    );
    Ok(())
}

#[tauri::command]
pub fn get_api_key_status() -> serde_json::Value {
    let configured = llm::configured_providers();
    let mut status = serde_json::Map::new();
    for p in llm::PROVIDERS {
        status.insert(
            p.to_string(),
            serde_json::Value::Bool(configured.iter().any(|c| c == p)),
        );
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
        .or_else(preferred_provider)
        .ok_or("No API key configured. Add one in Settings → AI.")?;

    // The one place the secret is genuinely needed, so the one place worth a
    // keychain authorization prompt.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn preference_order_beats_save_order() {
        // Saved groq first, then openai. openai is earlier in PROVIDERS, so it wins.
        assert_eq!(
            preferred_from(&v(&["groq", "openai"])),
            Some("openai".to_string())
        );
    }

    #[test]
    fn single_provider_is_chosen_whatever_it_is() {
        assert_eq!(preferred_from(&v(&["groq"])), Some("groq".to_string()));
        assert_eq!(preferred_from(&v(&["custom"])), Some("custom".to_string()));
    }

    #[test]
    fn no_providers_means_no_polish() {
        assert_eq!(preferred_from(&[]), None);
    }

    #[test]
    fn unknown_provider_names_are_ignored() {
        // A hand-edited settings.json must not be able to name a provider that
        // build_provider cannot construct.
        assert_eq!(preferred_from(&v(&["not-a-provider"])), None);
    }

}
