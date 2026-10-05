//! Dictation modes as Settings lists and edits them, answered on `queries`' thread.
//!
//! | Command | Answer |
//! |---|---|
//! | `modes.list` | `modes.listed`: the modes in the order they are matched, the default polish prompt, and the language models a mode can pick |
//! | `modes.save {"mode":{...}, "take_apps"?, "replace_unreadable"?}` | `modes.listed`, as saved, with the mode's id as `saved`: a mode added (no `id`) or changed (only the fields it names) |
//! | `modes.delete {"mode":"<id>"}` | `modes.listed`, without it |
//!
//! Each answer echoes the command's `id` as `ref`; a failure is `command.failed` with that id,
//! and a refusal the editor shows carries a `code` ([`ModeError::code`], or
//! [`LIST_UNREADABLE`]).
//!
//! **One reading for both.** Dictation ([`voice`](crate::voice)) and Settings read the modes the
//! same way ([`load`]): the user's own document ([`MODES_KEY`]), else the one the Inkwell 0.2
//! import brought, else the built-in default ([`default_modes`]), each through
//! [`ModeStore::from_json`]. A save or a delete writes 1.0's key, once, with the stored document
//! patched in place ([`ModeStore::write_into`]): the first one adopts the import, and a field this
//! build does not know (0.2's `model`, which named a transcription model) survives. A running
//! dictation takes the change at once. A document that cannot be read is an error in both, never
//! quietly the default, and a save over it is refused unless the user chose to start over
//! (`replace_unreadable`).
//!
//! **A mode's language model.** `polish_model` names one the core holds ([`ModelRef`]: an engine
//! the shell registered, or the chosen own-key provider), and `polish_model_name` optionally a
//! model at that provider; `modes.listed` lists the models (`polish_models`), each with where it
//! sends and whether polish may use it now (a consent covers it and local-only mode lets it), and
//! names the one a mode without its own uses (`setting_polish_model`). A save that picks the
//! model, or another name at it, records where it sends then ([`ModelPin::to`]); one that sends
//! the same pin back keeps what was recorded. A mode whose model sends elsewhere later (a custom
//! server re-pointed) is not polished until the user confirms it there (`polish_model_confirm`;
//! `polish_model_state` `moved`, or `unrecorded` for a pin saved before destinations were).
//! A save that picks a model the core does not hold is refused (`model_unknown`); one a mode
//! already names is kept, so a mode whose model was let go of stays editable (its takes go out
//! unpolished, and say so).
//!
//! **Words travel here.** Names, prompts and apps are the user's own: they reach the shell only in
//! `modes.listed`, and errors say what is wrong, never them (I5).

use ink_core::Store;
use ink_pipeline::consent::{self, Destination, Feature};
use ink_pipeline::modes::{Mode, ModeEdit, ModeError, ModeStore};
use ink_pipeline::style::Style;
use serde_json::{Map, Value, json};

use crate::events::event;
use crate::llms::ModelRef;
use crate::phrases::{Failure, LIST_UNREADABLE};
use crate::queries::MODES_KEY;
use crate::runtime::Shared;
use crate::voice::default_modes;

/// Why a save or a delete is refused over stored modes that cannot be read.
pub const REFUSED_MODES: &str = "the stored modes cannot be read, so a save would replace them; \
     send replace_unreadable to start over";

/// A command of this module, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModesQuery {
    /// `modes.list`.
    List,
    /// `modes.save`.
    Save {
        /// The mode added (no id) or changed.
        edit: ModeEdit,
        /// Move an app another mode names to this one, rather than refuse it (`app_taken`).
        take_apps: bool,
        /// Start over from the default mode when the stored modes cannot be read.
        replace_unreadable: bool,
    },
    /// `modes.delete`.
    Delete {
        /// The mode's id.
        mode: String,
    },
}

impl ModesQuery {
    /// Whether it changes what dictation reads.
    pub fn saves(&self) -> bool {
        !matches!(self, Self::List)
    }
}

/// Reads `v` as one of this module's commands: `None` when `name` is none of them.
pub fn parse(name: &str, v: &Value) -> Option<Result<ModesQuery, String>> {
    let kind = match name {
        "modes.list" => Kind::List,
        "modes.save" => Kind::Save,
        "modes.delete" => Kind::Delete,
        _ => return None,
    };
    Some(parse_known(name, kind, v))
}

/// Which command [`parse`] read the name of.
#[derive(Clone, Copy)]
enum Kind {
    List,
    Save,
    Delete,
}

fn parse_known(name: &str, kind: Kind, v: &Value) -> Result<ModesQuery, String> {
    let fields: &[&str] = match kind {
        Kind::List => &[],
        Kind::Save => &["mode", "take_apps", "replace_unreadable"],
        Kind::Delete => &["mode"],
    };
    let obj = v.as_object().ok_or("command: not an object")?;
    if let Some(k) = obj
        .keys()
        .find(|k| !["cmd", "id"].contains(&k.as_str()) && !fields.contains(&k.as_str()))
    {
        return Err(format!("{name}: unknown field \"{k}\""));
    }
    let flag = |k: &str| match obj.get(k) {
        None => Ok(false),
        Some(b) => b
            .as_bool()
            .ok_or_else(|| format!("{name}: \"{k}\" is true or false")),
    };
    Ok(match kind {
        Kind::List => ModesQuery::List,
        Kind::Save => ModesQuery::Save {
            edit: read_edit(name, obj.get("mode"))?,
            take_apps: flag("take_apps")?,
            replace_unreadable: flag("replace_unreadable")?,
        },
        Kind::Delete => ModesQuery::Delete {
            mode: obj
                .get("mode")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{name}: needs a string \"mode\", the mode's id"))?
                .to_owned(),
        },
    })
}

/// The fields a mode in `modes.save` may name, as `modes.listed` names them.
const MODE_FIELDS: &[&str] = &[
    "id",
    "name",
    "style",
    "polish",
    "remove_fillers",
    "polish_prompt",
    "apps",
    "polish_model",
    "polish_model_name",
    "polish_model_confirm",
];

/// `modes.save`'s mode. Only a field's name is ever said, never its value.
fn read_edit(name: &str, mode: Option<&Value>) -> Result<ModeEdit, String> {
    let m = mode
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{name}: needs an object \"mode\""))?;
    if let Some(k) = m.keys().find(|k| !MODE_FIELDS.contains(&k.as_str())) {
        return Err(format!("{name}: the mode has an unknown field \"{k}\""));
    }
    let text = |k: &str| match m.get(k) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("{name}: the mode's \"{k}\" must be a string")),
    };
    let flag = |k: &str| match m.get(k) {
        None => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("{name}: the mode's \"{k}\" is true or false")),
    };
    let style =
        match text("style")? {
            None => None,
            Some(s) => Some(Style::parse(&s).ok_or_else(|| {
                format!("{name}: the mode's \"style\" is formal, casual or relaxed")
            })?),
        };
    let apps = match m.get("apps") {
        None => None,
        Some(Value::Array(apps)) => Some(
            apps.iter()
                .map(|a| a.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| format!("{name}: the mode's \"apps\" is a list of text"))?,
        ),
        Some(_) => return Err(format!("{name}: the mode's \"apps\" is a list of text")),
    };
    let nullable = |k: &str, what: &str| match m.get(k) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(id)) => Ok(Some(Some(id.clone()))),
        Some(_) => Err(format!("{name}: the mode's \"{k}\" is {what}")),
    };
    let polish_model = nullable("polish_model", "a model's id, or null for the AI setting's")?;
    let polish_model_name = nullable(
        "polish_model_name",
        "a model's name at the provider, or null for the one chosen in Settings > AI",
    )?;
    Ok(ModeEdit {
        id: text("id")?,
        name: text("name")?,
        style,
        polish_enabled: flag("polish")?,
        remove_fillers: flag("remove_fillers")?,
        polish_prompt: text("polish_prompt")?,
        apps,
        polish_model,
        polish_model_name,
        polish_model_confirm: flag("polish_model_confirm")?.unwrap_or(false),
    })
}

/// The stored document: 1.0's own, else the one the 0.2 import brought.
fn stored_doc(store: &dyn Store) -> Result<Option<String>, String> {
    if let Some(doc) = store.setting(MODES_KEY).map_err(|e| e.to_string())? {
        return Ok(Some(doc));
    }
    store
        .setting(ink_store::import::MODES_KEY)
        .map_err(|e| e.to_string())
}

/// **Worker.** The user's modes: 1.0's own, else the import's, else [`default_modes`]. A document
/// that cannot be read is an error, never quietly the default.
pub fn load(store: &dyn Store) -> Result<ModeStore, String> {
    match stored_doc(store)? {
        Some(doc) => ModeStore::from_json(&doc).map_err(|e| e.to_string()),
        None => Ok(default_modes()),
    }
}

/// What a save or a delete starts from: the stored modes and their document, or (when they cannot
/// be read and the user chose to start over) the default.
fn base(store: &dyn Store, replace_unreadable: bool) -> Result<(ModeStore, String), Failure> {
    let doc = stored_doc(store)?;
    match doc.as_deref().map(ModeStore::from_json) {
        None => {
            let modes = default_modes();
            let doc = modes.to_json();
            Ok((modes, doc))
        }
        Some(Ok(modes)) => Ok((modes, doc.unwrap_or_default())),
        Some(Err(_)) if replace_unreadable => {
            let modes = default_modes();
            let doc = modes.to_json();
            Ok((modes, doc))
        }
        Some(Err(_)) => Err(Failure {
            message: REFUSED_MODES.to_owned(),
            code: Some(LIST_UNREADABLE),
        }),
    }
}

fn refused(e: ModeError) -> Failure {
    Failure {
        message: e.to_string(),
        code: Some(e.code()),
    }
}

/// **Queries thread.** Answers a command. A save or a delete writes 1.0's key; the caller then
/// hands the running dictation its new settings ([`ModesQuery::saves`]).
pub fn answer(
    shared: &Shared,
    query: ModesQuery,
    reference: Option<&str>,
) -> Result<Value, Failure> {
    let store = shared.store.as_ref();
    let (after, doc, saved) = match query {
        ModesQuery::List => return Ok(listed(shared, &load(store)?, None, reference)),
        ModesQuery::Save {
            edit,
            take_apps,
            replace_unreadable,
        } => {
            let (modes, doc) = base(store, replace_unreadable)?;
            let new_id = modes.fresh_id(shared.clock.unix_ms());
            // Where each model the core holds sends at this moment.
            let model_at = |id: &str, name: Option<&str>| {
                let r = ModelRef::parse(id)?;
                shared
                    .llms
                    .info_of(&r, name)
                    .map(|info| Destination::of(&info))
            };
            let (after, id) = modes
                .save(&edit, take_apps, &new_id, &model_at)
                .map_err(refused)?;
            (after, doc, Some(id))
        }
        ModesQuery::Delete { mode } => {
            let (modes, doc) = base(store, false)?;
            (modes.delete(&mode).map_err(refused)?, doc, None)
        }
    };
    let written = after.write_into(&doc).map_err(|e| e.to_string())?;
    store.set_setting(MODES_KEY, &written).map_err(|e| {
        log::error!("modes: the change could not be saved: {e}");
        "couldn't save the modes, so they stay as they were".to_owned()
    })?;
    Ok(listed(shared, &after, saved.as_deref(), reference))
}

/// **Queries thread.** `modes.listed`: the modes, the default polish prompt, and the models a mode can pick; `saved`
/// is the id of the mode a save saved. It carries the user's words (names, prompts, apps): never
/// log it.
pub fn listed(
    shared: &Shared,
    modes: &ModeStore,
    saved: Option<&str>,
    reference: Option<&str>,
) -> Value {
    let items: Vec<Value> = modes.modes.iter().map(|m| mode_item(shared, m)).collect();
    // Read once each, so every model is judged against the same consents and the same models.
    // Consents that cannot be read are none (logged by name): nothing is shown as allowed.
    let consents = consent::stored(shared.store.as_ref(), Feature::Polish);
    let choices = shared.llms.choices();
    let models: Vec<Value> = choices
        .iter()
        .map(|(r, info)| {
            let (_, needed, name) = crate::consent::described(info.clone());
            let blocked = shared.local_only.check(&info.endpoint).is_err();
            json!({
                "id": r.id(),
                "name": name,
                "model": info.model,
                "to": needed.kind(),
                "allowed": !blocked && consents.iter().any(|c| c.covers(info)),
                "blocked_local_only": blocked,
            })
        })
        .collect();
    event(
        "modes.listed",
        &[
            ("default_id", Some(modes.default_id.clone().into())),
            ("modes", Some(Value::Array(items))),
            (
                "default_polish_prompt",
                Some(ink_llm::tasks::polish::DEFAULT_POLISH_PROMPT.into()),
            ),
            ("polish_models", Some(Value::Array(models))),
            (
                "setting_polish_model",
                choices.first().map(|(r, _)| r.id().into()),
            ),
            ("saved", saved.map(Into::into)),
            ("ref", reference.map(Into::into)),
        ],
    )
}

/// One mode as `modes.listed` lists it.
fn mode_item(shared: &Shared, m: &Mode) -> Value {
    let mut item = Map::new();
    item.insert("id".into(), m.id.clone().into());
    item.insert("name".into(), m.name.clone().into());
    // A style this build does not know is shown as such, never as another style.
    let style = if m.style_unknown {
        "other"
    } else {
        m.style.as_str()
    };
    item.insert("style".into(), style.into());
    item.insert("polish".into(), m.polish_enabled.into());
    item.insert("remove_fillers".into(), m.remove_fillers.into());
    item.insert("polish_prompt".into(), m.polish_prompt.clone().into());
    item.insert("apps".into(), json!(m.apps));
    if let Some(pin) = &m.polish_model {
        item.insert("polish_model".into(), pin.id.clone().into());
        if let Some(name) = &pin.model {
            item.insert("polish_model_name".into(), name.clone().into());
        }
        // As a take would find it: held now, and sending where it did when the mode was saved.
        let now =
            ModelRef::parse(&pin.id).and_then(|r| shared.llms.info_of(&r, pin.model.as_deref()));
        let state = match (now, &pin.to) {
            (None, _) => "missing",
            (Some(_), None) => "unrecorded",
            (Some(info), Some(to)) if to.covers(&info) => "ready",
            (Some(_), Some(_)) => "moved",
        };
        item.insert("polish_model_state".into(), state.into());
    }
    Value::Object(item)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(json: &str) -> Option<Result<ModesQuery, String>> {
        let v: Value = serde_json::from_str(json).unwrap();
        parse(v["cmd"].as_str().unwrap(), &v)
    }

    #[test]
    fn commands_are_read_and_what_cannot_be_read_is_refused() {
        assert!(p(r#"{"cmd":"snippets.list"}"#).is_none());
        assert_eq!(
            p(r#"{"cmd":"modes.list","id":"r"}"#),
            Some(Ok(ModesQuery::List))
        );
        assert_eq!(
            p(r#"{"cmd":"modes.delete","mode":"m1"}"#),
            Some(Ok(ModesQuery::Delete { mode: "m1".into() }))
        );
        let Some(Ok(ModesQuery::Save {
            edit,
            take_apps,
            replace_unreadable,
        })) = p(
            r#"{"cmd":"modes.save","take_apps":true,"mode":{"name":"Mail","style":"Casual","polish":true,
                "remove_fillers":false,"polish_prompt":"Short.","apps":["com.example.mail"],"polish_model":null}}"#,
        )
        else {
            panic!("a save reads");
        };
        assert!(take_apps && !replace_unreadable);
        assert_eq!(
            edit,
            ModeEdit {
                id: None,
                name: Some("Mail".into()),
                style: Some(Style::Casual),
                polish_enabled: Some(true),
                remove_fillers: Some(false),
                polish_prompt: Some("Short.".into()),
                apps: Some(vec!["com.example.mail".into()]),
                polish_model: Some(None),
                polish_model_name: None,
                polish_model_confirm: false,
            }
        );
        let Some(Ok(ModesQuery::Save { edit, .. })) =
            p(r#"{"cmd":"modes.save","mode":{"id":"c","polish_model":"engine:x"}}"#)
        else {
            panic!("an update reads");
        };
        assert_eq!(edit.id.as_deref(), Some("c"));
        assert_eq!(edit.name, None, "absent fields keep their value");
        assert_eq!(edit.polish_model, Some(Some("engine:x".into())));
        for bad in [
            r#"{"cmd":"modes.list","x":1}"#,
            r#"{"cmd":"modes.save"}"#,
            r#"{"cmd":"modes.save","mode":"Mail"}"#,
            r#"{"cmd":"modes.save","mode":{"name":"x","model":"m"}}"#,
            r#"{"cmd":"modes.save","mode":{"name":"x","polish_enabled":true}}"#,
            r#"{"cmd":"modes.save","mode":{"name":"x","style":"shouting"}}"#,
            r#"{"cmd":"modes.save","mode":{"name":"x","apps":"slack"}}"#,
            r#"{"cmd":"modes.save","mode":{"name":"x","apps":[1]}}"#,
            r#"{"cmd":"modes.save","mode":{"name":"x","polish":"on"}}"#,
            r#"{"cmd":"modes.save","mode":{"name":7}}"#,
            r#"{"cmd":"modes.save","mode":{"name":"x","polish_model":3}}"#,
            r#"{"cmd":"modes.save","mode":{"name":"x"},"take_apps":"yes"}"#,
            r#"{"cmd":"modes.delete"}"#,
            r#"{"cmd":"modes.delete","mode":"m1","replace_unreadable":true}"#,
        ] {
            assert!(matches!(p(bad), Some(Err(_))), "{bad} must be refused");
        }
    }

    #[test]
    fn errors_never_quote_the_user_s_words() {
        for bad in [
            r#"{"cmd":"modes.save","mode":{"name":"Canary","style":"canary-style"}}"#,
            r#"{"cmd":"modes.save","mode":{"name":"Canary","apps":["canary", 2]}}"#,
            r#"{"cmd":"modes.save","mode":{"name":"Canary","polish_model":["canary"]}}"#,
        ] {
            let Some(Err(e)) = p(bad) else {
                panic!("{bad} refused");
            };
            assert!(!e.contains("Canary") && !e.contains("canary-style"), "{e}");
        }
    }
}
