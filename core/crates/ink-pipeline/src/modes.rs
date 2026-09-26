//! Modes: one named bundle of everything that decides how a dictation is written. Ported from
//! Inkwell 0.2's `modes.rs`, tests included.
//!
//! A mode carries the style, optionally a model and a polish prompt, the apps it activates in, and
//! whether filler removal runs. Resolution: a mode pinned by a voice command wins; otherwise the
//! first mode whose app list matches the frontmost application; otherwise the default mode.
//!
//! Changes in the port: the style is typed ([`Style`]) instead of free text that fell back to a
//! global setting when it did not parse; an empty model is `None`; and a store with no modes at
//! all (a damaged setting) resolves to a built-in default instead of panicking, as 0.2's
//! `expect` did.

use std::sync::LazyLock;

use crate::style::Style;

/// One mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mode {
    /// A stable id.
    pub id: String,
    /// The name the user sees and can say.
    pub name: String,
    /// How the text is written.
    pub style: Style,
    /// A model id, or `None` for whatever is loaded. Switching models costs a multi-second load,
    /// so it is opt-in.
    pub model: Option<String>,
    /// This mode's polish prompt; blank means the global prompt.
    pub polish_prompt: String,
    /// Whether dictations in this mode are polished (when a model is available).
    pub polish_enabled: bool,
    /// Substrings matched, without case, against the frontmost app's identity: the bundle id on
    /// macOS (`com.example.mail`), the executable name on Windows.
    pub apps: Vec<String>,
    /// Whether fillers and stutters are removed.
    pub remove_fillers: bool,
}

impl Mode {
    /// The mode a fresh install has.
    pub fn builtin_default() -> Self {
        Self {
            id: "default".to_owned(),
            name: "Default".to_owned(),
            style: Style::Formal,
            model: None,
            polish_prompt: String::new(),
            polish_enabled: false,
            apps: Vec::new(),
            remove_fillers: true,
        }
    }

    /// Whether the mode activates in the app identified by `app_id`. A mode with no apps never
    /// matches by identity, or the default mode would shadow every other one.
    pub fn matches_app(&self, app_id: &str) -> bool {
        let app = app_id.to_lowercase();
        self.apps
            .iter()
            .any(|a| !a.trim().is_empty() && app.contains(&a.to_lowercase()))
    }
}

/// The built-in default, for a store with no modes.
static BUILTIN_DEFAULT: LazyLock<Mode> = LazyLock::new(Mode::builtin_default);

/// The user's modes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModeStore {
    /// The mode used when nothing else matches.
    pub default_id: String,
    /// In precedence order: the first match wins.
    pub modes: Vec<Mode>,
}

impl Default for ModeStore {
    fn default() -> Self {
        Self {
            default_id: "default".to_owned(),
            modes: vec![Mode::builtin_default()],
        }
    }
}

impl ModeStore {
    /// The default mode: the one named by `default_id`, else the first, else the built-in one.
    pub fn default_mode(&self) -> &Mode {
        self.modes
            .iter()
            .find(|m| m.id == self.default_id)
            .or_else(|| self.modes.first())
            .unwrap_or(&BUILTIN_DEFAULT)
    }

    /// The mode for the frontmost app. See [`resolve_with_override`](Self::resolve_with_override).
    pub fn resolve(&self, app_id: Option<&str>) -> &Mode {
        self.resolve_with_override(app_id, None)
    }

    /// The mode to use: `pinned` (a voice command's choice) when it still exists, else the first
    /// mode matching `app_id`, else the default. Never fails: a stale pin falls through.
    pub fn resolve_with_override(&self, app_id: Option<&str>, pinned: Option<&str>) -> &Mode {
        pinned
            .and_then(|id| self.modes.iter().find(|m| m.id == id))
            .or_else(|| app_id.and_then(|id| self.modes.iter().find(|m| m.matches_app(id))))
            .unwrap_or_else(|| self.default_mode())
    }

    /// The first mode written in `style`, for a command that names a style ("formal mode").
    pub fn first_with_style(&self, style: Style) -> Option<&Mode> {
        self.modes.iter().find(|m| m.style == style)
    }

    /// A mode by spoken name, ignoring case and surrounding whitespace.
    pub fn find_by_name(&self, name: &str) -> Option<&Mode> {
        let want = name.trim().to_lowercase();
        self.modes
            .iter()
            .find(|m| m.name.trim().to_lowercase() == want)
    }

    /// Modes from an install that predates them: the default mode inherits the global style and
    /// polish settings, and the per-app style rules become one mode per style (rules for the global
    /// style add nothing and are dropped).
    pub fn migrate_from(
        style: Style,
        polish_enabled: bool,
        polish_prompt: &str,
        remove_fillers: bool,
        app_rules: &[(String, Style)],
    ) -> Self {
        let mut modes = vec![Mode {
            style,
            polish_prompt: polish_prompt.to_owned(),
            polish_enabled,
            remove_fillers,
            ..Mode::builtin_default()
        }];
        for rule_style in Style::ALL {
            if rule_style == style {
                continue;
            }
            let apps: Vec<String> = app_rules
                .iter()
                .filter(|(_, s)| *s == rule_style)
                .map(|(app, _)| app.clone())
                .collect();
            if apps.is_empty() {
                continue;
            }
            modes.push(Mode {
                id: format!("migrated-{}", rule_style.as_str()),
                name: format!("{} apps", capitalise(rule_style.as_str())),
                style: rule_style,
                model: None,
                polish_prompt: String::new(),
                polish_enabled: false,
                apps,
                remove_fillers,
            });
        }
        Self {
            default_id: "default".to_owned(),
            modes,
        }
    }
}

fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod pin_tests {
    use super::*;

    fn mode(id: &str, name: &str, style: Style, apps: &[&str]) -> Mode {
        Mode {
            id: id.into(),
            name: name.into(),
            style,
            apps: apps.iter().map(|a| (*a).into()).collect(),
            ..Mode::builtin_default()
        }
    }

    fn store() -> ModeStore {
        ModeStore {
            default_id: "default".into(),
            modes: vec![
                mode("default", "Default", Style::Formal, &[]),
                mode("chat", "Chat", Style::Casual, &["com.example.chat"]),
            ],
        }
    }

    #[test]
    fn a_pin_beats_app_matching() {
        // The chat app would resolve to Chat; the pin must win, or a voice command would appear to
        // do nothing the moment the user is in a matched app.
        let s = store();
        let m = s.resolve_with_override(Some("com.example.chat"), Some("default"));
        assert_eq!(m.id, "default");
    }

    #[test]
    fn no_pin_falls_back_to_app_matching() {
        let s = store();
        assert_eq!(
            s.resolve_with_override(Some("com.example.chat"), None).id,
            "chat"
        );
    }

    #[test]
    fn a_stale_pin_does_not_strand_the_user() {
        // The pinned mode was deleted. Resolution carries on rather than failing.
        let s = store();
        let m = s.resolve_with_override(Some("com.example.chat"), Some("deleted-mode"));
        assert_eq!(m.id, "chat");
    }

    #[test]
    fn style_lookup_finds_the_mode_a_spoken_command_means() {
        let s = store();
        assert_eq!(
            s.first_with_style(Style::Casual).map(|m| m.id.as_str()),
            Some("chat")
        );
        assert!(s.first_with_style(Style::Relaxed).is_none());
    }

    #[test]
    fn name_lookup_ignores_case_and_padding_from_speech() {
        let s = store();
        assert_eq!(
            s.find_by_name("  chat ").map(|m| m.id.as_str()),
            Some("chat")
        );
        assert!(s.find_by_name("nonexistent").is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Vec<(String, Style)> {
        vec![
            ("com.example.mail".into(), Style::Formal),
            ("com.example.chat".into(), Style::Casual),
            ("com.example.terminal".into(), Style::Relaxed),
            ("com.example.console".into(), Style::Relaxed),
        ]
    }

    fn with_apps(id: &str, style: Style, apps: &[&str]) -> Mode {
        Mode {
            id: id.into(),
            name: id.to_uppercase(),
            style,
            apps: apps.iter().map(|a| (*a).into()).collect(),
            ..Mode::builtin_default()
        }
    }

    #[test]
    fn a_fresh_install_has_exactly_one_mode() {
        let s = ModeStore::default();
        assert_eq!(s.modes.len(), 1);
        assert_eq!(s.default_mode().id, "default");
    }

    #[test]
    fn migration_carries_the_global_settings_onto_the_default_mode() {
        let s = ModeStore::migrate_from(Style::Casual, true, "Fix grammar", false, &[]);
        let d = s.default_mode();
        assert_eq!(d.style, Style::Casual);
        assert!(d.polish_enabled);
        assert_eq!(d.polish_prompt, "Fix grammar");
        assert!(!d.remove_fillers);
    }

    #[test]
    fn migration_groups_app_rules_by_style() {
        // Four rules, three distinct styles, one of which is the global style.
        let s = ModeStore::migrate_from(Style::Formal, false, "", true, &rules());
        // default + casual + relaxed. The formal rule folds into the default.
        assert_eq!(
            s.modes.len(),
            3,
            "got {:?}",
            s.modes.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
        let relaxed = s.modes.iter().find(|m| m.style == Style::Relaxed);
        assert_eq!(
            relaxed.map(|m| m.apps.len()),
            Some(2),
            "both terminals belong to one mode"
        );
    }

    #[test]
    fn migration_drops_rules_that_match_the_global_style() {
        let s = ModeStore::migrate_from(Style::Formal, false, "", true, &rules());
        assert!(
            !s.modes
                .iter()
                .any(|m| m.apps.iter().any(|a| a.contains("mail"))),
            "a mail rule for the global style is redundant with the default mode"
        );
    }

    #[test]
    fn resolve_prefers_a_matching_app_over_the_default() {
        let s = ModeStore::migrate_from(Style::Formal, false, "", true, &rules());
        assert_eq!(s.resolve(Some("com.example.chat")).style, Style::Casual);
        assert_eq!(s.resolve(Some("com.unknown.app")).style, Style::Formal);
        assert_eq!(s.resolve(None).style, Style::Formal);
    }

    #[test]
    fn first_match_wins_so_order_is_the_precedence_rule() {
        let mut s = ModeStore::default();
        s.modes.push(with_apps("a", Style::Casual, &["mail"]));
        s.modes.push(with_apps("b", Style::Relaxed, &["mail"]));
        assert_eq!(s.resolve(Some("com.example.mail")).id, "a");
    }

    #[test]
    fn a_mode_with_no_apps_never_matches_on_identity() {
        // Otherwise the default mode, which has an empty list, would match everything and shadow
        // every other mode.
        let m = with_apps("d", Style::Formal, &[]);
        assert!(!m.matches_app("anything at all"));
    }

    #[test]
    fn resolution_survives_a_default_id_pointing_at_nothing() {
        let s = ModeStore {
            default_id: "gone".into(),
            ..ModeStore::default()
        };
        assert_eq!(
            s.default_mode().id,
            "default",
            "falls back to the first mode"
        );
    }

    // New in 1.0: 0.2 panicked here.

    #[test]
    fn a_store_with_no_modes_resolves_to_the_builtin_default() {
        let s = ModeStore {
            default_id: "default".into(),
            modes: Vec::new(),
        };
        assert_eq!(
            s.resolve(Some("com.example.chat")),
            &Mode::builtin_default()
        );
    }
}
