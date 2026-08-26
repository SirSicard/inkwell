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
pub mod modkey;
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
pub mod streaming;
pub mod style;
mod tray;
mod vad;
pub mod voiceedit;
pub mod voicecommand;

/// Render transcribed text for a log line: the text itself in debug builds, a
/// character count in release.
///
/// Release builds log to a file the user never opens, cannot search, and does
/// not clear when they delete their history. Writing dictations into it would
/// quietly outlive the delete button in the Dashboard, in an app whose whole
/// claim is that your words stay where you can see them. A length is what
/// diagnosing a pipeline problem actually needs: it tells you whether a stage
/// dropped the text, doubled it, or returned nothing.
pub fn redact(text: &str) -> String {
    if cfg!(debug_assertions) {
        format!("\"{}\"", text)
    } else {
        format!("{} chars", text.chars().count())
    }
}

/// What the current recording will be used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// Transcribe and paste as text.
    Dictate,
    /// Transcribe as an instruction, apply it to the captured selection.
    Edit,
}

use audio::AudioState;
use engine_service::EngineService;
use streaming::StreamingService;
use std::sync::Mutex;

pub struct AppState {
    pub audio: Mutex<Option<AudioState>>,
    pub engine: EngineService,
    /// Draws words on the overlay while the key is held. Separate from
    /// `engine` because it answers a different question: `engine` produces the
    /// text the user keeps, this one produces the evidence that the app is
    /// listening. Idle and unloaded unless `show_partials` is on.
    pub streaming: StreamingService,
    pub models_dir: Mutex<String>,
    pub vad_model_path: Mutex<String>,
    pub style: Mutex<style::Style>,
    /// Mode id pinned by a voice command, overriding app matching until
    /// changed. Voice commands used to write `style` above, which stopped
    /// meaning anything the moment modes took ownership of style: the pipeline
    /// reads the resolved mode, so "formal mode" set a field nobody read. This
    /// is the field that makes those commands real again.
    pub pinned_mode: Mutex<Option<String>>,
    /// What the in-flight recording is for. Both hotkeys share one audio
    /// buffer, so the stop handler needs to know which pipeline to run; a
    /// second buffer would let the two interleave and produce a dictation made
    /// of half an edit instruction.
    pub recording_intent: Mutex<Intent>,
    /// Text captured from the frontmost app when an edit recording started.
    pub edit_selection: Mutex<Option<String>>,
    /// Last moment the mic was used for anything: press, stop, or stream
    /// (re)open. The idle watchdog measures from here before releasing the
    /// capture stream, which is what lets the machine sleep again.
    pub mic_last_used: Mutex<std::time::Instant>,
    /// When the in-flight recording started, None otherwise. Exists so the
    /// watchdog can distinguish "recording for 4 minutes because the Released
    /// event was lost" from "not recording at all".
    pub recording_started: Mutex<Option<std::time::Instant>>,
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
            streaming: StreamingService::start(),
            models_dir: Mutex::new(String::new()),
            vad_model_path: Mutex::new(String::new()),
            style: Mutex::new(style::Style::default()),
            pinned_mode: Mutex::new(None),
            recording_intent: Mutex::new(Intent::Dictate),
            edit_selection: Mutex::new(None),
            mic_last_used: Mutex::new(std::time::Instant::now()),
            recording_started: Mutex::new(None),
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
            commands::get_partials_status,
            commands::set_style,
            commands::get_style,
            commands::get_settings,
            commands::update_settings,
            commands::toggle_pause,
            commands::get_transcripts,
            commands::get_stats,
            commands::search_transcripts,
            commands::delete_transcript,
            commands::get_dictionary,
            commands::set_dictionary,
            commands::get_vad_threshold,
            commands::set_vad_threshold,
            commands::set_hotkey,
            commands::set_edit_hotkey,
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
            commands::get_pinned_mode,
            commands::set_pinned_mode,
            commands::open_debug_audio_folder,
            commands::open_log_folder,
            commands::get_voice_commands,
            commands::save_voice_commands,
            paste::check_accessibility_permission,
            paste::request_accessibility_permission,
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

#[cfg(test)]
mod redact_tests {
    use super::redact;

    /// The point of `redact` is that a release build never writes a dictation to
    /// disk. Asserted per build profile rather than skipped, so whichever way
    /// the suite is run, one of these two is doing the work.
    #[test]
    fn release_logs_a_length_and_never_the_words() {
        let secret = "my bank password is hunter2";
        let out = redact(secret);
        if cfg!(debug_assertions) {
            assert_eq!(out, format!("\"{}\"", secret));
        } else {
            assert!(!out.contains("hunter2"), "release build leaked text: {out}");
            assert!(!out.contains("bank"), "release build leaked text: {out}");
            assert_eq!(out, "27 chars");
        }
    }

    /// Counts characters, not bytes, so a length is not a proxy for the encoding.
    #[test]
    fn counts_characters_not_bytes() {
        let out = redact("héllo wörld");
        if !cfg!(debug_assertions) {
            assert_eq!(out, "11 chars");
        }
    }

    #[test]
    fn handles_empty() {
        let out = redact("");
        if cfg!(debug_assertions) {
            assert_eq!(out, "\"\"");
        } else {
            assert_eq!(out, "0 chars");
        }
    }
}
