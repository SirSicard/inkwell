mod appdetect;
mod audio;
mod commands;
pub mod cleanup;
pub mod dictionary;
pub mod engine;
pub mod engine_service;
pub mod export;
pub mod filetranscribe;
pub mod history;
mod llm;
pub mod merge;
pub mod modes;
pub mod models;
mod overlay;
mod paste;
mod pipeline;
mod polish;
pub mod recording;
mod settings;
pub mod sounds;
pub mod setup;
pub mod snippets;
pub mod store;
pub mod style;
mod tray;
mod vad;
pub mod voicecommand;

use audio::AudioState;
use engine_service::EngineService;
use std::sync::Mutex;

pub struct AppState {
    pub audio: Mutex<Option<AudioState>>,
    pub engine: EngineService,
    pub models_dir: Mutex<String>,
    pub vad_model_path: Mutex<String>,
    pub style: Mutex<style::Style>,
    pub settings: Mutex<settings::Settings>,
    pub settings_path: Mutex<String>,
    pub db: Mutex<Option<history::TranscriptDb>>,
    pub dict: store::Store<dictionary::Dictionary>,
    pub is_first_run: Mutex<bool>,
    // AI Polish (BYOK)
    pub polish_enabled: Mutex<bool>,
    pub polish_prompt: Mutex<String>,
    // Snippets
    pub snippet_store: store::Store<snippets::SnippetStore>,
    // Per-app styles
    pub app_styles: store::Store<appdetect::AppStyleRules>,
    pub modes: store::Store<modes::ModeStore>,
    // Voice commands
    pub voice_commands: store::Store<voicecommand::VoiceCommandStore>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Registered before everything else so a second launch dies before it
        // initializes audio or tries to register the global shortcut. Without
        // this, Start on Boot plus macOS window-restore could launch two copies
        // that fight over the microphone and the hotkey; the loser's shortcut
        // registration fails silently and dictation "randomly" stops working.
        // The duplicate's launch attempt fronts the settings window of the
        // surviving instance instead, which is what the user was after.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .manage(AppState {
            audio: Mutex::new(None),
            engine: EngineService::start(),
            models_dir: Mutex::new(String::new()),
            vad_model_path: Mutex::new(String::new()),
            style: Mutex::new(style::Style::default()),
            settings: Mutex::new(settings::Settings::default()),
            settings_path: Mutex::new(String::new()),
            db: Mutex::new(None),
            dict: store::Store::new(dictionary::Dictionary::default()),
            is_first_run: Mutex::new(false),
            polish_enabled: Mutex::new(false),
            polish_prompt: Mutex::new(llm::DEFAULT_POLISH_PROMPT.to_string()),
            snippet_store: store::Store::new(snippets::SnippetStore::default()),
            app_styles: store::Store::new(appdetect::AppStyleRules::default()),
            modes: store::Store::new(modes::ModeStore::default()),
            voice_commands: store::Store::new(voicecommand::VoiceCommandStore::default()),
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
            commands::list_models,
            commands::switch_model,
            commands::transcribe_file,
            commands::download_model,
            commands::remove_model,
            commands::set_style,
            commands::get_style,
            commands::get_settings,
            commands::update_settings,
            commands::toggle_pause,
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
            commands::get_snippets,
            commands::save_snippets,
            commands::test_snippet_expansion,
            commands::get_app_styles,
            commands::save_app_styles,
            commands::get_modes,
            commands::save_modes,
            commands::get_foreground_app,
            commands::get_voice_commands,
            commands::save_voice_commands,
            paste::check_accessibility_permission,
            paste::open_accessibility_settings,
            polish::save_api_key,
            polish::get_api_key_status,
            polish::get_polish_settings,
            polish::set_polish_settings,
            polish::run_ai_polish,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
