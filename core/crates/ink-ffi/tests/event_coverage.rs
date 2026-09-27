//! Every variant of the pipeline's event enums has its own arm in `src/events.rs`.
//!
//! Those enums are `#[non_exhaustive]`, so the compiler accepts a match that misses a variant added
//! later: the new variant would reach the shell as `other`. This test reads the enums' source and
//! fails for any variant `events.rs` does not name, so a change that adds an event (S1.5c part 2b,
//! for one) cannot merge without its schema mapping.

use std::path::Path;

/// The enums the shell's events are built from: (file under `core/crates`, enum name).
const ENUMS: &[(&str, &str)] = &[
    ("ink-pipeline/src/events.rs", "VadUnavailable"),
    ("ink-pipeline/src/events.rs", "Discard"),
    ("ink-pipeline/src/events.rs", "TakeFailure"),
    ("ink-pipeline/src/events.rs", "Warning"),
    ("ink-pipeline/src/events.rs", "DictationEvent"),
    ("ink-pipeline/src/meeting/events.rs", "Phase"),
    ("ink-pipeline/src/meeting/events.rs", "MeetingWarning"),
    ("ink-pipeline/src/meeting/events.rs", "KeptLive"),
    ("ink-pipeline/src/meeting/events.rs", "MeetingEvent"),
    ("ink-pipeline/src/meeting/watchdog.rs", "SideState"),
    ("ink-pipeline/src/voicecommand.rs", "CommandAction"),
    ("ink-pipeline/src/voicecommand.rs", "RiskLevel"),
    ("ink-core/src/platform.rs", "InsertOutcome"),
];

/// The variant names of `pub enum name` in `source`: identifiers at the enum body's first
/// indentation level (doc comments and attributes skipped).
fn variants(source: &str, name: &str) -> Vec<String> {
    let header = format!("pub enum {name} {{");
    let start = source
        .find(&header)
        .unwrap_or_else(|| panic!("enum {name} not found"));
    let mut out = Vec::new();
    for line in source[start + header.len()..].lines().skip(1) {
        if line.starts_with('}') {
            break;
        }
        let Some(rest) = line.strip_prefix("    ") else {
            continue;
        };
        if rest.starts_with(' ') || rest.starts_with("//") || rest.starts_with('#') {
            continue;
        }
        let ident: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if ident.starts_with(|c: char| c.is_ascii_uppercase()) {
            out.push(ident);
        }
    }
    assert!(!out.is_empty(), "no variants read for {name}");
    out
}

#[test]
fn every_pipeline_event_variant_has_its_own_mapping() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let source = std::fs::read_to_string(crates.join("ink-ffi/src/events.rs")).unwrap();
    // The mapping code only: the file's tests name every variant too, mapped or not.
    let mapping = &source[..source.find("#[cfg(test)]").unwrap_or(source.len())];
    let mut missing = Vec::new();
    let mut checked = 0;
    for (file, name) in ENUMS {
        let source = std::fs::read_to_string(crates.join(file)).unwrap();
        for variant in variants(&source, name) {
            checked += 1;
            if !mapping.contains(&format!("{name}::{variant}")) {
                missing.push(format!("{name}::{variant} ({file})"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "events.rs has no arm for these, so they would reach the shell as \"other\": map each in \
         events.rs and the schema, then run `cargo run -p ink-ffi --bin ink-schema`:\n{}",
        missing.join("\n")
    );
    assert!(
        checked > 70,
        "only {checked} variants read: the parser missed some"
    );
}

#[test]
fn the_parser_reads_variants_and_skips_the_rest() {
    let source = "pub enum Demo {\n    /// A doc line.\n    Plain,\n    #[doc = \"x\"]\n    \
                  Tuple(u32),\n    Named {\n        /// Field docs.\n        field: u64,\n    },\n}\n";
    assert_eq!(variants(source, "Demo"), ["Plain", "Tuple", "Named"]);
}
