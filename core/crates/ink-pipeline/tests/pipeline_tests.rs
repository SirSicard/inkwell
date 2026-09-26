//! Inkwell 0.2's `src-tauri/tests/pipeline_tests.rs`, ported test for test (57), module for
//! module, onto the 1.0 crate.
//!
//! What changed in the port, and why:
//! - Examples are synthetic: no real names.
//! - Snippet variables and export stamps take the time as an argument (the pipeline reads it from
//!   the platform clock), so the date and time tests check exact values instead of "contains 202".
//! - Export entries carry 1.0's text record ids.
//! - `recording_tests` ported onto the 1.0 capture path (`MicPath`: downmix and streaming
//!   resampler). 0.2's `save_wav_creates_file` tested a debug WAV writer that 1.0 does not have:
//!   a dictation keeps no audio at all. Its port proves exactly that.
//! - `dictionary_persistence_tests` ported from a JSON file to the store's settings, where 1.0
//!   keeps settings.

mod common;

use ink_pipeline::{dictionary, export, mic, snippets, style, voicecommand};

/// 2026-03-30 12:00:00 UTC.
const NOON_UTC: i64 = 1_774_872_000_000;

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
        assert_eq!(style::Style::Formal.format("hello. world"), "Hello. World.");
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
        // 0.2 accepted either case here; Rust's `to_uppercase` does uppercase `ü`, so 1.0 pins it.
        let result = style::Style::Formal.format("über cool");
        assert_eq!(result, "Über cool.");
    }
}

// ============================================================================
// Dictionary
// ============================================================================

mod dictionary_tests {
    use super::*;

    pub(super) fn make_dict(entries: Vec<(&str, &str)>) -> dictionary::Dictionary {
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
        let dict = make_dict(vec![("alyssa", "Alisa")]);
        assert_eq!(dict.apply("Hello Alyssa"), "Hello Alisa");
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
        // Not inside "inkwell".
        assert_eq!(dict.apply("inkwell"), "inkwell");
        // Standalone.
        assert_eq!(dict.apply("the ink is dry"), "the INK is dry");
    }

    #[test]
    fn multiple_occurrences() {
        let dict = make_dict(vec![("um", "")]);
        assert_eq!(dict.apply("um hello um world um"), " hello  world ");
    }

    #[test]
    fn multiple_rules() {
        let dict = make_dict(vec![("alyssa", "Alisa"), ("inkwell", "Inkwell")]);
        assert_eq!(
            dict.apply("hello alyssa from inkwell"),
            "hello Alisa from Inkwell"
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
                    id: format!("s{i}"),
                    trigger: trigger.to_string(),
                    expansion: expansion.to_string(),
                    category: String::new(),
                    enabled: true,
                })
                .collect(),
        }
    }

    fn noon() -> snippets::SnippetVars {
        snippets::SnippetVars::at(NOON_UTC, 0)
    }

    #[test]
    fn basic_expansion() {
        let store = make_store(vec![("sig", "Best regards,\nSam")]);
        assert_eq!(
            store.expand("email sig", &noon()),
            "email Best regards,\nSam"
        );
    }

    #[test]
    fn case_insensitive_trigger() {
        let store = make_store(vec![("hello", "Hi there!")]);
        assert_eq!(store.expand("HELLO world", &noon()), "Hi there! world");
    }

    #[test]
    fn word_boundary_trigger() {
        let store = make_store(vec![("test", "EXPANDED")]);
        // Not inside "testing".
        assert_eq!(store.expand("testing", &noon()), "testing");
        // Standalone.
        assert_eq!(store.expand("run test now", &noon()), "run EXPANDED now");
    }

    #[test]
    fn disabled_snippet_ignored() {
        let mut store = make_store(vec![("sig", "Sam")]);
        store.snippets[0].enabled = false;
        assert_eq!(store.expand("email sig", &noon()), "email sig");
    }

    #[test]
    fn empty_trigger_ignored() {
        let store = make_store(vec![("", "BOOM")]);
        assert_eq!(store.expand("hello world", &noon()), "hello world");
    }

    #[test]
    fn no_snippets_passthrough() {
        let store = make_store(vec![]);
        assert_eq!(store.expand("hello world", &noon()), "hello world");
    }

    #[test]
    fn multiple_triggers_in_one_text() {
        let store = make_store(vec![("hi", "hello"), ("bye", "goodbye")]);
        assert_eq!(store.expand("hi and bye", &noon()), "hello and goodbye");
    }

    #[test]
    fn date_variable_interpolation() {
        let store = make_store(vec![("today", "Date: {date}")]);
        assert_eq!(store.expand("today", &noon()), "Date: 2026-03-30");
    }

    #[test]
    fn time_variable_interpolation() {
        let store = make_store(vec![("now", "Time: {time}")]);
        assert_eq!(store.expand("now", &noon()), "Time: 12:00");
    }
}

// ============================================================================
// Export
// ============================================================================

mod export_tests {
    use super::*;

    fn sample_transcript() -> export::ExportEntry {
        export::ExportEntry {
            id: "rec-1".to_string(),
            text: "Hello world.".to_string(),
            raw_text: "hello world".to_string(),
            style: "formal".to_string(),
            model: "parakeet-v3".to_string(),
            audio_duration_ms: 2000,
            created_at: "2026-03-30T12:00:00Z".to_string(),
        }
    }

    fn two_transcripts() -> Vec<export::ExportEntry> {
        vec![
            sample_transcript(),
            export::ExportEntry {
                id: "rec-2".to_string(),
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
        assert!(
            result.starts_with("1\n") || result.starts_with("1\r\n"),
            "SRT should start with index 1"
        );
        let count = result.matches("-->").count();
        assert!(count >= 2, "expected >= 2 SRT entries, got {count}");
    }

    #[test]
    fn json_single_has_transcript_key() {
        let result = export::to_json(&[sample_transcript()], NOON_UTC);
        assert!(result.contains("\"transcript\""));
        assert!(!result.contains("\"transcripts\"")); // singular, not plural
        assert!(result.contains("\"Hello world.\""));
    }

    #[test]
    fn json_multiple_has_transcripts_array() {
        let result = export::to_json(&two_transcripts(), NOON_UTC);
        assert!(result.contains("\"transcripts\""));
        assert!(result.contains("\"count\":2"));
    }

    #[test]
    fn json_escapes_special_chars() {
        let t = export::ExportEntry {
            text: "He said \"hello\"\nnewline".to_string(),
            raw_text: "raw".to_string(),
            ..sample_transcript()
        };
        let result = export::to_json(&[t], NOON_UTC);
        assert!(result.contains("\\\"hello\\\""));
        assert!(result.contains("\\n"));
    }

    /// Text a dictation can realistically produce: quotes, newlines, tabs, backslashes, control
    /// characters and non-ASCII in one string.
    fn nasty_text() -> String {
        "He said \"hi\\bye\"\n\ttab\r\nend \u{7} — ünïcode 😀".to_string()
    }

    #[test]
    fn json_single_roundtrips_through_a_parser() {
        let t = export::ExportEntry {
            text: nasty_text(),
            raw_text: nasty_text(),
            ..sample_transcript()
        };
        let result = export::to_json(&[t], NOON_UTC);
        let parsed: serde_json::Value =
            serde_json::from_str(&result).expect("export must be valid JSON");
        assert_eq!(parsed["version"], 1);
        assert_eq!(parsed["transcript"]["text"], nasty_text());
        assert_eq!(parsed["transcript"]["raw_text"], nasty_text());
        assert_eq!(parsed["transcript"]["audio_duration_ms"], 2000);
    }

    #[test]
    fn json_multiple_roundtrips_through_a_parser() {
        let mut list = two_transcripts();
        list[0].text = nasty_text();
        let result = export::to_json(&list, NOON_UTC);
        let parsed: serde_json::Value =
            serde_json::from_str(&result).expect("export must be valid JSON");
        assert_eq!(parsed["count"], 2);
        assert_eq!(parsed["transcripts"][0]["text"], nasty_text());
        assert_eq!(parsed["transcripts"][1]["text"], "Goodbye.");
    }

    #[test]
    fn json_empty_list_is_valid_and_empty() {
        let result = export::to_json(&[], NOON_UTC);
        let parsed: serde_json::Value =
            serde_json::from_str(&result).expect("export must be valid JSON");
        assert_eq!(parsed["count"], 0);
        assert_eq!(parsed["transcripts"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn json_exported_at_is_iso8601() {
        let result = export::to_json(&[sample_transcript()], NOON_UTC);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        // 0.2 checked the shape of "now"; with the time passed in, the value is exact.
        assert_eq!(parsed["exported_at"], "2026-03-30T12:00:00Z");
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
        let t = export::ExportEntry {
            text: "Hello, world".to_string(),
            ..sample_transcript()
        };
        let result = export::to_csv(&[t]);
        assert!(result.contains("\"Hello, world\""));
    }
}

// ============================================================================
// Voice commands
// ============================================================================

mod voicecommand_tests {
    use super::*;

    fn make_store() -> voicecommand::VoiceCommandStore {
        voicecommand::VoiceCommandStore {
            enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn detects_scratch_that() {
        let store = make_store();
        let result = store.detect("inkwell scratch that");
        match result.map(|c| &c.action) {
            Some(voicecommand::CommandAction::Undo) => {}
            other => panic!("expected Undo, got {other:?}"),
        }
    }

    #[test]
    fn detects_style_change() {
        let store = make_store();
        match store.detect("inkwell casual mode").map(|c| &c.action) {
            Some(voicecommand::CommandAction::ChangeStyle { style }) => {
                assert_eq!(style, "casual")
            }
            other => panic!("expected ChangeStyle, got {other:?}"),
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
        assert!(store.detect("INKWELL scratch that").is_some());
    }
}

// ============================================================================
// Recording (0.2's resampling helpers; in 1.0 the capture path after the ring)
// ============================================================================

mod recording_tests {
    use super::*;
    use ink_core::{AudioBlock, StreamFormat};

    fn block(samples: &[f32], rate: u32) -> AudioBlock<'_> {
        AudioBlock {
            samples,
            format: StreamFormat {
                sample_rate: rate,
                channels: 1,
            },
            host_time_ns: 0,
        }
    }

    fn run(samples: &[f32], rate: u32) -> Vec<f32> {
        let format = StreamFormat {
            sample_rate: rate,
            channels: 1,
        };
        let mut path = mic::MicPath::new(format).unwrap();
        let mut out = path.push(&block(samples, rate)).unwrap().samples.to_vec();
        out.extend_from_slice(path.finish().unwrap());
        out
    }

    #[test]
    fn resample_passthrough_at_16k() {
        let samples = vec![0.1, 0.2, 0.3, 0.4];
        assert_eq!(run(&samples, 16_000), samples);
    }

    #[test]
    fn resample_48k_to_16k_reduces_length() {
        let samples: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.001).sin()).collect();
        let result = run(&samples, 48_000);
        // 0.2 allowed 15,000..17,000; 1.0's resampler is exact to the sample.
        assert_eq!(result.len(), 16_000);
    }

    #[test]
    fn resample_preserves_non_silence() {
        let samples: Vec<f32> = (0..4_800).map(|i| (i as f32 * 0.01).sin()).collect();
        let result = run(&samples, 48_000);
        let rms = (result.iter().map(|s| s * s).sum::<f32>() / result.len() as f32).sqrt();
        assert!(
            rms > 0.01,
            "resampled audio should not be silent, RMS={rms}"
        );
    }

    /// Replaces 0.2's `save_wav_creates_file`: 1.0 has no debug WAV writer. A dictation keeps no
    /// audio anywhere: its record says so, and nothing is written under the data directory.
    #[test]
    fn a_dictation_keeps_no_audio() {
        let rig = common::Rig::builder().build();
        rig.dictate_fixture("please check the notes", 2.0, -30.0);
        let records = rig.dictation_records();
        assert_eq!(records.len(), 1, "one dictation, one record");
        assert_eq!(records[0].audio_dir, None, "a dictation keeps no audio");
    }
}

// ============================================================================
// Dictionary persistence (0.2: a JSON file; 1.0: the store's settings)
// ============================================================================

mod dictionary_persistence_tests {
    use super::*;
    use ink_core::mock::MemStore;

    #[test]
    fn save_and_load_roundtrip() {
        let store = MemStore::new();
        let dict = dictionary::Dictionary {
            entries: vec![dictionary::DictEntry {
                find: "test".to_string(),
                replace: "TEST".to_string(),
            }],
        };
        dict.save(&store).unwrap();

        let loaded = dictionary::Dictionary::load(&store).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].find, "test");
        assert_eq!(loaded.entries[0].replace, "TEST");
    }

    #[test]
    fn load_missing_returns_empty() {
        let store = MemStore::new();
        let dict = dictionary::Dictionary::load(&store).unwrap();
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
        let raw = "um hello alyssa can you check the sig please";

        // 1. Style
        let styled = style::Style::Formal.format(raw);
        assert!(styled.starts_with("Um hello"));
        assert!(styled.ends_with('.'));

        // 2. Dictionary
        let dict = dictionary_tests::make_dict(vec![("alyssa", "Alisa")]);
        let corrected = dict.apply(&styled);
        assert!(corrected.contains("Alisa"));
        assert!(!corrected.contains("alyssa"));

        // 3. Snippets
        let store = snippets::SnippetStore {
            snippets: vec![snippets::Snippet {
                id: "sig".to_string(),
                trigger: "sig".to_string(),
                expansion: "Best regards, Sam".to_string(),
                category: String::new(),
                enabled: true,
            }],
        };
        let expanded = store.expand(&corrected, &snippets::SnippetVars::at(NOON_UTC, 0));
        assert!(expanded.contains("Best regards, Sam"));
        assert!(!expanded.contains(" sig "));
    }

    #[test]
    fn pipeline_empty_input() {
        let styled = style::Style::Formal.format("");
        assert_eq!(styled, "");
        let corrected = dictionary::Dictionary::default().apply(&styled);
        assert_eq!(corrected, "");
        let expanded = snippets::SnippetStore::default()
            .expand(&corrected, &snippets::SnippetVars::at(NOON_UTC, 0));
        assert_eq!(expanded, "");
    }

    #[test]
    fn pipeline_no_modifications() {
        let styled = style::Style::Formal.format("Hello world.");
        let corrected = dictionary::Dictionary::default().apply(&styled);
        let expanded = snippets::SnippetStore::default()
            .expand(&corrected, &snippets::SnippetVars::at(NOON_UTC, 0));
        assert_eq!(expanded, "Hello world.");
    }
}
