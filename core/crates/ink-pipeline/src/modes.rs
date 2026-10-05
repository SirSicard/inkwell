//! Modes: one named bundle of everything that decides how a dictation is written. Ported from
//! Inkwell 0.2's `modes.rs`, tests included.
//!
//! A mode carries the style, optionally its own language model and a polish prompt, the apps it
//! activates in, and whether filler removal runs. Resolution: a mode pinned by a voice command
//! wins; otherwise the first mode whose app list matches the frontmost application; otherwise the
//! default mode.
//!
//! Changes in the port: the style is typed ([`Style`]) instead of free text that fell back to a
//! global setting when it did not parse; and a store with no modes at all (a damaged setting)
//! resolves to a built-in default instead of panicking, as 0.2's `expect` did.
//!
//! # The stored document
//!
//! The modes are one JSON document, `{"default_id", "modes": [{"id", "name", "style",
//! "polish_enabled", "remove_fillers", "polish_prompt", "apps", "polish_model"}]}` (the shape the
//! 0.2 import writes, plus `polish_model`). [`ModeStore::from_json`] is its one reader, for
//! dictation and Settings alike; [`ModeStore::write_into`] writes a store back into the stored
//! document, changing only the fields this build knows. A field it does not know survives a save:
//! 0.2's `model` (which named a transcription model, never a language model, so 1.0 does not read
//! it), or a newer build's; and so does a style it does not know, until the user picks another.
//!
//! # Editing
//!
//! [`ModeStore::save`] and [`ModeStore::delete`] are pure: each checks the change against the
//! rules ([`ModeError`]) and gives the store as it would be after it. A save is checked on what it
//! changes, so a mode the 0.2 import brought that breaks a rule (an app two modes name, a prompt
//! over the cap, a name that is a style's) stays editable in its other fields.
//!
//! # A mode's language model
//!
//! [`Mode::polish_model`] names the model a mode's dictations are polished on, by the id the core
//! gives it (the chain finds it at each take: [`ModeModels`](crate::chain::ModeModels)), or `None`
//! for the model Settings > AI chose. A mode whose model the core does not hold at that moment is
//! not polished at all, never polished on another model in its place: that could send the words
//! somewhere the user did not pick for this mode.

use std::collections::HashSet;
use std::sync::LazyLock;

use serde_json::{Map, Value};

use crate::style::Style;

/// The most modes a store holds.
pub const MAX_MODES: usize = 50;

/// The longest mode name, in characters, after trimming.
pub const MAX_NAME_CHARS: usize = 64;

/// The longest polish prompt, in characters.
pub const MAX_PROMPT_CHARS: usize = 2_000;

/// The most apps one mode names.
pub const MAX_APPS: usize = 64;

/// The longest app identity, in characters.
pub const MAX_APP_CHARS: usize = 256;

/// One mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mode {
    /// A stable id.
    pub id: String,
    /// The name the user sees and can say.
    pub name: String,
    /// How the text is written.
    pub style: Style,
    /// The stored style is one this build does not know (a newer build's). The mode writes as
    /// [`style`](Self::style) says (the built-in default's), Settings shows it as its own, and a
    /// save keeps the stored one until the user picks a style.
    pub style_unknown: bool,
    /// The language model this mode is polished on, by the id the core gives it, or `None` for the
    /// model Settings > AI chose. When the core holds no model by this id, the mode's dictations go
    /// out unpolished and say so; never to another model.
    pub polish_model: Option<String>,
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
            style_unknown: false,
            polish_model: None,
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

/// A name as it is compared: without case, and with its spaces collapsed to one, as speech gives
/// it. Two names that read alike here sound alike, so a voice command could not tell them apart.
pub fn spoken_name(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// An app identity as two modes are compared on it: trimmed, without case.
fn app_key(app: &str) -> String {
    app.trim().to_lowercase()
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

/// A stored modes document that does not read. Reported, never read as the defaults: the next
/// save would then replace the user's modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModesUnreadable;

impl std::fmt::Display for ModesUnreadable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the stored modes cannot be read")
    }
}

impl std::error::Error for ModesUnreadable {}

/// One mode added or changed (Settings' editor). `None` keeps what the mode has (for a new mode,
/// the default: formal, fillers removed, no polish, no apps, the AI setting's model); the name is
/// needed for a new mode.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModeEdit {
    /// The mode changed, or `None` to add one.
    pub id: Option<String>,
    /// Its name, trimmed.
    pub name: Option<String>,
    /// How it writes.
    pub style: Option<Style>,
    /// Whether it polishes.
    pub polish_enabled: Option<bool>,
    /// Whether fillers are removed.
    pub remove_fillers: Option<bool>,
    /// Its polish prompt; blank for the default.
    pub polish_prompt: Option<String>,
    /// Its apps, the whole list. Blank entries are dropped, and an app named twice is kept once.
    pub apps: Option<Vec<String>>,
    /// Its language model: `Some(None)` (or a blank id) for the AI setting's.
    pub polish_model: Option<Option<String>>,
}

/// Which limit a change went over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Limit {
    /// The name, [`MAX_NAME_CHARS`].
    Name,
    /// The polish prompt, [`MAX_PROMPT_CHARS`].
    PolishPrompt,
    /// How many apps a mode names, [`MAX_APPS`].
    Apps,
    /// One app identity, [`MAX_APP_CHARS`].
    App,
    /// How many modes there are, [`MAX_MODES`].
    Modes,
}

/// Why a save or a delete was refused. Each says what is wrong, never the user's words (a name, a
/// prompt, an app): those reach the shell only in the listing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModeError {
    /// The name is blank.
    NameBlank,
    /// Another mode has a name that sounds the same ([`spoken_name`]).
    NameTaken,
    /// The name is a style's (formal, casual, relaxed): a voice command naming it would pick the
    /// style, never this mode.
    NameIsStyle,
    /// Over a limit.
    TooLong(Limit),
    /// The default mode cannot be deleted or given apps: it is the mode of every app without one.
    DefaultMode,
    /// An app the mode is given is another mode's (and the save did not say to move it).
    AppTaken,
    /// No mode has that id.
    NotFound,
    /// The core holds no language model by that id.
    ModelUnknown,
}

impl ModeError {
    /// The code a shell tells it apart by (`command.failed`'s `code`).
    pub fn code(self) -> &'static str {
        match self {
            Self::NameBlank => "name_blank",
            Self::NameTaken => "name_taken",
            Self::NameIsStyle => "name_is_style",
            Self::TooLong(_) => "too_long",
            Self::DefaultMode => "default_mode",
            Self::AppTaken => "app_taken",
            Self::NotFound => "mode_not_found",
            Self::ModelUnknown => "model_unknown",
        }
    }
}

impl std::fmt::Display for ModeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NameBlank => f.write_str("a mode needs a name"),
            Self::NameTaken => f.write_str("another mode has that name"),
            Self::NameIsStyle => f.write_str(
                "a mode can't be named formal, casual or relaxed: voice commands use those names for styles",
            ),
            Self::TooLong(Limit::Name) => {
                write!(f, "a mode's name is at most {MAX_NAME_CHARS} characters")
            }
            Self::TooLong(Limit::PolishPrompt) => {
                write!(f, "polish instructions are at most {MAX_PROMPT_CHARS} characters")
            }
            Self::TooLong(Limit::Apps) => write!(f, "a mode names at most {MAX_APPS} apps"),
            Self::TooLong(Limit::App) => {
                write!(f, "an app's identity is at most {MAX_APP_CHARS} characters")
            }
            Self::TooLong(Limit::Modes) => write!(f, "there are at most {MAX_MODES} modes"),
            Self::DefaultMode => f.write_str(
                "the default mode is used in every app without a mode of its own: it can't be deleted or given apps",
            ),
            Self::AppTaken => f.write_str("an app this mode is given is in another mode"),
            Self::NotFound => f.write_str("no mode has that id"),
            Self::ModelUnknown => f.write_str("there is no language model by that id"),
        }
    }
}

impl std::error::Error for ModeError {}

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

    /// A mode by spoken name, ignoring case and spacing ([`spoken_name`]).
    pub fn find_by_name(&self, name: &str) -> Option<&Mode> {
        let want = spoken_name(name);
        self.modes.iter().find(|m| spoken_name(&m.name) == want)
    }

    /// Reads the stored document. `default_id`, and each mode's `id` and `name`, are needed;
    /// `polish_enabled` defaults to off, `remove_fillers` to on, the prompt to blank, the apps to
    /// none and `polish_model` (absent, `null` or blank) to the AI setting's model. A style that is
    /// absent or unknown writes as the default's ([`Mode::style_unknown`]). Fields this build does
    /// not know are ignored here and kept by [`write_into`](Self::write_into).
    pub fn from_json(text: &str) -> Result<Self, ModesUnreadable> {
        let v: Value = serde_json::from_str(text).map_err(|_| ModesUnreadable)?;
        let default_id = v
            .get("default_id")
            .and_then(Value::as_str)
            .ok_or(ModesUnreadable)?
            .to_owned();
        let list = v
            .get("modes")
            .and_then(Value::as_array)
            .ok_or(ModesUnreadable)?;
        let modes = list.iter().map(read_mode).collect::<Result<_, _>>()?;
        Ok(Self { default_id, modes })
    }

    /// The stored form of this store, as a new document.
    pub fn to_json(&self) -> String {
        self.fill(Map::new(), Vec::new()).to_string()
    }

    /// Writes this store into `doc`, the stored document it was read from: every mode's known
    /// fields as this store has them, each mode's unknown fields as `doc` had them (matched by
    /// id), the modes in this store's order, and `doc`'s other fields left alone. A mode `doc` has
    /// and this store has not is gone; a mode with an unknown style keeps the stored one.
    pub fn write_into(&self, doc: &str) -> Result<String, ModesUnreadable> {
        let Value::Object(mut top) = serde_json::from_str(doc).map_err(|_| ModesUnreadable)? else {
            return Err(ModesUnreadable);
        };
        let stored = match top.remove("modes") {
            None => Vec::new(),
            Some(Value::Array(list)) => list,
            Some(_) => return Err(ModesUnreadable),
        };
        Ok(self.fill(top, stored).to_string())
    }

    /// `top` with this store's `default_id` and modes, each written over its stored object in
    /// `stored` (the first one by its id not used yet, so two modes the import gave one id keep
    /// their own fields).
    fn fill(&self, mut top: Map<String, Value>, stored: Vec<Value>) -> Value {
        let mut stored: Vec<Option<Map<String, Value>>> = stored
            .into_iter()
            .map(|v| match v {
                Value::Object(o) => Some(o),
                _ => None,
            })
            .collect();
        let modes = self
            .modes
            .iter()
            .map(|m| {
                let mut item = stored
                    .iter_mut()
                    .find(|o| {
                        o.as_ref()
                            .is_some_and(|o| o.get("id").and_then(Value::as_str) == Some(&m.id))
                    })
                    .and_then(Option::take)
                    .unwrap_or_default();
                item.insert("id".into(), m.id.clone().into());
                item.insert("name".into(), m.name.clone().into());
                if !m.style_unknown {
                    item.insert("style".into(), m.style.as_str().into());
                }
                item.insert("polish_enabled".into(), m.polish_enabled.into());
                item.insert("remove_fillers".into(), m.remove_fillers.into());
                item.insert("polish_prompt".into(), m.polish_prompt.clone().into());
                item.insert(
                    "apps".into(),
                    Value::Array(m.apps.iter().map(|a| a.clone().into()).collect()),
                );
                match &m.polish_model {
                    Some(model) => {
                        item.insert("polish_model".into(), model.clone().into());
                    }
                    None => {
                        item.remove("polish_model");
                    }
                }
                Value::Object(item)
            })
            .collect();
        top.insert("default_id".into(), self.default_id.clone().into());
        top.insert("modes".into(), Value::Array(modes));
        Value::Object(top)
    }

    /// An id for a new mode: `m` and the time in milliseconds, past any id taken.
    pub fn fresh_id(&self, now_unix_ms: i64) -> String {
        let mut ms = now_unix_ms.max(0);
        loop {
            let id = format!("m{ms}");
            if !self.modes.iter().any(|m| m.id == id) {
                return id;
            }
            ms = ms.saturating_add(1);
        }
    }

    /// The store after `edit`, and the id of the mode saved. `new_id` is a new mode's id
    /// ([`fresh_id`](Self::fresh_id)); `take_apps` moves an app another mode names to this one
    /// (otherwise [`ModeError::AppTaken`]); `model_known` says whether the core holds a language
    /// model by an id. Each rule is checked only on what the edit changes.
    pub fn save(
        &self,
        edit: &ModeEdit,
        take_apps: bool,
        new_id: &str,
        model_known: &dyn Fn(&str) -> bool,
    ) -> Result<(Self, String), ModeError> {
        let (index, before) = match &edit.id {
            Some(id) => {
                let i = self
                    .modes
                    .iter()
                    .position(|m| &m.id == id)
                    .ok_or(ModeError::NotFound)?;
                (Some(i), Some(&self.modes[i]))
            }
            None if self.modes.len() >= MAX_MODES => {
                return Err(ModeError::TooLong(Limit::Modes));
            }
            None => (None, None),
        };
        let mut mode = before.cloned().unwrap_or_else(|| Mode {
            id: new_id.to_owned(),
            name: String::new(),
            ..Mode::builtin_default()
        });
        if let Some(name) = &edit.name {
            name.trim().clone_into(&mut mode.name);
        }
        if let Some(style) = edit.style {
            mode.style = style;
            mode.style_unknown = false;
        }
        if let Some(on) = edit.polish_enabled {
            mode.polish_enabled = on;
        }
        if let Some(on) = edit.remove_fillers {
            mode.remove_fillers = on;
        }
        if let Some(prompt) = &edit.polish_prompt {
            // Blank is the default prompt; the user's text is otherwise kept as typed.
            mode.polish_prompt = if prompt.trim().is_empty() {
                String::new()
            } else {
                prompt.clone()
            };
        }
        if let Some(apps) = &edit.apps {
            let mut seen = HashSet::new();
            mode.apps = apps
                .iter()
                .map(|a| a.trim())
                .filter(|a| !a.is_empty() && seen.insert(app_key(a)))
                .map(str::to_owned)
                .collect();
        }
        if let Some(model) = &edit.polish_model {
            mode.polish_model = model
                .as_deref()
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .map(str::to_owned);
        }

        let added = self.check(&mode, before, take_apps, model_known)?;

        let mut after = self.clone();
        if take_apps && !added.is_empty() {
            for other in after.modes.iter_mut().filter(|m| m.id != mode.id) {
                other.apps.retain(|a| !added.contains(&app_key(a)));
            }
        }
        let id = mode.id.clone();
        match index {
            Some(i) => after.modes[i] = mode,
            None => after.modes.push(mode),
        }
        Ok((after, id))
    }

    /// The rules, on what `mode` changes from `before` (all of it for a new mode). Gives the apps
    /// the mode is given that it did not name before, by [`app_key`].
    fn check(
        &self,
        mode: &Mode,
        before: Option<&Mode>,
        take_apps: bool,
        model_known: &dyn Fn(&str) -> bool,
    ) -> Result<HashSet<String>, ModeError> {
        if before.is_none_or(|b| b.name != mode.name) {
            if mode.name.is_empty() {
                return Err(ModeError::NameBlank);
            }
            if mode.name.chars().count() > MAX_NAME_CHARS {
                return Err(ModeError::TooLong(Limit::Name));
            }
            if Style::parse(&mode.name).is_some() {
                return Err(ModeError::NameIsStyle);
            }
            let spoken = spoken_name(&mode.name);
            if self
                .modes
                .iter()
                .any(|m| m.id != mode.id && spoken_name(&m.name) == spoken)
            {
                return Err(ModeError::NameTaken);
            }
        }
        if before.is_none_or(|b| b.polish_prompt != mode.polish_prompt)
            && mode.polish_prompt.chars().count() > MAX_PROMPT_CHARS
        {
            return Err(ModeError::TooLong(Limit::PolishPrompt));
        }
        let had: HashSet<String> = before
            .map(|b| b.apps.iter().map(|a| app_key(a)).collect())
            .unwrap_or_default();
        let added: HashSet<String> = mode
            .apps
            .iter()
            .map(|a| app_key(a))
            .filter(|a| !had.contains(a))
            .collect();
        if before.is_none_or(|b| b.apps != mode.apps) {
            if mode.apps.len() > MAX_APPS {
                return Err(ModeError::TooLong(Limit::Apps));
            }
            if mode.apps.iter().any(|a| a.chars().count() > MAX_APP_CHARS) {
                return Err(ModeError::TooLong(Limit::App));
            }
        }
        if !added.is_empty() {
            if mode.id == self.default_mode().id {
                return Err(ModeError::DefaultMode);
            }
            let taken = self
                .modes
                .iter()
                .filter(|m| m.id != mode.id)
                .any(|m| m.apps.iter().any(|a| added.contains(&app_key(a))));
            if taken && !take_apps {
                return Err(ModeError::AppTaken);
            }
        }
        if before.and_then(|b| b.polish_model.as_deref()) != mode.polish_model.as_deref()
            && let Some(model) = &mode.polish_model
            && !model_known(model)
        {
            return Err(ModeError::ModelUnknown);
        }
        Ok(added)
    }

    /// The store without mode `id`. Its apps go back to the default mode, which cannot be deleted.
    pub fn delete(&self, id: &str) -> Result<Self, ModeError> {
        let i = self
            .modes
            .iter()
            .position(|m| m.id == id)
            .ok_or(ModeError::NotFound)?;
        if self.modes[i].id == self.default_mode().id {
            return Err(ModeError::DefaultMode);
        }
        let mut after = self.clone();
        after.modes.remove(i);
        Ok(after)
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
                polish_enabled: false,
                apps,
                remove_fillers,
                ..Mode::builtin_default()
            });
        }
        Self {
            default_id: "default".to_owned(),
            modes,
        }
    }
}

/// One mode of the stored document. See [`ModeStore::from_json`].
fn read_mode(m: &Value) -> Result<Mode, ModesUnreadable> {
    let s = |k: &str| m.get(k).and_then(Value::as_str);
    let flag = |k: &str, default: bool| match m.get(k) {
        None => Ok(default),
        Some(b) => b.as_bool().ok_or(ModesUnreadable),
    };
    let apps = match m.get("apps") {
        None => Vec::new(),
        Some(Value::Array(apps)) => apps
            .iter()
            .map(|a| a.as_str().map(str::to_owned))
            .collect::<Option<_>>()
            .ok_or(ModesUnreadable)?,
        Some(_) => return Err(ModesUnreadable),
    };
    // A model that does not read is refused with the document rather than read as none: none is
    // the AI setting's model, which may send somewhere this mode was set up not to.
    let polish_model = match m.get("polish_model") {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) => Some(id.trim()).filter(|id| !id.is_empty()),
        Some(_) => return Err(ModesUnreadable),
    };
    let style = s("style").and_then(Style::parse);
    Ok(Mode {
        id: s("id").ok_or(ModesUnreadable)?.to_owned(),
        name: s("name").ok_or(ModesUnreadable)?.to_owned(),
        style: style.unwrap_or(Mode::builtin_default().style),
        style_unknown: style.is_none(),
        polish_model: polish_model.map(str::to_owned),
        polish_prompt: s("polish_prompt").unwrap_or_default().to_owned(),
        polish_enabled: flag("polish_enabled", false)?,
        apps,
        remove_fillers: flag("remove_fillers", true)?,
    })
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

/// The stored document, the rules a save is checked against, and what a save keeps.
#[cfg(test)]
mod edit_tests {
    use super::*;

    /// What the 0.2 import writes (with 0.2's `model`, a transcription model's name), plus a field
    /// from a newer build and a style this build does not know.
    const IMPORTED: &str = r#"{"default_id":"d","later":{"x":1},"modes":[
        {"id":"d","name":"Everywhere else","style":"formal","model":"","polish_prompt":"","polish_enabled":true,"apps":[],"remove_fillers":true},
        {"id":"c","name":"Chat","style":"casual","model":"ggml-base.en","polish_prompt":"Keep it short.","polish_enabled":false,"apps":["com.example.chat"],"remove_fillers":false,"newer":true},
        {"id":"x","name":"Odd","style":"shouting","polish_model":"engine:local"}]}"#;

    fn imported() -> ModeStore {
        ModeStore::from_json(IMPORTED).unwrap()
    }

    fn known(id: &str) -> bool {
        ["engine:local", "provider:anthropic"].contains(&id)
    }

    fn save(store: &ModeStore, edit: ModeEdit) -> Result<(ModeStore, String), ModeError> {
        store.save(&edit, false, "m100", &known)
    }

    fn named(name: &str) -> ModeEdit {
        ModeEdit {
            name: Some(name.into()),
            ..ModeEdit::default()
        }
    }

    fn edit(id: &str) -> ModeEdit {
        ModeEdit {
            id: Some(id.into()),
            ..ModeEdit::default()
        }
    }

    fn mode<'a>(store: &'a ModeStore, id: &str) -> &'a Mode {
        store.modes.iter().find(|m| m.id == id).unwrap()
    }

    #[test]
    fn the_stored_document_reads_as_dictation_and_settings_use_it() {
        let s = imported();
        assert_eq!(s.default_id, "d");
        assert_eq!(s.modes.len(), 3);
        let chat = mode(&s, "c");
        assert_eq!(chat.style, Style::Casual);
        assert_eq!(chat.polish_prompt, "Keep it short.");
        assert_eq!(chat.apps, ["com.example.chat"]);
        assert!(!chat.remove_fillers);
        assert_eq!(
            chat.polish_model, None,
            "0.2's model named a transcription model: never read as a language model"
        );
        let odd = mode(&s, "x");
        assert!(odd.style_unknown, "a newer build's style");
        assert_eq!(odd.style, Style::Formal, "writes as the default's");
        assert_eq!(odd.polish_model.as_deref(), Some("engine:local"));
        assert!(odd.remove_fillers, "0.2's default");
        assert!(!odd.polish_enabled);
        for blank in [r#""polish_model":null"#, r#""polish_model":"  ""#] {
            let doc = format!(r#"{{"default_id":"d","modes":[{{"id":"d","name":"D",{blank}}}]}}"#);
            assert_eq!(
                ModeStore::from_json(&doc).unwrap().modes[0].polish_model,
                None
            );
        }
    }

    #[test]
    fn a_damaged_document_is_refused_never_read_as_the_defaults() {
        for bad in [
            "not json",
            "[]",
            r#"{"modes":[]}"#,
            r#"{"default_id":"d"}"#,
            r#"{"default_id":"d","modes":[{"name":"x"}]}"#,
            r#"{"default_id":"d","modes":[{"id":"x"}]}"#,
            r#"{"default_id":"d","modes":[{"id":"x","name":"x","apps":[3]}]}"#,
            r#"{"default_id":"d","modes":[{"id":"x","name":"x","apps":"slack"}]}"#,
            r#"{"default_id":"d","modes":[{"id":"x","name":"x","polish_enabled":"yes"}]}"#,
            r#"{"default_id":"d","modes":[{"id":"x","name":"x","polish_model":5}]}"#,
        ] {
            assert_eq!(ModeStore::from_json(bad), Err(ModesUnreadable), "{bad}");
        }
    }

    #[test]
    fn a_save_keeps_every_field_this_build_does_not_know() {
        let (after, _) = save(
            &imported(),
            ModeEdit {
                polish_prompt: Some("Shorter.".into()),
                ..edit("c")
            },
        )
        .unwrap();
        let written = after.write_into(IMPORTED).unwrap();
        let v: Value = serde_json::from_str(&written).unwrap();
        assert_eq!(v["later"]["x"], 1, "a newer build's top-level field");
        assert_eq!(v["modes"][1]["model"], "ggml-base.en", "0.2's model");
        assert_eq!(v["modes"][1]["newer"], true);
        assert_eq!(v["modes"][1]["polish_prompt"], "Shorter.");
        assert_eq!(
            v["modes"][2]["style"], "shouting",
            "kept until the user picks one"
        );
        assert_eq!(ModeStore::from_json(&written).unwrap(), after, "reads back");
        // Picking a style replaces the unknown one.
        let (picked, _) = save(
            &after,
            ModeEdit {
                style: Some(Style::Relaxed),
                ..edit("x")
            },
        )
        .unwrap();
        let v: Value = serde_json::from_str(&picked.write_into(&written).unwrap()).unwrap();
        assert_eq!(v["modes"][2]["style"], "relaxed");
    }

    #[test]
    fn a_new_document_reads_back_and_a_cleared_model_leaves_no_field() {
        let s = ModeStore::default();
        assert_eq!(ModeStore::from_json(&s.to_json()).unwrap(), s);
        let (picked, _) = save(
            &imported(),
            ModeEdit {
                polish_model: Some(Some("provider:anthropic".into())),
                ..edit("c")
            },
        )
        .unwrap();
        let doc = picked.write_into(IMPORTED).unwrap();
        assert!(doc.contains("provider:anthropic"));
        let (cleared, _) = save(
            &picked,
            ModeEdit {
                polish_model: Some(None),
                ..edit("c")
            },
        )
        .unwrap();
        let v: Value = serde_json::from_str(&cleared.write_into(&doc).unwrap()).unwrap();
        assert!(v["modes"][1].get("polish_model").is_none(), "{v}");
        let (blank, _) = save(
            &picked,
            ModeEdit {
                polish_model: Some(Some(" ".into())),
                ..edit("c")
            },
        )
        .unwrap();
        assert_eq!(
            mode(&blank, "c").polish_model,
            None,
            "blank is the AI setting's"
        );
    }

    #[test]
    fn a_save_without_an_id_adds_a_mode_with_the_defaults() {
        let (after, id) = save(&imported(), named("  Mail  ")).unwrap();
        assert_eq!(id, "m100");
        let added = after.modes.last().unwrap();
        assert_eq!(added.id, "m100");
        assert_eq!(added.name, "Mail", "trimmed");
        assert_eq!(added.style, Style::Formal);
        assert!(added.remove_fillers);
        assert!(!added.polish_enabled);
        assert!(added.apps.is_empty());
        assert_eq!(added.polish_model, None);
        assert_eq!(
            save(&imported(), ModeEdit::default()),
            Err(ModeError::NameBlank)
        );
    }

    #[test]
    fn a_save_with_an_id_changes_only_the_fields_it_names() {
        let before = imported();
        let (after, id) = save(
            &before,
            ModeEdit {
                polish_enabled: Some(true),
                ..edit("c")
            },
        )
        .unwrap();
        assert_eq!(id, "c");
        let (b, a) = (mode(&before, "c"), mode(&after, "c"));
        assert!(a.polish_enabled);
        assert_eq!(
            Mode {
                polish_enabled: false,
                ..a.clone()
            },
            *b,
            "nothing else changed"
        );
        assert_eq!(after.modes.len(), 3);
        assert_eq!(save(&before, edit("gone")), Err(ModeError::NotFound));
    }

    #[test]
    fn names_are_checked_for_blanks_length_styles_and_twins() {
        let s = imported();
        assert_eq!(save(&s, named(" ")), Err(ModeError::NameBlank));
        assert!(
            save(&s, named(&"é".repeat(MAX_NAME_CHARS))).is_ok(),
            "characters, not bytes"
        );
        assert_eq!(
            save(&s, named(&"x".repeat(MAX_NAME_CHARS + 1))),
            Err(ModeError::TooLong(Limit::Name))
        );
        for style in ["formal", " Casual ", "RELAXED"] {
            assert_eq!(
                save(&s, named(style)),
                Err(ModeError::NameIsStyle),
                "{style}"
            );
        }
        assert!(save(&s, named("Casual notes")).is_ok());
        assert_eq!(save(&s, named("chat")), Err(ModeError::NameTaken));
        assert_eq!(
            save(&s, named("everywhere   ELSE")),
            Err(ModeError::NameTaken)
        );
        // A mode keeps its own name, or changes its case.
        assert!(
            save(
                &s,
                ModeEdit {
                    name: Some("CHAT".into()),
                    ..edit("c")
                }
            )
            .is_ok()
        );
        // The default mode can be renamed.
        let (renamed, _) = save(
            &s,
            ModeEdit {
                name: Some("Anywhere".into()),
                ..edit("d")
            },
        )
        .unwrap();
        assert_eq!(renamed.default_mode().name, "Anywhere");
    }

    #[test]
    fn the_prompt_and_the_apps_are_capped() {
        let s = imported();
        let prompt = |text: String| ModeEdit {
            polish_prompt: Some(text),
            ..edit("c")
        };
        assert!(save(&s, prompt("é".repeat(MAX_PROMPT_CHARS))).is_ok());
        assert_eq!(
            save(&s, prompt("x".repeat(MAX_PROMPT_CHARS + 1))),
            Err(ModeError::TooLong(Limit::PolishPrompt))
        );
        let apps = |apps: Vec<String>| ModeEdit {
            apps: Some(apps),
            ..edit("c")
        };
        let many = |n: usize| {
            (0..n)
                .map(|i| format!("com.example.app{i}"))
                .collect::<Vec<_>>()
        };
        assert!(save(&s, apps(many(MAX_APPS))).is_ok());
        assert_eq!(
            save(&s, apps(many(MAX_APPS + 1))),
            Err(ModeError::TooLong(Limit::Apps))
        );
        assert_eq!(
            save(&s, apps(vec!["x".repeat(MAX_APP_CHARS + 1)])),
            Err(ModeError::TooLong(Limit::App))
        );
        let (tidy, _) = save(
            &s,
            apps(vec![
                " com.example.mail ".into(),
                String::new(),
                "COM.example.mail".into(),
                "slack".into(),
            ]),
        )
        .unwrap();
        assert_eq!(
            mode(&tidy, "c").apps,
            ["com.example.mail", "slack"],
            "trimmed, blanks and twins dropped"
        );
    }

    #[test]
    fn there_are_at_most_fifty_modes() {
        let mut s = ModeStore::default();
        for i in 1..MAX_MODES {
            s = s
                .save(
                    &named(&format!("Mode {i}")),
                    false,
                    &format!("m{i}"),
                    &known,
                )
                .unwrap()
                .0;
        }
        assert_eq!(s.modes.len(), MAX_MODES);
        assert_eq!(
            save(&s, named("One more")),
            Err(ModeError::TooLong(Limit::Modes))
        );
        assert!(
            save(
                &s,
                ModeEdit {
                    name: Some("Renamed".into()),
                    ..edit("m1")
                }
            )
            .is_ok(),
            "an edit still saves"
        );
    }

    #[test]
    fn the_default_mode_is_never_deleted_or_given_apps() {
        let s = imported();
        assert_eq!(s.delete("d"), Err(ModeError::DefaultMode));
        assert_eq!(
            save(
                &s,
                ModeEdit {
                    apps: Some(vec!["com.example.mail".into()]),
                    ..edit("d")
                }
            ),
            Err(ModeError::DefaultMode)
        );
        assert!(
            save(
                &s,
                ModeEdit {
                    apps: Some(Vec::new()),
                    ..edit("d")
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn an_app_is_in_one_mode_and_moves_only_when_asked() {
        let s = imported();
        let mail = ModeEdit {
            apps: Some(vec!["COM.EXAMPLE.CHAT".into(), "com.example.mail".into()]),
            ..named("Mail")
        };
        assert_eq!(s.save(&mail, false, "m1", &known), Err(ModeError::AppTaken));
        let (moved, id) = s.save(&mail, true, "m1", &known).unwrap();
        assert!(mode(&moved, "c").apps.is_empty(), "moved out of Chat");
        assert_eq!(mode(&moved, &id).apps.len(), 2);
        // Keeping an app a mode already has is no move, even where the import let two modes share one.
        let shared = ModeStore {
            modes: vec![
                Mode::builtin_default(),
                Mode {
                    id: "a".into(),
                    name: "A".into(),
                    apps: vec!["slack".into()],
                    ..Mode::builtin_default()
                },
                Mode {
                    id: "b".into(),
                    name: "B".into(),
                    apps: vec!["slack".into()],
                    ..Mode::builtin_default()
                },
            ],
            ..ModeStore::default()
        };
        assert!(
            save(
                &shared,
                ModeEdit {
                    polish_enabled: Some(true),
                    apps: Some(vec!["slack".into()]),
                    ..edit("a")
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn what_the_import_brought_stays_editable() {
        // A name that is a style's and a prompt over the cap, as 0.2 allowed.
        let s = ModeStore {
            modes: vec![
                Mode::builtin_default(),
                Mode {
                    id: "c".into(),
                    name: "Casual".into(),
                    polish_prompt: "x".repeat(MAX_PROMPT_CHARS + 10),
                    ..Mode::builtin_default()
                },
            ],
            ..ModeStore::default()
        };
        let sent_back = ModeEdit {
            name: Some("Casual".into()),
            polish_prompt: Some("x".repeat(MAX_PROMPT_CHARS + 10)),
            remove_fillers: Some(false),
            ..edit("c")
        };
        assert!(
            save(&s, sent_back).is_ok(),
            "checked only on what it changes"
        );
    }

    #[test]
    fn a_mode_s_model_must_be_one_the_core_holds_when_it_changes() {
        let s = imported();
        let model = |id: &str| ModeEdit {
            polish_model: Some(Some(id.into())),
            ..edit("c")
        };
        assert_eq!(save(&s, model("engine:gone")), Err(ModeError::ModelUnknown));
        assert_eq!(save(&s, model("anthropic")), Err(ModeError::ModelUnknown));
        let (picked, _) = save(&s, model(" provider:anthropic ")).unwrap();
        assert_eq!(
            mode(&picked, "c").polish_model.as_deref(),
            Some("provider:anthropic")
        );
        // A model let go of since stays named, so the mode can still be edited (and says it is gone).
        let none_known = |_: &str| false;
        let resent = ModeEdit {
            polish_model: Some(Some("engine:local".into())),
            name: Some("Odder".into()),
            ..edit("x")
        };
        assert!(s.save(&resent, false, "m1", &none_known).is_ok());
    }

    #[test]
    fn deleting_a_mode_gives_its_apps_back_to_the_default() {
        let s = imported();
        let after = s.delete("c").unwrap();
        assert_eq!(after.modes.len(), 2);
        assert_eq!(after.resolve(Some("com.example.chat")).id, "d");
        assert_eq!(s.delete("gone"), Err(ModeError::NotFound));
    }

    #[test]
    fn a_new_mode_s_id_is_never_one_taken() {
        let s = ModeStore {
            modes: vec![
                Mode::builtin_default(),
                Mode {
                    id: "m5".into(),
                    ..Mode::builtin_default()
                },
                Mode {
                    id: "m6".into(),
                    ..Mode::builtin_default()
                },
            ],
            ..ModeStore::default()
        };
        assert_eq!(s.fresh_id(4), "m4");
        assert_eq!(s.fresh_id(5), "m7");
        assert_eq!(
            s.fresh_id(-3),
            "m0",
            "a clock before 1970 still gives an id"
        );
    }

    #[test]
    fn errors_have_codes_of_their_own_and_never_quote_the_user_s_words() {
        let all = [
            ModeError::NameBlank,
            ModeError::NameTaken,
            ModeError::NameIsStyle,
            ModeError::TooLong(Limit::Name),
            ModeError::DefaultMode,
            ModeError::AppTaken,
            ModeError::NotFound,
            ModeError::ModelUnknown,
        ];
        let codes: HashSet<_> = all.iter().map(|e| e.code()).collect();
        assert_eq!(codes.len(), all.len());
        let s = imported();
        let canary = "Canary Words";
        let refused = [
            save(&s, named("Chat")),
            save(
                &s,
                ModeEdit {
                    apps: Some(vec!["com.example.chat".into()]),
                    ..named(canary)
                },
            ),
            save(
                &s,
                ModeEdit {
                    name: Some(format!("{canary}{}", "x".repeat(MAX_NAME_CHARS))),
                    ..edit("c")
                },
            ),
        ];
        for r in refused {
            let e = r.unwrap_err().to_string();
            assert!(
                !e.contains("Canary") && !e.contains("chat") && !e.contains("Chat"),
                "{e}"
            );
        }
    }
}
