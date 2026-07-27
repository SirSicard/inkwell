use crate::llm;
use crate::AppState;

/// Which providers have a key, answered without touching the keychain.
///
/// macOS asks the user to authorize every *read* of a keychain item, and the only
/// read primitive keyring exposes returns the secret itself. Answering the
/// boolean "is a key set?" by fetching the key therefore put a system dialog
/// asking for "confidential information stored in inkwell" in front of the AI
/// tab on every single open, with nothing on screen explaining why.
///
/// Which providers are configured is not itself a secret, so it lives in
/// settings.json. The keychain is now read only when a key is about to be used.
fn known_providers(state: &AppState) -> Vec<String> {
    // Cloned out before the branch so the guard is dropped: set_known_providers
    // takes the same lock.
    let recorded = state.settings.lock().unwrap().configured_providers.clone();
    if let Some(list) = recorded {
        return list;
    }
    // No record yet, which is what an install from before this field looks like.
    // The keyring is then the only source of truth, so pay for one probe and
    // write the answer down so it is never paid again.
    let found = llm::probe_configured_providers();
    log::info!("Reconciled API key status from keyring: {:?}", found);
    set_known_providers(state, found.clone());
    found
}

fn set_known_providers(state: &AppState, providers: Vec<String>) {
    let mut settings = state.settings.lock().unwrap();
    settings.configured_providers = Some(providers);
    let path = state.settings_path.lock().unwrap().clone();
    let _ = settings.save(std::path::Path::new(&path));
}

/// Stop claiming a provider has a key. Called when the keychain turns out to
/// disagree with the record, so the two cannot stay out of step.
fn forget_provider(state: &AppState, provider: &str) {
    let providers = with_provider(&known_providers(state), provider, false);
    log::warn!(
        "No keyring entry for {} despite settings recording one; clearing the record",
        provider
    );
    set_known_providers(state, providers);
}

/// First provider with a key, in `llm::PROVIDERS` preference order rather than
/// the order the user happened to save them in.
pub fn preferred_provider(state: &AppState) -> Option<String> {
    preferred_from(&known_providers(state))
}

/// Split out from `preferred_provider` so the ordering rule can be tested
/// without an AppState, which needs a running Tauri app to build.
fn preferred_from(configured: &[String]) -> Option<String> {
    llm::PROVIDERS
        .iter()
        .find(|p| configured.iter().any(|c| c == *p))
        .map(|p| p.to_string())
}

/// The record after saving or clearing a key for `provider`. Pure so the
/// add/remove semantics are pinned down: saving twice must not list a provider
/// twice, and clearing must actually remove it.
fn with_provider(current: &[String], provider: &str, configured: bool) -> Vec<String> {
    let mut next: Vec<String> = current.iter().filter(|p| *p != provider).cloned().collect();
    if configured {
        next.push(provider.to_string());
    }
    next
}

#[tauri::command]
pub fn save_api_key(
    state: tauri::State<AppState>,
    provider: String,
    key: String,
) -> Result<(), String> {
    let entry = keyring::Entry::new("inkwell", &provider)
        .map_err(|e| format!("Keyring error: {}", e))?;
    let configured = if key.is_empty() {
        let _ = entry.delete_credential();
        false
    } else {
        entry
            .set_password(&key)
            .map_err(|e| format!("Failed to save key: {}", e))?;
        true
    };

    let providers = with_provider(&known_providers(&state), &provider, configured);
    set_known_providers(&state, providers);

    // Keyring only: an API key is never written to settings.json. Only the fact
    // that one exists is recorded there.
    log::info!(
        "API key {} for provider: {}",
        if configured { "saved" } else { "cleared" },
        provider
    );
    Ok(())
}

#[tauri::command]
pub fn get_api_key_status(state: tauri::State<AppState>) -> serde_json::Value {
    let configured = known_providers(&state);
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
        .or_else(|| preferred_provider(&state))
        .ok_or("No API key configured. Add one in Settings → AI.")?;

    // The one place the secret is genuinely needed, so the one place worth a
    // keychain authorization prompt.
    let api_key = match llm::api_key_for(&provider) {
        Some(k) => k,
        None => {
            // The record said this provider had a key and the keychain says
            // otherwise: the item was deleted outside the app, or settings.json
            // arrived from another machine. Believe the keychain.
            forget_provider(&state, &provider);
            return Err(format!(
                "No API key configured for {}. Add one in Settings → AI.",
                provider
            ));
        }
    };

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

    #[test]
    fn saving_twice_does_not_duplicate() {
        let once = with_provider(&[], "groq", true);
        let twice = with_provider(&once, "groq", true);
        assert_eq!(twice, v(&["groq"]));
    }

    #[test]
    fn clearing_removes_only_that_provider() {
        let both = with_provider(&with_provider(&[], "groq", true), "openai", true);
        assert_eq!(with_provider(&both, "groq", false), v(&["openai"]));
    }

    #[test]
    fn clearing_an_absent_provider_is_a_no_op() {
        let one = v(&["groq"]);
        assert_eq!(with_provider(&one, "openai", false), one);
    }
}
