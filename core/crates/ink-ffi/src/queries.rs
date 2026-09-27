//! The screens' commands: permissions, what is owed, a live meeting's notes, settings, modes, the
//! model catalogue, and the library's records ([`library`](crate::library)). They run on their own thread, `ink-queries`, in the order they were sent.
//!
//! Apart from the command thread on purpose: a model update holds that thread for as long as its
//! download takes, and a note typed during it, or a permission card the user is looking at, must
//! not wait minutes for it. Each of these is quick (a store call, or the system-audio probe's
//! second or so). They may overtake commands queued earlier on the command thread; nothing here
//! depends on one of those.
//!
//! Errors name what failed, never what was said: a note's or a commitment's text reaches the shell
//! only in the event that answers the command that asked for it (I5).

use std::collections::BTreeMap;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};

use ink_core::{
    Commitment, CommitmentId, NoteId, Permission, PermissionProbe, PermissionState, PlatformError,
    RecordId, Store,
};
use ink_engines::{ModelDir, Os};
use ink_pipeline::modes::Mode;
use ink_pipeline::style::Style;
use serde_json::{Map, Value, json};

use crate::events::{self, event};
use crate::runtime::Shared;

/// The store setting that remembers the app has asked for System Audio. Until it is set, a check
/// never runs the tone probe, because the probe would make macOS show its prompt.
pub const SYSTEM_AUDIO_ASKED_KEY: &str = "permissions.system_audio_asked";

/// The store setting holding the user's modes, as a JSON document:
/// `{"default_id", "modes": [{"id", "name", "style", "polish_enabled", "remove_fillers", "apps"}]}`
/// (the shape the 0.2 import writes). Until it is set, `modes.list` reads the imported modes, and
/// without those the built-in default. The dictation chain reads the same key (S2.7).
pub const MODES_KEY: &str = "dictation.modes";

/// The settings a shell may read and write through `setting.get` and `setting.set`, with the values
/// each accepts. Everything else in the store is the core's.
pub const SHELL_SETTINGS: &[(&str, &[&str])] = &[
    // The first-run state has been completed (or skipped).
    ("onboarding.done", &["true", "false"]),
    // The user's wish for dictation polish. Whether polish runs also needs a working language
    // model; the shell shows the two apart.
    ("dictation.polish", &["on", "off"]),
    // Whether the app watches for calls and offers to record them (the consent Drop). On unless
    // turned off; the meetings thread starts or stops detection when it changes.
    (crate::control::DETECT_KEY, &["on", "off"]),
    // Record the Bluetooth headset's own mic instead of the built-in one (call-quality audio).
    (crate::control::HEADSET_MIC_KEY, &["on", "off"]),
    // Local-only mode (architecture rule 6): on unless turned off; while on, a language model
    // that is not on this machine is never called (crate::llms::PolishModel).
    (crate::llms::LOCAL_ONLY_KEY, &["on", "off"]),
    // How long the library keeps records (crate::retention): changing it sweeps at once.
    (
        crate::retention::RETENTION_KEY,
        crate::retention::RETENTION_VALUES,
    ),
];

/// The most commitments `commitments.list` returns when the command names no limit.
pub const DEFAULT_COMMITMENTS_LIMIT: usize = 200;

/// The most it returns at all.
pub const MAX_COMMITMENTS_LIMIT: usize = 1_000;

/// A screen's command, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Query {
    /// `permissions.check`: every permission's state now.
    PermissionsCheck,
    /// `permission.request`: the system prompt, or the settings pane.
    PermissionRequest(Permission),
    /// `commitments.list`: the open commitments.
    CommitmentsList {
        /// At most this many.
        limit: usize,
    },
    /// `commitment.set_done`.
    CommitmentSetDone {
        /// The commitment.
        id: String,
        /// Done, or open again.
        done: bool,
    },
    /// `commitment.not_yet`: its looks-done suggestion is dismissed; it stays open.
    CommitmentNotYet {
        /// The commitment.
        id: String,
    },
    /// `note.add`.
    NoteAdd {
        /// The record it belongs to.
        record: String,
        /// Where in the record it was written, ms.
        at_ms: u64,
        /// Its text.
        text: String,
    },
    /// `note.update`.
    NoteUpdate {
        /// The note.
        note: String,
        /// Its new text.
        text: String,
    },
    /// `note.delete`.
    NoteDelete {
        /// The note.
        note: String,
    },
    /// `models.list`: the catalogue's models for this OS.
    ModelsList,
    /// `setting.get`.
    SettingGet {
        /// One of [`SHELL_SETTINGS`].
        key: String,
    },
    /// `setting.set`.
    SettingSet {
        /// One of [`SHELL_SETTINGS`].
        key: String,
        /// One of the values it accepts.
        value: String,
    },
    /// `modes.list`: the user's modes.
    ModesList,
    /// The library's records, a search, one record, or counts ([`library`](crate::library)).
    Library(crate::library::LibraryQuery),
}

/// A query with the command's name and id, for its events.
struct Job {
    name: String,
    id: Option<String>,
    query: Query,
}

/// The fields each query takes besides `cmd` and `id`; `None` when `name` is not a query.
fn fields(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "permissions.check" | "models.list" | "modes.list" => &[],
        "permission.request" => &["permission"],
        "commitments.list" => &["limit"],
        "commitment.set_done" => &["commitment", "done"],
        "commitment.not_yet" => &["commitment"],
        "note.add" => &["record", "at_ms", "text"],
        "note.update" => &["note", "text"],
        "note.delete" => &["note"],
        "setting.get" => &["key"],
        "setting.set" => &["key", "value"],
        _ => return None,
    })
}

/// A command read as a query: its name, its `id`, and the query.
pub type Read = (String, Option<String>, Query);

/// Reads a command's JSON as a query: `Ok(None)` when it is some other command (or one the command
/// thread's parser refuses), else the query or why it cannot be read.
pub fn read(json: &str) -> Result<Option<Read>, String> {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return Ok(None);
    };
    let Some(name) = v.get("cmd").and_then(Value::as_str) else {
        return Ok(None);
    };
    let id = match v.get("id") {
        None => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err("command: \"id\" must be a string".into()),
    };
    match parse(name, &v) {
        None => Ok(None),
        Some(query) => Ok(Some((name.to_owned(), id, query?))),
    }
}

/// Reads `v` as the query named `name`: `None` when `name` is not one of these commands, else the
/// query or why it cannot be read. Unknown fields are refused, as for every other command.
pub fn parse(name: &str, v: &Value) -> Option<Result<Query, String>> {
    if let Some(query) = crate::library::parse(name, v) {
        return Some(query.map(Query::Library));
    }
    let allowed = fields(name)?;
    Some(parse_known(name, allowed, v))
}

fn parse_known(name: &str, allowed: &[&str], v: &Value) -> Result<Query, String> {
    let obj = v.as_object().ok_or("command: not an object")?;
    if let Some(k) = obj
        .keys()
        .find(|k| !["cmd", "id"].contains(&k.as_str()) && !allowed.contains(&k.as_str()))
    {
        return Err(format!("{name}: unknown field \"{k}\""));
    }
    let text = |k: &str| -> Result<String, String> {
        obj.get(k)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("{name}: needs a string \"{k}\""))
    };
    Ok(match name {
        "permissions.check" => Query::PermissionsCheck,
        "permission.request" => Query::PermissionRequest(
            parse_permission(&text("permission")?)
                .ok_or_else(|| format!("{name}: unknown permission"))?,
        ),
        "commitments.list" => Query::CommitmentsList {
            limit: match obj.get("limit") {
                None => DEFAULT_COMMITMENTS_LIMIT,
                Some(n) => n
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|n| (1..=MAX_COMMITMENTS_LIMIT).contains(n))
                    .ok_or_else(|| {
                        format!(
                            "{name}: \"limit\" is a whole number from 1 to {MAX_COMMITMENTS_LIMIT}"
                        )
                    })?,
            },
        },
        "commitment.set_done" => Query::CommitmentSetDone {
            id: text("commitment")?,
            done: obj
                .get("done")
                .and_then(Value::as_bool)
                .ok_or_else(|| format!("{name}: needs \"done\", true or false"))?,
        },
        "commitment.not_yet" => Query::CommitmentNotYet {
            id: text("commitment")?,
        },
        "note.add" => Query::NoteAdd {
            record: text("record")?,
            at_ms: obj
                .get("at_ms")
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("{name}: needs \"at_ms\", a whole number of ms"))?,
            text: text("text")?,
        },
        "note.update" => Query::NoteUpdate {
            note: text("note")?,
            text: text("text")?,
        },
        "note.delete" => Query::NoteDelete {
            note: text("note")?,
        },
        "models.list" => Query::ModelsList,
        "setting.get" => Query::SettingGet {
            key: shell_setting(name, &text("key")?)?,
        },
        "setting.set" => {
            let key = shell_setting(name, &text("key")?)?;
            let value = text("value")?;
            let accepted = SHELL_SETTINGS
                .iter()
                .find(|(k, _)| *k == key)
                .map_or(&[][..], |(_, values)| *values);
            if !accepted.contains(&value.as_str()) {
                return Err(format!(
                    "{name}: \"{key}\" takes one of: {}",
                    accepted.join(", ")
                ));
            }
            Query::SettingSet { key, value }
        }
        "modes.list" => Query::ModesList,
        _ => unreachable!("fields() lists every query"),
    })
}

/// `key` if the shell may use it.
fn shell_setting(name: &str, key: &str) -> Result<String, String> {
    if SHELL_SETTINGS.iter().any(|(k, _)| *k == key) {
        Ok(key.to_owned())
    } else {
        Err(format!(
            "{name}: \"{key}\" is not a setting the shell may use"
        ))
    }
}

/// A permission for its schema name.
pub fn parse_permission(name: &str) -> Option<Permission> {
    Some(match name {
        "microphone" => Permission::Microphone,
        "system_audio" => Permission::SystemAudio,
        "accessibility" => Permission::Accessibility,
        "input_monitoring" => Permission::InputMonitoring,
        _ => return None,
    })
}

/// A permission's schema name.
fn permission(p: Permission) -> &'static str {
    match p {
        Permission::Microphone => "microphone",
        Permission::SystemAudio => "system_audio",
        Permission::Accessibility => "accessibility",
        Permission::InputMonitoring => "input_monitoring",
        // Non-exhaustive: a permission this build cannot name is never parsed, so never asked.
        _ => "unknown",
    }
}

/// A permission state's schema name.
fn state(s: PermissionState) -> &'static str {
    match s {
        PermissionState::Granted => "granted",
        PermissionState::Denied => "denied",
        PermissionState::NotDetermined => "not_determined",
        PermissionState::Unknown => "unknown",
    }
}

/// The permission probe for platforms without one: every state is unknown, and nothing can be
/// asked for. Nothing is made up.
pub struct NoPermissionProbe;

impl PermissionProbe for NoPermissionProbe {
    fn check(&self, _: Permission) -> PermissionState {
        PermissionState::Unknown
    }

    fn request(&self, _: Permission) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported(
            "this platform has no permission probe yet",
        ))
    }
}

/// The thread that runs the queries.
pub struct QueryWorker {
    tx: Sender<Job>,
    thread: JoinHandle<()>,
}

impl QueryWorker {
    /// Starts `ink-queries`. It holds `shared` until [`stop`](Self::stop).
    pub fn start(
        shared: Arc<Shared>,
        probe: Arc<dyn PermissionProbe>,
        models: ModelDir,
    ) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Job>();
        let thread = thread::Builder::new()
            .name("ink-queries".into())
            .spawn(move || {
                let ctx = Ctx {
                    shared: &shared,
                    probe: probe.as_ref(),
                    models: &models,
                };
                while let Ok(job) = rx.recv() {
                    ctx.guarded(job);
                }
            })?;
        Ok(Self { tx, thread })
    }

    /// Queues a query.
    pub fn send(&self, name: String, id: Option<String>, query: Query) -> Result<(), String> {
        self.tx
            .send(Job { name, id, query })
            .map_err(|_| "the queries thread has stopped".to_owned())
    }

    /// Runs what is queued, then ends the thread. Its events are emitted before this returns.
    pub fn stop(self) {
        drop(self.tx);
        if self.thread.join().is_err() {
            log::error!("the queries thread panicked outside its per-query boundary");
        }
    }
}

struct Ctx<'a> {
    shared: &'a Shared,
    probe: &'a dyn PermissionProbe,
    models: &'a ModelDir,
}

impl Ctx<'_> {
    /// One query behind a panic boundary, as the command thread runs commands.
    fn guarded(&self, job: Job) {
        let (name, id) = (job.name.clone(), job.id.clone());
        if panic::catch_unwind(AssertUnwindSafe(|| self.run(job))).is_err() {
            // The payload is not logged: it could hold a note's words (I5).
            log::error!("command {name} panicked; the next one still runs");
            self.shared.events.emit(events::command_failed(
                &name,
                id.as_deref(),
                "a bug in the core stopped this command",
            ));
        }
    }

    fn run(&self, job: Job) {
        let Job { name, id, query } = job;
        let emit = |e: Value| self.shared.events.emit(e);
        let fail = |message: String| {
            log::warn!("command {name} failed: {message}");
            emit(events::command_failed(&name, id.as_deref(), &message));
        };
        let store = self.shared.store.as_ref();
        match query {
            Query::PermissionsCheck => emit(self.permissions()),
            Query::PermissionRequest(p) => match self.probe.request(p) {
                Ok(()) => {
                    if p == Permission::SystemAudio
                        && let Err(e) = store.set_setting(SYSTEM_AUDIO_ASKED_KEY, "true")
                    {
                        // The probe remembers for this run; the next launch would treat System
                        // Audio as never asked (and show it as such) until the next request.
                        log::error!("could not remember that system audio was asked for: {e}");
                    }
                    emit(event(
                        "permission.requested",
                        &[("permission", Some(permission(p).into()))],
                    ));
                }
                Err(e) => fail(e.to_string()),
            },
            Query::CommitmentsList { limit } => match commitments(store, limit) {
                Ok(e) => emit(e),
                Err(e) => fail(e),
            },
            Query::CommitmentSetDone {
                id: commitment,
                done,
            } => match store.set_commitment_done(&CommitmentId(commitment.clone()), done) {
                Ok(()) => emit(event(
                    "commitment.updated",
                    &[
                        ("commitment", Some(commitment.into())),
                        ("done", Some(done.into())),
                    ],
                )),
                Err(e) => fail(e.to_string()),
            },
            Query::CommitmentNotYet { id: commitment } => {
                match store.set_done_evidence(&CommitmentId(commitment.clone()), None) {
                    Ok(()) => emit(event(
                        "commitment.updated",
                        &[
                            ("commitment", Some(commitment.into())),
                            ("done", Some(false.into())),
                        ],
                    )),
                    Err(e) => fail(e.to_string()),
                }
            }
            Query::NoteAdd {
                record,
                at_ms,
                text,
            } => match store.add_note(&RecordId(record.clone()), at_ms, &text) {
                Ok(note) => emit(event(
                    "note.added",
                    &[
                        ("record", Some(record.into())),
                        ("note", Some(note.0.into())),
                        ("at_ms", Some(at_ms.into())),
                        ("ref", id.clone().map(Into::into)),
                    ],
                )),
                Err(e) => fail(e.to_string()),
            },
            Query::NoteUpdate { note, text } => {
                match store.update_note(&NoteId(note.clone()), &text) {
                    Ok(()) => emit(event(
                        "note.updated",
                        &[
                            ("note", Some(note.into())),
                            ("ref", id.clone().map(Into::into)),
                        ],
                    )),
                    Err(e) => fail(e.to_string()),
                }
            }
            Query::NoteDelete { note } => match store.delete_note(&NoteId(note.clone())) {
                Ok(()) => emit(event(
                    "note.deleted",
                    &[
                        ("note", Some(note.into())),
                        ("ref", id.clone().map(Into::into)),
                    ],
                )),
                Err(e) => fail(e.to_string()),
            },
            Query::ModelsList => emit(self.catalogue()),
            Query::SettingGet { key } => match store.setting(&key) {
                Ok(value) => emit(setting(&key, value)),
                Err(e) => fail(e.to_string()),
            },
            Query::SettingSet { key, value } => match store.set_setting(&key, &value) {
                Ok(()) => {
                    if key == crate::llms::LOCAL_ONLY_KEY {
                        self.shared.local_only.set(value != "off");
                    }
                    if key == crate::control::DETECT_KEY {
                        self.shared.tell_meetings(crate::control::Msg::Detect {
                            on: value == "on",
                            why_off: None,
                        });
                    }
                    let sweep = key == crate::retention::RETENTION_KEY;
                    emit(setting(&key, Some(value)));
                    if sweep {
                        self.shared.sweep_soon();
                    }
                }
                Err(e) => fail(e.to_string()),
            },
            Query::ModesList => match modes(store) {
                Ok(e) => emit(e),
                Err(e) => fail(e),
            },
            Query::Library(query) => {
                match crate::library::answer(self.shared, query, id.as_deref()) {
                    Ok(e) => emit(e),
                    Err(e) => fail(e),
                }
            }
        }
    }

    /// `permissions.checked`. Never prompts (the probe's contract); System Audio takes about a
    /// second once it has been asked for.
    fn permissions(&self) -> Value {
        let check = |p: Permission| Some(Value::from(state(self.probe.check(p))));
        event(
            "permissions.checked",
            &[
                ("microphone", check(Permission::Microphone)),
                ("system_audio", check(Permission::SystemAudio)),
                ("accessibility", check(Permission::Accessibility)),
                ("input_monitoring", check(Permission::InputMonitoring)),
            ],
        )
    }

    /// `models.listed`: every registry model this OS runs, with whether it is installed.
    fn catalogue(&self) -> Value {
        let os = Os::current();
        let models: Vec<Value> = self
            .shared
            .registry
            .rows()
            .iter()
            .filter(|row| os.is_some_and(|os| row.runs_on(os)))
            .map(|row| {
                json!({
                    "id": row.id,
                    "licence": row.licence,
                    "size_bytes": row.total_size(),
                    "installed": self.models.is_installed(row),
                    "jobs": row
                        .scores
                        .iter()
                        .map(|s| json!({"job": events::job(s.job), "wer": s.wer}))
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        event("models.listed", &[("models", Some(Value::Array(models)))])
    }
}

fn setting(key: &str, value: Option<String>) -> Value {
    event(
        "setting.value",
        &[("key", Some(key.into())), ("value", value.map(Into::into))],
    )
}

/// Puts `value` under `key` when there is one: optional fields are left out, never null.
fn put(map: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        map.insert(key.into(), value);
    }
}

/// `commitments.listed`: the open commitments (not done, not merged into another), soonest due
/// first, each with its record's title and start and how many others were merged into it.
///
/// Merges are counted within the records that hold an open commitment. A commitment merged into
/// one from another record that holds no open commitment of its own is not counted: the store
/// has no reverse lookup, and reading every record for it would grow with the whole library.
fn commitments(store: &dyn Store, limit: usize) -> Result<Value, String> {
    let open = store.open_commitments(limit).map_err(|e| e.to_string())?;
    let mut records: BTreeMap<RecordId, (Option<String>, i64)> = BTreeMap::new();
    let mut merged: BTreeMap<CommitmentId, u64> = BTreeMap::new();
    for c in &open {
        if records.contains_key(&c.record) {
            continue;
        }
        let Some(record) = store.record(&c.record).map_err(|e| e.to_string())? else {
            // Deleting a record deletes its commitments; one listed without its record is a
            // race with that delete, and it is gone by the next list.
            continue;
        };
        records.insert(c.record.clone(), (record.title, record.started_at_unix_ms));
        for other in store.commitments(&c.record).map_err(|e| e.to_string())? {
            if let Some(into) = other.merged_into {
                *merged.entry(into).or_default() += 1;
            }
        }
    }
    let items: Vec<Value> = open
        .iter()
        .filter_map(|c| {
            let (title, started) = records.get(&c.record)?;
            Some(owed(
                store,
                c,
                title.as_deref(),
                *started,
                merged.get(&c.id).copied(),
            ))
        })
        .collect();
    Ok(event(
        "commitments.listed",
        &[("items", Some(Value::Array(items)))],
    ))
}

fn owed(
    store: &dyn Store,
    c: &Commitment,
    title: Option<&str>,
    started: i64,
    merged: Option<u64>,
) -> Value {
    let mut item = Map::new();
    item.insert("id".into(), c.id.0.clone().into());
    item.insert("record".into(), c.record.0.clone().into());
    put(&mut item, "record_title", title.map(Into::into));
    item.insert("record_started_at_unix_ms".into(), started.into());
    item.insert("text".into(), c.text.clone().into());
    put(&mut item, "owner", c.owner.clone().map(Into::into));
    put(&mut item, "recipient", c.recipient.clone().map(Into::into));
    put(&mut item, "due", c.due.clone().map(Into::into));
    put(
        &mut item,
        "due_at_unix_ms",
        c.due_at_unix_ms.map(Into::into),
    );
    if let Some(span) = c.provenance.iter().min_by_key(|s| s.start_ms) {
        item.insert("said_at_ms".into(), span.start_ms.into());
        item.insert("channel".into(), events::channel(span.channel).into());
    }
    item.insert("merged".into(), merged.unwrap_or(0).into());
    put(
        &mut item,
        "looks_done",
        c.looks_done
            .as_ref()
            .map(|e| crate::library::done_evidence(store, e)),
    );
    Value::Object(item)
}

/// `modes.listed`, from [`MODES_KEY`], else the modes the 0.2 import brought, else the built-in
/// default. A stored document that cannot be read is an error, never quietly the default.
fn modes(store: &dyn Store) -> Result<Value, String> {
    let stored = match store.setting(MODES_KEY).map_err(|e| e.to_string())? {
        Some(doc) => Some(doc),
        None => store
            .setting(ink_store::import::MODES_KEY)
            .map_err(|e| e.to_string())?,
    };
    let (default_id, modes) = match stored {
        Some(doc) => read_modes(&doc)?,
        None => {
            let mode = Mode::builtin_default();
            (
                mode.id.clone(),
                vec![json!({
                    "id": mode.id,
                    "name": mode.name,
                    "style": mode.style.as_str(),
                    "polish": mode.polish_enabled,
                    "remove_fillers": mode.remove_fillers,
                    "apps": mode.apps,
                })],
            )
        }
    };
    Ok(event(
        "modes.listed",
        &[
            ("default_id", Some(default_id.into())),
            ("modes", Some(Value::Array(modes))),
        ],
    ))
}

fn read_modes(doc: &str) -> Result<(String, Vec<Value>), String> {
    const UNREADABLE: &str = "the stored modes cannot be read";
    let v: Value = serde_json::from_str(doc).map_err(|_| UNREADABLE.to_owned())?;
    let default_id = v
        .get("default_id")
        .and_then(Value::as_str)
        .ok_or(UNREADABLE)?
        .to_owned();
    let list = v.get("modes").and_then(Value::as_array).ok_or(UNREADABLE)?;
    let mut out = Vec::with_capacity(list.len());
    for m in list {
        let s = |k: &str| m.get(k).and_then(Value::as_str);
        let flag = |k: &str, default: bool| match m.get(k) {
            None => Some(default),
            Some(b) => b.as_bool(),
        };
        let apps: Vec<&str> = match m.get("apps") {
            None => Vec::new(),
            Some(Value::Array(apps)) => apps
                .iter()
                .map(Value::as_str)
                .collect::<Option<_>>()
                .ok_or(UNREADABLE)?,
            Some(_) => return Err(UNREADABLE.into()),
        };
        out.push(json!({
            "id": s("id").ok_or(UNREADABLE)?,
            "name": s("name").ok_or(UNREADABLE)?,
            // A style this build does not know is shown as such, never as another style.
            "style": s("style").and_then(Style::parse).map_or("other", Style::as_str),
            "polish": flag("polish_enabled", false).ok_or(UNREADABLE)?,
            "remove_fillers": flag("remove_fillers", true).ok_or(UNREADABLE)?,
            "apps": apps,
        }));
    }
    Ok((default_id, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_parse_and_refuse_what_they_cannot_read() {
        let p = |json: &str| {
            let v: Value = serde_json::from_str(json).unwrap();
            let name = v["cmd"].as_str().unwrap().to_owned();
            parse(&name, &v)
        };
        assert!(p(r#"{"cmd":"model.warm","job":"dictation_final"}"#).is_none());
        assert_eq!(
            p(r#"{"cmd":"permissions.check","id":"c"}"#),
            Some(Ok(Query::PermissionsCheck))
        );
        assert_eq!(
            p(r#"{"cmd":"permission.request","permission":"system_audio"}"#),
            Some(Ok(Query::PermissionRequest(Permission::SystemAudio)))
        );
        assert_eq!(
            p(r#"{"cmd":"commitments.list"}"#),
            Some(Ok(Query::CommitmentsList {
                limit: DEFAULT_COMMITMENTS_LIMIT
            }))
        );
        assert_eq!(
            p(r#"{"cmd":"note.add","record":"r","at_ms":1200,"text":"hi"}"#),
            Some(Ok(Query::NoteAdd {
                record: "r".into(),
                at_ms: 1200,
                text: "hi".into()
            }))
        );
        assert_eq!(
            p(r#"{"cmd":"setting.set","key":"dictation.polish","value":"on"}"#),
            Some(Ok(Query::SettingSet {
                key: "dictation.polish".into(),
                value: "on".into()
            }))
        );
        for bad in [
            r#"{"cmd":"permissions.check","deep":true}"#,
            r#"{"cmd":"permission.request","permission":"camera"}"#,
            r#"{"cmd":"permission.request"}"#,
            r#"{"cmd":"commitments.list","limit":0}"#,
            r#"{"cmd":"commitments.list","limit":"5"}"#,
            r#"{"cmd":"commitment.set_done","commitment":"c","done":"yes"}"#,
            r#"{"cmd":"note.add","record":"r","at_ms":-1,"text":"hi"}"#,
            r#"{"cmd":"note.add","record":"r","text":"hi"}"#,
            r#"{"cmd":"note.update","note":"n"}"#,
            r#"{"cmd":"setting.get","key":"permissions.system_audio_asked"}"#,
            r#"{"cmd":"setting.set","key":"dictation.polish","value":"maybe"}"#,
            r#"{"cmd":"setting.set","key":"library.path","value":"/x"}"#,
        ] {
            assert!(matches!(p(bad), Some(Err(_))), "{bad} must be refused");
        }
    }

    #[test]
    fn stored_modes_are_read_and_a_damaged_document_is_an_error() {
        let doc = r#"{"default_id":"d","modes":[
            {"id":"d","name":"Everywhere else","style":"formal","polish_enabled":true,"apps":[]},
            {"id":"c","name":"Chat","style":"casual","apps":["com.example.chat"],"remove_fillers":false},
            {"id":"x","name":"Odd","style":"shouting"}]}"#;
        let (default_id, modes) = read_modes(doc).unwrap();
        assert_eq!(default_id, "d");
        assert_eq!(modes[0]["polish"], true);
        assert_eq!(modes[0]["remove_fillers"], true, "the 0.2 default");
        assert_eq!(modes[1]["apps"], json!(["com.example.chat"]));
        assert_eq!(modes[1]["polish"], false);
        assert_eq!(modes[2]["style"], "other");
        for bad in [
            "not json",
            r#"{"modes":[]}"#,
            r#"{"default_id":"d","modes":[{"name":"x","style":"formal"}]}"#,
            r#"{"default_id":"d","modes":[{"id":"x","name":"x","apps":[3]}]}"#,
            r#"{"default_id":"d","modes":[{"id":"x","name":"x","polish_enabled":"yes"}]}"#,
        ] {
            assert!(read_modes(bad).is_err(), "{bad}");
        }
    }
}
