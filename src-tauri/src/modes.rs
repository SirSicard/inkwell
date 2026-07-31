//! Modes: one named bundle of everything that decides how a dictation is
//! written.
//!
//! Before this, three separate concepts each owned a slice of that decision: a
//! global `style`, a list of per-app style rules, and a global polish prompt.
//! None of them could vary together, so "formal with punctuation, polished for
//! email, only in Outlook" was not expressible even though every ingredient
//! existed. superwhisper sells exactly that bundle as its headline feature.
//!
//! A mode carries the style, optionally a model and a polish prompt, the apps it
//! activates in, and whether cleanup runs. Resolution is: the first mode whose
//! app list matches the frontmost application wins; otherwise the default mode.
//!
//! Modes absorb per-app rules rather than sitting beside them. `migrate_from`
//! turns an existing install's style plus its app rules into modes, so nobody
//! has to rebuild configuration they already had.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mode {
    pub id: String,
    pub name: String,
    /// "formal" | "casual" | "relaxed".
    pub style: String,
    /// Model id, or empty to use whatever is loaded. Switching models per mode
    /// costs a multi-second load, so this is opt-in rather than assumed.
    #[serde(default)]
    pub model: String,
    /// Per-mode polish prompt. Empty means fall back to the global prompt.
    #[serde(default)]
    pub polish_prompt: String,
    #[serde(default)]
    pub polish_enabled: bool,
    /// Substrings matched against the frontmost app's identity: bundle id on
    /// macOS ("com.microsoft.Outlook"), executable name on Windows.
    #[serde(default)]
    pub apps: Vec<String>,
    #[serde(default = "default_true")]
    pub remove_fillers: bool,
}

fn default_true() -> bool {
    true
}

impl Mode {
    pub fn matches_app(&self, app_id: &str) -> bool {
        if self.apps.is_empty() {
            return false;
        }
        let lower = app_id.to_lowercase();
        self.apps
            .iter()
            .any(|a| !a.trim().is_empty() && lower.contains(&a.to_lowercase()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeStore {
    /// Id of the mode used when nothing else matches.
    pub default_id: String,
    pub modes: Vec<Mode>,
}

impl Default for ModeStore {
    fn default() -> Self {
        Self {
            default_id: "default".to_string(),
            modes: vec![Mode {
                id: "default".to_string(),
                name: "Default".to_string(),
                style: "formal".to_string(),
                model: String::new(),
                polish_prompt: String::new(),
                polish_enabled: false,
                apps: Vec::new(),
                remove_fillers: true,
            }],
        }
    }
}

impl ModeStore {
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Serialize error: {}", e))?;
        std::fs::write(path, json).map_err(|e| format!("Write error: {}", e))
    }

    pub fn default_mode(&self) -> &Mode {
        self.modes
            .iter()
            .find(|m| m.id == self.default_id)
            .or_else(|| self.modes.first())
            .expect("ModeStore always holds at least one mode")
    }

    /// The mode to use for the frontmost app.
    ///
    /// First match wins, so ordering in the list is the precedence rule and the
    /// user can reorder to disambiguate. Falls back to the default mode, which
    /// is why this cannot return None.
    pub fn resolve(&self, app_id: Option<&str>) -> &Mode {
        self.resolve_with_override(app_id, None)
    }

    /// As `resolve`, but an explicit pin wins over app matching.
    ///
    /// The pin exists for voice commands: saying "formal mode" has to change
    /// something the pipeline reads, and since modes took ownership of style
    /// that has to be the mode, not the old global style field.
    pub fn resolve_with_override(&self, app_id: Option<&str>, pinned: Option<&str>) -> &Mode {
        if let Some(id) = pinned {
            if let Some(m) = self.modes.iter().find(|m| m.id == id) {
                return m;
            }
        }
        if let Some(id) = app_id {
            if let Some(m) = self.modes.iter().find(|m| m.matches_app(id)) {
                return m;
            }
        }
        self.default_mode()
    }

    /// The first mode written in `style`, for voice commands that name a style
    /// ("formal mode") rather than a mode.
    pub fn first_with_style(&self, style: &str) -> Option<&Mode> {
        self.modes.iter().find(|m| m.style == style)
    }

    /// A mode by spoken name, matched loosely because speech recognition will
    /// not reproduce capitalisation or surrounding punctuation.
    pub fn find_by_name(&self, name: &str) -> Option<&Mode> {
        let want = name.trim().to_lowercase();
        self.modes.iter().find(|m| m.name.trim().to_lowercase() == want)
    }

    /// Build modes from an install that predates them.
    ///
    /// The default mode inherits the global style and polish settings. Each
    /// per-app style rule becomes its own mode, grouped by style so three rules
    /// pointing at "casual" produce one mode listing three apps rather than
    /// three near-identical modes.
    pub fn migrate_from(
        style: &str,
        polish_enabled: bool,
        polish_prompt: &str,
        remove_fillers: bool,
        app_rules: &[(String, String)],
    ) -> Self {
        let mut modes = vec![Mode {
            id: "default".to_string(),
            name: "Default".to_string(),
            style: style.to_string(),
            model: String::new(),
            polish_prompt: polish_prompt.to_string(),
            polish_enabled,
            apps: Vec::new(),
            remove_fillers,
        }];

        for rule_style in ["formal", "casual", "relaxed"] {
            // A rule matching the global style adds nothing: the default mode
            // already produces that result for those apps.
            if rule_style == style {
                continue;
            }
            let apps: Vec<String> = app_rules
                .iter()
                .filter(|(_, s)| s == rule_style)
                .map(|(app, _)| app.clone())
                .collect();
            if apps.is_empty() {
                continue;
            }
            modes.push(Mode {
                id: format!("migrated-{rule_style}"),
                name: format!("{} apps", capitalize(rule_style)),
                style: rule_style.to_string(),
                model: String::new(),
                polish_prompt: String::new(),
                polish_enabled: false,
                apps,
                remove_fillers,
            });
        }

        Self {
            default_id: "default".to_string(),
            modes,
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod pin_tests {
    use super::*;

    fn store() -> ModeStore {
        ModeStore {
            default_id: "default".into(),
            modes: vec![
                Mode { id: "default".into(), name: "Default".into(), style: "formal".into(), model: String::new(), polish_prompt: String::new(), polish_enabled: false, apps: vec![], remove_fillers: true },
                Mode { id: "chat".into(), name: "Chat".into(), style: "casual".into(), model: String::new(), polish_prompt: String::new(), polish_enabled: false, apps: vec!["com.tinyspeck.slackmacgap".into()], remove_fillers: true },
            ],
        }
    }

    #[test]
    fn a_pin_beats_app_matching() {
        // Slack would resolve to Chat; the pin must win, or a voice command
        // would appear to do nothing the moment the user is in a matched app.
        let s = store();
        let m = s.resolve_with_override(Some("com.tinyspeck.slackmacgap"), Some("default"));
        assert_eq!(m.id, "default");
    }

    #[test]
    fn no_pin_falls_back_to_app_matching() {
        let s = store();
        assert_eq!(s.resolve_with_override(Some("com.tinyspeck.slackmacgap"), None).id, "chat");
    }

    #[test]
    fn a_stale_pin_does_not_strand_the_user() {
        // The pinned mode was deleted. Resolution must carry on rather than
        // panic or return nothing.
        let s = store();
        let m = s.resolve_with_override(Some("com.tinyspeck.slackmacgap"), Some("deleted-mode"));
        assert_eq!(m.id, "chat");
    }

    #[test]
    fn style_lookup_finds_the_mode_a_spoken_command_means() {
        let s = store();
        assert_eq!(s.first_with_style("casual").map(|m| m.id.as_str()), Some("chat"));
        assert!(s.first_with_style("relaxed").is_none());
    }

    #[test]
    fn name_lookup_ignores_case_and_padding_from_speech() {
        let s = store();
        assert_eq!(s.find_by_name("  chat ").map(|m| m.id.as_str()), Some("chat"));
        assert!(s.find_by_name("nonexistent").is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Vec<(String, String)> {
        vec![
            ("com.microsoft.Outlook".into(), "formal".into()),
            ("com.tinyspeck.slackmacgap".into(), "casual".into()),
            ("com.apple.Terminal".into(), "relaxed".into()),
            ("com.googlecode.iterm2".into(), "relaxed".into()),
        ]
    }

    #[test]
    fn a_fresh_install_has_exactly_one_mode() {
        let s = ModeStore::default();
        assert_eq!(s.modes.len(), 1);
        assert_eq!(s.default_mode().id, "default");
    }

    #[test]
    fn migration_carries_the_global_settings_onto_the_default_mode() {
        let s = ModeStore::migrate_from("casual", true, "Fix grammar", false, &[]);
        let d = s.default_mode();
        assert_eq!(d.style, "casual");
        assert!(d.polish_enabled);
        assert_eq!(d.polish_prompt, "Fix grammar");
        assert!(!d.remove_fillers);
    }

    #[test]
    fn migration_groups_app_rules_by_style() {
        // Four rules, three distinct styles, one of which is the global style.
        let s = ModeStore::migrate_from("formal", false, "", true, &rules());
        // default + casual + relaxed. The formal rule folds into the default.
        assert_eq!(s.modes.len(), 3, "got {:?}", s.modes.iter().map(|m| &m.name).collect::<Vec<_>>());
        let relaxed = s.modes.iter().find(|m| m.style == "relaxed").unwrap();
        assert_eq!(relaxed.apps.len(), 2, "both terminals belong to one mode");
    }

    #[test]
    fn migration_drops_rules_that_match_the_global_style() {
        let s = ModeStore::migrate_from("formal", false, "", true, &rules());
        assert!(
            !s.modes.iter().any(|m| m.apps.iter().any(|a| a.contains("Outlook"))),
            "an Outlook rule for the global style is redundant with the default mode"
        );
    }

    #[test]
    fn resolve_prefers_a_matching_app_over_the_default() {
        let s = ModeStore::migrate_from("formal", false, "", true, &rules());
        assert_eq!(s.resolve(Some("com.tinyspeck.slackmacgap")).style, "casual");
        assert_eq!(s.resolve(Some("com.unknown.app")).style, "formal");
        assert_eq!(s.resolve(None).style, "formal");
    }

    #[test]
    fn first_match_wins_so_order_is_the_precedence_rule() {
        let mut s = ModeStore::default();
        s.modes.push(Mode {
            id: "a".into(), name: "A".into(), style: "casual".into(),
            model: String::new(), polish_prompt: String::new(), polish_enabled: false,
            apps: vec!["mail".into()], remove_fillers: true,
        });
        s.modes.push(Mode {
            id: "b".into(), name: "B".into(), style: "relaxed".into(),
            model: String::new(), polish_prompt: String::new(), polish_enabled: false,
            apps: vec!["mail".into()], remove_fillers: true,
        });
        assert_eq!(s.resolve(Some("com.apple.mail")).id, "a");
    }

    #[test]
    fn a_mode_with_no_apps_never_matches_on_identity() {
        // Otherwise the default mode, which has an empty list, would match
        // everything and shadow every other mode.
        let m = Mode {
            id: "d".into(), name: "D".into(), style: "formal".into(),
            model: String::new(), polish_prompt: String::new(), polish_enabled: false,
            apps: vec![], remove_fillers: true,
        };
        assert!(!m.matches_app("anything at all"));
    }

    #[test]
    fn resolution_survives_a_default_id_pointing_at_nothing() {
        let mut s = ModeStore::default();
        s.default_id = "gone".into();
        assert_eq!(s.default_mode().id, "default", "falls back to the first mode");
    }
}
