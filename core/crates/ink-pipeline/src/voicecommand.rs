//! Stage 5: voice commands. A dictation that starts with the wake word ("inkwell, formal mode") is
//! a command, not text. Ported from Inkwell 0.2's `voicecommand.rs`.
//!
//! Changes in the port:
//! - **The wake word is compared without case** on both sides. 0.2 lowercased the text but not
//!   the wake word, so a wake word the user saved as "Inkwell" never matched anything.
//! - **A blank wake word matches nothing.** In 0.2 it matched everything, so any dictation that
//!   contained "undo" was taken as a command.
//! - **Triggers match as whole words.** 0.2's contains-match found "undo" inside "undone".
//! - **The wake word must be a whole word**: "inkwellish" is not "inkwell".

use aho_corasick::AhoCorasick;

use crate::dictionary::is_whole_word;

/// How much confirmation an action needs before it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiskLevel {
    /// Runs at once.
    Safe,
    /// A brief notice the user can cancel.
    Moderate,
    /// A confirmation first.
    Dangerous,
}

/// What a command does. The dictation chain carries out [`ChangeStyle`](Self::ChangeStyle) and
/// [`TogglePolish`](Self::TogglePolish) itself; the rest are the shell's, and reach it as events.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CommandAction {
    /// Undo the last insertion.
    Undo,
    /// Pin the first mode written in this style (or, failing that, the mode with this name).
    ChangeStyle {
        /// A style name or a mode name.
        style: String,
    },
    /// Switch the dictation model.
    SwitchModel {
        /// The model id.
        model: String,
    },
    /// Turn polish on or off.
    TogglePolish,
    /// Stop or resume listening for the hotkey.
    ToggleDictation,
    /// Open a URL.
    OpenUrl {
        /// The URL.
        url: String,
    },
    /// Open an application.
    OpenApp {
        /// Its path.
        path: String,
    },
    /// Insert fixed text.
    InsertText {
        /// The text.
        text: String,
    },
}

impl CommandAction {
    /// How much confirmation the action needs.
    pub fn risk(&self) -> RiskLevel {
        match self {
            Self::OpenUrl { .. } | Self::OpenApp { .. } => RiskLevel::Moderate,
            _ => RiskLevel::Safe,
        }
    }
}

/// One command and the phrases that trigger it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceCommand {
    /// A stable id.
    pub id: String,
    /// Phrases, any of which triggers it ("scratch that", "undo that").
    pub triggers: Vec<String>,
    /// What it does.
    pub action: CommandAction,
    /// Disabled commands never match.
    pub enabled: bool,
}

/// The user's voice commands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceCommandStore {
    /// Off by default: then nothing is ever a command.
    pub enabled: bool,
    /// The word a command starts with.
    pub wake_prefix: String,
    /// The commands, in order of precedence.
    pub commands: Vec<VoiceCommand>,
}

impl Default for VoiceCommandStore {
    fn default() -> Self {
        Self {
            enabled: false,
            wake_prefix: "inkwell".to_owned(),
            commands: default_commands(),
        }
    }
}

fn command(id: &str, triggers: &[&str], action: CommandAction) -> VoiceCommand {
    VoiceCommand {
        id: id.to_owned(),
        triggers: triggers.iter().map(|t| (*t).to_owned()).collect(),
        action,
        enabled: true,
    }
}

/// The commands a fresh install has (0.2's set).
pub fn default_commands() -> Vec<VoiceCommand> {
    let style = |s: &str| CommandAction::ChangeStyle { style: s.into() };
    vec![
        command(
            "undo",
            &["scratch that", "undo that", "undo", "never mind"],
            CommandAction::Undo,
        ),
        command(
            "style-formal",
            &["formal mode", "switch to formal", "use formal"],
            style("formal"),
        ),
        command(
            "style-casual",
            &["casual mode", "switch to casual", "use casual"],
            style("casual"),
        ),
        command(
            "style-relaxed",
            &["relaxed mode", "switch to relaxed", "use relaxed"],
            style("relaxed"),
        ),
        command(
            "toggle-polish",
            &["toggle polish", "polish on", "polish off"],
            CommandAction::TogglePolish,
        ),
        command(
            "stop-listening",
            &["stop listening", "pause dictation", "go to sleep"],
            CommandAction::ToggleDictation,
        ),
    ]
}

impl VoiceCommandStore {
    /// The command `text` is, or `None` when it is ordinary dictation.
    ///
    /// The text must start with the wake word (case and surrounding punctuation ignored, an
    /// optional `,` `:` or `.` after it). What follows must be a trigger exactly, or contain one
    /// as a whole phrase ("please use formal mode"). Exact matches win over contained ones.
    pub fn detect(&self, text: &str) -> Option<&VoiceCommand> {
        if !self.enabled {
            return None;
        }
        let wake = self.wake_prefix.trim().to_lowercase();
        if wake.is_empty() {
            return None;
        }
        let clean = text.trim().to_lowercase();
        let clean = clean.trim_matches(|c: char| c.is_ascii_punctuation());
        let rest = clean.strip_prefix(wake.as_str())?;
        if rest.chars().next().is_some_and(char::is_alphanumeric) {
            return None;
        }
        let rest = rest.trim_start().trim_start_matches([',', ':', '.']).trim();
        if rest.is_empty() {
            return None;
        }

        let active: Vec<(&VoiceCommand, &str)> = self
            .commands
            .iter()
            .filter(|c| c.enabled)
            .flat_map(|c| c.triggers.iter().map(move |t| (c, t.trim())))
            .filter(|(_, t)| !t.is_empty())
            .collect();
        if let Some((cmd, _)) = active.iter().find(|(_, t)| t.eq_ignore_ascii_case(rest)) {
            return Some(cmd);
        }
        let matcher = match AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(active.iter().map(|(_, t)| *t))
        {
            Ok(m) => m,
            Err(e) => {
                log::warn!("voice commands not matched: the trigger matcher failed to build: {e}");
                return None;
            }
        };
        matcher
            .find_iter(rest)
            .find(|m| is_whole_word(rest, m.start(), m.end()))
            .map(|m| active[m.pattern().as_usize()].0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled() -> VoiceCommandStore {
        VoiceCommandStore {
            enabled: true,
            ..Default::default()
        }
    }

    // Ported from 0.2's `voicecommand.rs` (2 tests).

    #[test]
    fn detects_wake_prefix_commands() {
        let store = enabled();

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

    // New in 1.0: the fixes listed in the module docs.

    #[test]
    fn a_capitalised_wake_word_setting_still_matches() {
        let store = VoiceCommandStore {
            wake_prefix: "Inkwell".into(),
            ..enabled()
        };
        assert!(store.detect("inkwell undo").is_some());
    }

    #[test]
    fn a_blank_wake_word_matches_nothing() {
        let store = VoiceCommandStore {
            wake_prefix: "  ".into(),
            ..enabled()
        };
        assert!(store.detect("undo").is_none());
        assert!(store.detect("I will undo it").is_none());
    }

    #[test]
    fn triggers_and_the_wake_word_match_as_whole_words() {
        let store = enabled();
        assert!(store.detect("inkwell the task is undone").is_none());
        assert!(store.detect("inkwellish undo").is_none());
        let contained = store.detect("inkwell please use formal mode now");
        assert_eq!(contained.map(|c| c.id.as_str()), Some("style-formal"));
    }

    #[test]
    fn only_opening_things_needs_confirming() {
        assert_eq!(CommandAction::Undo.risk(), RiskLevel::Safe);
        assert_eq!(
            CommandAction::OpenUrl {
                url: "https://example.com".into()
            }
            .risk(),
            RiskLevel::Moderate
        );
    }
}
