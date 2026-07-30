use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictEntry {
    pub find: String,
    pub replace: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Dictionary {
    pub entries: Vec<DictEntry>,
}

impl Dictionary {
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize dictionary: {}", e))?;
        std::fs::write(path, json)
            .map_err(|e| format!("Failed to write dictionary: {}", e))?;
        Ok(())
    }

    /// The dictionary's correction targets as decoder bias phrases, one per
    /// line, or None when there is nothing to bias toward.
    ///
    /// The `replace` side is what the user actually means ("Inkwell"); feeding
    /// it to the recognizer as a hotword lets the model decode the word right
    /// in the first place, instead of this module repairing whatever came out.
    /// The find/replace pass still runs afterwards, both as a backstop and for
    /// models that do not support biasing.
    pub fn hotwords(&self) -> Option<String> {
        let mut seen = std::collections::HashSet::new();
        let words: Vec<&str> = self
            .entries
            .iter()
            .map(|e| e.replace.trim())
            .filter(|w| !w.is_empty() && w.chars().any(|c| c.is_alphabetic()))
            .filter(|w| seen.insert(w.to_lowercase()))
            // A huge bias list dilutes itself and slows the beam; nobody's
            // personal vocabulary needs more than this.
            .take(100)
            .collect();
        if words.is_empty() {
            None
        } else {
            Some(words.join("\n"))
        }
    }

    /// Apply all dictionary replacements to text (case-insensitive find, exact replace).
    pub fn apply(&self, text: &str) -> String {
        let mut result = text.to_string();
        for entry in &self.entries {
            if entry.find.is_empty() { continue; }
            // Case-insensitive word boundary replacement
            let lower = result.to_lowercase();
            let find_lower = entry.find.to_lowercase();
            let mut new_result = String::with_capacity(result.len());
            let mut last_end = 0;

            for (idx, _) in lower.match_indices(&find_lower) {
                // Check word boundaries
                let before_ok = idx == 0 || !result.as_bytes()[idx - 1].is_ascii_alphanumeric();
                let after_idx = idx + entry.find.len();
                let after_ok = after_idx >= result.len() || !result.as_bytes()[after_idx].is_ascii_alphanumeric();

                if before_ok && after_ok {
                    new_result.push_str(&result[last_end..idx]);
                    new_result.push_str(&entry.replace);
                    last_end = after_idx;
                }
            }
            new_result.push_str(&result[last_end..]);
            result = new_result;
        }
        result
    }
}
