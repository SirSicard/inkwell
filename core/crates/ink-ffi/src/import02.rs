//! Inkwell 0.2's data brought into the library from the screens: onboarding's import step and
//! Settings. ink-store does the reading and the writing ([`Inkwell02`]); this module finds 0.2's
//! data, and answers on the queries thread.
//!
//! | Command | Answer |
//! |---|---|
//! | `import.check` | `import.checked`: `found` with the dry run's counts, `absent`, `imported` (this library holds an import already), or `unreadable` with why, in words |
//! | `import.run` | `import.finished` with what was written; a failure is `command.failed`, in words |
//!
//! Each answer echoes the command's `id` as `ref`; `command.failed` carries it as its `id`.
//!
//! **Where 0.2's data is** is the core's to know, never the shell's: Tauri's app data directory
//! for 0.2's identifier, `com.inkwell.app`, where 0.2 wrote every file (its `setup.rs`,
//! `app.path().app_data_dir()`). Tauri takes that from the `dirs` crate's data directory:
//! `~/Library/Application Support` on a Mac and the roaming app data folder (`%APPDATA%`) on
//! Windows ([`source_dir`]).
//!
//! **Looking costs nothing.** The check opens nothing once the library holds an import (the
//! marker is enough), reads 0.2's data read-only otherwise (ink-store's rules: `mode=ro`, regular
//! files only, a write in progress refused), and never asks the keychain, so it cannot bring up a
//! system prompt; its `linked_keys` is 0. Only an import asks the keychain which providers have a
//! key, through ink-llm's existence check (attributes, never a secret), as the checklist binary
//! does.
//!
//! **After an import** the queries thread hands a running dictation its settings again
//! ([`voice::settings_changed`](crate::voice::settings_changed)), so the imported key, snippets,
//! voice commands and modes are used at once, and the screens list the library again on
//! `import.finished`.
//!
//! Messages name a file and what is wrong with it, never a path (it can name the user) or
//! anything the user said or typed (I5).

use std::path::PathBuf;
use std::sync::Arc;

use ink_core::Store;
use ink_llm::KeyStore;
use ink_store::SqliteStore;
use ink_store::import::{
    Counts, ImportError, Inkwell02, KeyProbe, KeyProbeError, MARKER_KEY, NoKeychain,
};
use serde_json::{Map, Value};

use crate::events::event;

/// Inkwell 0.2's Tauri identifier: the name of its data directory.
pub const IDENTIFIER_0_2: &str = "com.inkwell.app";

/// What the import needs besides the core's state ([`Core::set_import02`](crate::runtime::Core::set_import02)).
pub struct Import02 {
    /// The library as itself: the same store the core runs on, since an import is one SQLite
    /// transaction, which the `Store` trait has no way to ask for.
    pub library: Arc<SqliteStore>,
    /// 0.2's data directory on this computer ([`source_dir`]); `None` where there is none.
    pub source: Option<PathBuf>,
    /// The keychain an import asks which providers have a key; `None`: not asked (each provider
    /// is recorded as unchecked in the import's marker).
    pub keys: Option<Arc<dyn KeyStore>>,
}

impl Import02 {
    /// This computer's: `library`, 0.2's own data directory and the OS keychain (not touched
    /// until an import runs).
    pub fn production(library: Arc<SqliteStore>) -> Self {
        let keys = match ink_llm::OsKeyStore::native() {
            Ok(store) => Some(Arc::new(store) as Arc<dyn KeyStore>),
            Err(e) => {
                log::warn!("no keychain ({e:?}): a 0.2 import leaves its keys unchecked");
                None
            }
        };
        Self {
            library,
            source: source_dir(),
            keys,
        }
    }
}

/// Where Inkwell 0.2 kept its data on this computer (see the module docs), or `None` where the
/// OS cannot say or 1.0 has no 0.2 to import (neither macOS nor Windows). Nothing is opened.
pub fn source_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        mac_source_dir(std::env::home_dir())
    }
    #[cfg(windows)]
    {
        ink_platform_win::roaming_app_data().map(|dir| dir.join(IDENTIFIER_0_2))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        None
    }
}

/// The Mac's, under `home` (`dirs::data_dir()` is `$HOME/Library/Application Support`).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn mac_source_dir(home: Option<PathBuf>) -> Option<PathBuf> {
    let home = home.filter(|h| h.is_absolute())?;
    Some(
        home.join("Library")
            .join("Application Support")
            .join(IDENTIFIER_0_2),
    )
}

/// A command of this module, read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Import02Query {
    /// `import.check`.
    Check,
    /// `import.run`.
    Run,
}

impl Import02Query {
    /// Whether it may change what dictation reads (an import writes its key and documents).
    pub fn imports(self) -> bool {
        self == Self::Run
    }
}

/// Reads `v` as one of this module's commands: `None` when `name` is none of them. They take no
/// fields besides `cmd` and `id`.
pub fn parse(name: &str, v: &Value) -> Option<Result<Import02Query, String>> {
    let query = match name {
        "import.check" => Import02Query::Check,
        "import.run" => Import02Query::Run,
        _ => return None,
    };
    let Some(obj) = v.as_object() else {
        return Some(Err("command: not an object".into()));
    };
    if let Some(k) = obj.keys().find(|k| !["cmd", "id"].contains(&k.as_str())) {
        return Some(Err(format!("{name}: unknown field \"{k}\"")));
    }
    Some(Ok(query))
}

/// **Queries thread.** Answers a query: its event, or the words for `command.failed`.
pub fn answer(
    import: Option<&Import02>,
    query: Import02Query,
    id: Option<&str>,
) -> Result<Value, String> {
    let import = import.ok_or("this core has no library to import into")?;
    let reference = id.map(Value::from);
    match query {
        Import02Query::Check => check(import, reference),
        Import02Query::Run => run(import, reference),
    }
}

fn check(import: &Import02, reference: Option<Value>) -> Result<Value, String> {
    let checked = |state: &str, counts: Option<Value>, message: Option<String>| {
        event(
            "import.checked",
            &[
                ("state", Some(state.into())),
                ("counts", counts),
                ("message", message.map(Value::from)),
                ("ref", reference.clone()),
            ],
        )
    };
    if imported_already(import)? {
        return Ok(checked("imported", None, None));
    }
    let Some(dir) = &import.source else {
        return Ok(checked("absent", None, None));
    };
    Ok(match Inkwell02::read(dir, &NoKeychain) {
        Ok(source) => checked("found", Some(counts_json(&source.counts())), None),
        Err(ImportError::NoSource) => checked("absent", None, None),
        Err(e) => {
            log::warn!("import.check: Inkwell 0.2's data cannot be read now: {e}");
            checked("unreadable", None, Some(words(&e)))
        }
    })
}

fn run(import: &Import02, reference: Option<Value>) -> Result<Value, String> {
    // Neither 0.2's data nor the keychain is opened for a library that holds an import already.
    // (The importer checks again inside its transaction.)
    if imported_already(import)? {
        return Err(words(&ImportError::AlreadyImported));
    }
    let dir = import
        .source
        .as_ref()
        .ok_or_else(|| words(&ImportError::NoSource))?;
    let keys = import.keys.as_deref().map(LlmKeys);
    let probe: &dyn KeyProbe = match &keys {
        Some(keys) => keys,
        None => &NoKeychain,
    };
    let source = Inkwell02::read(dir, probe).map_err(|e| words(&e))?;
    let written = import
        .library
        .import_inkwell02(&source)
        .map_err(|e| words(&e))?;
    Ok(event(
        "import.finished",
        &[("counts", Some(counts_json(&written))), ("ref", reference)],
    ))
}

/// Whether the library holds an import already (its marker).
fn imported_already(import: &Import02) -> Result<bool, String> {
    Ok(import
        .library
        .setting(MARKER_KEY)
        .map_err(|e| format!("the library could not be read: {e}"))?
        .is_some())
}

/// The importer's view of ink-llm's key store: existence only.
struct LlmKeys<'a>(&'a dyn KeyStore);

impl KeyProbe for LlmKeys<'_> {
    fn has_key(&self, account: &str) -> Result<bool, KeyProbeError> {
        self.0.has_key(account).map_err(|_| KeyProbeError)
    }
}

/// The counts as the schema's `ImportCounts`: each kind's label with `_` for spaces.
fn counts_json(counts: &Counts) -> Value {
    let mut out = Map::new();
    for (kind, n) in counts.by_kind() {
        out.insert(kind.replace(' ', "_"), n.into());
    }
    Value::Object(out)
}

/// Why an import cannot happen, in words a screen shows after "Couldn't import". The importer's
/// own messages name a file and its problem, never a path or content; the ones a user can act on
/// are said plainly here.
pub fn words(e: &ImportError) -> String {
    match e {
        // A write in progress, or one a crash left (which 0.2 rolls back when it next opens).
        ImportError::InUse { .. } => "Inkwell 0.2 is in the middle of saving its history. Quit \
                                      Inkwell 0.2 (if it is not open, open it and quit it once), \
                                      then try again"
            .into(),
        ImportError::NoSource => "there is no Inkwell 0.2 data on this computer".into(),
        ImportError::AlreadyImported | ImportError::MarkerPresent => {
            "Inkwell 0.2's data is in this library already".into()
        }
        other => format!("Inkwell 0.2's data: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(json: &str) -> Option<Result<Import02Query, String>> {
        let v: Value = serde_json::from_str(json).unwrap();
        parse(v["cmd"].as_str().unwrap(), &v)
    }

    #[test]
    fn the_commands_take_no_fields() {
        assert_eq!(
            p(r#"{"cmd":"import.check","id":"c"}"#),
            Some(Ok(Import02Query::Check))
        );
        assert_eq!(p(r#"{"cmd":"import.run"}"#), Some(Ok(Import02Query::Run)));
        assert!(p(r#"{"cmd":"import.notes"}"#).is_none(), "phrases' command");
        // The path is the core's: a shell cannot name one.
        let refused = p(r#"{"cmd":"import.run","dir":"/tmp/x"}"#).unwrap();
        assert_eq!(refused, Err("import.run: unknown field \"dir\"".into()));
        assert!(Import02Query::Run.imports() && !Import02Query::Check.imports());
    }

    /// Only the path is computed: nothing is created or opened. (An absolute home for this OS.)
    #[test]
    fn the_mac_directory_is_tauris_app_data_dir_for_0_2() {
        let home = std::env::temp_dir().join("someone");
        assert_eq!(
            mac_source_dir(Some(home.clone())),
            Some(home.join("Library/Application Support/com.inkwell.app"))
        );
        assert_eq!(mac_source_dir(None), None);
        assert_eq!(mac_source_dir(Some("relative".into())), None);
    }

    /// Only the path is computed and compared: nothing under it is opened.
    #[cfg(target_os = "macos")]
    #[test]
    fn on_a_mac_it_is_under_the_home_directory() {
        let dir = source_dir().expect("a home directory");
        assert!(dir.starts_with(std::env::home_dir().unwrap()));
        assert!(dir.ends_with("Library/Application Support/com.inkwell.app"));
    }

    /// Only the path is computed and compared: nothing under it is opened.
    #[cfg(windows)]
    #[test]
    fn on_windows_it_is_in_the_roaming_app_data_folder() {
        let dir = source_dir().expect("a roaming app data folder");
        assert_eq!(dir.file_name().unwrap(), IDENTIFIER_0_2);
        if let Some(appdata) = std::env::var_os("APPDATA") {
            assert_eq!(dir.parent().unwrap(), std::path::Path::new(&appdata));
        }
    }

    #[test]
    fn what_a_user_can_act_on_is_said_plainly() {
        let in_use = words(&ImportError::InUse {
            file: "transcripts.db",
        });
        assert!(in_use.contains("Quit Inkwell 0.2"), "{in_use}");
        assert!(words(&ImportError::AlreadyImported).contains("already"));
        // The rest keep the importer's own words: the file and the problem, never a path.
        let malformed = words(&ImportError::Malformed {
            file: "modes.json",
            problem: "\"modes\" is not a list".into(),
        });
        assert_eq!(
            malformed,
            "Inkwell 0.2's data: modes.json: \"modes\" is not a list"
        );
    }

    #[test]
    fn the_counts_are_the_schemas_fields() {
        let counts = Counts {
            dictations: 3,
            linked_keys: 1,
            ..Counts::default()
        };
        let v = counts_json(&counts);
        assert_eq!(v["dictations"], 3);
        assert_eq!(v["dictionary_entries"], 0);
        assert_eq!(v["app_style_rules"], 0);
        assert_eq!(v["linked_keys"], 1);
        assert_eq!(v.as_object().unwrap().len(), 8);
    }
}
