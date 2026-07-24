/// Text style formatting applied after transcription, before paste.

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    Formal,
    Casual,
    Relaxed,
}

impl Default for Style {
    fn default() -> Self {
        Style::Formal
    }
}

impl Style {
    /// Apply style formatting to transcribed text.
    pub fn format(&self, text: &str) -> String {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return String::new();
        }

        match self {
            Style::Formal => format_formal(trimmed),
            Style::Casual => format_casual(trimmed),
            Style::Relaxed => format_relaxed(trimmed),
        }
    }
}

/// Formal: capitalize sentences, ensure ending punctuation.
fn format_formal(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut capitalize_next = true;

    for ch in text.chars() {
        if capitalize_next && ch.is_alphabetic() {
            result.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }

        if ch == '.' || ch == '!' || ch == '?' {
            capitalize_next = true;
        }
    }

    // Ensure ends with punctuation
    if let Some(last) = result.trim_end().chars().last() {
        if !matches!(last, '.' | '!' | '?' | ':' | ';') {
            result.push('.');
        }
    }

    result
}

/// Casual: capitalize first letter of each sentence, light punctuation.
/// Removes trailing periods from single sentences (feels more natural).
fn format_casual(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut capitalize_next = true;

    for ch in text.chars() {
        if capitalize_next && ch.is_alphabetic() {
            result.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }

        if ch == '.' || ch == '!' || ch == '?' {
            capitalize_next = true;
        }
    }

    // If single sentence ending with period, strip it
    let trimmed = result.trim_end();
    let period_count = trimmed.chars().filter(|&c| c == '.').count();
    if period_count == 1 && trimmed.ends_with('.') {
        result = trimmed.trim_end_matches('.').to_string();
    }

    result
}

/// Relaxed: all lowercase, strip most punctuation, minimal formatting.
fn format_relaxed(text: &str) -> String {
    let lower = text.to_lowercase();
    let mut result = String::with_capacity(lower.len());

    for ch in lower.chars() {
        match ch {
            // Keep commas, question marks, exclamation marks, spaces, alphanumeric
            ',' | '?' | '!' | ' ' | '\'' => result.push(ch),
            '.' => {} // strip periods
            c if c.is_alphanumeric() => result.push(c),
            _ => result.push(ch),
        }
    }

    // Trim trailing whitespace/punctuation
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formal_capitalizes_and_adds_period() {
        assert_eq!(Style::Formal.format("hello world"), "Hello world.");
        assert_eq!(Style::Formal.format("hello. goodbye"), "Hello. Goodbye.");
    }

    #[test]
    fn casual_strips_trailing_period() {
        assert_eq!(Style::Casual.format("hello world."), "Hello world");
        assert_eq!(Style::Casual.format("hello. goodbye."), "Hello. Goodbye."); // multiple sentences keep periods
    }

    #[test]
    fn relaxed_lowercases_and_strips_periods() {
        assert_eq!(Style::Relaxed.format("Hello World."), "hello world");
        assert_eq!(Style::Relaxed.format("What is this?"), "what is this?");
    }
}
