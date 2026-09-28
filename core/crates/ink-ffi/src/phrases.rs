//! Snippets and voice commands as dictation uses them and Settings lists and edits them, and what
//! the Inkwell 0.2 import has to say about the dictation key.
//!
//! | Command | Answer |
//! |---|---|
//! | `snippets.list` | `snippets.listed`: the snippets, in the user's order |
//! | `snippets.save` | `snippets.listed`: the list as saved (the whole list replaces the stored one) |
//! | `voice_commands.list` | `voice_commands.listed`: the switch, the wake word and the commands, each saying whether this build carries it out |
//! | `voice_commands.save` | `voice_commands.listed`, as saved |
//! | `import.notes` | `import.notes`: what became of 0.2's dictation hotkey, until the user dismisses it |
//!
//! Each answer echoes the command's `id` as `ref`; a failure is `command.failed` with that id.
//!
//! **One reading for both.** Dictation ([`voice`](crate::voice)) and Settings read the same
//! documents the same way ([`load_snippets`], [`load_commands`]): the user's own under 1.0's keys,
//! else what the 0.2 import brought (the same shape), else the defaults (no snippets; 0.2's
//! commands, off). A save writes 1.0's key, so the first save adopts an import, and a running
//! dictation takes it at once. A stored document that cannot be read is an error in both, never
//! quietly the defaults: the next save would then erase the user's list.
//!
//! **Words travel here.** Triggers, expansions and a command's text are the user's own; they reach
//! the shell only in the answers, and errors name what is wrong, never the text (I5).

use std::collections::HashSet;

use ink_core::Store;
use ink_pipeline::snippets::{self, Snippet, SnippetStore};
use ink_pipeline::voicecommand::{self, CommandAction, VoiceCommand, VoiceCommandStore};
use ink_store::import::{KEY_NOTE_KEY, SNIPPETS_KEY, VOICE_COMMANDS_KEY};
use serde_json::{Map, Value, json};

use crate::events::event;

/// The shell setting that dismisses the import's key note (`dismissed`): the note is said once.
pub const KEY_NOTE_SETTING: &str = "import.key_note";

/// The most snippets or voice commands one save may hold: far beyond a hand-kept list, and a
/// bound on what one command can make the store and every take's matcher hold.
pub const MAX_ITEMS: usize = 2_000;

/// A query of this module, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhrasesQuery {
    /// `snippets.list`.
    SnippetsList,
    /// `snippets.save`.
    SnippetsSave(SnippetStore),
    /// `voice_commands.list`.
    CommandsList,
    /// `voice_commands.save`.
    CommandsSave(VoiceCommandStore),
    /// `import.notes`.
    ImportNotes,
}

impl PhrasesQuery {
    /// Whether it changes what dictation reads.
    pub fn saves(&self) -> bool {
        matches!(self, Self::SnippetsSave(_) | Self::CommandsSave(_))
    }
}

/// The snippets: the user's own, else the imported ones, else none. The flag says they are the
/// import's (not yet saved in 1.0).
pub fn load_snippets(store: &dyn Store) -> Result<(SnippetStore, bool), String> {
    const UNREADABLE: &str = "the stored snippets cannot be read";
    match stored(store, snippets::SETTING_KEY, SNIPPETS_KEY)? {
        None => Ok((SnippetStore::default(), false)),
        Some((doc, imported)) => SnippetStore::from_json(&doc)
            .map(|s| (s, imported))
            .map_err(|_| UNREADABLE.to_owned()),
    }
}

/// The voice commands: the user's own, else the imported ones, else the defaults (off). The flag
/// says they are the import's.
pub fn load_commands(store: &dyn Store) -> Result<(VoiceCommandStore, bool), String> {
    const UNREADABLE: &str = "the stored voice commands cannot be read";
    match stored(store, voicecommand::SETTING_KEY, VOICE_COMMANDS_KEY)? {
        None => Ok((VoiceCommandStore::default(), false)),
        Some((doc, imported)) => VoiceCommandStore::from_json(&doc)
            .map(|c| (c, imported))
            .map_err(|_| UNREADABLE.to_owned()),
    }
}

/// The document under `own`, else under `imported` (flagged).
fn stored(store: &dyn Store, own: &str, imported: &str) -> Result<Option<(String, bool)>, String> {
    if let Some(doc) = store.setting(own).map_err(|e| e.to_string())? {
        return Ok(Some((doc, false)));
    }
    Ok(store
        .setting(imported)
        .map_err(|e| e.to_string())?
        .map(|doc| (doc, true)))
}

/// Reads `v` as one of this module's commands: `None` when `name` is none of them.
pub fn parse(name: &str, v: &Value) -> Option<Result<PhrasesQuery, String>> {
    let fields: &[&str] = match name {
        "snippets.list" | "voice_commands.list" | "import.notes" => &[],
        "snippets.save" => &["snippets"],
        "voice_commands.save" => &["enabled", "wake_prefix", "commands"],
        _ => return None,
    };
    Some(parse_known(name, fields, v))
}

fn parse_known(name: &str, fields: &[&str], v: &Value) -> Result<PhrasesQuery, String> {
    let obj = v.as_object().ok_or("command: not an object")?;
    if let Some(k) = obj
        .keys()
        .find(|k| !["cmd", "id"].contains(&k.as_str()) && !fields.contains(&k.as_str()))
    {
        return Err(format!("{name}: unknown field \"{k}\""));
    }
    Ok(match name {
        "snippets.list" => PhrasesQuery::SnippetsList,
        "voice_commands.list" => PhrasesQuery::CommandsList,
        "import.notes" => PhrasesQuery::ImportNotes,
        "snippets.save" => PhrasesQuery::SnippetsSave(read_snippets(name, obj)?),
        "voice_commands.save" => PhrasesQuery::CommandsSave(read_commands(name, obj)?),
        _ => unreachable!("parse() lists every command"),
    })
}

/// A list of at most [`MAX_ITEMS`] under `field`.
fn list<'a>(name: &str, obj: &'a Map<String, Value>, field: &str) -> Result<&'a [Value], String> {
    let items = obj
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{name}: needs a list \"{field}\""))?;
    if items.len() > MAX_ITEMS {
        return Err(format!("{name}: at most {MAX_ITEMS} in \"{field}\""));
    }
    Ok(items)
}

/// Ids are present and unique: Settings edits and deletes by them.
fn check_id(name: &str, what: &str, id: &str, seen: &mut HashSet<String>) -> Result<(), String> {
    if id.trim().is_empty() {
        return Err(format!("{name}: {what} has a blank id"));
    }
    if !seen.insert(id.to_owned()) {
        return Err(format!("{name}: {what} has the id of an earlier one"));
    }
    Ok(())
}

fn read_snippets(name: &str, obj: &Map<String, Value>) -> Result<SnippetStore, String> {
    let items = list(name, obj, "snippets")?;
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let what = format!("snippet {}", i + 1);
        // The stored form's own reader, one item at a time, so the shapes cannot drift apart.
        let one = SnippetStore::from_json(&Value::Array(vec![item.clone()]).to_string()).map_err(
            |_| format!("{name}: {what} needs \"id\", \"trigger\" and \"expansion\" text"),
        )?;
        let snippet: Snippet = one
            .snippets
            .into_iter()
            .next()
            .ok_or_else(|| format!("{name}: {what} was not read"))?;
        check_id(name, &what, &snippet.id, &mut seen)?;
        // A blank trigger is kept (it never expands): an imported list may hold one, and refusing
        // it would make every later save of that list fail.
        out.push(snippet);
    }
    Ok(SnippetStore { snippets: out })
}

fn read_commands(name: &str, obj: &Map<String, Value>) -> Result<VoiceCommandStore, String> {
    let enabled = obj
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{name}: needs \"enabled\", true or false"))?;
    let wake_prefix = obj
        .get("wake_prefix")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name}: needs a string \"wake_prefix\""))?
        .trim()
        .to_owned();
    // A blank wake word is kept, as 0.2 let the user leave it: it matches nothing.
    let items = list(name, obj, "commands")?;
    let mut seen = HashSet::new();
    let mut commands = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let what = format!("command {}", i + 1);
        let c = item
            .as_object()
            .ok_or_else(|| format!("{name}: {what} is not an object"))?;
        let text = |k: &str| c.get(k).and_then(Value::as_str);
        let id = text("id").ok_or_else(|| format!("{name}: {what} needs a string \"id\""))?;
        check_id(name, &what, id, &mut seen)?;
        let triggers: Vec<String> = c
            .get("triggers")
            .and_then(Value::as_array)
            .and_then(|t| {
                t.iter()
                    .map(|t| t.as_str().map(|t| t.trim().to_owned()))
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or_else(|| format!("{name}: {what} needs \"triggers\", a list of text"))?
            .into_iter()
            .filter(|t| !t.is_empty())
            .collect();
        let kind = text("action").ok_or_else(|| format!("{name}: {what} needs an \"action\""))?;
        let action = CommandAction::from_parts(kind, text("value")).ok_or_else(|| {
            format!("{name}: {what} has an unknown action, or lacks the \"value\" it needs")
        })?;
        commands.push(VoiceCommand {
            id: id.to_owned(),
            triggers,
            action,
            enabled: c
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or_else(|| format!("{name}: {what} needs \"enabled\", true or false"))?,
        });
    }
    Ok(VoiceCommandStore {
        enabled,
        wake_prefix,
        commands,
    })
}

/// **Queries thread.** Answers a query. A save writes 1.0's key; the caller then hands the
/// running dictation its new settings ([`PhrasesQuery::saves`]).
pub fn answer(store: &dyn Store, query: PhrasesQuery, id: Option<&str>) -> Result<Value, String> {
    let reference = id.map(Value::from);
    Ok(match query {
        PhrasesQuery::SnippetsList => {
            let (s, imported) = load_snippets(store)?;
            snippets_listed(&s, imported, reference)
        }
        PhrasesQuery::SnippetsSave(s) => {
            store
                .set_setting(snippets::SETTING_KEY, &s.to_json())
                .map_err(|e| e.to_string())?;
            snippets_listed(&s, false, reference)
        }
        PhrasesQuery::CommandsList => {
            let (c, imported) = load_commands(store)?;
            commands_listed(&c, imported, reference)
        }
        PhrasesQuery::CommandsSave(c) => {
            store
                .set_setting(voicecommand::SETTING_KEY, &c.to_json())
                .map_err(|e| e.to_string())?;
            commands_listed(&c, false, reference)
        }
        PhrasesQuery::ImportNotes => import_notes(store, reference)?,
    })
}

fn snippets_listed(s: &SnippetStore, imported: bool, reference: Option<Value>) -> Value {
    let items: Vec<Value> = s
        .snippets
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "trigger": s.trigger,
                "expansion": s.expansion,
                "category": s.category,
                "enabled": s.enabled,
            })
        })
        .collect();
    event(
        "snippets.listed",
        &[
            ("snippets", Some(Value::Array(items))),
            ("from_import", Some(imported.into())),
            ("ref", reference),
        ],
    )
}

fn commands_listed(c: &VoiceCommandStore, imported: bool, reference: Option<Value>) -> Value {
    let items: Vec<Value> = c
        .commands
        .iter()
        .map(|c| {
            let mut item = Map::new();
            item.insert("id".into(), c.id.clone().into());
            item.insert("triggers".into(), json!(c.triggers));
            item.insert("action".into(), c.action.kind().into());
            if let Some(value) = c.action.value() {
                item.insert("value".into(), value.into());
            }
            item.insert("enabled".into(), c.enabled.into());
            item.insert("carried_out".into(), c.action.carried_out().into());
            Value::Object(item)
        })
        .collect();
    event(
        "voice_commands.listed",
        &[
            ("enabled", Some(c.enabled.into())),
            ("wake_prefix", Some(c.wake_prefix.clone().into())),
            ("commands", Some(Value::Array(items))),
            ("from_import", Some(imported.into())),
            ("ref", reference),
        ],
    )
}

/// `import.notes`: the key note while there is something to say (0.2's hotkey did not carry over,
/// or 1.0 no longer toggles) and the user has not dismissed it.
fn import_notes(store: &dyn Store, reference: Option<Value>) -> Result<Value, String> {
    const UNREADABLE: &str = "the import's key note cannot be read";
    let dismissed = store
        .setting(KEY_NOTE_SETTING)
        .map_err(|e| e.to_string())?
        .is_some_and(|v| v == "dismissed");
    let mut key = None;
    if !dismissed && let Some(doc) = store.setting(KEY_NOTE_KEY).map_err(|e| e.to_string())? {
        let v: Value = serde_json::from_str(&doc).map_err(|_| UNREADABLE)?;
        let hotkey = v["hotkey"].as_str().ok_or(UNREADABLE)?;
        let outcome = v["outcome"].as_str().ok_or(UNREADABLE)?;
        if !["mapped", "combination", "other_key"].contains(&outcome) {
            return Err(UNREADABLE.into());
        }
        let toggle = v["toggle"].as_bool().ok_or(UNREADABLE)?;
        let applied = v["applied"].as_bool().ok_or(UNREADABLE)?;
        if outcome != "mapped" || toggle {
            let mut note = Map::new();
            note.insert("hotkey".into(), hotkey.into());
            note.insert("outcome".into(), outcome.into());
            if let Some(k) = v["key"].as_str() {
                note.insert("key".into(), k.into());
            }
            note.insert("applied".into(), applied.into());
            note.insert("toggle".into(), toggle.into());
            key = Some(Value::Object(note));
        }
    }
    Ok(event("import.notes", &[("key", key), ("ref", reference)]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ink_core::mock::MemStore;

    fn p(json: &str) -> Option<Result<PhrasesQuery, String>> {
        let v: Value = serde_json::from_str(json).unwrap();
        parse(v["cmd"].as_str().unwrap(), &v)
    }

    #[test]
    fn saves_are_read_and_what_cannot_be_read_is_refused() {
        assert!(p(r#"{"cmd":"modes.list"}"#).is_none());
        assert_eq!(
            p(r#"{"cmd":"snippets.list","id":"x"}"#),
            Some(Ok(PhrasesQuery::SnippetsList))
        );
        let Some(Ok(PhrasesQuery::SnippetsSave(s))) = p(
            r#"{"cmd":"snippets.save","snippets":[{"id":"a","trigger":"brb","expansion":"be right back"}]}"#,
        ) else {
            panic!("a snippet list reads");
        };
        assert_eq!(s.snippets[0].trigger, "brb");
        assert!(s.snippets[0].enabled, "0.2's default");
        let Some(Ok(PhrasesQuery::CommandsSave(c))) = p(
            r#"{"cmd":"voice_commands.save","enabled":true,"wake_prefix":" inkwell ","commands":[
                {"id":"sig","triggers":["sign off",""],"action":"insert_text","value":"Best","enabled":true},
                {"id":"undo","triggers":["scratch that"],"action":"undo","enabled":false}]}"#,
        ) else {
            panic!("a command list reads");
        };
        assert_eq!(c.wake_prefix, "inkwell");
        assert_eq!(c.commands[0].triggers, ["sign off"]);
        assert_eq!(c.commands[1].action, CommandAction::Undo);
        for bad in [
            r#"{"cmd":"snippets.save"}"#,
            r#"{"cmd":"snippets.save","snippets":[{"id":" ","trigger":"x","expansion":"x"}]}"#,
            r#"{"cmd":"snippets.save","snippets":[{"id":"a","trigger":"x","expansion":"y"},{"id":"a","trigger":"z","expansion":"y"}]}"#,
            r#"{"cmd":"snippets.save","snippets":[{"id":"a","trigger":"x"}]}"#,
            r#"{"cmd":"snippets.save","snippets":[],"extra":1}"#,
            r#"{"cmd":"voice_commands.save","enabled":true,"commands":[]}"#,
            r#"{"cmd":"voice_commands.save","enabled":true,"wake_prefix":"inkwell","commands":[{"id":"a","triggers":"x","action":"undo","enabled":true}]}"#,
            r#"{"cmd":"voice_commands.save","enabled":true,"wake_prefix":"inkwell","commands":[{"id":"a","triggers":["x"],"action":"undo","enabled":true},{"id":"a","triggers":["y"],"action":"undo","enabled":true}]}"#,
            r#"{"cmd":"voice_commands.save","enabled":true,"wake_prefix":"inkwell","commands":[{"id":"a","triggers":["x"],"action":"open_url","enabled":true}]}"#,
            r#"{"cmd":"voice_commands.save","enabled":true,"wake_prefix":"inkwell","commands":[{"id":"a","triggers":["x"],"action":"dance","enabled":true}]}"#,
            r#"{"cmd":"voice_commands.save","enabled":"yes","wake_prefix":"inkwell","commands":[]}"#,
        ] {
            assert!(matches!(p(bad), Some(Err(_))), "{bad} must be refused");
        }
    }

    /// What an import may hold and 0.2 allowed is saved back as it is, so a list never becomes
    /// unsaveable: a blank wake word, a blank trigger, a command left without one.
    #[test]
    fn blanks_the_0_2_app_allowed_can_be_saved_back() {
        assert!(matches!(
            p(r#"{"cmd":"snippets.save","snippets":[{"id":"a","trigger":"","expansion":"x"}]}"#),
            Some(Ok(_))
        ));
        let Some(Ok(PhrasesQuery::CommandsSave(c))) = p(
            r#"{"cmd":"voice_commands.save","enabled":true,"wake_prefix":"","commands":[{"id":"a","triggers":[" "],"action":"undo","enabled":true}]}"#,
        ) else {
            panic!("saved as it is");
        };
        assert!(c.commands[0].triggers.is_empty());
        assert!(
            c.detect("undo").is_none(),
            "a blank wake word matches nothing"
        );
    }

    #[test]
    fn errors_never_quote_the_user_s_words() {
        let Some(Err(e)) = p(
            r#"{"cmd":"snippets.save","snippets":[{"id":"a","trigger":"canary","expansion":7}]}"#,
        ) else {
            panic!("refused");
        };
        assert!(!e.contains("canary"), "{e}");
    }

    #[test]
    fn the_import_is_listed_until_the_first_save_adopts_it() {
        let store = MemStore::new();
        let (none, imported) = load_snippets(&store).unwrap();
        assert!(none.snippets.is_empty() && !imported);
        let (defaults, imported) = load_commands(&store).unwrap();
        assert!(!defaults.enabled, "off by default");
        assert!(!imported);

        store
            .set_setting(
                SNIPPETS_KEY,
                r#"[{"id":"s","trigger":"brb","expansion":"be right back","category":"","enabled":true}]"#,
            )
            .unwrap();
        let listed = answer(&store, PhrasesQuery::SnippetsList, Some("r")).unwrap();
        assert_eq!(listed["snippets"][0]["trigger"], "brb");
        assert_eq!(listed["from_import"], true);
        assert_eq!(listed["ref"], "r");

        let mut edited = load_snippets(&store).unwrap().0;
        edited.snippets[0].expansion = "back soon".into();
        answer(&store, PhrasesQuery::SnippetsSave(edited), None).unwrap();
        let (now, imported) = load_snippets(&store).unwrap();
        assert_eq!(now.snippets[0].expansion, "back soon");
        assert!(!imported, "saved in 1.0's own key");
        // The import's document is left as it was written.
        assert!(
            store
                .setting(SNIPPETS_KEY)
                .unwrap()
                .unwrap()
                .contains("be right back")
        );

        store
            .set_setting(snippets::SETTING_KEY, "not json")
            .unwrap();
        assert!(load_snippets(&store).is_err(), "never quietly empty");
        assert!(answer(&store, PhrasesQuery::SnippetsList, None).is_err());
    }

    #[test]
    fn commands_are_listed_with_whether_this_build_carries_them_out() {
        let store = MemStore::new();
        store
            .set_setting(
                VOICE_COMMANDS_KEY,
                r#"{"enabled":true,"wake_prefix":"inkwell","commands":[
                    {"id":"sig","triggers":["sign off"],"action":{"type":"insert_text","text":"Best"},"enabled":true},
                    {"id":"site","triggers":["open the site"],"action":{"type":"open_url","url":"https://example.com"},"enabled":true}]}"#,
            )
            .unwrap();
        let listed = answer(&store, PhrasesQuery::CommandsList, None).unwrap();
        assert_eq!(listed["enabled"], true);
        assert_eq!(listed["from_import"], true);
        let commands = listed["commands"].as_array().unwrap();
        assert_eq!(commands[0]["action"], "insert_text");
        assert_eq!(commands[0]["value"], "Best");
        assert_eq!(commands[0]["carried_out"], true);
        assert_eq!(commands[1]["carried_out"], false);
    }

    /// The importer (ink-store, below this crate) writes the dictation key under its own copy of
    /// the setting's name, with its own list of tokens: both must be dictation's.
    #[test]
    fn the_importer_writes_the_key_dictation_reads_with_tokens_it_holds() {
        use ink_store::import::{DICTATION_KEY_SETTING, map_hotkey};
        assert_eq!(DICTATION_KEY_SETTING, crate::voice::KEY_SETTING);
        for old in ["fn", "right_cmd", "right_opt", "right_ctrl"] {
            let key = map_hotkey(old).unwrap();
            assert!(crate::voice::KEYS.contains(&key), "{key}");
        }
    }

    #[test]
    fn the_key_note_is_said_until_dismissed_and_only_when_there_is_something_to_say() {
        let store = MemStore::new();
        let notes = |s: &MemStore| answer(s, PhrasesQuery::ImportNotes, None).unwrap();
        assert!(notes(&store).get("key").is_none(), "no import");
        store
            .set_setting(
                KEY_NOTE_KEY,
                r#"{"hotkey":"right_opt","key":"right_option","outcome":"mapped","applied":true,"toggle":false}"#,
            )
            .unwrap();
        assert!(notes(&store).get("key").is_none(), "carried over as it was");
        store
            .set_setting(
                KEY_NOTE_KEY,
                r#"{"hotkey":"super+shift+space","key":null,"outcome":"combination","applied":false,"toggle":false}"#,
            )
            .unwrap();
        let note = notes(&store);
        assert_eq!(note["key"]["outcome"], "combination");
        assert_eq!(note["key"]["hotkey"], "super+shift+space");
        assert!(note["key"].get("key").is_none());
        store.set_setting(KEY_NOTE_SETTING, "dismissed").unwrap();
        assert!(notes(&store).get("key").is_none(), "said once");
    }
}
