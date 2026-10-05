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
//! "polish_enabled", "remove_fillers", "polish_prompt", "apps", "polish_model",
//! "polish_model_name", "polish_model_to"}]}` (the shape the 0.2 import writes, plus the three
//! `polish_model` fields: [`ModelPin`]). [`ModeStore::from_json`] is its one reader, for dictation
//! and Settings alike; [`ModeStore::write_into`] writes a store back into the stored document,
//! changing only the fields this build knows. A field it does not know survives a save: 0.2's
//! `model` (which named a transcription model, never a language model, so 1.0 does not read it),
//! or a newer build's; and so does a style it does not know, until the user picks another.
//!
//! Every mode has its own id once read: the 0.2 import can give two modes one id, and the second
//! (and any after it) is read as `<id>~2` (`~3`, …), so Settings can address each and a rule never
//! mistakes one for the other. The next save writes the new ids.
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
//! [`Mode::polish_model`] names the model a mode's dictations are polished on ([`ModelPin`]: by
//! the id the core gives it, optionally a model at that provider, and where it sent when the user
//! picked it; the chain finds it at each take: [`ModeModels`](crate::chain::ModeModels)), or
//! `None` for the model Settings > AI chose. A mode whose model the core does not hold at that
//! moment, or that sends somewhere else now than when the mode was saved, is not polished at all,
//! never polished on another model in its place: that could send the words somewhere the user did
//! not pick for this mode.

use std::collections::HashSet;
use std::sync::LazyLock;

use serde_json::{Map, Value};

use crate::consent::Destination;
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

/// The longest model name a mode pins at a provider ([`ModelPin::model`]), in characters.
pub const MAX_MODEL_NAME_CHARS: usize = 128;

/// A mode's own language model, as the user picked it in Settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelPin {
    /// The model, by the id the core gives it: `engine:<id>` (one the shell registered) or
    /// `provider:<id>` (the own-key provider chosen in Settings > AI). Blank in a stored document
    /// is a model that never resolves (the mode is not polished), never the AI setting's model.
    pub id: String,
    /// A model at that provider (`provider:` only), sent as the request's model on the same
    /// endpoint; `None` for the model chosen with the provider in Settings > AI. Free text: the
    /// provider is the judge, and one it refuses fails the take's polish.
    pub model: Option<String>,
    /// Where the model sent when the user saved the mode, or `None` if that was never recorded.
    /// A model that sends anywhere else now (a custom server re-pointed from this machine to
    /// another), or one with nothing recorded, is not called: the mode is not polished until the
    /// user saves it again.
    pub to: Option<Destination>,
}

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
    /// The language model this mode is polished on ([`ModelPin`]), or `None` for the model
    /// Settings > AI chose. When the core holds no such model, or it sends elsewhere than when the
    /// mode was saved, the mode's dictations go out unpolished and say so; never to another model.
    pub polish_model: Option<ModelPin>,
    /// This mode's polish prompt; blank means the global prompt.
    pub polish_prompt: String,
    /// Whether dictations in this mode are polished (when a model is available).
    pub polish_enabled: bool,
    /// Substrings matched, without case or the spaces around them, against the frontmost app's
    /// identity: the bundle id on macOS (`com.example.mail`), the executable name on Windows. A
    /// substring, as 0.2 matched: `mail` matches `com.example.mail` and `com.example.mailbox`
    /// alike, so a save refuses an app of one character or with no letter in it
    /// ([`ModeError::AppInvalid`]), which would match nearly everything.
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
        let app = app_id.trim().to_lowercase();
        self.apps.iter().any(|a| {
            let a = a.trim();
            !a.is_empty() && app.contains(&a.to_lowercase())
        })
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
/// needed for a new mode. Text is taken as sent: [`ModeStore::save`] trims it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModeEdit {
    /// The mode changed, or `None` to add one.
    pub id: Option<String>,
    /// Its name.
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
    /// Its language model, by id: `Some(None)` (or a blank id) for the AI setting's. Named, the
    /// pin is replaced whole: its model name is then [`polish_model_name`](Self::polish_model_name)
    /// (none when absent), and where it sends is recorded again.
    pub polish_model: Option<Option<String>>,
    /// A model at the pinned provider ([`ModelPin::model`]): `Some(None)` (or blank) for the one
    /// chosen in Settings > AI. Named without `polish_model`, it changes the pin the mode has.
    pub polish_model_name: Option<Option<String>>,
    /// The user confirmed the mode's model where it sends now: record that again, though the
    /// model and its name are the same (a model that moved, or one saved before destinations were
    /// recorded). Refused without a model ([`ModeError::ModelUnknown`]).
    pub polish_model_confirm: bool,
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
    /// A model name that is blank, longer than [`MAX_MODEL_NAME_CHARS`] or holds a control
    /// character, or one given for a model that is not a provider's (`engine:`), or without a
    /// model to name one at.
    ModelNameInvalid,
    /// An app identity holding a control character (a line break), or one of a single character
    /// or with no letter: as a substring it would match nearly every app.
    AppInvalid,
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
            Self::ModelNameInvalid => "model_name_invalid",
            Self::AppInvalid => "app_invalid",
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
            Self::TooLong(Limit::Apps) => write!(
                f,
                "a mode names at most {MAX_APPS} apps: shorten its list to {MAX_APPS} or fewer"
            ),
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
            Self::ModelNameInvalid => write!(
                f,
                "a model's name is up to {MAX_MODEL_NAME_CHARS} printable characters, given only for a provider's model"
            ),
            Self::AppInvalid => f.write_str(
                "an app's identity is printable, at least two characters long and has a letter in it",
            ),
        }
    }
}

impl std::error::Error for ModeError {}

impl ModeStore {
    /// The default mode: the one named by `default_id`, else the first, else the built-in one.
    pub fn default_mode(&self) -> &Mode {
        self.default_index()
            .map_or(&BUILTIN_DEFAULT, |i| &self.modes[i])
    }

    /// Where the default mode is: the one named by `default_id`, else the first; `None` with no
    /// modes at all.
    fn default_index(&self) -> Option<usize> {
        self.modes
            .iter()
            .position(|m| m.id == self.default_id)
            .or_else(|| (!self.modes.is_empty()).then_some(0))
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
    /// none and `polish_model` (absent or `null`) to the AI setting's model. A field of the wrong
    /// type does not read (a prompt that is not text would otherwise read as blank and be
    /// overwritten by the next save), and a blank `polish_model` is a model that never resolves,
    /// not the AI setting's ([`ModelPin::id`]). A style that is absent or unknown writes as the
    /// default's ([`Mode::style_unknown`]). A mode with an id an earlier one has gets one of its
    /// own (`<id>~2`: see the module docs). Fields this build does not know are ignored here and
    /// kept by [`write_into`](Self::write_into).
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
        let mut modes: Vec<Mode> = list.iter().map(read_mode).collect::<Result<_, _>>()?;
        let own = own_ids(modes.iter().map(|m| Some(m.id.as_str())));
        let mut renamed = 0;
        for (m, id) in modes.iter_mut().zip(own) {
            if let Some(id) = id.filter(|id| *id != m.id) {
                m.id = id;
                renamed += 1;
            }
        }
        if renamed > 0 {
            // By count only: ids can be the user's words in a document an older build wrote.
            log::warn!("modes: {renamed} modes shared an id with another; each now has its own");
        }
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
    /// `stored`: the one whose id, as [`from_json`](Self::from_json) reads it (each its own, by
    /// [`own_ids`] on the document's order), is the mode's. So two modes the import gave one id
    /// keep their own fields, whichever of them is deleted or moved.
    fn fill(&self, mut top: Map<String, Value>, stored: Vec<Value>) -> Value {
        let read_ids = own_ids(stored.iter().map(|v| v.get("id").and_then(Value::as_str)));
        let mut stored: Vec<Option<(String, Map<String, Value>)>> = stored
            .into_iter()
            .zip(read_ids)
            .map(|(v, id)| match (v, id) {
                (Value::Object(o), Some(id)) => Some((id, o)),
                _ => None,
            })
            .collect();
        let mut take = |id: &str| {
            stored
                .iter_mut()
                .find(|o| o.as_ref().is_some_and(|(read, _)| read == id))
                .and_then(Option::take)
                .map(|(_, o)| o)
        };
        let modes = self
            .modes
            .iter()
            .map(|m| {
                let mut item = take(&m.id).unwrap_or_default();
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
                write_pin(&mut item, m.polish_model.as_ref());
                Value::Object(item)
            })
            .collect();
        top.insert("default_id".into(), self.default_id.clone().into());
        top.insert("modes".into(), Value::Array(modes));
        Value::Object(top)
    }

    /// An id for a new mode: `m` and the time in milliseconds, past any id taken (a mode's, or
    /// the default's while it names no mode, so a new mode never becomes the default by its id).
    pub fn fresh_id(&self, now_unix_ms: i64) -> String {
        let mut ms = now_unix_ms.max(0);
        loop {
            let id = format!("m{ms}");
            if id != self.default_id && !self.modes.iter().any(|m| m.id == id) {
                return id;
            }
            ms = ms.saturating_add(1);
        }
    }

    /// This store with a default mode it names: with no modes, the built-in default; with a
    /// `default_id` that names none, the first mode (the one resolution already uses). So a mode
    /// a save adds is never the default by accident, and the default's rules hold for the mode
    /// that is one.
    fn with_a_default(&self) -> Self {
        let mut store = self.clone();
        if store.modes.is_empty() {
            let default = Mode::builtin_default();
            store.default_id.clone_from(&default.id);
            store.modes.push(default);
        } else if !store.modes.iter().any(|m| m.id == store.default_id) {
            store.default_id.clone_from(&store.modes[0].id);
        }
        store
    }

    /// The store after `edit`, and the id of the mode saved. `new_id` is a new mode's id
    /// ([`fresh_id`](Self::fresh_id)); `take_apps` moves an app another mode names to this one
    /// (otherwise [`ModeError::AppTaken`]); `model_at` says where a language model the core holds
    /// sends now, by its id and a model name at it ([`ModelPin`]), or `None` when the core holds
    /// none. Each rule is checked only on what the edit changes. A store without a default mode
    /// it names gets one first (see [`default_mode`](Self::default_mode)).
    pub fn save(
        &self,
        edit: &ModeEdit,
        take_apps: bool,
        new_id: &str,
        model_at: &dyn Fn(&str, Option<&str>) -> Option<Destination>,
    ) -> Result<(Self, String), ModeError> {
        let base = self.with_a_default();
        let index = match &edit.id {
            Some(id) => Some(
                base.modes
                    .iter()
                    .position(|m| &m.id == id)
                    .ok_or(ModeError::NotFound)?,
            ),
            None if base.modes.len() >= MAX_MODES => {
                return Err(ModeError::TooLong(Limit::Modes));
            }
            None => None,
        };
        let before = index.map(|i| &base.modes[i]);
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
        mode.polish_model =
            pin_after(edit, before.and_then(|b| b.polish_model.as_ref()), model_at)?;

        let added = base.check(&mode, index, take_apps)?;

        let mut after = base.clone();
        if take_apps && !added.is_empty() {
            for (_, other) in after
                .modes
                .iter_mut()
                .enumerate()
                .filter(|(i, _)| Some(*i) != index)
            {
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

    /// The rules, on what `mode` (at `index`, or new) changes from the mode it was (all of it for
    /// a new mode). Other modes are told apart by place, never by id. Gives the apps the mode is
    /// given that it did not name before, by [`app_key`].
    fn check(
        &self,
        mode: &Mode,
        index: Option<usize>,
        take_apps: bool,
    ) -> Result<HashSet<String>, ModeError> {
        let before = index.map(|i| &self.modes[i]);
        let mut others = self
            .modes
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != index)
            .map(|(_, m)| m);
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
            if others.clone().any(|m| spoken_name(&m.name) == spoken) {
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
            if added.iter().any(|a| !valid_app(a)) {
                return Err(ModeError::AppInvalid);
            }
            if index.is_some() && index == self.default_index() {
                return Err(ModeError::DefaultMode);
            }
            let taken = others.any(|m| m.apps.iter().any(|a| added.contains(&app_key(a))));
            if taken && !take_apps {
                return Err(ModeError::AppTaken);
            }
        }
        Ok(added)
    }

    /// The store without mode `id`. Its apps go back to the default mode, which cannot be deleted.
    pub fn delete(&self, id: &str) -> Result<Self, ModeError> {
        let mut after = self.with_a_default();
        let i = after
            .modes
            .iter()
            .position(|m| m.id == id)
            .ok_or(ModeError::NotFound)?;
        if Some(i) == after.default_index() {
            return Err(ModeError::DefaultMode);
        }
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
    // Text that may be absent or null, but is text when it is there.
    let text = |k: &str| match m.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(t)) => Ok(Some(t.as_str())),
        Some(_) => Err(ModesUnreadable),
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
    // the AI setting's model, which may send somewhere this mode was set up not to. For the same
    // reason a blank one is kept blank (a model that never resolves), never read as none.
    let polish_model = match text("polish_model")? {
        None => None,
        Some(id) => Some(ModelPin {
            id: id.trim().to_owned(),
            // Held to the rules a save holds it to: a name only at a provider, printable and
            // short. One that breaks them is a document this build did not write.
            model: match model_name(text("polish_model_name")?).map_err(|_| ModesUnreadable)? {
                Some(_) if !id.trim().starts_with("provider:") => return Err(ModesUnreadable),
                name => name,
            },
            to: match m.get("polish_model_to") {
                None | Some(Value::Null) => None,
                Some(v) => Some(Destination::from_value(v).ok_or(ModesUnreadable)?),
            },
        }),
    };
    let style = s("style").and_then(Style::parse);
    Ok(Mode {
        id: s("id").ok_or(ModesUnreadable)?.to_owned(),
        name: s("name").ok_or(ModesUnreadable)?.to_owned(),
        style: style.unwrap_or(Mode::builtin_default().style),
        style_unknown: style.is_none(),
        polish_model,
        polish_prompt: text("polish_prompt")?.unwrap_or_default().to_owned(),
        polish_enabled: flag("polish_enabled", false)?,
        apps,
        remove_fillers: flag("remove_fillers", true)?,
    })
}

/// Each mode's own id, from the ids in document order (`None` for one without): an id an earlier
/// mode has becomes `<id>~2` (or the next number no mode has), the rest stay. The same document
/// always gives the same ids, which is how [`ModeStore::fill`] finds a mode's stored object.
fn own_ids<'a>(ids: impl Iterator<Item = Option<&'a str>>) -> Vec<Option<String>> {
    let ids: Vec<Option<&str>> = ids.collect();
    let all: HashSet<&str> = ids.iter().flatten().copied().collect();
    let mut seen: HashSet<String> = HashSet::new();
    ids.iter()
        .map(|id| {
            let id = (*id)?;
            if seen.insert(id.to_owned()) {
                return Some(id.to_owned());
            }
            let mut n = 2u32;
            let own = loop {
                let own = format!("{id}~{n}");
                if !seen.contains(&own) && !all.contains(own.as_str()) {
                    break own;
                }
                n = n.saturating_add(1);
            };
            seen.insert(own.clone());
            Some(own)
        })
        .collect()
}

/// Writes `pin` into a stored mode: its three fields, or none of them.
fn write_pin(item: &mut Map<String, Value>, pin: Option<&ModelPin>) {
    let Some(pin) = pin else {
        for k in ["polish_model", "polish_model_name", "polish_model_to"] {
            item.remove(k);
        }
        return;
    };
    item.insert("polish_model".into(), pin.id.clone().into());
    match &pin.model {
        Some(model) => item.insert("polish_model_name".into(), model.clone().into()),
        None => item.remove("polish_model_name"),
    };
    match &pin.to {
        Some(to) => item.insert("polish_model_to".into(), to.to_value()),
        None => item.remove("polish_model_to"),
    };
}

/// A model name at a provider, as a pin holds it: trimmed, `None` when blank; refused over
/// [`MAX_MODEL_NAME_CHARS`] or with a control character.
fn model_name(n: Option<&str>) -> Result<Option<String>, ModeError> {
    let Some(n) = n.map(str::trim).filter(|n| !n.is_empty()) else {
        return Ok(None);
    };
    if n.chars().count() > MAX_MODEL_NAME_CHARS || n.chars().any(char::is_control) {
        return Err(ModeError::ModelNameInvalid);
    }
    Ok(Some(n.to_owned()))
}

/// The mode's [`ModelPin`] after `edit`, from `before`'s. Unnamed in the edit, it stays as it was.
/// Named, the pin is the edit's. Where it sends is recorded from `model_at` when the model or its
/// name changes, or when the edit confirms it ([`ModeEdit::polish_model_confirm`]); a save that
/// names the same model and name again (an editor sends every field back) keeps what was
/// recorded, so a model that moved since stays refused until the user says so. Refused when the
/// core holds no such model and it is to be recorded ([`ModeError::ModelUnknown`]).
fn pin_after(
    edit: &ModeEdit,
    before: Option<&ModelPin>,
    model_at: &dyn Fn(&str, Option<&str>) -> Option<Destination>,
) -> Result<Option<ModelPin>, ModeError> {
    let name = |n: &Option<String>| model_name(n.as_deref());
    let (id, model) = match (&edit.polish_model, &edit.polish_model_name) {
        (None, None) if !edit.polish_model_confirm => return Ok(before.cloned()),
        (None, None) => (
            before.map(|p| p.id.clone()),
            before.and_then(|p| p.model.clone()),
        ),
        (Some(id), model) => (
            id.as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_owned),
            match model {
                Some(model) => name(model)?,
                None => None,
            },
        ),
        (None, Some(model)) => (before.map(|p| p.id.clone()), name(model)?),
    };
    let Some(id) = id else {
        // The AI setting's model: no name to give it, and nothing to confirm.
        return match model {
            Some(_) => Err(ModeError::ModelNameInvalid),
            None if edit.polish_model_confirm => Err(ModeError::ModelUnknown),
            None => Ok(None),
        };
    };
    if model.is_some() && !id.starts_with("provider:") {
        return Err(ModeError::ModelNameInvalid);
    }
    let same = before.is_some_and(|b| b.id == id && b.model == model);
    let to = if same && !edit.polish_model_confirm {
        before.and_then(|b| b.to.clone())
    } else {
        Some(model_at(&id, model.as_deref()).ok_or(ModeError::ModelUnknown)?)
    };
    Ok(Some(ModelPin { id, model, to }))
}

/// Whether `app` (as [`app_key`] gives it) is an identity a mode may be given: no control
/// character, at least two characters, and a letter.
fn valid_app(app: &str) -> bool {
    !app.chars().any(char::is_control)
        && app.chars().count() >= 2
        && app.chars().any(char::is_alphabetic)
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

    /// Where the models the tests' core holds send: one on this machine, one provider (which
    /// takes a model name, the engine none).
    fn known(id: &str, name: Option<&str>) -> Option<Destination> {
        match (id, name) {
            ("engine:local", None) => Some(Destination::OnDevice),
            ("provider:anthropic", _) => Some(Destination::Cloud(ANTHROPIC.into())),
            _ => None,
        }
    }

    const ANTHROPIC: &str = "https://api.anthropic.com";

    fn pin_id(m: &Mode) -> Option<&str> {
        m.polish_model.as_ref().map(|p| p.id.as_str())
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
        assert_eq!(pin_id(odd), Some("engine:local"));
        assert_eq!(
            odd.polish_model.as_ref().unwrap().to,
            None,
            "nothing recorded: never called until saved again"
        );
        assert!(odd.remove_fillers, "0.2's default");
        assert!(!odd.polish_enabled);
        let read = |field: &str| {
            let doc = format!(r#"{{"default_id":"d","modes":[{{"id":"d","name":"D",{field}}}]}}"#);
            ModeStore::from_json(&doc).unwrap().modes.remove(0)
        };
        assert_eq!(read(r#""polish_model":null"#).polish_model, None);
        assert_eq!(
            pin_id(&read(r#""polish_model":"  ""#)),
            Some(""),
            "a blank stored model fails closed: never the AI setting's"
        );
        assert_eq!(read(r#""polish_prompt":null"#).polish_prompt, "");
        let pinned = read(
            r#""polish_model":"provider:anthropic","polish_model_name":"model-b","polish_model_to":{"to":"cloud","endpoint":"https://api.anthropic.com"}"#,
        );
        assert_eq!(
            pinned.polish_model,
            Some(ModelPin {
                id: "provider:anthropic".into(),
                model: Some("model-b".into()),
                to: Some(Destination::Cloud(ANTHROPIC.into())),
            })
        );
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
            r#"{"default_id":"d","modes":[{"id":"x","name":"x","polish_prompt":5}]}"#,
            r#"{"default_id":"d","modes":[{"id":"x","name":"x","polish_prompt":["a"]}]}"#,
            r#"{"default_id":"d","modes":[{"id":"x","name":"x","polish_model":"engine:x","polish_model_name":3}]}"#,
            r#"{"default_id":"d","modes":[{"id":"x","name":"x","polish_model":"engine:x","polish_model_to":"on_device"}]}"#,
            r#"{"default_id":"d","modes":[{"id":"x","name":"x","polish_model":"engine:x","polish_model_to":{"to":"cloud"}}]}"#,
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
            mode(&picked, "c").polish_model,
            Some(ModelPin {
                id: "provider:anthropic".into(),
                model: None,
                to: Some(Destination::Cloud(ANTHROPIC.into())),
            }),
            "trimmed, and where it sends recorded"
        );
        // A model let go of since stays named, so the mode can still be edited (and says it is gone).
        let none_known = |_: &str, _: Option<&str>| None;
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

    /// The 0.2 import can give two modes one id. Each is read with its own, so a save tells them
    /// apart by place: a name or an app the other has is still refused, and each keeps its own
    /// stored fields.
    #[test]
    fn modes_that_shared_an_id_each_get_their_own_and_stay_apart() {
        const TWINS: &str = r#"{"default_id":"d","modes":[
            {"id":"d","name":"Everywhere else"},
            {"id":"c","name":"Chat","apps":["com.example.chat"],"mine":1},
            {"id":"c","name":"Mail","apps":["com.example.mail"],"mine":2}]}"#;
        let s = ModeStore::from_json(TWINS).unwrap();
        let ids: Vec<&str> = s.modes.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["d", "c", "c~2"]);
        let rename = |id: &str, name: &str| ModeEdit {
            name: Some(name.into()),
            ..edit(id)
        };
        assert_eq!(save(&s, rename("c~2", "chat")), Err(ModeError::NameTaken));
        assert_eq!(save(&s, rename("c", "MAIL")), Err(ModeError::NameTaken));
        let give = |id: &str, app: &str| ModeEdit {
            apps: Some(vec![app.into()]),
            ..edit(id)
        };
        assert_eq!(
            save(&s, give("c~2", "com.example.chat")),
            Err(ModeError::AppTaken)
        );
        let (moved, _) = s
            .save(&give("c~2", "com.example.chat"), true, "m1", &known)
            .unwrap();
        assert!(mode(&moved, "c").apps.is_empty(), "taken from the other");
        // Written back: the new id, and each mode's own unknown field.
        let (renamed, _) = save(&s, rename("c~2", "Letters")).unwrap();
        let v: Value = serde_json::from_str(&renamed.write_into(TWINS).unwrap()).unwrap();
        assert_eq!(v["modes"][1]["mine"], 1);
        assert_eq!(v["modes"][2]["id"], "c~2");
        assert_eq!(v["modes"][2]["mine"], 2);
        assert_eq!(v["modes"][2]["name"], "Letters");
        let deleted = s.delete("c~2").unwrap();
        assert_eq!(deleted.modes.len(), 2);
        assert_eq!(mode(&deleted, "c").name, "Chat", "the other one stays");
        // An id an earlier mode was given already is skipped.
        let three = r#"{"default_id":"d","modes":[{"id":"d","name":"D"},{"id":"d~2","name":"E"},{"id":"d","name":"F"}]}"#;
        let ids: Vec<String> = ModeStore::from_json(three)
            .unwrap()
            .modes
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, ["d", "d~2", "d~3"]);
    }

    /// Moving an app compares identities without case or padding, as matching does.
    #[test]
    fn take_apps_moves_an_app_however_it_is_cased() {
        let s = imported();
        let (moved, id) = s
            .save(
                &ModeEdit {
                    apps: Some(vec!["  Com.Example.CHAT ".into()]),
                    ..named("Talk")
                },
                true,
                "m1",
                &known,
            )
            .unwrap();
        assert!(mode(&moved, "c").apps.is_empty());
        assert_eq!(mode(&moved, &id).apps, ["Com.Example.CHAT"]);
        assert_eq!(moved.resolve(Some(" com.example.chat ")).id, id);
    }

    #[test]
    fn a_stored_document_that_is_not_an_object_is_never_written_into() {
        let s = imported();
        for bad in [
            "[]",
            "3",
            r#""modes""#,
            r#"{"modes":{}}"#,
            r#"{"modes":"x"}"#,
            "not json",
        ] {
            assert_eq!(s.write_into(bad), Err(ModesUnreadable), "{bad}");
        }
    }

    /// With no modes, or a default_id that names none, the mode a save adds never becomes the
    /// default, so it can hold apps; and the mode that is the default (the first) holds the
    /// default's rules.
    #[test]
    fn a_new_mode_never_becomes_the_default_by_accident() {
        let with_apps = ModeEdit {
            apps: Some(vec!["com.example.mail".into()]),
            ..named("Mail")
        };
        let empty = ModeStore {
            default_id: "gone".into(),
            modes: Vec::new(),
        };
        let (after, id) = empty.save(&with_apps, false, "m1", &known).unwrap();
        assert_eq!(
            after.modes.len(),
            2,
            "the built-in default, then the new one"
        );
        assert_ne!(after.default_mode().id, id);
        assert_eq!(after.resolve(Some("com.example.mail")).id, id);
        assert_eq!(after.default_id, after.default_mode().id, "names it now");

        let dangling = ModeStore {
            default_id: "m5".into(),
            modes: vec![with_apps_mode("a", &[]), with_apps_mode("b", &["slack"])],
        };
        assert_eq!(
            dangling.fresh_id(5),
            "m6",
            "the default's id is never given"
        );
        let (after, id) = dangling.save(&with_apps, false, "m6", &known).unwrap();
        assert_eq!(after.default_mode().id, "a");
        assert_ne!(id, "a");
        assert_eq!(
            save(
                &dangling,
                ModeEdit {
                    apps: Some(vec!["com.example.notes".into()]),
                    ..edit("a")
                }
            ),
            Err(ModeError::DefaultMode),
            "the first is the default, so it holds the default's rules"
        );
        assert_eq!(dangling.delete("a"), Err(ModeError::DefaultMode));
        assert!(dangling.delete("b").is_ok());
    }

    fn with_apps_mode(id: &str, apps: &[&str]) -> Mode {
        Mode {
            id: id.into(),
            name: id.to_uppercase(),
            apps: apps.iter().map(|a| (*a).into()).collect(),
            ..Mode::builtin_default()
        }
    }

    /// An app is a substring of the frontmost app's identity: one that would match nearly
    /// everything, or holds a line break, is refused when it is given (and one the import brought
    /// stays editable).
    #[test]
    fn an_app_identity_is_printable_and_specific_enough() {
        let s = imported();
        let apps = |app: &str| ModeEdit {
            apps: Some(vec!["com.example.chat".into(), app.into()]),
            ..edit("c")
        };
        for bad in [
            "a",
            "é",
            "42",
            "1.2.3",
            "com.example\nmail",
            "mail\u{7}",
            ".-",
        ] {
            assert_eq!(save(&s, apps(bad)), Err(ModeError::AppInvalid), "{bad:?}");
        }
        for good in ["ab", "Slack.exe", "com.example.mail"] {
            assert!(save(&s, apps(good)).is_ok(), "{good}");
        }
        let imported_short = ModeStore {
            modes: vec![Mode::builtin_default(), with_apps_mode("x", &["x"])],
            ..ModeStore::default()
        };
        assert!(
            save(
                &imported_short,
                ModeEdit {
                    apps: Some(vec!["x".into()]),
                    polish_enabled: Some(true),
                    ..edit("x")
                }
            )
            .is_ok(),
            "kept, not given: no refusal"
        );
    }

    /// A mode may pin a model at a provider: printable, at most 128 characters, a provider's
    /// only; blank is the provider's chosen one. Where it sends is recorded at each save that
    /// names the model, and kept by one that does not.
    #[test]
    fn a_mode_pins_a_model_at_a_provider_and_where_it_sends() {
        let s = imported();
        let pin = |id: Option<&str>, name: Option<&str>| ModeEdit {
            polish_model: Some(id.map(Into::into)),
            polish_model_name: Some(name.map(Into::into)),
            ..edit("c")
        };
        let (named, _) = save(&s, pin(Some("provider:anthropic"), Some(" model-b "))).unwrap();
        assert_eq!(
            mode(&named, "c").polish_model,
            Some(ModelPin {
                id: "provider:anthropic".into(),
                model: Some("model-b".into()),
                to: Some(Destination::Cloud(ANTHROPIC.into())),
            })
        );
        let doc = named.write_into(IMPORTED).unwrap();
        assert_eq!(ModeStore::from_json(&doc).unwrap(), named, "reads back");
        assert_eq!(
            save(&s, pin(Some("provider:anthropic"), Some("  ")))
                .unwrap()
                .0
                .modes[1]
                .polish_model
                .as_ref()
                .unwrap()
                .model,
            None,
            "blank: the provider's chosen model"
        );
        for bad in [
            pin(Some("engine:local"), Some("model-b")),
            pin(None, Some("model-b")),
            pin(
                Some("provider:anthropic"),
                Some(&"m".repeat(MAX_MODEL_NAME_CHARS + 1)),
            ),
            pin(Some("provider:anthropic"), Some("model\nb")),
            ModeEdit {
                polish_model_name: Some(Some("model-b".into())),
                ..edit("d")
            },
        ] {
            assert_eq!(save(&s, bad), Err(ModeError::ModelNameInvalid));
        }
        assert!(
            save(
                &s,
                pin(
                    Some("provider:anthropic"),
                    Some(&"é".repeat(MAX_MODEL_NAME_CHARS))
                )
            )
            .is_ok()
        );
        // Only the name, on a mode that has a pin: the pin's model changes.
        let (renamed, _) = save(
            &named,
            ModeEdit {
                polish_model_name: Some(Some("model-c".into())),
                ..edit("c")
            },
        )
        .unwrap();
        let p = mode(&renamed, "c").polish_model.clone().unwrap();
        assert_eq!(
            (p.id.as_str(), p.model.as_deref()),
            ("provider:anthropic", Some("model-c"))
        );

        // The model now sends elsewhere. A save that does not name it, or names the same model
        // and name again (an editor sends every field back), keeps what was recorded, so the take
        // still refuses it; a save that confirms it, or changes the model, records where it sends.
        let elsewhere =
            |_: &str, _: Option<&str>| Some(Destination::Cloud("https://b.example".into()));
        let recorded = |edit: ModeEdit| {
            named.save(&edit, false, "m1", &elsewhere).unwrap().0.modes[1]
                .polish_model
                .clone()
                .unwrap()
                .to
        };
        let was = Some(Destination::Cloud(ANTHROPIC.into()));
        let moved = Some(Destination::Cloud("https://b.example".into()));
        assert_eq!(
            recorded(ModeEdit {
                polish_enabled: Some(true),
                ..edit("c")
            }),
            was
        );
        assert_eq!(
            recorded(pin(Some("provider:anthropic"), Some("model-b"))),
            was,
            "the same pin sent back"
        );
        assert_eq!(
            recorded(ModeEdit {
                polish_model_confirm: true,
                ..pin(Some("provider:anthropic"), Some("model-b"))
            }),
            moved,
            "confirmed"
        );
        assert_eq!(
            recorded(ModeEdit {
                polish_model_confirm: true,
                ..edit("c")
            }),
            moved,
            "confirmed without naming it"
        );
        assert_eq!(
            recorded(pin(Some("provider:anthropic"), Some("model-c"))),
            moved,
            "another model"
        );
        // Nothing to confirm on the AI setting's model, nor a model the core does not hold.
        assert_eq!(
            save(
                &s,
                ModeEdit {
                    polish_model_confirm: true,
                    ..edit("d")
                }
            ),
            Err(ModeError::ModelUnknown)
        );
        let none = |_: &str, _: Option<&str>| None;
        assert_eq!(
            named.save(
                &ModeEdit {
                    polish_model_confirm: true,
                    ..edit("c")
                },
                false,
                "m1",
                &none
            ),
            Err(ModeError::ModelUnknown)
        );
    }

    /// A pin on a model the core no longer holds can be cleared (back to the AI setting's).
    #[test]
    fn a_pin_on_a_model_that_is_gone_can_be_cleared() {
        let s = imported();
        assert_eq!(pin_id(mode(&s, "x")), Some("engine:local"));
        let none = |_: &str, _: Option<&str>| None;
        let (cleared, _) = s
            .save(
                &ModeEdit {
                    polish_model: Some(None),
                    ..edit("x")
                },
                false,
                "m1",
                &none,
            )
            .unwrap();
        assert_eq!(mode(&cleared, "x").polish_model, None);
    }

    /// Deleting the first of two modes the import gave one id leaves the other its own stored
    /// fields: the stored objects are matched by the id each was read with, in document order.
    #[test]
    fn deleting_the_first_twin_keeps_the_second_its_own_fields() {
        const TWINS: &str = r#"{"default_id":"d","modes":[
            {"id":"d","name":"Everywhere else"},
            {"id":"c","name":"Chat","mine":1},
            {"id":"c","name":"Mail","mine":2},
            {"id":"c~2","name":"Odd","mine":3}]}"#;
        let s = ModeStore::from_json(TWINS).unwrap();
        let ids: Vec<&str> = s.modes.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            ["d", "c", "c~3", "c~2"],
            "an id the document has is never given"
        );
        let after = s.delete("c").unwrap();
        let v: Value = serde_json::from_str(&after.write_into(TWINS).unwrap()).unwrap();
        let by_name = |name: &str| {
            v["modes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["name"] == name)
                .unwrap()
                .clone()
        };
        assert_eq!(by_name("Mail")["mine"], 2, "{v}");
        assert_eq!(by_name("Mail")["id"], "c~3");
        assert_eq!(by_name("Odd")["mine"], 3, "{v}");
        let written = after.write_into(TWINS).unwrap();
        assert_eq!(ModeStore::from_json(&written).unwrap(), after, "reads back");
    }

    /// A stored pin is held to the rules a save holds it to; one that breaks them does not read.
    #[test]
    fn a_stored_pin_that_breaks_the_rules_does_not_read() {
        let doc =
            |pin: &str| format!(r#"{{"default_id":"d","modes":[{{"id":"d","name":"D",{pin}}}]}}"#);
        for bad in [
            format!(
                r#""polish_model":"provider:openai","polish_model_name":"{}""#,
                "m".repeat(MAX_MODEL_NAME_CHARS + 1)
            ),
            r#""polish_model":"provider:openai","polish_model_name":"m
b""#
            .to_owned(),
            r#""polish_model":"engine:local","polish_model_name":"m""#.to_owned(),
            r#""polish_model":"","polish_model_name":"m""#.to_owned(),
        ] {
            assert_eq!(
                ModeStore::from_json(&doc(&bad)),
                Err(ModesUnreadable),
                "{bad}"
            );
        }
        let ok = ModeStore::from_json(&doc(
            r#""polish_model":"provider:openai","polish_model_name":"  ""#,
        ))
        .unwrap();
        assert_eq!(
            ok.modes[0].polish_model.as_ref().unwrap().model,
            None,
            "blank: none"
        );
    }
}
