//! Integration tests for the Inkwell dictation pipeline.
//! Tests the pure-logic stages: style → dictionary → snippets → export → usage.
//! Excludes hardware-dependent stages (audio capture, VAD, STT engine, paste).

use app_lib::{dictionary, export, history, recording, snippets, style, usage, voicecommand};

// ============================================================================
// Style formatting
// ============================================================================

mod style_tests {
    use super::*;

    #[test]
    fn formal_capitalizes_first_letter() {
        assert_eq!(style::Style::Formal.format("hello"), "Hello.");
    }

    #[test]
    fn formal_capitalizes_after_sentence_end() {
        assert_eq!(
            style::Style::Formal.format("hello. world"),
            "Hello. World."
        );
        assert_eq!(
            style::Style::Formal.format("what? yes! ok"),
            "What? Yes! Ok."
        );
    }

    #[test]
    fn formal_adds_period_if_missing() {
        assert_eq!(style::Style::Formal.format("hello world"), "Hello world.");
    }

    #[test]
    fn formal_preserves_existing_terminal_punctuation() {
        assert_eq!(style::Style::Formal.format("hello!"), "Hello!");
        assert_eq!(style::Style::Formal.format("really?"), "Really?");
    }

    #[test]
    fn casual_strips_single_trailing_period() {
        assert_eq!(style::Style::Casual.format("hello world."), "Hello world");
    }

    #[test]
    fn casual_keeps_periods_in_multi_sentence() {
        assert_eq!(
            style::Style::Casual.format("hello. goodbye."),
            "Hello. Goodbye."
        );
    }

    #[test]
    fn relaxed_lowercases_everything() {
        assert_eq!(style::Style::Relaxed.format("Hello WORLD"), "hello world");
    }

    #[test]
    fn relaxed_strips_periods_keeps_questions() {
        assert_eq!(style::Style::Relaxed.format("What?"), "what?");
        assert_eq!(style::Style::Relaxed.format("Hello."), "hello");
    }

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(style::Style::Formal.format(""), "");
        assert_eq!(style::Style::Casual.format("  "), "");
        assert_eq!(style::Style::Relaxed.format(""), "");
    }

    #[test]
    fn whitespace_only_returns_empty() {
        assert_eq!(style::Style::Formal.format("   "), "");
    }

    #[test]
    fn formal_handles_unicode() {
        let result = style::Style::Formal.format("über cool");
        assert!(result.starts_with("Ü") || result.starts_with("ü")); // depends on unicode uppercase
        assert!(result.ends_with('.'));
    }
}

// ============================================================================
// Dictionary
// ============================================================================

mod dictionary_tests {
    use super::*;

    fn make_dict(entries: Vec<(&str, &str)>) -> dictionary::Dictionary {
        dictionary::Dictionary {
            entries: entries
                .into_iter()
                .map(|(f, r)| dictionary::DictEntry {
                    find: f.to_string(),
                    replace: r.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn simple_replacement() {
        let dict = make_dict(vec![("matthias", "Mattias")]);
        assert_eq!(dict.apply("Hello Matthias"), "Hello Mattias");
    }

    #[test]
    fn case_insensitive() {
        let dict = make_dict(vec![("inkwell", "Inkwell")]);
        assert_eq!(dict.apply("hello INKWELL"), "hello Inkwell");
        assert_eq!(dict.apply("hello inkwell"), "hello Inkwell");
        assert_eq!(dict.apply("hello Inkwell"), "hello Inkwell");
    }

    #[test]
    fn word_boundary_respected() {
        let dict = make_dict(vec![("ink", "INK")]);
        // Should NOT replace "ink" inside "inkwell"
        assert_eq!(dict.apply("inkwell"), "inkwell");
        // Should replace standalone "ink"
        assert_eq!(dict.apply("the ink is dry"), "the INK is dry");
    }

    #[test]
    fn multiple_occurrences() {
        let dict = make_dict(vec![("um", "")]);
        assert_eq!(dict.apply("um hello um world um"), " hello  world ");
    }

    #[test]
    fn multiple_rules() {
        let dict = make_dict(vec![("matthias", "Mattias"), ("inkwell", "Inkwell")]);
        assert_eq!(
            dict.apply("hello matthias from inkwell"),
            "hello Mattias from Inkwell"
        );
    }

    #[test]
    fn empty_find_skipped() {
        let dict = make_dict(vec![("", "BOOM")]);
        assert_eq!(dict.apply("hello world"), "hello world");
    }

    #[test]
    fn no_entries_passthrough() {
        let dict = make_dict(vec![]);
        assert_eq!(dict.apply("hello world"), "hello world");
    }

    #[test]
    fn replacement_at_start_and_end() {
        let dict = make_dict(vec![("hi", "hello")]);
        assert_eq!(dict.apply("hi there hi"), "hello there hello");
    }
}

// ============================================================================
// Snippets
// ============================================================================

mod snippet_tests {
    use super::*;

    fn make_store(snippets: Vec<(&str, &str)>) -> snippets::SnippetStore {
        snippets::SnippetStore {
            snippets: snippets
                .into_iter()
                .enumerate()
                .map(|(i, (trigger, expansion))| snippets::Snippet {
                    id: format!("s{}", i),
                    trigger: trigger.to_string(),
                    expansion: expansion.to_string(),
                    category: String::new(),
                    enabled: true,
                })
                .collect(),
        }
    }

    #[test]
    fn basic_expansion() {
        let store = make_store(vec![("sig", "Best regards,\nMattias")]);
        assert_eq!(store.expand("email sig"), "email Best regards,\nMattias");
    }

    #[test]
    fn case_insensitive_trigger() {
        let store = make_store(vec![("hello", "Hi there!")]);
        assert_eq!(store.expand("HELLO world"), "Hi there! world");
    }

    #[test]
    fn word_boundary_trigger() {
        let store = make_store(vec![("test", "EXPANDED")]);
        // Should not expand inside "testing"
        assert_eq!(store.expand("testing"), "testing");
        // Should expand standalone
        assert_eq!(store.expand("run test now"), "run EXPANDED now");
    }

    #[test]
    fn disabled_snippet_ignored() {
        let mut store = make_store(vec![("sig", "Mattias")]);
        store.snippets[0].enabled = false;
        assert_eq!(store.expand("email sig"), "email sig");
    }

    #[test]
    fn empty_trigger_ignored() {
        let store = make_store(vec![("", "BOOM")]);
        assert_eq!(store.expand("hello world"), "hello world");
    }

    #[test]
    fn no_snippets_passthrough() {
        let store = make_store(vec![]);
        assert_eq!(store.expand("hello world"), "hello world");
    }

    #[test]
    fn multiple_triggers_in_one_text() {
        let store = make_store(vec![("hi", "hello"), ("bye", "goodbye")]);
        assert_eq!(store.expand("hi and bye"), "hello and goodbye");
    }

    #[test]
    fn date_variable_interpolation() {
        let store = make_store(vec![("today", "Date: {date}")]);
        let result = store.expand("today");
        // Should contain a date like 2026-03-30
        assert!(result.contains("202"), "Expected date in: {}", result);
        assert!(result.contains("-"), "Expected date format: {}", result);
    }

    #[test]
    fn time_variable_interpolation() {
        let store = make_store(vec![("now", "Time: {time}")]);
        let result = store.expand("now");
        assert!(result.contains(":"), "Expected time with colon: {}", result);
    }
}

// ============================================================================
// Usage tracking
// ============================================================================

mod usage_tests {
    use super::*;

    #[test]
    fn new_week_starts_at_zero() {
        let u = usage::UsageData::new_week();
        assert_eq!(u.words_used, 0);
        assert!(!u.over_limit());
        assert_eq!(u.remaining(), usage::FREE_TIER_WORDS);
    }

    #[test]
    fn add_words_counts_correctly() {
        let mut u = usage::UsageData::new_week();
        u.add_words("hello world foo bar");
        assert_eq!(u.words_used, 4);
        assert_eq!(u.remaining(), usage::FREE_TIER_WORDS - 4);
    }

    #[test]
    fn over_limit_triggers_at_threshold() {
        let mut u = usage::UsageData::new_week();
        u.words_used = usage::FREE_TIER_WORDS - 1;
        assert!(!u.over_limit());
        u.words_used = usage::FREE_TIER_WORDS;
        assert!(u.over_limit());
    }

    #[test]
    fn ensure_current_resets_on_new_week() {
        let mut u = usage::UsageData {
            week_start: "2020-01-06".to_string(), // old Monday
            words_used: 500,
        };
        u.ensure_current();
        assert_eq!(u.words_used, 0);
        assert_ne!(u.week_start, "2020-01-06");
    }

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = std::env::temp_dir().join("inkwell_test_usage.json");
        let mut u = usage::UsageData::new_week();
        u.add_words("hello world");
        u.save(&tmp).unwrap();

        let loaded = usage::UsageData::load(&tmp);
        assert_eq!(loaded.words_used, u.words_used);
        assert_eq!(loaded.week_start, u.week_start);

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn load_missing_file_returns_new_week() {
        let u = usage::UsageData::load(std::path::Path::new("/nonexistent/usage.json"));
        assert_eq!(u.words_used, 0);
    }

    #[test]
    fn empty_text_adds_zero_words() {
        let mut u = usage::UsageData::new_week();
        u.add_words("");
        assert_eq!(u.words_used, 0);
    }
}

// ============================================================================
// Export
// ============================================================================

mod export_tests {
    use super::*;

    fn sample_transcript() -> history::Transcript {
        history::Transcript {
            id: 1,
            text: "Hello world.".to_string(),
            raw_text: "hello world".to_string(),
            style: "formal".to_string(),
            model: "parakeet-v3".to_string(),
            audio_duration_ms: 2000,
            created_at: "2026-03-30T12:00:00Z".to_string(),
        }
    }

    fn two_transcripts() -> Vec<history::Transcript> {
        vec![
            sample_transcript(),
            history::Transcript {
                id: 2,
                text: "Goodbye.".to_string(),
                raw_text: "goodbye".to_string(),
                style: "casual".to_string(),
                model: "whisper-small".to_string(),
                audio_duration_ms: 1500,
                created_at: "2026-03-30T12:01:00Z".to_string(),
            },
        ]
    }

    #[test]
    fn txt_single() {
        let result = export::to_txt(&[sample_transcript()]);
        assert!(result.contains("Hello world."));
        assert!(result.contains("parakeet-v3"));
        assert!(result.contains("2000ms"));
    }

    #[test]
    fn txt_empty() {
        assert_eq!(export::to_txt(&[]), "");
    }

    #[test]
    fn txt_multiple_separated_by_divider() {
        let result = export::to_txt(&two_transcripts());
        assert!(result.contains("---"));
        assert!(result.contains("Hello world."));
        assert!(result.contains("Goodbye."));
    }

    #[test]
    fn srt_has_timestamps() {
        let result = export::to_srt(&[sample_transcript()]);
        assert!(result.contains("-->"));
        assert!(result.contains("00:00:00,000"));
        assert!(result.contains("Hello world."));
    }

    #[test]
    fn srt_multiple_indices() {
        let result = export::to_srt(&two_transcripts());
        // Should start with index 1
        assert!(result.starts_with("1\n") || result.starts_with("1\r\n"), "SRT should start with index 1");
        // Should have at least 2 subtitle blocks
        let count = result.matches("-->").count();
        assert!(count >= 2, "Expected >= 2 SRT entries, got {}", count);
    }

    #[test]
    fn json_single_has_transcript_key() {
        let result = export::to_json(&[sample_transcript()]);
        assert!(result.contains("\"transcript\""));
        assert!(!result.contains("\"transcripts\"")); // singular, not plural
        assert!(result.contains("\"Hello world.\""));
    }

    #[test]
    fn json_multiple_has_transcripts_array() {
        let result = export::to_json(&two_transcripts());
        assert!(result.contains("\"transcripts\""));
        assert!(result.contains("\"count\":2"));
    }

    #[test]
    fn json_escapes_special_chars() {
        let t = history::Transcript {
            id: 1,
            text: "He said \"hello\"\nnewline".to_string(),
            raw_text: "raw".to_string(),
            style: "formal".to_string(),
            model: "test".to_string(),
            audio_duration_ms: 1000,
            created_at: "2026-01-01".to_string(),
        };
        let result = export::to_json(&[t]);
        assert!(result.contains("\\\"hello\\\""));
        assert!(result.contains("\\n"));
    }

    #[test]
    fn csv_has_header() {
        let result = export::to_csv(&[sample_transcript()]);
        let lines: Vec<&str> = result.lines().collect();
        assert!(lines[0].contains("id,text,raw_text,style,model"));
    }

    #[test]
    fn csv_has_data_row() {
        let result = export::to_csv(&[sample_transcript()]);
        assert!(result.contains("Hello world."));
        assert!(result.contains("parakeet-v3"));
    }

    #[test]
    fn csv_escapes_commas() {
        let t = history::Transcript {
            id: 1,
            text: "Hello, world".to_string(),
            raw_text: "raw".to_string(),
            style: "formal".to_string(),
            model: "test".to_string(),
            audio_duration_ms: 1000,
            created_at: "2026-01-01".to_string(),
        };
        let result = export::to_csv(&[t]);
        // CSV should quote fields with commas
        assert!(result.contains("\"Hello, world\""));
    }
}

// ============================================================================
// Voice commands
// ============================================================================

mod voicecommand_tests {
    use super::*;

    fn make_store() -> voicecommand::VoiceCommandStore {
        let mut store = voicecommand::VoiceCommandStore::default();
        store.enabled = true;
        store
    }

    #[test]
    fn detects_scratch_that() {
        let store = make_store();
        let result = store.detect("inkwell scratch that");
        assert!(result.is_some());
        match &result.unwrap().action {
            voicecommand::CommandAction::Undo => {}
            other => panic!("Expected Undo, got {:?}", other),
        }
    }

    #[test]
    fn detects_style_change() {
        let store = make_store();
        let result = store.detect("inkwell casual mode");
        assert!(result.is_some());
        match &result.unwrap().action {
            voicecommand::CommandAction::ChangeStyle { style } => {
                assert_eq!(style, "casual");
            }
            other => panic!("Expected ChangeStyle, got {:?}", other),
        }
    }

    #[test]
    fn no_wake_prefix_returns_none() {
        let store = make_store();
        assert!(store.detect("scratch that").is_none());
        assert!(store.detect("hello world").is_none());
    }

    #[test]
    fn empty_text_returns_none() {
        let store = make_store();
        assert!(store.detect("").is_none());
    }

    #[test]
    fn case_insensitive_wake() {
        let store = make_store();
        let result = store.detect("INKWELL scratch that");
        assert!(result.is_some());
    }
}

// ============================================================================
// Recording (resampling)
// ============================================================================

mod recording_tests {
    use super::*;

    #[test]
    fn resample_passthrough_at_16k() {
        let samples = vec![0.1, 0.2, 0.3, 0.4];
        let result = recording::resample_to_16k(&samples, 16000).unwrap();
        assert_eq!(result.len(), samples.len());
    }

    #[test]
    fn resample_48k_to_16k_reduces_length() {
        // 48000Hz -> 16000Hz should be ~3:1 ratio
        let samples: Vec<f32> = (0..48000).map(|i| (i as f32 * 0.001).sin()).collect();
        let result = recording::resample_to_16k(&samples, 48000).unwrap();
        // Should be approximately 16000 samples (with some resampler padding)
        assert!(
            result.len() > 15000 && result.len() < 17000,
            "Expected ~16000 samples, got {}",
            result.len()
        );
    }

    #[test]
    fn resample_preserves_non_silence() {
        let samples: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.01).sin()).collect();
        let result = recording::resample_to_16k(&samples, 48000).unwrap();
        let rms = (result.iter().map(|s| s * s).sum::<f32>() / result.len() as f32).sqrt();
        assert!(rms > 0.01, "Resampled audio should not be silent, RMS={}", rms);
    }

    #[test]
    fn save_wav_creates_file() {
        let samples = vec![0.0f32; 1600]; // 0.1s of silence
        let path = std::env::temp_dir().join("inkwell_test.wav");
        recording::save_wav(&samples, &path).unwrap();
        assert!(path.exists());
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
        let _ = std::fs::remove_file(&path);
    }
}

// ============================================================================
// Dictionary persistence
// ============================================================================

mod dictionary_persistence_tests {
    use super::*;

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = std::env::temp_dir().join("inkwell_test_dict.json");
        let dict = dictionary::Dictionary {
            entries: vec![dictionary::DictEntry {
                find: "test".to_string(),
                replace: "TEST".to_string(),
            }],
        };
        dict.save(&tmp).unwrap();

        let loaded = dictionary::Dictionary::load(&tmp);
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].find, "test");
        assert_eq!(loaded.entries[0].replace, "TEST");

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn load_missing_returns_empty() {
        let dict = dictionary::Dictionary::load(std::path::Path::new("/nonexistent/dict.json"));
        assert!(dict.entries.is_empty());
    }
}

// ============================================================================
// Full pipeline simulation (style -> dict -> snippets)
// ============================================================================

mod pipeline_integration {
    use super::*;

    #[test]
    fn full_text_processing_chain() {
        let raw = "um hello matthias can you check the sig please";

        // 1. Style
        let styled = style::Style::Formal.format(raw);
        assert!(styled.starts_with("Um hello"));
        assert!(styled.ends_with('.'));

        // 2. Dictionary (fix name, remove filler)
        let dict = dictionary::Dictionary {
            entries: vec![
                dictionary::DictEntry {
                    find: "matthias".to_string(),
                    replace: "Mattias".to_string(),
                },
            ],
        };
        let corrected = dict.apply(&styled);
        assert!(corrected.contains("Mattias"));
        assert!(!corrected.contains("matthias"));

        // 3. Snippets
        let store = snippets::SnippetStore {
            snippets: vec![snippets::Snippet {
                id: "sig".to_string(),
                trigger: "sig".to_string(),
                expansion: "Best regards, Mattias".to_string(),
                category: String::new(),
                enabled: true,
            }],
        };
        let expanded = store.expand(&corrected);
        assert!(expanded.contains("Best regards, Mattias"));
        assert!(!expanded.contains(" sig "));
    }

    #[test]
    fn pipeline_empty_input() {
        let styled = style::Style::Formal.format("");
        assert_eq!(styled, "");
        let dict = dictionary::Dictionary::default();
        let corrected = dict.apply(&styled);
        assert_eq!(corrected, "");
        let store = snippets::SnippetStore::default();
        let expanded = store.expand(&corrected);
        assert_eq!(expanded, "");
    }

    #[test]
    fn pipeline_no_modifications() {
        let raw = "Hello world.";
        let styled = style::Style::Formal.format(raw);
        let dict = dictionary::Dictionary::default();
        let corrected = dict.apply(&styled);
        let store = snippets::SnippetStore::default();
        let expanded = store.expand(&corrected);
        assert_eq!(expanded, "Hello world.");
    }
}
