//! Exporting dictations as plain text, SRT subtitles, JSON or CSV. Ported from Inkwell 0.2's
//! `export.rs`, with its test suite (in `tests/pipeline_tests.rs`).
//!
//! Pure formatting over [`ExportEntry`] rows; building the rows from the store is the caller's.
//! Changes in the port: the export stamp is passed in (the caller reads the platform clock), JSON
//! is built as a `serde_json::Value` (no derive macros), and CSV quoting is written out here
//! (RFC 4180) rather than pulling in a CSV crate for one function.

use serde_json::{Value, json};

use crate::civil::CivilTime;

/// One dictation, as exported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportEntry {
    /// The record id.
    pub id: String,
    /// The text as inserted.
    pub text: String,
    /// The engine's text before any transform.
    pub raw_text: String,
    /// The style it was written in.
    pub style: String,
    /// The engine that transcribed it.
    pub model: String,
    /// How long the user spoke, ms.
    pub audio_duration_ms: u64,
    /// When, ISO 8601.
    pub created_at: String,
}

impl ExportEntry {
    fn to_value(&self) -> Value {
        json!({
            "id": self.id,
            "text": self.text,
            "raw_text": self.raw_text,
            "style": self.style,
            "model": self.model,
            "audio_duration_ms": self.audio_duration_ms,
            "created_at": self.created_at,
        })
    }
}

/// Plain text: a header line per entry, entries separated by `---`.
pub fn to_txt(entries: &[ExportEntry]) -> String {
    entries
        .iter()
        .map(|t| {
            format!(
                "[{}] [{}] [{}ms]\n{}",
                t.created_at, t.model, t.audio_duration_ms, t.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

/// SRT subtitles. Entries follow each other with 500 ms between them; within one, each sentence
/// gets a share of its duration (at least a second) in proportion to its words, which is an
/// approximation: a dictation keeps no word timings.
pub fn to_srt(entries: &[ExportEntry]) -> String {
    let mut output = String::new();
    let mut index = 1u32;
    let mut cumulative_ms = 0u64;
    for t in entries {
        let total_ms = t.audio_duration_ms.max(1_000);
        let sentences = split_sentences(&t.text);
        let words_total = sentences
            .iter()
            .map(|s| word_count(s))
            .sum::<usize>()
            .max(1) as u64;
        let mut offset = cumulative_ms;
        for sentence in &sentences {
            let duration = total_ms * word_count(sentence) as u64 / words_total;
            let end = offset + duration;
            output.push_str(&format!(
                "{index}\n{} --> {}\n{}\n\n",
                srt_time(offset),
                srt_time(end),
                sentence.trim()
            ));
            index += 1;
            offset = end;
        }
        cumulative_ms += total_ms + 500;
    }
    output
}

/// JSON: `{version, exported_at, transcript}` for one entry, `{version, exported_at, count,
/// transcripts}` otherwise. `exported_at_unix_ms` is stamped as UTC, so exports from several
/// machines sort together.
pub fn to_json(entries: &[ExportEntry], exported_at_unix_ms: i64) -> String {
    let exported_at = CivilTime::at(exported_at_unix_ms, 0).iso_utc();
    let doc = match entries {
        [one] => json!({
            "version": 1,
            "exported_at": exported_at,
            "transcript": one.to_value(),
        }),
        many => json!({
            "version": 1,
            "exported_at": exported_at,
            "count": many.len(),
            "transcripts": many.iter().map(ExportEntry::to_value).collect::<Vec<_>>(),
        }),
    };
    doc.to_string()
}

/// CSV (RFC 4180): a header row, then one row per entry.
pub fn to_csv(entries: &[ExportEntry]) -> String {
    let mut out = String::from("id,text,raw_text,style,model,audio_duration_ms,created_at\n");
    for t in entries {
        let duration = t.audio_duration_ms.to_string();
        let fields = [
            t.id.as_str(),
            &t.text,
            &t.raw_text,
            &t.style,
            &t.model,
            &duration,
            &t.created_at,
        ];
        let row: Vec<String> = fields.iter().map(|f| csv_field(f)).collect();
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}

/// A CSV field, quoted when it holds a comma, a quote or a line break, with quotes doubled.
fn csv_field(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

/// Sentences, each keeping its `.`, `!` or `?`; trailing text without one is a sentence too.
fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if matches!(ch, '.' | '!' | '?') {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                sentences.push(trimmed.to_owned());
            }
            current.clear();
        }
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        sentences.push(trimmed.to_owned());
    }
    if sentences.is_empty() {
        sentences.push(text.to_owned());
    }
    sentences
}

/// Words, at least one, so every sentence gets some time.
fn word_count(s: &str) -> usize {
    s.split_whitespace().count().max(1)
}

/// `HH:MM:SS,mmm`.
fn srt_time(ms: u64) -> String {
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ms / 3_600_000,
        ms % 3_600_000 / 60_000,
        ms % 60_000 / 1_000,
        ms % 1_000
    )
}
