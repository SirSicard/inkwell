//! Importing an Inkwell 0.2 data directory (`~/Library/Application Support/com.inkwell.app/` on
//! a Mac) into the 1.0 store.
//!
//! An import is two calls. [`Inkwell02::read`] opens every source read-only, validates all of it
//! and holds the result in memory; its [`counts`](Inkwell02::counts) are the dry run, and it is
//! never handed a store, so it cannot write one. [`SqliteStore::import_inkwell02`] then writes
//! everything in **one transaction**: a failure anywhere leaves the store as it was.
//!
//! # What goes where
//!
//! | 0.2 source | 1.0 |
//! |---|---|
//! | `transcripts.db`, one row per dictation | a [`RecordKind::Dictation`] record with one mic segment: the text that was pasted, from 0 to the recording's length |
//! | `dictionary.json` | setting [`DICTIONARY_KEY`]: a JSON array of `{find, replace}` |
//! | `snippets.json` | setting [`SNIPPETS_KEY`]: a JSON array of `{id, trigger, expansion, category, enabled}` |
//! | `modes.json` | setting [`MODES_KEY`]: `{default_id, modes: [{id, name, style, model, polish_prompt, polish_enabled, apps, remove_fillers}]}` |
//! | `settings.json` | one setting per field under [`SETTINGS_PREFIX`], holding the field's JSON value |
//! | `voice-commands.json` | setting [`VOICE_COMMANDS_KEY`]: `{enabled, wake_prefix, commands: [{id, triggers, action: {type, ...}, enabled}]}` |
//! | `app-styles.json` | setting [`APP_STYLES_KEY`]: `{enabled, rules: [{process_name, style}]}` |
//! | API keys in the keychain | nothing copied: see below |
//!
//! Documents keep 0.2's shapes, with 0.2's serde defaults filled in, so code ported from 0.2 can
//! read them unchanged. They sit under an `import.inkwell-0.2` prefix rather than in 1.0's own keys:
//! 1.0's settings do not exist yet, a value written under a guessed name would be read with a
//! guessed meaning, and the prefix never collides with a setting the user already has. The screens
//! that own each setting adopt these values.
//!
//! **Not carried over**, and left untouched in the source: a transcript's raw (pre-cleanup) text,
//! its style and its model name, which the 1.0 schema has no place for;
//! [`SourceReport::raw_text_differs`] counts the rows whose raw text differed.
//!
//! # Decisions
//!
//! - **Read-only, and refused rather than risked.** `transcripts.db` is opened with
//!   `SQLITE_OPEN_READONLY` (what a URI's `mode=ro` sets) and `query_only`. 0.2 always used
//!   SQLite's default rollback journal. A `-journal` or `-wal` file with content beside it means
//!   a write in progress or a crash, and opening would have to roll it back or read it, so it is
//!   refused ([`ImportError::InUse`]). A database in WAL mode is refused too, before SQLite
//!   opens it. Observed with this crate's SQLite: a read-only open of a WAL-mode file creates
//!   `-wal` and `-shm` files beside it and leaves them there, and with a live writer it reads
//!   the log and writes into the `-shm`. `immutable=1` would avoid both, but it reads the main
//!   file alone and silently skips every committed transaction still in a live log, so it is
//!   not used. Nothing in the source directory is ever opened for writing (invariant I6).
//! - **Regular files only, and only so much of them.** Every source must be a regular file, not a
//!   symbolic link, directory, FIFO or device: it is inspected without following links before
//!   anything opens it, and on Unix the file opened must be the one inspected. SQLite opens the
//!   database with `SQLITE_OPEN_NOFOLLOW`. A JSON file is read through a cap of
//!   [`MAX_JSON_BYTES`], so a file that grows while it is read cannot exceed it either. A copy of
//!   the folder made with `cp -R` or `rsync -a` keeps regular files regular.
//! - **All or nothing.** A source that 0.2 itself could not have written (a file that does not
//!   parse, a field of the wrong type, an unreadable date) fails the whole import with an error
//!   naming the file and the problem. 0.2 fell back to defaults for a file it could not parse,
//!   so importing the rest of such a file would import settings the user never had.
//! - **One import per store.** The import writes a marker ([`MARKER_KEY`]) in its transaction
//!   and refuses to run when the marker is there ([`ImportError::AlreadyImported`]). A refused
//!   or failed import writes no marker.
//! - **Times.** 0.2 stamped a transcript with `datetime('now', 'localtime')` when it was saved,
//!   just after the key was released. SQLite's `'utc'` modifier inverts that with the same time
//!   zone rules, per date: the record ends at that instant and starts the recording's length
//!   before it. The conversion uses the importing machine's time zone, so rows dictated in
//!   another zone are off by the difference, and a row saved in the hour a daylight-saving
//!   change repeats can come out an hour off.
//! - **API keys stay where they are.** 0.2 kept each key as a keychain item with service
//!   [`KEYRING_SERVICE`] and the provider's id as the account, and ink-llm looks keys up under
//!   exactly that pair, so an existing item is already re-linked: nothing is read, copied or
//!   written. The import asks a [`KeyProbe`] which of the [`PROVIDERS`] have an item (existence
//!   only; ink-llm's check reads attributes, never the secret) and records their ids in the
//!   marker for the record. ink-llm never reads that list back: it asks the keychain each time,
//!   as 0.2 did after a cached "which providers have keys" went stale.
//! - **Plaintext keys are skipped.** Builds before 0.2's secret stripper wrote keys into
//!   `settings.json`. Only 0.2's own settings fields are imported; the secret fields are named
//!   in [`SourceReport::secret_fields_skipped`] (never their values). The importer wipes the
//!   file's bytes and the secret strings once parsed; the JSON parser's own scratch space is
//!   beyond its reach.
//!
//! Errors name the file and what is wrong with it, never a path (it can name the user) or
//! anything the user said or typed (I5).

use std::fmt;
use std::io::{ErrorKind, Read};
use std::path::Path;

use ink_core::store::{NewRecord, RecordId, Segment};
use ink_core::{Channel, RecordKind, StoreError};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde_json::{Map, Value, json};
use zeroize::Zeroize;

use crate::{
    SqliteStore, get_setting, insert_record, insert_segments, new_id, put_setting, segment_rows,
    set_ended,
};

/// The keychain service 0.2 stored every API key under (`llm.rs`), with the provider's id as
/// the account. ink-llm uses the same pair.
pub const KEYRING_SERVICE: &str = "inkwell";

/// The providers 0.2 could hold a key for, in its preference order: each is a keychain account.
pub const PROVIDERS: [&str; 5] = ["openai", "groq", "anthropic", "openrouter", "custom"];

/// The setting that marks a store as holding an import: a JSON object with the counts, the
/// linked providers and when it ran. Its presence refuses a second import.
pub const MARKER_KEY: &str = "import.inkwell-0.2";

/// The imported dictionary: a JSON array of `{find, replace}`.
pub const DICTIONARY_KEY: &str = "import.inkwell-0.2.dictionary";

/// The imported snippets: a JSON array of `{id, trigger, expansion, category, enabled}`.
pub const SNIPPETS_KEY: &str = "import.inkwell-0.2.snippets";

/// The imported modes: `{default_id, modes: [...]}`.
pub const MODES_KEY: &str = "import.inkwell-0.2.modes";

/// The prefix of the imported `settings.json` fields: one setting per field (for example
/// `import.inkwell-0.2.settings.hotkey`), holding the field's value as JSON text.
pub const SETTINGS_PREFIX: &str = "import.inkwell-0.2.settings.";

/// The imported voice commands: `{enabled, wake_prefix, commands: [...]}`.
pub const VOICE_COMMANDS_KEY: &str = "import.inkwell-0.2.voice-commands";

/// The imported per-app style rules (0.2 folded them into modes on first launch of a build with
/// modes, but kept the file): `{enabled, rules: [{process_name, style}]}`.
pub const APP_STYLES_KEY: &str = "import.inkwell-0.2.app-styles";

/// The largest JSON file the import reads: 8 MiB. 0.2's files are small documents: a settings
/// file of about 1 KB, modes with a few polish prompts, and a dictionary or snippet list that
/// stays in the hundreds of KB even with thousands of entries. The cap is more than ten times
/// any of that, and it bounds the memory a parse can take (a parsed document holds a few times
/// its bytes).
pub const MAX_JSON_BYTES: u64 = 8 * 1024 * 1024;

const TRANSCRIPTS: &str = "transcripts.db";
const JOURNAL: &str = "transcripts.db-journal";
const WAL: &str = "transcripts.db-wal";
const DICTIONARY: &str = "dictionary.json";
const SNIPPETS: &str = "snippets.json";
const MODES: &str = "modes.json";
const SETTINGS: &str = "settings.json";
const VOICE_COMMANDS: &str = "voice-commands.json";
const APP_STYLES: &str = "app-styles.json";

/// Every file the import reads.
const SOURCES: [&str; 7] = [
    TRANSCRIPTS,
    DICTIONARY,
    SNIPPETS,
    MODES,
    SETTINGS,
    VOICE_COMMANDS,
    APP_STYLES,
];

/// Fields 0.2 stripped from `settings.json` as plaintext secrets (`settings.rs`), sorted.
const SECRET_FIELDS: [&str; 6] = [
    "agent_token",
    "anthropic_key",
    "api_key",
    "groq_key",
    "openai_key",
    "openrouter_key",
];

/// A settings field's JSON type, as 0.2's serde read it.
#[derive(Clone, Copy)]
enum FieldType {
    Text,
    Flag,
    /// An `f32`: any JSON number.
    Number,
    /// A `u64`: a non-negative whole number.
    Count,
}

/// 0.2's settings fields (`settings.rs`, `struct Settings`), in its order.
const SETTINGS_FIELDS: [(&str, FieldType); 20] = [
    ("style", FieldType::Text),
    ("model", FieldType::Text),
    ("hotkey", FieldType::Text),
    ("recording_mode", FieldType::Text),
    ("start_on_boot", FieldType::Flag),
    ("show_overlay", FieldType::Flag),
    ("theme", FieldType::Text),
    ("overlay_position", FieldType::Text),
    ("advanced_mode", FieldType::Flag),
    ("mic_device", FieldType::Text),
    ("vad_threshold", FieldType::Number),
    ("polish_enabled", FieldType::Flag),
    ("polish_prompt", FieldType::Text),
    ("sound_dictation", FieldType::Flag),
    ("remove_fillers", FieldType::Flag),
    ("debug_save_audio", FieldType::Flag),
    ("edit_hotkey", FieldType::Text),
    ("mic_idle_release_mins", FieldType::Count),
    ("append_space", FieldType::Flag),
    ("show_partials", FieldType::Flag),
];

/// Asks the keychain whether an item exists under [`KEYRING_SERVICE`] and `account`.
///
/// Implementations answer from the item's attributes and **never read its secret**: on macOS a
/// read can raise a system prompt, and the import has no use for the value. The app wires this to
/// ink-llm's `KeyStore::has_key`, which makes exactly that query. The trait has no way to return
/// a secret.
pub trait KeyProbe {
    /// Whether an item exists. An error means the keychain could not answer (locked, denied).
    fn has_key(&self, account: &str) -> Result<bool, KeyProbeError>;
}

/// The keychain could not answer. Carries nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyProbeError;

/// A [`KeyProbe`] for when no keychain is to be asked: every provider is left unchecked.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoKeychain;

impl KeyProbe for NoKeychain {
    fn has_key(&self, _account: &str) -> Result<bool, KeyProbeError> {
        Err(KeyProbeError)
    }
}

/// How many items of each kind: in the source ([`Inkwell02::counts`]) or written
/// ([`SqliteStore::import_inkwell02`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Dictation records, one per `transcripts.db` row.
    pub dictations: usize,
    /// Dictionary entries.
    pub dictionary_entries: usize,
    /// Snippets.
    pub snippets: usize,
    /// Modes.
    pub modes: usize,
    /// `settings.json` fields.
    pub settings: usize,
    /// Voice commands.
    pub voice_commands: usize,
    /// Per-app style rules.
    pub app_style_rules: usize,
    /// Providers with a key in the keychain, linked by reference.
    pub linked_keys: usize,
}

impl Counts {
    /// Every kind with its label, in a fixed order, for printing and comparing.
    pub fn by_kind(&self) -> [(&'static str, usize); 8] {
        [
            ("dictations", self.dictations),
            ("dictionary entries", self.dictionary_entries),
            ("snippets", self.snippets),
            ("modes", self.modes),
            ("settings", self.settings),
            ("voice commands", self.voice_commands),
            ("app style rules", self.app_style_rules),
            ("linked keys", self.linked_keys),
        ]
    }
}

impl fmt::Display for Counts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, (kind, n)) in self.by_kind().into_iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{kind} {n}")?;
        }
        Ok(())
    }
}

/// What the read found beyond the counts: what is left behind, and why.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceReport {
    /// Transcripts whose raw text differs from the text that was pasted. The raw text stays in
    /// the source only.
    pub raw_text_differs: usize,
    /// Plaintext secret fields found in `settings.json` and not imported, by name (never value).
    pub secret_fields_skipped: Vec<&'static str>,
    /// Other `settings.json` fields that are not 0.2 settings, skipped.
    pub unknown_fields_skipped: usize,
    /// Providers the keychain could not answer for.
    pub keys_unchecked: usize,
}

/// Why an import did not happen. Messages name the file and the problem, never a path or content.
#[derive(Debug)]
pub enum ImportError {
    /// The directory does not exist, or holds none of the files 0.2 wrote.
    NoSource,
    /// A source is not a regular file: a symbolic link, directory, FIFO or device.
    NotAFile {
        /// The file.
        file: &'static str,
    },
    /// A source file exists but could not be read.
    Unreadable {
        /// The file.
        file: &'static str,
        /// The I/O error's kind.
        kind: ErrorKind,
    },
    /// A source file is not something 0.2 wrote.
    Malformed {
        /// The file.
        file: &'static str,
        /// What is wrong, without quoting the file.
        problem: String,
    },
    /// The database has an unfinished write beside it: a `-journal` or `-wal` file with content.
    InUse {
        /// The file.
        file: &'static str,
    },
    /// The store already holds an Inkwell 0.2 import.
    AlreadyImported,
    /// The store failed; the import's transaction was rolled back.
    Store(StoreError),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSource => write!(
                f,
                "no Inkwell 0.2 data: the directory has none of {}",
                SOURCES.join(", ")
            ),
            Self::NotAFile { file } => write!(
                f,
                "{file}: is not a regular file (a symbolic link, directory or device); import a \
                 plain copy of the folder"
            ),
            Self::Unreadable { file, kind } => write!(f, "{file}: could not be read ({kind:?})"),
            Self::Malformed { file, problem } => write!(f, "{file}: {problem}"),
            Self::InUse { file } => write!(
                f,
                "{file}: an unfinished write sits beside it (a -journal or -wal file). Quit \
                 Inkwell 0.2 so it finishes, then import again (copy the folder only while the \
                 app is quit)"
            ),
            Self::AlreadyImported => {
                f.write_str("this store already holds an Inkwell 0.2 import; a second is refused")
            }
            Self::Store(e) => write!(f, "the store refused the import: {e}"),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<StoreError> for ImportError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

fn unreadable(file: &'static str, e: &std::io::Error) -> ImportError {
    ImportError::Unreadable {
        file,
        kind: e.kind(),
    }
}

fn malformed(file: &'static str, problem: impl Into<String>) -> ImportError {
    ImportError::Malformed {
        file,
        problem: problem.into(),
    }
}

/// One `transcripts.db` row, converted.
struct Dictation {
    started_at_unix_ms: i64,
    ended_at_unix_ms: i64,
    duration_ms: u64,
    text: String,
}

/// A validated JSON document for one setting, with the number of items it holds.
struct Document {
    json: String,
    items: usize,
}

/// Everything read from an Inkwell 0.2 data directory, validated and ready to write.
///
/// Its `Debug` shows the counts only: it holds the user's dictations.
pub struct Inkwell02 {
    dictations: Vec<Dictation>,
    dictionary: Option<Document>,
    snippets: Option<Document>,
    modes: Option<Document>,
    voice_commands: Option<Document>,
    app_styles: Option<Document>,
    /// `(field, JSON value)`, in 0.2's field order.
    settings: Vec<(&'static str, String)>,
    linked_keys: Vec<&'static str>,
    report: SourceReport,
}

impl fmt::Debug for Inkwell02 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Inkwell02")
            .field("counts", &self.counts())
            .field("report", &self.report)
            .finish()
    }
}

impl Inkwell02 {
    /// Reads and validates the 0.2 data directory `dir`, read-only, then asks `keys` which
    /// providers have a keychain item. Writes nothing anywhere.
    ///
    /// **Worker**: reads files and may wait on the keychain.
    pub fn read(dir: &Path, keys: &dyn KeyProbe) -> Result<Self, ImportError> {
        if !dir.is_dir() {
            return Err(ImportError::NoSource);
        }
        let dir = &resolve(dir).map_err(|e| unreadable("the source directory", &e))?;
        // Without following links: a dangling link is present, and refused below.
        let any = SOURCES
            .iter()
            .any(|file| std::fs::symlink_metadata(dir.join(file)).is_ok());
        if !any {
            return Err(ImportError::NoSource);
        }
        let mut report = SourceReport::default();

        let dictations = match read_transcripts(dir)? {
            Some((rows, raw_differs)) => {
                report.raw_text_differs = raw_differs;
                rows
            }
            None => Vec::new(),
        };
        let dictionary = read_json(dir, DICTIONARY)?
            .map(|v| dictionary_document(&v))
            .transpose()?;
        let snippets = read_json(dir, SNIPPETS)?
            .map(|v| snippets_document(&v))
            .transpose()?;
        let modes = read_json(dir, MODES)?
            .map(|v| modes_document(&v))
            .transpose()?;
        let voice_commands = read_json(dir, VOICE_COMMANDS)?
            .map(|v| voice_commands_document(&v))
            .transpose()?;
        let app_styles = read_json(dir, APP_STYLES)?
            .map(|v| app_styles_document(&v))
            .transpose()?;
        let settings = read_settings(dir, &mut report)?;

        // Asked last, so a source that fails validation never reaches the keychain.
        let mut linked_keys = Vec::new();
        for provider in PROVIDERS {
            match keys.has_key(provider) {
                Ok(true) => linked_keys.push(provider),
                Ok(false) => {}
                Err(KeyProbeError) => report.keys_unchecked += 1,
            }
        }

        Ok(Self {
            dictations,
            dictionary,
            snippets,
            modes,
            voice_commands,
            app_styles,
            settings,
            linked_keys,
            report,
        })
    }

    /// How many items of each kind the source holds: the dry run.
    pub fn counts(&self) -> Counts {
        let items = |doc: &Option<Document>| doc.as_ref().map_or(0, |d| d.items);
        Counts {
            dictations: self.dictations.len(),
            dictionary_entries: items(&self.dictionary),
            snippets: items(&self.snippets),
            modes: items(&self.modes),
            settings: self.settings.len(),
            voice_commands: items(&self.voice_commands),
            app_style_rules: items(&self.app_styles),
            linked_keys: self.linked_keys.len(),
        }
    }

    /// What was found and is left behind.
    pub fn report(&self) -> &SourceReport {
        &self.report
    }
}

impl SqliteStore {
    /// Writes an Inkwell 0.2 import in one transaction and returns what it wrote.
    ///
    /// Refused with [`ImportError::AlreadyImported`] when the store already holds one; any
    /// failure rolls every row back. Records and segments go through the same inserts and
    /// checks as [`Store::create_record`](ink_core::Store::create_record) and
    /// [`Store::append_segments`](ink_core::Store::append_segments), with new UUID ids.
    ///
    /// **Worker**: one write transaction, which holds the store for the length of the import.
    pub fn import_inkwell02(&self, source: &Inkwell02) -> Result<Counts, ImportError> {
        // Everything that can fail without the database happens before the lock.
        let segments: Vec<[Segment; 1]> = source
            .dictations
            .iter()
            .map(|d| {
                [Segment {
                    channel: Channel::Mic,
                    start_ms: 0,
                    end_ms: d.duration_ms,
                    text: d.text.clone(),
                    speaker: None,
                }]
            })
            .collect();
        let mut records = Vec::with_capacity(segments.len());
        for (dictation, segment) in source.dictations.iter().zip(&segments) {
            records.push((new_id()?, dictation, segment_rows(segment)?));
        }
        let mut documents: Vec<(String, &str)> = Vec::new();
        for (key, doc) in [
            (DICTIONARY_KEY, &source.dictionary),
            (SNIPPETS_KEY, &source.snippets),
            (MODES_KEY, &source.modes),
            (VOICE_COMMANDS_KEY, &source.voice_commands),
            (APP_STYLES_KEY, &source.app_styles),
        ] {
            if let Some(doc) = doc {
                documents.push((key.to_string(), &doc.json));
            }
        }
        for (field, value) in &source.settings {
            documents.push((format!("{SETTINGS_PREFIX}{field}"), value));
        }
        let counts = source.counts();
        let marker = marker(source, counts);

        let outcome = self.write("import", |tx| {
            if get_setting(tx, MARKER_KEY)?.is_some() {
                // Nothing written yet: the empty transaction commits harmlessly.
                return Ok(Err(ImportError::AlreadyImported));
            }
            for (id, dictation, rows) in &records {
                insert_record(
                    tx,
                    id,
                    &NewRecord {
                        kind: RecordKind::Dictation,
                        title: None,
                        started_at_unix_ms: dictation.started_at_unix_ms,
                        source_app: None,
                        audio_dir: None,
                    },
                )?;
                insert_segments(tx, &RecordId(id.clone()), 1, rows)?;
                set_ended(tx, id, dictation.ended_at_unix_ms)?;
            }
            for (key, value) in &documents {
                put_setting(tx, key, value)?;
            }
            put_setting(tx, MARKER_KEY, &marker)?;
            Ok(Ok(()))
        })?;
        outcome?;
        Ok(counts)
    }
}

/// The marker's JSON: the counts, the linked providers and when the import ran.
fn marker(source: &Inkwell02, counts: Counts) -> String {
    // Informational only: a clock before 1970 records 0 rather than failing the import.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    let mut value = Map::new();
    value.insert("source".into(), json!("inkwell-0.2"));
    value.insert("imported_at_unix_ms".into(), json!(now_ms));
    for (kind, n) in counts.by_kind() {
        value.insert(kind.replace(' ', "_"), json!(n));
    }
    value.insert("keyring_service".into(), json!(KEYRING_SERVICE));
    value.insert("linked_keys".into(), json!(source.linked_keys));
    value.insert("unchecked_keys".into(), json!(source.report.keys_unchecked));
    Value::Object(value).to_string()
}

// ---------------------------------------------------------------------------------------------
// transcripts.db
// ---------------------------------------------------------------------------------------------

/// The 100-byte SQLite header's magic string.
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// Reads every transcript, oldest first, and counts the rows whose raw text differs. `None`
/// when there is no database.
fn read_transcripts(dir: &Path) -> Result<Option<(Vec<Dictation>, usize)>, ImportError> {
    read_transcripts_with(dir, &mut || {})
}

/// [`read_transcripts`], with `before_open` run between the checks and SQLite's open: the seam a
/// test uses to start a write in that window.
fn read_transcripts_with(
    dir: &Path,
    before_open: &mut dyn FnMut(),
) -> Result<Option<(Vec<Dictation>, usize)>, ImportError> {
    let file = TRANSCRIPTS;
    let Some(opened) = open_regular(dir, file)? else {
        return Ok(None);
    };
    let header = read_header(opened).map_err(|e| unreadable(file, &e))?;
    for side in [JOURNAL, WAL] {
        match std::fs::symlink_metadata(dir.join(side)) {
            Ok(meta) if !meta.file_type().is_file() => {
                return Err(ImportError::NotAFile { file: side });
            }
            Ok(meta) if meta.len() > 0 => return Err(ImportError::InUse { file }),
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(unreadable(side, &e)),
        }
    }
    if header.len() < 100 || header[..16] != SQLITE_MAGIC[..] {
        return Err(malformed(file, "is not an SQLite database"));
    }
    // Bytes 18 and 19: the write and read format versions, 1 for a rollback journal, 2 for WAL.
    match (header[18], header[19]) {
        (1, 1) => {}
        // Refused before SQLite sees it: see the module docs.
        (2, _) | (_, 2) => {
            return Err(malformed(
                file,
                "is in WAL mode, which Inkwell 0.2 never used; a read-only import cannot read it \
                 without writing beside it. Import a copy converted with \
                 `sqlite3 <copy> 'PRAGMA journal_mode=DELETE'`",
            ));
        }
        _ => {
            return Err(malformed(
                file,
                "has an SQLite file format this import does not know",
            ));
        }
    }

    before_open();
    let read = Connection::open_with_flags(
        dir.join(file),
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(RowsError::Sql)
    .and_then(|mut conn| read_rows(&mut conn));
    match read {
        Ok(rows) => Ok(Some(rows)),
        // A write that began after the check above leaves its journal or log beside the file,
        // and is the likelier cause of the failure than damage.
        Err(_)
            if [JOURNAL, WAL]
                .iter()
                .any(|side| std::fs::symlink_metadata(dir.join(side)).is_ok()) =>
        {
            Err(ImportError::InUse { file })
        }
        Err(RowsError::Sql(e)) => Err(sql_problem(&e)),
        Err(RowsError::Import(e)) => Err(e),
    }
}

/// The source directory with every link in its path resolved, on Unix: SQLite's
/// `SQLITE_OPEN_NOFOLLOW` refuses a link anywhere in the path (macOS's `/var` and `/tmp` are
/// links), and the directory is the user's choice. The flag is then left guarding the one name
/// the import checked itself. Windows keeps the path as given: SQLite's Windows VFS never
/// reports a link, so the flag does nothing there.
#[cfg(unix)]
fn resolve(dir: &Path) -> std::io::Result<std::path::PathBuf> {
    std::fs::canonicalize(dir)
}

#[cfg(not(unix))]
fn resolve(dir: &Path) -> std::io::Result<std::path::PathBuf> {
    Ok(dir.to_path_buf())
}

/// Opens a source for reading if it is a regular file; `None` when there is nothing there.
///
/// The entry is inspected without following links before anything opens it (opening a FIFO
/// would block), and on Unix the file opened must be the one inspected (same device and inode),
/// so a swap in between is refused too.
fn open_regular(dir: &Path, file: &'static str) -> Result<Option<std::fs::File>, ImportError> {
    let path = dir.join(file);
    let seen = match std::fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(unreadable(file, &e)),
    };
    if !seen.file_type().is_file() {
        return Err(ImportError::NotAFile { file });
    }
    let opened = std::fs::File::open(&path).map_err(|e| unreadable(file, &e))?;
    let meta = opened.metadata().map_err(|e| unreadable(file, &e))?;
    if !meta.is_file() || !same_file(&seen, &meta) {
        return Err(ImportError::NotAFile { file });
    }
    Ok(Some(opened))
}

#[cfg(unix)]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}

/// Windows has no stable file id in `std`; the link check before opening stands alone there.
#[cfg(not(unix))]
fn same_file(_: &std::fs::Metadata, _: &std::fs::Metadata) -> bool {
    true
}

/// The first 100 bytes of the file, or fewer if it is shorter.
fn read_header(file: std::fs::File) -> std::io::Result<Vec<u8>> {
    let mut header = Vec::with_capacity(100);
    file.take(100).read_to_end(&mut header)?;
    Ok(header)
}

enum RowsError {
    Sql(rusqlite::Error),
    Import(ImportError),
}

impl From<rusqlite::Error> for RowsError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sql(e)
    }
}

impl From<ImportError> for RowsError {
    fn from(e: ImportError) -> Self {
        Self::Import(e)
    }
}

/// The columns the import reads, as 0.2's DDL names them.
const COLUMNS: [&str; 5] = ["id", "text", "raw_text", "audio_duration_ms", "created_at"];

fn read_rows(conn: &mut Connection) -> Result<(Vec<Dictation>, usize), RowsError> {
    let file = TRANSCRIPTS;
    // Belt and braces: the connection is already read-only.
    conn.pragma_update(None, "query_only", true)?;
    // One read transaction, so the integrity check and the rows see the same snapshot. It only
    // reads; dropping it at the end rolls back nothing.
    let conn = conn.transaction()?;
    let check: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if check != "ok" {
        return Err(malformed(
            file,
            "fails SQLite's integrity check (damaged or truncated)",
        )
        .into());
    }
    let tables: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'transcripts'",
        [],
        |row| row.get(0),
    )?;
    if tables == 0 {
        return Err(malformed(file, "has no transcripts table").into());
    }
    let mut info = conn.prepare("SELECT name FROM pragma_table_info('transcripts')")?;
    let present: Vec<String> = info
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    for column in COLUMNS {
        if !present.iter().any(|p| p == column) {
            return Err(malformed(
                file,
                format!("the transcripts table has no `{column}` column"),
            )
            .into());
        }
    }

    // `unixepoch(created_at, 'utc')` reads the text as local time, the inverse of the
    // `datetime('now', 'localtime')` that wrote it, with this machine's zone rules for that date.
    let mut select = conn.prepare(
        "SELECT id, text, audio_duration_ms, created_at, unixepoch(created_at, 'utc'),
                raw_text IS NOT text
         FROM transcripts ORDER BY id",
    )?;
    let mut rows = select.query([])?;
    let mut dictations = Vec::new();
    let mut raw_differs = 0;
    while let Some(row) = rows.next()? {
        let ValueRef::Integer(id) = row.get_ref(0)? else {
            return Err(malformed(file, "a row id is not a whole number").into());
        };
        let bad = |what: &str| malformed(file, format!("row {id}: {what}"));
        let text = match row.get_ref(1)? {
            ValueRef::Text(bytes) => std::str::from_utf8(bytes)
                .map_err(|_| bad("`text` is not valid UTF-8"))?
                .to_string(),
            _ => return Err(bad("`text` is not text").into()),
        };
        let duration = match row.get_ref(2)? {
            ValueRef::Integer(ms) if ms >= 0 => ms,
            ValueRef::Integer(_) => return Err(bad("`audio_duration_ms` is negative").into()),
            _ => return Err(bad("`audio_duration_ms` is not a whole number").into()),
        };
        let stamped = match row.get_ref(3)? {
            ValueRef::Text(bytes) if is_local_datetime(bytes) => row.get_ref(4)?,
            _ => ValueRef::Null,
        };
        let ValueRef::Integer(saved_s) = stamped else {
            return Err(bad("`created_at` is not a date and time in the form 0.2 wrote").into());
        };
        let ended_at_unix_ms = saved_s
            .checked_mul(1_000)
            .ok_or_else(|| bad("`created_at` is out of range"))?;
        // The start is the save time less the recording's length: either can put it out of range.
        let started_at_unix_ms = ended_at_unix_ms
            .checked_sub(duration)
            .ok_or_else(|| bad("`created_at` minus `audio_duration_ms` is out of range"))?;
        if row.get::<_, bool>(5)? {
            raw_differs += 1;
        }
        dictations.push(Dictation {
            started_at_unix_ms,
            ended_at_unix_ms,
            duration_ms: duration.unsigned_abs(),
            text,
        });
    }
    Ok((dictations, raw_differs))
}

/// `YYYY-MM-DD HH:MM:SS`, the shape of SQLite's `datetime()`, which is all 0.2 ever wrote.
/// Checked before SQLite parses it, because SQLite also reads a bare number as a Julian day.
fn is_local_datetime(bytes: &[u8]) -> bool {
    bytes.len() == 19
        && bytes.iter().enumerate().all(|(i, &b)| match i {
            4 | 7 => b == b'-',
            10 => b == b' ',
            13 | 16 => b == b':',
            _ => b.is_ascii_digit(),
        })
}

/// An SQLite error on `transcripts.db`, by result code only: SQLite's messages can quote the
/// file's content.
fn sql_problem(e: &rusqlite::Error) -> ImportError {
    use rusqlite::ErrorCode as C;
    let file = TRANSCRIPTS;
    if let rusqlite::Error::SqliteFailure(err, _) = e
        && err.extended_code == rusqlite::ffi::SQLITE_CANTOPEN_SYMLINK
    {
        return ImportError::NotAFile { file };
    }
    match e.sqlite_error_code() {
        Some(C::DatabaseBusy | C::DatabaseLocked) => ImportError::InUse { file },
        Some(C::NotADatabase) => malformed(file, "is not an SQLite database"),
        Some(C::DatabaseCorrupt) => malformed(file, "is damaged (SQLite reports corruption)"),
        Some(code) => malformed(file, format!("could not be read (SQLite {code:?})")),
        None => malformed(file, "could not be read (a value has an unexpected type)"),
    }
}

// ---------------------------------------------------------------------------------------------
// The JSON files
// ---------------------------------------------------------------------------------------------

/// Reads and parses a JSON file; `None` when it does not exist.
fn read_json(dir: &Path, file: &'static str) -> Result<Option<Value>, ImportError> {
    let Some(bytes) = read_file(dir, file)? else {
        return Ok(None);
    };
    parse_json(file, &bytes).map(Some)
}

/// A JSON source's bytes, through the [`MAX_JSON_BYTES`] cap; `None` when it does not exist.
fn read_file(dir: &Path, file: &'static str) -> Result<Option<Vec<u8>>, ImportError> {
    let Some(opened) = open_regular(dir, file)? else {
        return Ok(None);
    };
    // Sized from the file up front, so `settings.json`'s bytes stay in one allocation that the
    // caller can wipe; one byte over the cap is enough to know it is too big.
    let size = opened.metadata().map_or(0, |m| m.len()).min(MAX_JSON_BYTES);
    let mut bytes = Vec::with_capacity(usize::try_from(size + 1).unwrap_or(0));
    opened
        .take(MAX_JSON_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| unreadable(file, &e))?;
    if bytes.len() as u64 > MAX_JSON_BYTES {
        bytes.zeroize();
        return Err(malformed(
            file,
            "is larger than 8 MiB, far beyond any file Inkwell 0.2 wrote",
        ));
    }
    Ok(Some(bytes))
}

/// Parses JSON. serde_json's messages can quote a value, so only the error's category and
/// position are kept.
fn parse_json(file: &'static str, bytes: &[u8]) -> Result<Value, ImportError> {
    serde_json::from_slice(bytes).map_err(|e| {
        malformed(
            file,
            format!(
                "is not valid JSON ({:?} error at line {}, column {})",
                e.classify(),
                e.line(),
                e.column()
            ),
        )
    })
}

/// A JSON object, or a problem naming `what`.
fn object<'v>(
    file: &'static str,
    value: &'v Value,
    what: &str,
) -> Result<&'v Map<String, Value>, ImportError> {
    value
        .as_object()
        .ok_or_else(|| malformed(file, format!("{what} is not an object")))
}

/// A required array field.
fn array<'v>(
    file: &'static str,
    obj: &'v Map<String, Value>,
    field: &str,
) -> Result<&'v Vec<Value>, ImportError> {
    obj.get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| malformed(file, format!("`{field}` is missing or not a list")))
}

/// A string field, as serde read it: missing takes `default` (required when `None`); present
/// with another type, `null` included, is an error.
fn string_field(
    file: &'static str,
    obj: &Map<String, Value>,
    field: &str,
    what: &str,
    default: Option<&str>,
) -> Result<Value, ImportError> {
    match (obj.get(field), default) {
        (Some(Value::String(s)), _) => Ok(Value::String(s.clone())),
        (None, Some(default)) => Ok(Value::String(default.to_string())),
        (None, None) => Err(malformed(file, format!("{what}: `{field}` is missing"))),
        (Some(_), _) => Err(malformed(file, format!("{what}: `{field}` is not text"))),
    }
}

/// A boolean field: missing takes `default` (required when `None`).
fn bool_field(
    file: &'static str,
    obj: &Map<String, Value>,
    field: &str,
    what: &str,
    default: Option<bool>,
) -> Result<Value, ImportError> {
    match (obj.get(field), default) {
        (Some(Value::Bool(b)), _) => Ok(Value::Bool(*b)),
        (None, Some(default)) => Ok(Value::Bool(default)),
        (None, None) => Err(malformed(file, format!("{what}: `{field}` is missing"))),
        (Some(_), _) => Err(malformed(
            file,
            format!("{what}: `{field}` is not true or false"),
        )),
    }
}

/// A required list of text.
fn text_list(
    file: &'static str,
    obj: &Map<String, Value>,
    field: &str,
    what: &str,
) -> Result<Value, ImportError> {
    match obj.get(field) {
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => {
            Ok(Value::Array(items.clone()))
        }
        _ => Err(malformed(
            file,
            format!("{what}: `{field}` is missing or not a list of text"),
        )),
    }
}

/// `dictionary.json`: `{"entries": [{"find", "replace"}]}`.
fn dictionary_document(value: &Value) -> Result<Document, ImportError> {
    let file = DICTIONARY;
    let entries = array(file, object(file, value, "the file")?, "entries")?;
    let mut out = Vec::with_capacity(entries.len());
    for (i, entry) in entries.iter().enumerate() {
        let what = format!("entry {}", i + 1);
        let entry = object(file, entry, &what)?;
        out.push(json!({
            "find": string_field(file, entry, "find", &what, None)?,
            "replace": string_field(file, entry, "replace", &what, None)?,
        }));
    }
    Ok(Document {
        items: out.len(),
        json: Value::Array(out).to_string(),
    })
}

/// `snippets.json`: `{"snippets": [{"id", "trigger", "expansion", "category"?, "enabled"?}]}`.
fn snippets_document(value: &Value) -> Result<Document, ImportError> {
    let file = SNIPPETS;
    let snippets = array(file, object(file, value, "the file")?, "snippets")?;
    let mut out = Vec::with_capacity(snippets.len());
    for (i, snippet) in snippets.iter().enumerate() {
        let what = format!("snippet {}", i + 1);
        let s = object(file, snippet, &what)?;
        out.push(json!({
            "id": string_field(file, s, "id", &what, None)?,
            "trigger": string_field(file, s, "trigger", &what, None)?,
            "expansion": string_field(file, s, "expansion", &what, None)?,
            "category": string_field(file, s, "category", &what, Some(""))?,
            "enabled": bool_field(file, s, "enabled", &what, Some(true))?,
        }));
    }
    Ok(Document {
        items: out.len(),
        json: Value::Array(out).to_string(),
    })
}

/// `modes.json`: `{"default_id", "modes": [{"id", "name", "style", ...}]}`.
fn modes_document(value: &Value) -> Result<Document, ImportError> {
    let file = MODES;
    let top = object(file, value, "the file")?;
    let default_id = string_field(file, top, "default_id", "the file", None)?;
    let modes = array(file, top, "modes")?;
    let mut out = Vec::with_capacity(modes.len());
    for (i, mode) in modes.iter().enumerate() {
        let what = format!("mode {}", i + 1);
        let m = object(file, mode, &what)?;
        let apps = match m.get("apps") {
            None => Vec::new(),
            Some(Value::Array(apps)) if apps.iter().all(Value::is_string) => apps.clone(),
            Some(_) => {
                return Err(malformed(
                    file,
                    format!("{what}: `apps` is not a list of text"),
                ));
            }
        };
        out.push(json!({
            "id": string_field(file, m, "id", &what, None)?,
            "name": string_field(file, m, "name", &what, None)?,
            "style": string_field(file, m, "style", &what, None)?,
            "model": string_field(file, m, "model", &what, Some(""))?,
            "polish_prompt": string_field(file, m, "polish_prompt", &what, Some(""))?,
            "polish_enabled": bool_field(file, m, "polish_enabled", &what, Some(false))?,
            "apps": apps,
            "remove_fillers": bool_field(file, m, "remove_fillers", &what, Some(true))?,
        }));
    }
    Ok(Document {
        items: out.len(),
        json: json!({ "default_id": default_id, "modes": out }).to_string(),
    })
}

/// `voice-commands.json` (`VoiceCommandStore`): `{"enabled", "wake_prefix", "commands": [{"id",
/// "triggers", "action", "enabled"}]}`. 0.2 gave none of these a default.
fn voice_commands_document(value: &Value) -> Result<Document, ImportError> {
    let file = VOICE_COMMANDS;
    let top = object(file, value, "the file")?;
    let enabled = bool_field(file, top, "enabled", "the file", None)?;
    let wake_prefix = string_field(file, top, "wake_prefix", "the file", None)?;
    let commands = array(file, top, "commands")?;
    let mut out = Vec::with_capacity(commands.len());
    for (i, command) in commands.iter().enumerate() {
        let what = format!("command {}", i + 1);
        let c = object(file, command, &what)?;
        out.push(json!({
            "id": string_field(file, c, "id", &what, None)?,
            "triggers": text_list(file, c, "triggers", &what)?,
            "action": action(c.get("action"), &what)?,
            "enabled": bool_field(file, c, "enabled", &what, None)?,
        }));
    }
    Ok(Document {
        items: out.len(),
        json: json!({ "enabled": enabled, "wake_prefix": wake_prefix, "commands": out })
            .to_string(),
    })
}

/// A voice command's `action`: 0.2's `CommandAction`, tagged by `type` in snake case, with the
/// one text field its variant carries. The tag is never quoted back: it is the file's content.
fn action(value: Option<&Value>, what: &str) -> Result<Value, ImportError> {
    let file = VOICE_COMMANDS;
    let what = format!("{what}: `action`");
    let a = object(file, value.unwrap_or(&Value::Null), &what)?;
    let Some(Value::String(kind)) = a.get("type") else {
        return Err(malformed(file, format!("{what} has no `type`")));
    };
    let payload = match kind.as_str() {
        "undo" | "toggle_polish" | "toggle_dictation" => None,
        "change_style" => Some("style"),
        "switch_model" => Some("model"),
        "open_url" => Some("url"),
        "open_app" => Some("path"),
        "insert_text" => Some("text"),
        _ => {
            return Err(malformed(
                file,
                format!("{what} has a `type` Inkwell 0.2 does not know"),
            ));
        }
    };
    let mut out = Map::new();
    out.insert("type".into(), Value::String(kind.clone()));
    if let Some(field) = payload {
        out.insert(field.into(), string_field(file, a, field, &what, None)?);
    }
    Ok(Value::Object(out))
}

/// `app-styles.json` (`AppStyleRules`): `{"enabled", "rules": [{"process_name", "style"}]}`, every
/// field required.
fn app_styles_document(value: &Value) -> Result<Document, ImportError> {
    let file = APP_STYLES;
    let top = object(file, value, "the file")?;
    let enabled = bool_field(file, top, "enabled", "the file", None)?;
    let rules = array(file, top, "rules")?;
    let mut out = Vec::with_capacity(rules.len());
    for (i, rule) in rules.iter().enumerate() {
        let what = format!("rule {}", i + 1);
        let r = object(file, rule, &what)?;
        out.push(json!({
            "process_name": string_field(file, r, "process_name", &what, None)?,
            "style": string_field(file, r, "style", &what, None)?,
        }));
    }
    Ok(Document {
        items: out.len(),
        json: json!({ "enabled": enabled, "rules": out }).to_string(),
    })
}

/// `settings.json`: 0.2's own fields that are present, as `(field, JSON value)`. Secret and
/// unknown fields are counted in `report` and skipped; the importer's copies of secrets are wiped.
fn read_settings(
    dir: &Path,
    report: &mut SourceReport,
) -> Result<Vec<(&'static str, String)>, ImportError> {
    let file = SETTINGS;
    let Some(mut bytes) = read_file(dir, file)? else {
        return Ok(Vec::new());
    };
    let parsed = parse_json(file, &bytes);
    bytes.zeroize();
    let mut value = parsed?;
    let Some(obj) = value.as_object_mut() else {
        return Err(malformed(file, "the file is not an object"));
    };
    for field in SECRET_FIELDS {
        if let Some(mut secret) = obj.remove(field) {
            report.secret_fields_skipped.push(field);
            if let Value::String(s) = &mut secret {
                s.zeroize();
            }
        }
    }
    let mut settings = Vec::new();
    for (field, kind) in SETTINGS_FIELDS {
        let Some(v) = obj.remove(field) else {
            continue;
        };
        let fits = match kind {
            FieldType::Text => v.is_string(),
            FieldType::Flag => v.is_boolean(),
            FieldType::Number => v.is_number(),
            FieldType::Count => v.is_u64(),
        };
        if !fits {
            let expected = match kind {
                FieldType::Text => "text",
                FieldType::Flag => "true or false",
                FieldType::Number => "a number",
                FieldType::Count => "a whole number of at least 0",
            };
            return Err(malformed(
                file,
                format!(
                    "`{field}` is not {expected} (Inkwell 0.2 would have ignored the whole file)"
                ),
            ));
        }
        settings.push((field, v.to_string()));
    }
    // What is left is neither a secret nor a 0.2 setting (an abandoned cache, a hand edit).
    report.unknown_fields_skipped = obj.len();
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_datetimes_are_the_shape_sqlite_writes() {
        assert!(is_local_datetime(b"2026-01-15 13:00:00"));
        for bad in [
            &b"2026-01-15T13:00:00"[..],
            b"2026-01-15 13:00",
            b"2461055.5",
            b"2026-01-15 13:00:00.123",
            b"",
        ] {
            assert!(!is_local_datetime(bad), "{bad:?}");
        }
    }

    #[test]
    fn json_errors_keep_the_position_not_the_text() {
        let err = parse_json(SETTINGS, br#"{"theme": "canary" "#).unwrap_err();
        let message = err.to_string();
        assert!(
            message.starts_with("settings.json: is not valid JSON"),
            "{message}"
        );
        assert!(!message.contains("canary"), "{message}");
        // A type error from serde would quote the value; the importer's own checks do not.
        let Err(err) = modes_document(&json!({ "default_id": 5, "modes": [] })) else {
            panic!("a number is not a mode id");
        };
        assert!(!err.to_string().contains('5'), "{err}");
    }

    /// A directory holding a minimal 0.2 `transcripts.db` with one row, links resolved.
    fn legacy_db(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ink-store-import-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = resolve(&dir).unwrap();
        let conn = Connection::open(dir.join(TRANSCRIPTS)).unwrap();
        conn.execute_batch(
            "CREATE TABLE transcripts (id INTEGER PRIMARY KEY AUTOINCREMENT, text TEXT NOT NULL,
                 raw_text TEXT NOT NULL, audio_duration_ms INTEGER NOT NULL DEFAULT 0,
                 created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime')));
             INSERT INTO transcripts (text, raw_text) VALUES ('synthetic', 'synthetic');",
        )
        .unwrap();
        dir
    }

    #[test]
    fn a_journal_that_appears_during_the_read_is_reported_as_in_use() {
        let dir = legacy_db("late-journal");
        let journal = dir.join("transcripts.db-journal");

        // 0.2 starts a write after the side files were checked and before SQLite opens the file.
        let late = read_transcripts_with(&dir, &mut || std::fs::write(&journal, b"late").unwrap());
        let err = late.map(|_| ()).unwrap_err();
        assert!(
            matches!(err, ImportError::InUse { file: TRANSCRIPTS }),
            "{err:?}"
        );

        std::fs::remove_file(&journal).unwrap();
        let rows = read_transcripts_with(&dir, &mut || {}).map(|r| r.map(|(d, _)| d.len()));
        assert!(matches!(rows, Ok(Some(1))), "{rows:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The link check runs before SQLite opens the file; `SQLITE_OPEN_NOFOLLOW` covers a swap in
    /// between.
    #[cfg(unix)]
    #[test]
    fn a_database_swapped_for_a_link_before_sqlite_opens_it_is_refused() {
        let dir = legacy_db("late-link");
        let elsewhere = dir.join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        let swap = &mut || {
            std::fs::rename(dir.join(TRANSCRIPTS), elsewhere.join(TRANSCRIPTS)).unwrap();
            std::os::unix::fs::symlink(elsewhere.join(TRANSCRIPTS), dir.join(TRANSCRIPTS)).unwrap();
        };
        let err = read_transcripts_with(&dir, swap).map(|_| ()).unwrap_err();
        assert!(
            matches!(err, ImportError::NotAFile { file: TRANSCRIPTS }),
            "{err:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn same_file_tells_two_files_apart() {
        let dir = legacy_db("same-file");
        std::fs::write(dir.join(DICTIONARY), b"{}").unwrap();
        let db = std::fs::metadata(dir.join(TRANSCRIPTS)).unwrap();
        let json = std::fs::metadata(dir.join(DICTIONARY)).unwrap();
        let opened = std::fs::File::open(dir.join(TRANSCRIPTS)).unwrap();
        assert!(same_file(&db, &opened.metadata().unwrap()));
        if cfg!(unix) {
            assert!(!same_file(&db, &json));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn counts_print_every_kind_in_order() {
        let counts = Counts {
            dictations: 1,
            dictionary_entries: 2,
            snippets: 3,
            modes: 4,
            settings: 5,
            voice_commands: 6,
            app_style_rules: 7,
            linked_keys: 8,
        };
        assert_eq!(
            counts.to_string(),
            "dictations 1, dictionary entries 2, snippets 3, modes 4, settings 5, \
             voice commands 6, app style rules 7, linked keys 8"
        );
    }
}
