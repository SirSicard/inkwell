use crate::history::Transcript;
use serde::Serialize;

/// Format a list of transcripts as plain text.
pub fn to_txt(transcripts: &[Transcript]) -> String {
    if transcripts.is_empty() {
        return String::new();
    }
    transcripts
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

/// Format a list of transcripts as SRT subtitles.
/// Timestamps are approximate (sentence-proportional distribution).
pub fn to_srt(transcripts: &[Transcript]) -> String {
    let mut output = String::new();
    let mut global_index = 1u32;
    let mut cumulative_ms: u64 = 0;

    for t in transcripts {
        let total_ms = t.audio_duration_ms.max(1000) as u64;
        let sentences = split_sentences(&t.text);
        let total_words_sum: usize = sentences.iter().map(|s| word_count(s)).sum::<usize>().max(1);

        let mut offset = cumulative_ms;
        for sentence in &sentences {
            let words = word_count(sentence).max(1);
            let duration = (total_ms * words as u64) / total_words_sum as u64;
            let start = offset;
            let end = offset + duration;

            output.push_str(&format!(
                "{}\n{} --> {}\n{}\n\n",
                global_index,
                ms_to_srt(start),
                ms_to_srt(end),
                sentence.trim()
            ));

            global_index += 1;
            offset = end;
        }
        cumulative_ms += total_ms + 500; // 500ms gap between transcripts
    }

    output
}

/// Envelope for a one-transcript export (singular `transcript` key).
#[derive(Serialize)]
struct SingleExport<'a> {
    version: u32,
    exported_at: String,
    transcript: &'a Transcript,
}

/// Envelope for a multi-transcript export (plural `transcripts` array).
#[derive(Serialize)]
struct MultiExport<'a> {
    version: u32,
    exported_at: String,
    count: usize,
    transcripts: &'a [Transcript],
}

/// Format a list of transcripts as JSON.
pub fn to_json(transcripts: &[Transcript]) -> String {
    let exported_at = now_iso8601();

    let json = if transcripts.len() == 1 {
        // Single transcript: use singular envelope
        serde_json::to_string(&SingleExport {
            version: 1,
            exported_at,
            transcript: &transcripts[0],
        })
    } else {
        serde_json::to_string(&MultiExport {
            version: 1,
            exported_at,
            count: transcripts.len(),
            transcripts,
        })
    };

    json.unwrap_or_else(|e| {
        log::error!("JSON export failed: {}", e);
        String::new()
    })
}

/// Format a list of transcripts as CSV (RFC 4180).
pub fn to_csv(transcripts: &[Transcript]) -> String {
    let mut wtr = csv::Writer::from_writer(vec![]);

    // Header
    wtr.write_record(&["id", "text", "raw_text", "style", "model", "audio_duration_ms", "created_at"])
        .ok();

    for t in transcripts {
        wtr.write_record(&[
            t.id.to_string(),
            t.text.clone(),
            t.raw_text.clone(),
            t.style.clone(),
            t.model.clone(),
            t.audio_duration_ms.to_string(),
            t.created_at.clone(),
        ])
        .ok();
    }

    wtr.flush().ok();
    String::from_utf8(wtr.into_inner().unwrap_or_default()).unwrap_or_default()
}

// --- Helpers ---

fn split_sentences(text: &str) -> Vec<String> {
    // Split on sentence-ending punctuation, keeping the punctuation
    let mut sentences = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        current.push(ch);
        if matches!(ch, '.' | '!' | '?') {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                sentences.push(trimmed);
            }
            current.clear();
        }
    }

    // Remaining text without terminal punctuation
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        sentences.push(trimmed);
    }

    if sentences.is_empty() {
        sentences.push(text.to_string());
    }

    sentences
}

fn word_count(s: &str) -> usize {
    s.split_whitespace().count().max(1)
}

fn ms_to_srt(ms: u64) -> String {
    let h = ms / 3_600_000;
    let m = (ms % 3_600_000) / 60_000;
    let s = (ms % 60_000) / 1_000;
    let millis = ms % 1_000;
    format!("{:02}:{:02}:{:02},{:03}", h, m, s, millis)
}

/// Export metadata stamp. UTC so exports collated from several machines sort.
fn now_iso8601() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}
