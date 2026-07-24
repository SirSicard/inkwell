use aho_corasick::AhoCorasick;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snippet {
    pub id: String,
    pub trigger: String,
    pub expansion: String,
    #[serde(default)]
    pub category: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnippetStore {
    pub snippets: Vec<Snippet>,
}

impl Default for SnippetStore {
    fn default() -> Self {
        Self { snippets: Vec::new() }
    }
}

impl SnippetStore {
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Serialize error: {}", e))?;
        std::fs::write(path, json)
            .map_err(|e| format!("Write error: {}", e))?;
        Ok(())
    }

    /// Expand all snippet triggers found in text.
    /// Returns the text with triggers replaced by their expansions.
    pub fn expand(&self, text: &str) -> String {
        let active: Vec<&Snippet> = self.snippets.iter()
            .filter(|s| s.enabled && !s.trigger.is_empty())
            .collect();

        if active.is_empty() {
            return text.to_string();
        }

        let triggers: Vec<&str> = active.iter().map(|s| s.trigger.as_str()).collect();

        // Build case-insensitive automaton
        let ac = match AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(&triggers)
        {
            Ok(ac) => ac,
            Err(_) => return text.to_string(),
        };

        // Find all matches, but only expand if they appear as whole words/phrases
        let mut result = String::with_capacity(text.len());
        let mut last_end = 0;
        let text_lower = text.to_lowercase();

        for mat in ac.find_iter(&text_lower) {
            let start = mat.start();
            let end = mat.end();
            let pattern_idx = mat.pattern().as_usize();

            // Word boundary check: ensure the match isn't part of a larger word
            let at_word_start = start == 0 || !text.as_bytes()[start - 1].is_ascii_alphanumeric();
            let at_word_end = end >= text.len() || !text.as_bytes()[end].is_ascii_alphanumeric();

            if at_word_start && at_word_end {
                result.push_str(&text[last_end..start]);
                let expansion = interpolate_variables(&active[pattern_idx].expansion);
                result.push_str(&expansion);
                last_end = end;
            }
        }

        result.push_str(&text[last_end..]);
        result
    }
}

/// Interpolate built-in variables in expansion text.
fn interpolate_variables(text: &str) -> String {
    let mut result = text.to_string();

    // {date} - current date YYYY-MM-DD
    if result.contains("{date}") {
        let now = current_date();
        result = result.replace("{date}", &now);
    }

    // {time} - current time HH:MM
    if result.contains("{time}") {
        let now = current_time();
        result = result.replace("{time}", &now);
    }

    // {clipboard} - current clipboard content
    if result.contains("{clipboard}") {
        let clip = arboard::Clipboard::new()
            .ok()
            .and_then(|mut c| c.get_text().ok())
            .unwrap_or_default();
        result = result.replace("{clipboard}", &clip);
    }

    result
}

// {date} and {time} are typed into the user's own document, so they follow the
// machine's local clock, not UTC.
fn current_date() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn current_time() -> String {
    chrono::Local::now().format("%H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(trigger: &str, expansion: &str) -> SnippetStore {
        SnippetStore {
            snippets: vec![Snippet {
                id: "t".into(),
                trigger: trigger.into(),
                expansion: expansion.into(),
                category: String::new(),
                enabled: true,
            }],
        }
    }

    #[test]
    fn date_variable_uses_local_date() {
        let before = chrono::Local::now().format("%Y-%m-%d").to_string();
        let result = store("today", "{date}").expand("today");
        let after = chrono::Local::now().format("%Y-%m-%d").to_string();
        assert!(
            result == before || result == after,
            "expected local date {} or {}, got {}",
            before,
            after,
            result
        );
    }

    #[test]
    fn time_variable_uses_local_clock_not_utc() {
        // The hour is what differed between local and UTC before this fix.
        // Sample either side of the call so an hour rollover can't flake it.
        let before = chrono::Local::now().format("%H").to_string();
        let result = store("now", "{time}").expand("now");
        let after = chrono::Local::now().format("%H").to_string();

        assert_eq!(result.len(), 5, "expected HH:MM, got {}", result);
        assert_eq!(&result[2..3], ":");
        assert!(
            result.starts_with(&before) || result.starts_with(&after),
            "expected local hour {} or {}, got {}",
            before,
            after,
            result
        );
    }
}
