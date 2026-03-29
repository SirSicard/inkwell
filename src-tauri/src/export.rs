use crate::history::Transcript;

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
        let total_words: usize = sentences.iter().map(|s| word_count(s)).max_by(|a, b| a.cmp(b)).unwrap_or(1);
        let total_words_sum: usize = sentences.iter().map(|s| word_count(s)).sum::<usize>().max(1);

        let _ = total_words; // suppress unused warning

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

/// Format a list of transcripts as JSON.
pub fn to_json(transcripts: &[Transcript]) -> String {
    if transcripts.len() == 1 {
        // Single transcript: use singular envelope
        let t = &transcripts[0];
        format!(
            r#"{{"version":1,"exported_at":"{}","transcript":{{"id":{},"text":{},"raw_text":{},"style":{},"model":{},"audio_duration_ms":{},"created_at":{}}}}}"#,
            chrono_now(),
            t.id,
            json_str(&t.text),
            json_str(&t.raw_text),
            json_str(&t.style),
            json_str(&t.model),
            t.audio_duration_ms,
            json_str(&t.created_at),
        )
    } else {
        let items: Vec<String> = transcripts
            .iter()
            .map(|t| {
                format!(
                    r#"{{"id":{},"text":{},"raw_text":{},"style":{},"model":{},"audio_duration_ms":{},"created_at":{}}}"#,
                    t.id,
                    json_str(&t.text),
                    json_str(&t.raw_text),
                    json_str(&t.style),
                    json_str(&t.model),
                    t.audio_duration_ms,
                    json_str(&t.created_at),
                )
            })
            .collect();

        format!(
            r#"{{"version":1,"exported_at":"{}","count":{},"transcripts":[{}]}}"#,
            chrono_now(),
            transcripts.len(),
            items.join(",")
        )
    }
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

fn json_str(s: &str) -> String {
    // Basic JSON string escaping
    let escaped = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!("\"{}\"", escaped)
}

fn chrono_now() -> String {
    // Simple UTC timestamp without pulling in chrono
    // Format: 2026-03-27T14:00:00Z (approximate)
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| {
            let secs = d.as_secs();
            // Rough ISO 8601 from epoch
            let s = secs % 60;
            let m = (secs / 60) % 60;
            let h = (secs / 3600) % 24;
            let days = secs / 86400;
            // Days since epoch to year/month/day (simplified, good enough for export metadata)
            let (y, mo, d) = days_to_ymd(days);
            format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, m, s)
        })
        .unwrap_or_else(|_| "unknown".to_string())
}

fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Gregorian calendar from days since 1970-01-01
    let mut remaining = days;
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        year += 1;
    }
    let months = [31, if is_leap(year) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u64;
    for &days_in_month in &months {
        if remaining < days_in_month {
            break;
        }
        remaining -= days_in_month;
        month += 1;
    }
    (year, month, remaining + 1)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
