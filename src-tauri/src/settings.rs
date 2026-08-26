use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Field names that must never live in settings.json. Secrets belong in the OS
/// keyring only; these are stripped on load to clean up installs written by
/// older builds that persisted them in plaintext.
const PLAINTEXT_SECRET_KEYS: &[&str] = &[
    "agent_token",
    "api_key",
    "openai_key",
    "groq_key",
    "anthropic_key",
    "openrouter_key",
];

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
    /// "system", "light" or "dark". Shipping light mode as a bare
    /// prefers-color-scheme media query meant the app followed the OS with no
    /// way to disagree with it, which is not a preference, it is a constraint.
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Where the recording overlay sits. Wispr Flow hardcodes bottom-centre and
    /// someone shipped a whole third-party utility just to move it, which is a
    /// loud signal about a small feature.
    /// One of: top-left, top-center, top-right, bottom-left, bottom-center, bottom-right.
    #[serde(default = "default_overlay_position")]
    pub overlay_position: String,
    #[serde(default)]
    pub advanced_mode: bool,
    #[serde(default = "default_mic")]
    pub mic_device: String,
    #[serde(default = "default_vad_threshold")]
    pub vad_threshold: f32,
    /// AI Polish: whether post-transcription LLM cleanup is enabled.
    /// Polish is BYOK-only: it runs against the user's own API key.
    #[serde(default)]
    pub polish_enabled: bool,
    /// AI Polish: system prompt for the LLM.
    #[serde(default = "default_polish_prompt")]
    pub polish_prompt: String,
    // Sound feedback
    #[serde(default = "default_true")]
    pub sound_dictation: bool,
    /// Strip "um", "uh" and immediate stutters before pasting. On by default:
    /// leaving them in is the single most-cited complaint in this category, and
    /// the removal is conservative enough that it cannot change a sentence.
    #[serde(default = "default_true")]
    pub remove_fillers: bool,
    /// Opt-in only: writes each dictation's resampled audio to the temp dir for
    /// debugging. Must default to false, because this app never leaves voice on disk.
    #[serde(default)]
    pub debug_save_audio: bool,
    /// Second hotkey: hold it to speak an instruction that rewrites whatever is
    /// selected in the frontmost app. Empty disables the feature and leaves the
    /// shortcut unregistered, so it cannot collide with anything for users who
    /// do not want it.
    #[serde(default = "default_edit_hotkey")]
    pub edit_hotkey: String,
    /// Minutes of inactivity before the capture stream is dropped and the
    /// microphone released. 0 keeps the old always-on behaviour. The always-on
    /// stream held a CoreAudio power assertion (verified with
    /// `pmset -g assertions`: PreventUserIdleSystemSleep, created for
    /// Inkwell's pid), so a machine with Inkwell running never idle-slept and
    /// never auto-locked, and the orange mic indicator was on all day in an
    /// app whose whole pitch is privacy.
    #[serde(default = "default_mic_idle_release_mins")]
    pub mic_idle_release_mins: u64,
    /// Append one space after a pasted dictation, so consecutive dictations do
    /// not run together. Only dictation pastes: voice edits replace a
    /// selection exactly and must not grow it by a character.
    #[serde(default = "default_true")]
    pub append_space: bool,
    /// Show words on the overlay while you are still speaking.
    ///
    /// Off by default and it must stay that way: turning it on costs a 73 MB
    /// download and a second model decoding alongside the first. The text it
    /// draws is feedback only, never pasted and never stored, so nothing about
    /// a dictation changes when this is on except that you can watch it happen.
    #[serde(default)]
    pub show_partials: bool,
    // Which providers have a key was briefly cached here, to spare the user a
    // keychain prompt per AI-tab open. It was the wrong fix: a lookup that failed
    // for any reason other than absence got written down as "no key" forever.
    // llm::has_api_key answers it without a prompt instead, so there is nothing
    // to cache. Any leftover `configured_providers` in an existing settings.json
    // is ignored on load and dropped on the next save.
}

fn default_style() -> String { "formal".to_string() }
fn default_model() -> String { "parakeet".to_string() }
// macOS reserves ctrl+space for "Select previous input source", so a fresh Mac
// install would collide with the OS on its very first dictation.
#[cfg(target_os = "macos")]
fn default_hotkey() -> String { "super+shift+space".to_string() }
#[cfg(not(target_os = "macos"))]
fn default_hotkey() -> String { "ctrl+space".to_string() }
fn default_recording_mode() -> String { "ptt".to_string() }
fn default_true() -> bool { true }
fn default_mic() -> String { "auto".to_string() }
fn default_overlay_position() -> String { "bottom-center".to_string() }
fn default_theme() -> String { "system".to_string() }
fn default_vad_threshold() -> f32 { 0.5 }
fn default_polish_prompt() -> String { crate::llm::DEFAULT_POLISH_PROMPT.to_string() }
// Shift distinguishes it from the dictation hotkey on both platforms, and E is
// the only mnemonic that is not already spoken for by the OS.
#[cfg(target_os = "macos")]
fn default_edit_hotkey() -> String { "super+shift+e".to_string() }
#[cfg(not(target_os = "macos"))]
fn default_edit_hotkey() -> String { "ctrl+shift+e".to_string() }
fn default_mic_idle_release_mins() -> u64 { 3 }

impl Default for Settings {
    fn default() -> Self {
        Self {
            style: default_style(),
            model: default_model(),
            hotkey: default_hotkey(),
            recording_mode: default_recording_mode(),
            start_on_boot: false,
            show_overlay: true,
            theme: default_theme(),
            overlay_position: default_overlay_position(),
            advanced_mode: false,
            mic_device: default_mic(),
            vad_threshold: default_vad_threshold(),
            polish_enabled: false,
            polish_prompt: default_polish_prompt(),
            sound_dictation: true,
            remove_fillers: true,
            debug_save_audio: false,
            edit_hotkey: default_edit_hotkey(),
            mic_idle_release_mins: default_mic_idle_release_mins(),
            append_space: true,
            show_partials: false,
        }
    }
}

impl Settings {
    /// Load from file, or return defaults if missing/corrupt.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => {
                let contents = strip_plaintext_secrets(path, contents);
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

/// One-time migration: remove any secret that an older build persisted in
/// plaintext and rewrite the file so it never gets read (or leaked) again.
/// Returns the JSON to deserialize from.
fn strip_plaintext_secrets(path: &Path, contents: String) -> String {
    let mut value: serde_json::Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        Err(_) => return contents,
    };
    let obj = match value.as_object_mut() {
        Some(o) => o,
        None => return contents,
    };

    let mut stripped: Vec<&str> = Vec::new();
    for key in PLAINTEXT_SECRET_KEYS {
        if obj.remove(*key).is_some() {
            stripped.push(key);
        }
    }
    if stripped.is_empty() {
        return contents;
    }

    match serde_json::to_string_pretty(&value) {
        Ok(cleaned) => {
            match std::fs::write(path, &cleaned) {
                Ok(_) => log::info!(
                    "Settings: stripped plaintext secret field(s) {:?} from {}",
                    stripped,
                    path.display()
                ),
                Err(e) => log::warn!("Settings: could not rewrite after stripping secrets: {}", e),
            }
            cleaned
        }
        Err(_) => contents,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live settings files still carry `configured_providers` from the build that
    /// briefly cached it. Loading must ignore it rather than fail, or the upgrade
    /// resets every setting the user has.
    #[test]
    fn a_field_from_the_abandoned_cache_does_not_break_load() {
        let s: Settings =
            serde_json::from_str(r#"{"style":"casual","configured_providers":[]}"#).unwrap();
        assert_eq!(s.style, "casual");
    }

    /// A provider name is not a secret, but the key is. The stripper must not
    /// start treating the record as one, and must still strip the real thing.
    #[test]
    fn the_record_is_not_mistaken_for_a_secret() {
        let dir = std::env::temp_dir().join("inkwell-settings-strip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{"style":"casual","groq_key":"sk-leaked"}"#,
        )
        .unwrap();

        let loaded = Settings::load(&path);
        assert_eq!(loaded.style, "casual", "the stripper ate a real setting");

        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(!on_disk.contains("sk-leaked"), "plaintext key survived load");
        std::fs::remove_dir_all(&dir).ok();
    }
}
