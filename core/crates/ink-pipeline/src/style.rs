//! Stage 6: the style transform, casing and punctuation. Ported from Inkwell 0.2's `style.rs`.
//!
//! Pure: no I/O, no allocation beyond the returned string.

/// How a dictation is written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Style {
    /// Sentences capitalised, and ending punctuation added when missing.
    #[default]
    Formal,
    /// Sentences capitalised; a single sentence loses its trailing full stop.
    Casual,
    /// All lowercase, full stops removed.
    Relaxed,
}

impl Style {
    /// Every style, in the order the settings list them.
    pub const ALL: [Self; 3] = [Self::Formal, Self::Casual, Self::Relaxed];

    /// The stored and spoken name: `formal`, `casual` or `relaxed`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Formal => "formal",
            Self::Casual => "casual",
            Self::Relaxed => "relaxed",
        }
    }

    /// The style named `name`, ignoring case and surrounding whitespace, or `None`.
    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim();
        Self::ALL
            .into_iter()
            .find(|s| s.as_str().eq_ignore_ascii_case(name))
    }

    /// Applies the style to transcribed text. Blank input gives an empty string.
    pub fn format(self, text: &str) -> String {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        match self {
            Self::Formal => format_formal(trimmed),
            Self::Casual => format_casual(trimmed),
            Self::Relaxed => format_relaxed(trimmed),
        }
    }
}

/// Capitalises the first letter of the text and of every sentence after `.`, `!` or `?`.
fn capitalise_sentences(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut capitalise_next = true;
    for ch in text.chars() {
        if capitalise_next && ch.is_alphabetic() {
            result.extend(ch.to_uppercase());
            capitalise_next = false;
        } else {
            result.push(ch);
        }
        if matches!(ch, '.' | '!' | '?') {
            capitalise_next = true;
        }
    }
    result
}

/// Formal: capitalised sentences, and a full stop when the text ends without punctuation.
fn format_formal(text: &str) -> String {
    let mut result = capitalise_sentences(text);
    if let Some(last) = result.trim_end().chars().last()
        && !matches!(last, '.' | '!' | '?' | ':' | ';')
    {
        result.push('.');
    }
    result
}

/// Casual: capitalised sentences; a single sentence ending in a full stop loses it, which reads
/// more like a chat message.
fn format_casual(text: &str) -> String {
    let result = capitalise_sentences(text);
    let trimmed = result.trim_end();
    let full_stops = trimmed.chars().filter(|&c| c == '.').count();
    if full_stops == 1 && trimmed.ends_with('.') {
        return trimmed.trim_end_matches('.').to_owned();
    }
    result
}

/// Relaxed: lowercase, full stops removed, other punctuation kept.
fn format_relaxed(text: &str) -> String {
    let result: String = text.to_lowercase().chars().filter(|&c| c != '.').collect();
    result.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported from 0.2's `style.rs` (3 tests).

    #[test]
    fn formal_capitalizes_and_adds_period() {
        assert_eq!(Style::Formal.format("hello world"), "Hello world.");
        assert_eq!(Style::Formal.format("hello. goodbye"), "Hello. Goodbye.");
    }

    #[test]
    fn casual_strips_trailing_period() {
        assert_eq!(Style::Casual.format("hello world."), "Hello world");
        // Several sentences keep their full stops.
        assert_eq!(Style::Casual.format("hello. goodbye."), "Hello. Goodbye.");
    }

    #[test]
    fn relaxed_lowercases_and_strips_periods() {
        assert_eq!(Style::Relaxed.format("Hello World."), "hello world");
        assert_eq!(Style::Relaxed.format("What is this?"), "what is this?");
    }

    // New in 1.0: the style is a typed setting, parsed once.

    #[test]
    fn names_round_trip_and_unknown_names_are_none() {
        for s in Style::ALL {
            assert_eq!(Style::parse(s.as_str()), Some(s));
        }
        assert_eq!(Style::parse(" Casual "), Some(Style::Casual));
        assert_eq!(Style::parse("shouty"), None);
    }
}
