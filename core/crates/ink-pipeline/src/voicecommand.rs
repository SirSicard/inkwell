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
use serde_json::{Map, Value, json};

use crate::dictionary::{SettingsError, is_whole_word};

/// The settings key the user's voice commands are stored under, in the stored form of
/// [`VoiceCommandStore::to_json`]. Until it is set, dictation reads the commands an Inkwell 0.2
/// import brought (the same form), and without those the [defaults](default_commands), off.
pub const SETTING_KEY: &str = "dictation.voice_commands";

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
    /// Every action's stored name (0.2's `type` tag), in the order Settings offers them.
    pub const KINDS: [&'static str; 8] = [
        "undo",
        "change_style",
        "switch_model",
        "toggle_polish",
        "toggle_dictation",
        "open_url",
        "open_app",
        "insert_text",
    ];

    /// Its stored name (0.2's `type` tag).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Undo => "undo",
            Self::ChangeStyle { .. } => "change_style",
            Self::SwitchModel { .. } => "switch_model",
            Self::TogglePolish => "toggle_polish",
            Self::ToggleDictation => "toggle_dictation",
            Self::OpenUrl { .. } => "open_url",
            Self::OpenApp { .. } => "open_app",
            Self::InsertText { .. } => "insert_text",
        }
    }

    /// The one text it carries, if its kind carries one.
    pub fn value(&self) -> Option<&str> {
        match self {
            Self::ChangeStyle { style } => Some(style),
            Self::SwitchModel { model } => Some(model),
            Self::OpenUrl { url } => Some(url),
            Self::OpenApp { path } => Some(path),
            Self::InsertText { text } => Some(text),
            Self::Undo | Self::TogglePolish | Self::ToggleDictation => None,
        }
    }

    /// The name of the field that carries a kind's text in the stored form: `None` for a kind
    /// that carries none (and for a kind 0.2 never had, which [`from_parts`](Self::from_parts)
    /// refuses).
    pub fn field(kind: &str) -> Option<&'static str> {
        match kind {
            "change_style" => Some("style"),
            "switch_model" => Some("model"),
            "open_url" => Some("url"),
            "open_app" => Some("path"),
            "insert_text" => Some("text"),
            _ => None,
        }
    }

    /// The action of `kind` with `value`: `None` when the kind is unknown, or when a value is
    /// missing where the kind needs one (a value where it needs none is ignored).
    pub fn from_parts(kind: &str, value: Option<&str>) -> Option<Self> {
        let text = || value.map(str::to_owned);
        Some(match kind {
            "undo" => Self::Undo,
            "toggle_polish" => Self::TogglePolish,
            "toggle_dictation" => Self::ToggleDictation,
            "change_style" => Self::ChangeStyle { style: text()? },
            "switch_model" => Self::SwitchModel { model: text()? },
            "open_url" => Self::OpenUrl { url: text()? },
            "open_app" => Self::OpenApp { path: text()? },
            "insert_text" => Self::InsertText { text: text()? },
            _ => return None,
        })
    }

    /// Whether this build carries the action out. The dictation chain does
    /// [`ChangeStyle`](Self::ChangeStyle), [`TogglePolish`](Self::TogglePolish) and
    /// [`InsertText`](Self::InsertText) itself. The rest are still recognised, so their words are
    /// never typed as a dictation, and the shell says the action is not available yet (Inkwell
    /// 0.2 recognised them too and carried none of them out).
    pub fn carried_out(&self) -> bool {
        // Exhaustive on purpose, as `risk` is.
        match self {
            Self::ChangeStyle { .. } | Self::TogglePolish | Self::InsertText { .. } => true,
            Self::Undo
            | Self::SwitchModel { .. }
            | Self::ToggleDictation
            | Self::OpenUrl { .. }
            | Self::OpenApp { .. } => false,
        }
    }

    /// How much confirmation the action needs.
    pub fn risk(&self) -> RiskLevel {
        // Exhaustive on purpose: a new action must make its own risk decision.
        match self {
            Self::OpenUrl { .. } | Self::OpenApp { .. } => RiskLevel::Moderate,
            Self::Undo
            | Self::ChangeStyle { .. }
            | Self::SwitchModel { .. }
            | Self::TogglePolish
            | Self::ToggleDictation
            | Self::InsertText { .. } => RiskLevel::Safe,
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
    /// The stored form, 0.2's `voice-commands.json` shape (and what the 0.2 import writes):
    /// `{"enabled", "wake_prefix", "commands": [{"id", "triggers", "action": {"type", ...},
    /// "enabled"}]}`, the action tagged by `type` with its one text field.
    pub fn to_json(&self) -> String {
        let commands: Vec<Value> = self
            .commands
            .iter()
            .map(|c| {
                let mut action = Map::new();
                action.insert("type".into(), c.action.kind().into());
                if let (Some(field), Some(value)) =
                    (CommandAction::field(c.action.kind()), c.action.value())
                {
                    action.insert(field.into(), value.into());
                }
                json!({
                    "id": c.id,
                    "triggers": c.triggers,
                    "action": Value::Object(action),
                    "enabled": c.enabled,
                })
            })
            .collect();
        json!({
            "enabled": self.enabled,
            "wake_prefix": self.wake_prefix,
            "commands": commands,
        })
        .to_string()
    }

    /// Parses the stored form. Every field is required, as 0.2 gave none of them a default;
    /// anything out of shape is [`SettingsError::Malformed`], never the defaults.
    pub fn from_json(text: &str) -> Result<Self, SettingsError> {
        let malformed = |what| SettingsError::Malformed {
            key: SETTING_KEY,
            what,
        };
        let value: Value = serde_json::from_str(text).map_err(|_| malformed("not JSON"))?;
        let top = value
            .as_object()
            .ok_or_else(|| malformed("not an object"))?;
        let enabled = top
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| malformed("no `enabled` switch"))?;
        let wake_prefix = top
            .get("wake_prefix")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("no `wake_prefix` text"))?
            .to_owned();
        let commands = top
            .get("commands")
            .and_then(Value::as_array)
            .ok_or_else(|| malformed("no `commands` list"))?
            .iter()
            .map(|c| Self::command(c).ok_or_else(|| malformed("a command out of shape")))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            enabled,
            wake_prefix,
            commands,
        })
    }

    fn command(value: &Value) -> Option<VoiceCommand> {
        let c = value.as_object()?;
        let triggers = c
            .get("triggers")?
            .as_array()?
            .iter()
            .map(|t| t.as_str().map(str::to_owned))
            .collect::<Option<_>>()?;
        let action = c.get("action")?.as_object()?;
        let kind = action.get("type")?.as_str()?;
        let value = match CommandAction::field(kind) {
            None => None,
            Some(field) => Some(action.get(field)?.as_str()?),
        };
        Some(VoiceCommand {
            id: c.get("id")?.as_str()?.to_owned(),
            triggers,
            action: CommandAction::from_parts(kind, value)?,
            enabled: c.get("enabled")?.as_bool()?,
        })
    }

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
    fn the_stored_form_reads_back_with_every_action() {
        let mut store = enabled();
        store.wake_prefix = "computer".into();
        for (i, kind) in CommandAction::KINDS.iter().enumerate() {
            let value = CommandAction::field(kind).map(|_| "x");
            store.commands.push(VoiceCommand {
                id: format!("c{i}"),
                triggers: vec![format!("do {kind}")],
                action: CommandAction::from_parts(kind, value).unwrap(),
                enabled: i % 2 == 0,
            });
        }
        assert_eq!(VoiceCommandStore::from_json(&store.to_json()), Ok(store));
    }

    #[test]
    fn the_import_s_document_reads_as_0_2_wrote_it() {
        // The shape the 0.2 import writes (every field present, the action tagged by `type`).
        let doc = r#"{"enabled":true,"wake_prefix":"inkwell","commands":[
            {"id":"sig","triggers":["sign off"],"action":{"type":"insert_text","text":"Best, A."},"enabled":true},
            {"id":"site","triggers":["open the site"],"action":{"type":"open_url","url":"https://example.com"},"enabled":false}]}"#;
        let store = VoiceCommandStore::from_json(doc).unwrap();
        assert!(store.enabled);
        assert_eq!(
            store.commands[0].action,
            CommandAction::InsertText {
                text: "Best, A.".into()
            }
        );
        assert!(!store.commands[1].enabled);
        assert_eq!(
            store.detect("inkwell sign off").map(|c| c.id.as_str()),
            Some("sig")
        );
    }

    #[test]
    fn a_damaged_setting_is_an_error_not_the_defaults() {
        for bad in [
            "",
            "[]",
            r#"{"wake_prefix":"inkwell","commands":[]}"#,
            r#"{"enabled":true,"commands":[]}"#,
            r#"{"enabled":true,"wake_prefix":"inkwell","commands":[{"id":"a","triggers":["x"],"action":{"type":"dance"},"enabled":true}]}"#,
            r#"{"enabled":true,"wake_prefix":"inkwell","commands":[{"id":"a","triggers":["x"],"action":{"type":"open_url"},"enabled":true}]}"#,
            r#"{"enabled":true,"wake_prefix":"inkwell","commands":[{"id":"a","triggers":"x","action":{"type":"undo"},"enabled":true}]}"#,
            r#"{"enabled":true,"wake_prefix":"inkwell","commands":[{"id":"a","triggers":["x"],"action":{"type":"undo"}}]}"#,
        ] {
            assert!(
                matches!(
                    VoiceCommandStore::from_json(bad),
                    Err(SettingsError::Malformed {
                        key: SETTING_KEY,
                        ..
                    })
                ),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn only_style_polish_and_text_are_carried_out() {
        let carried: Vec<&str> = CommandAction::KINDS
            .iter()
            .filter(|k| {
                let value = CommandAction::field(k).map(|_| "x");
                CommandAction::from_parts(k, value).unwrap().carried_out()
            })
            .copied()
            .collect();
        assert_eq!(carried, ["change_style", "toggle_polish", "insert_text"]);
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
