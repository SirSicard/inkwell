pub mod agent;
mod appdetect;
mod audio;
mod commands;
pub mod dictionary;
mod engine;
pub mod export;
mod filetranscribe;
pub mod history;
mod llm;
mod overlay;
mod paste;
mod pipeline;
mod polish;
pub mod recording;
mod settings;
mod setup;
pub mod snippets;
pub mod style;
mod tray;
pub mod usage;
mod vad;
pub mod voicecommand;

use audio::AudioState;
use engine::SpeechEngine;
use std::sync::Mutex;

pub struct AppState {
    pub audio: Mutex<Option<AudioState>>,
    pub engine: Mutex<Option<SpeechEngine>>,
    pub models_dir: Mutex<String>,
    pub vad_model_path: Mutex<String>,
    pub model_name: Mutex<String>,
    pub style: Mutex<style::Style>,
    pub settings: Mutex<settings::Settings>,
    pub settings_path: Mutex<String>,
    pub db: Mutex<Option<history::TranscriptDb>>,
    pub dict: Mutex<dictionary::Dictionary>,
    pub dict_path: Mutex<String>,
    pub is_first_run: Mutex<bool>,
    // AI Polish
    pub polish_enabled: Mutex<bool>,
    pub polish_prompt: Mutex<String>,
    pub usage: Mutex<usage::UsageData>,
    pub usage_path: Mutex<String>,
    // Snippets
    pub snippet_store: Mutex<snippets::SnippetStore>,
    pub snippets_path: Mutex<String>,
    // Per-app styles
    pub app_styles: Mutex<appdetect::AppStyleRules>,
    pub app_styles_path: Mutex<String>,
    // Voice commands
    pub voice_commands: Mutex<voicecommand::VoiceCommandStore>,
    pub voice_commands_path: Mutex<String>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            audio: Mutex::new(None),
            engine: Mutex::new(None),
            models_dir: Mutex::new(String::new()),
            vad_model_path: Mutex::new(String::new()),
            model_name: Mutex::new("Loading...".to_string()),
            style: Mutex::new(style::Style::default()),
            settings: Mutex::new(settings::Settings::default()),
            settings_path: Mutex::new(String::new()),
            db: Mutex::new(None),
            dict: Mutex::new(dictionary::Dictionary::default()),
            dict_path: Mutex::new(String::new()),
            is_first_run: Mutex::new(false),
            polish_enabled: Mutex::new(false),
            polish_prompt: Mutex::new(llm::DEFAULT_POLISH_PROMPT.to_string()),
            usage: Mutex::new(usage::UsageData::new_week()),
            usage_path: Mutex::new(String::new()),
            snippet_store: Mutex::new(snippets::SnippetStore::default()),
            snippets_path: Mutex::new(String::new()),
            app_styles: Mutex::new(appdetect::AppStyleRules::default()),
            app_styles_path: Mutex::new(String::new()),
            voice_commands: Mutex::new(voicecommand::VoiceCommandStore::default()),
            voice_commands_path: Mutex::new(String::new()),
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_fs::init())
        .setup(setup::setup)
        .invoke_handler(tauri::generate_handler![
            audio::get_input_devices,
            commands::get_model_name,
            commands::get_installed_models,
            commands::switch_model,
            commands::transcribe_file,
            commands::download_model,
            commands::remove_model,
            commands::set_style,
            commands::get_style,
            commands::get_settings,
            commands::update_settings,
            commands::get_transcripts,
            commands::search_transcripts,
            commands::delete_transcript,
            commands::get_dictionary,
            commands::set_dictionary,
            commands::get_vad_threshold,
            commands::set_vad_threshold,
            commands::set_hotkey,
            commands::check_first_run,
            commands::export_transcripts,
            commands::download_parakeet,
            commands::get_snippets,
            commands::save_snippets,
            commands::test_snippet_expansion,
            commands::get_app_styles,
            commands::save_app_styles,
            commands::get_voice_commands,
            commands::save_voice_commands,
            polish::save_api_key,
            polish::get_api_key_status,
            polish::get_polish_settings,
            polish::set_polish_settings,
            polish::get_usage,
            polish::run_ai_polish,
            commands::save_agent_token,
            commands::get_agent_token_status,
            commands::test_agent_connection,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
