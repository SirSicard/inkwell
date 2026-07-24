use aho_corasick::AhoCorasick;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// How risky is this command to execute?
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Safe,      // no confirmation
    Moderate,  // brief toast with cancel
    Dangerous, // modal confirmation required
}

/// What to do when a command triggers
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommandAction {
    Undo,
    ChangeStyle { style: String },
    SwitchModel { model: String },
    TogglePolish,
    ToggleDictation,
    OpenUrl { url: String },
    OpenApp { path: String },
    InsertText { text: String },
}

impl CommandAction {
    pub fn risk(&self) -> RiskLevel {
        match self {
            CommandAction::Undo => RiskLevel::Safe,
            CommandAction::ChangeStyle { .. } => RiskLevel::Safe,
            CommandAction::SwitchModel { .. } => RiskLevel::Safe,
            CommandAction::TogglePolish => RiskLevel::Safe,
            CommandAction::ToggleDictation => RiskLevel::Safe,
            CommandAction::InsertText { .. } => RiskLevel::Safe,
            CommandAction::OpenUrl { .. } => RiskLevel::Moderate,
            CommandAction::OpenApp { .. } => RiskLevel::Moderate,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceCommand {
    pub id: String,
    pub triggers: Vec<String>,  // e.g. ["scratch that", "undo that", "undo"]
    pub action: CommandAction,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceCommandStore {
    pub enabled: bool,
    pub wake_prefix: String,  // e.g. "inkwell" - text must start with this (or be in command mode)
    pub commands: Vec<VoiceCommand>,
}

impl Default for VoiceCommandStore {
    fn default() -> Self {
        Self {
            enabled: false,
            wake_prefix: "inkwell".to_string(),
            commands: default_commands(),
        }
    }
}

fn default_commands() -> Vec<VoiceCommand> {
    vec![
        VoiceCommand {
            id: "undo".into(),
            triggers: vec!["scratch that".into(), "undo that".into(), "undo".into(), "never mind".into()],
            action: CommandAction::Undo,
            enabled: true,
        },
        VoiceCommand {
            id: "style-formal".into(),
            triggers: vec!["formal mode".into(), "switch to formal".into(), "use formal".into()],
            action: CommandAction::ChangeStyle { style: "formal".into() },
            enabled: true,
        },
        VoiceCommand {
            id: "style-casual".into(),
            triggers: vec!["casual mode".into(), "switch to casual".into(), "use casual".into()],
            action: CommandAction::ChangeStyle { style: "casual".into() },
            enabled: true,
        },
        VoiceCommand {
            id: "style-relaxed".into(),
            triggers: vec!["relaxed mode".into(), "switch to relaxed".into(), "use relaxed".into()],
            action: CommandAction::ChangeStyle { style: "relaxed".into() },
            enabled: true,
        },
        VoiceCommand {
            id: "toggle-polish".into(),
            triggers: vec!["toggle polish".into(), "polish on".into(), "polish off".into()],
            action: CommandAction::TogglePolish,
            enabled: true,
        },
        VoiceCommand {
            id: "stop-listening".into(),
            triggers: vec!["stop listening".into(), "pause dictation".into(), "go to sleep".into()],
            action: CommandAction::ToggleDictation,
            enabled: true,
        },
    ]
}

impl VoiceCommandStore {
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Serialize error: {}", e))?;
        std::fs::write(path, json).map_err(|e| format!("Write error: {}", e))?;
        Ok(())
    }

    /// Check if transcribed text is a voice command.
    /// Returns Some(command) if matched, None if it's regular dictation.
    ///
    /// Detection rules:
    /// 1. Text must start with wake_prefix (e.g. "inkwell formal mode")
    /// 2. Strip the prefix, match remaining against command triggers
    /// 3. Case-insensitive, strips punctuation
    pub fn detect(&self, text: &str) -> Option<&VoiceCommand> {
        if !self.enabled {
            return None;
        }

        let clean = text.trim().to_lowercase();
        let clean = clean.trim_matches(|c: char| c.is_ascii_punctuation());

        // Check for wake prefix
        let after_wake = if clean.starts_with(&self.wake_prefix) {
            let rest = clean[self.wake_prefix.len()..].trim_start();
            // Strip optional comma/colon after wake word
            let rest = rest.trim_start_matches(|c: char| c == ',' || c == ':' || c == '.');
            rest.trim()
        } else {
            return None;
        };

        if after_wake.is_empty() {
            return None;
        }

        // Build automaton from active triggers
        let active: Vec<(&VoiceCommand, &str)> = self.commands.iter()
            .filter(|c| c.enabled)
            .flat_map(|c| c.triggers.iter().map(move |t| (c, t.as_str())))
            .collect();

        if active.is_empty() {
            return None;
        }

        // Try exact match first (most reliable)
        for (cmd, trigger) in &active {
            if after_wake == *trigger {
                return Some(cmd);
            }
        }

        // Try contains match (e.g. "please use formal mode" matches "formal mode")
        let triggers: Vec<&str> = active.iter().map(|(_, t)| *t).collect();
        if let Ok(ac) = AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(&triggers)
        {
            if let Some(mat) = ac.find(after_wake) {
                let idx = mat.pattern().as_usize();
                // Map back through active list to find the command
                let mut count = 0;
                for cmd in &self.commands {
                    if !cmd.enabled { continue; }
                    for _ in &cmd.triggers {
                        if count == idx {
                            return Some(cmd);
                        }
                        count += 1;
                    }
                }
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_wake_prefix_commands() {
        let store = VoiceCommandStore {
            enabled: true,
            wake_prefix: "inkwell".into(),
            commands: default_commands(),
        };

        // Exact match
        assert!(store.detect("inkwell scratch that").is_some());
        assert!(store.detect("Inkwell, formal mode").is_some());
        assert!(store.detect("inkwell: use casual").is_some());

        // No wake prefix = not a command
        assert!(store.detect("scratch that").is_none());
        assert!(store.detect("formal mode").is_none());

        // Just wake word alone = not a command
        assert!(store.detect("inkwell").is_none());

        // Regular speech = not a command
        assert!(store.detect("inkwell is a great app").is_none());
    }

    #[test]
    fn disabled_store_matches_nothing() {
        let store = VoiceCommandStore {
            enabled: false,
            ..Default::default()
        };
        assert!(store.detect("inkwell scratch that").is_none());
    }
}
