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
