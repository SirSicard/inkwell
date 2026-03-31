use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_style")]
    pub style: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    #[serde(default = "default_recording_mode")]
    pub recording_mode: String,
    #[serde(default)]
    pub start_on_boot: bool,
    #[serde(default = "default_true")]
    pub show_overlay: bool,
    #[serde(default)]
    pub advanced_mode: bool,
    #[serde(default = "default_mic")]
    pub mic_device: String,
    #[serde(default = "default_vad_threshold")]
    pub vad_threshold: f32,
    /// Unique install ID for rate limiting the free AI Polish tier.
    /// Generated once on first run, persisted in settings.json.
    #[serde(default = "generate_install_id")]
    pub install_id: String,
    /// AI Polish: whether post-transcription LLM cleanup is enabled.
    #[serde(default)]
    pub polish_enabled: bool,
    /// AI Polish: system prompt for the LLM.
    #[serde(default = "default_polish_prompt")]
    pub polish_prompt: String,
    // Voice Agent (OpenClaw integration)
    #[serde(default)]
    pub agent_enabled: bool,
    #[serde(default = "default_agent_hotkey")]
    pub agent_hotkey: String,
    #[serde(default = "default_agent_url")]
    pub agent_url: String,
    #[serde(default = "default_agent_id")]
    pub agent_id: String,
    #[serde(default)]
    pub agent_token: String,
}

fn default_style() -> String { "formal".to_string() }
fn default_model() -> String { "parakeet".to_string() }
fn default_hotkey() -> String { "ctrl+space".to_string() }
fn default_recording_mode() -> String { "ptt".to_string() }
fn default_true() -> bool { true }
fn default_mic() -> String { "auto".to_string() }
fn default_vad_threshold() -> f32 { 0.5 }
fn default_polish_prompt() -> String { crate::llm::DEFAULT_POLISH_PROMPT.to_string() }
fn default_agent_hotkey() -> String { "ctrl+shift+space".to_string() }
fn default_agent_url() -> String { "http://127.0.0.1:41738".to_string() }
fn default_agent_id() -> String { "main".to_string() }
pub fn generate_install_id() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut hasher = DefaultHasher::new();
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos().hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    format!("ink-{:016x}", hasher.finish())
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            style: default_style(),
            model: default_model(),
            hotkey: default_hotkey(),
            recording_mode: default_recording_mode(),
            start_on_boot: false,
            show_overlay: true,
            advanced_mode: false,
            mic_device: default_mic(),
            vad_threshold: default_vad_threshold(),
            install_id: generate_install_id(),
            polish_enabled: false,
            polish_prompt: default_polish_prompt(),
            agent_enabled: false,
            agent_hotkey: default_agent_hotkey(),
            agent_url: default_agent_url(),
            agent_id: default_agent_id(),
            agent_token: String::new(),
        }
    }
}

impl Settings {
    /// Load from file, or return defaults if missing/corrupt.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => {
                match serde_json::from_str(&contents) {
                    Ok(s) => {
                        log::info!("Settings loaded from {}", path.display());
                        s
                    }
                    Err(e) => {
                        log::warn!("Settings parse error ({}), using defaults", e);
                        Self::default()
                    }
                }
            }
            Err(_) => {
                log::info!("No settings file, using defaults");
                Self::default()
            }
        }
    }

    /// Save to file.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;
        std::fs::write(path, json)
            .map_err(|e| format!("Failed to write settings: {}", e))?;
        log::info!("Settings saved to {}", path.display());
        Ok(())
    }

    /// Get the settings file path in app data dir.
    pub fn path(app_data_dir: &Path) -> PathBuf {
        app_data_dir.join("settings.json")
    }
}
