//! The library as the shell's screens read it (Today, Library, a record): read-only queries on the
//! store, the one write they need (a commitment marked done or not), and the permission check the
//! Today screen's "needs you" banner reads. Each is a command answered by one event.
//!
//! | Command | Answer |
//! |---|---|
//! | `records.list` | `library.records`: records newest first (by start, then id), one kind or all, paged by a keyset cursor |
//! | `records.search` | `library.search`: full-text hits across every record's current transcript |
//! | `record.open` | `library.record`: one record whole: transcript, notes, summary, commitments, speakers, and where its audio is on disk |
//! | `commitments.open` | `library.owed`: open commitments, soonest due first, with their record's title |
//! | `commitment.set_done` | `library.commitment_done` |
//! | `library.stats` | `library.stats`: counts, time and words per kind since a moment, and the run of recent meetings that kept no far end |
//! | `permissions.check` | `permissions.checked`: each permission's state, never prompting |
//!
//! **Their own thread**, "ink-library", one query at a time in the order they came. Not the command
//! thread: a model download there can take minutes, and the library must not wait behind it. The
//! store serialises its own writes, so a query here and a meeting's final pass never interleave
//! within a call.
//!
//! **Words travel in these answers** (the transcript, notes, summaries, search snippets): they are
//! the library the screens show. As for every event that carries them, never log them. A failure is
//! `command.failed` naming what failed (a record id, a store error), never the text.
//!
//! **The meeting's timeline.** A record's segments are in ms from the meeting's start, which is a
//! host time on the capture clock. Its chunks carry host times too, so a chunk's place on the
//! timeline is its first frame's host time minus that start. The meeting worker writes the start
//! beside the chunks ([`write_timeline`]); a record without it (one recorded before it existed)
//! falls back to its earliest chunk, and says the timeline is estimated.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};

use ink_audio::ChunkStore;
use ink_core::store::word_count;
use ink_core::{
    Channel, Commitment, CommitmentId, Permission, PermissionState, Record, RecordCursor, RecordId,
    RecordKind, RecordQuery, Segment, Span,
};
use serde_json::{Map, Value, json};

use crate::events::{self, event};
use crate::runtime::{Shared, only_fields};

/// The file beside a meeting's chunks that holds where its timeline starts.
pub const TIMELINE_FILE: &str = "timeline.json";

/// The setting that records whether the app has asked for System Audio (the probe must not run
/// before, or macOS prompts): `"true"` once asked. The shell writes it when it asks.
pub const SYSTEM_AUDIO_ASKED: &str = "permissions.system_audio_asked";

/// Records per `records.list` when the command names no limit.
pub const DEFAULT_LIMIT: usize = 50;
/// The most records, hits or commitments one answer carries.
pub const MAX_LIMIT: usize = 500;
/// How many of the newest meetings `library.stats` looks at for a far end that kept nothing.
pub const FAR_SILENT_WINDOW: usize = 20;
/// The longest preview of an untitled record's words.
pub const PREVIEW_CHARS: usize = 140;

const COMMANDS: &[&str] = &[
    "records.list",
    "records.search",
    "record.open",
    "commitments.open",
    "commitment.set_done",
    "library.stats",
    "permissions.check",
];

/// A query, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Query {
    Records {
        kind: Option<RecordKind>,
        before: Option<RecordCursor>,
        limit: usize,
    },
    Search {
        query: String,
        limit: usize,
    },
    Open {
        record: RecordId,
    },
    Owed {
        limit: usize,
    },
    SetDone {
        commitment: CommitmentId,
        done: bool,
    },
    Stats {
        since_unix_ms: i64,
    },
    Permissions,
}

/// A query with its command's name and the shell's id for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Request {
    name: String,
    id: Option<String>,
    query: Query,
}

/// A record kind's schema name.
pub fn kind_name(kind: RecordKind) -> &'static str {
    match kind {
        RecordKind::Dictation => "dictation",
        RecordKind::Meeting => "meeting",
        RecordKind::FileImport => "file_import",
    }
}

fn parse_kind(name: &str) -> Option<RecordKind> {
    Some(match name {
        "dictation" => RecordKind::Dictation,
        "meeting" => RecordKind::Meeting,
        "file_import" => RecordKind::FileImport,
        _ => return None,
    })
}

/// Reads `json` as a library query: `Ok(None)` when its `cmd` is not one (the command thread's
/// parser reads it then), an error when it is one and cannot be read.
pub(crate) fn parse(json: &str) -> Result<Option<Request>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("command: {e}"))?;
    let Some(obj) = v.as_object() else {
        return Ok(None);
    };
    let Some(name) = obj.get("cmd").and_then(Value::as_str) else {
        return Ok(None);
    };
    if !COMMANDS.contains(&name) {
        return Ok(None);
    }
    let name = name.to_owned();
    let id = match obj.get("id") {
        None => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err("command: \"id\" must be a string".into()),
    };
    let fields: &[&str] = match name.as_str() {
        "records.list" => &["kind", "before", "limit"],
        "records.search" => &["query", "limit"],
        "record.open" => &["record"],
        "commitments.open" => &["limit"],
        "commitment.set_done" => &["commitment", "done"],
        "library.stats" => &["since_unix_ms"],
        _ => &[],
    };
    let allowed: Vec<&str> = ["cmd", "id"].iter().chain(fields).copied().collect();
    only_fields(obj, &allowed, &name)?;
    let text = |k: &str| -> Result<String, String> {
        obj.get(k)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("{name}: needs a string \"{k}\""))
    };
    let limit = |default: usize| -> Result<usize, String> {
        match obj.get("limit") {
            None => Ok(default),
            Some(l) => l
                .as_u64()
                .filter(|l| (1..=MAX_LIMIT as u64).contains(l))
                .map(|l| l as usize)
                .ok_or_else(|| format!("{name}: \"limit\" must be 1 to {MAX_LIMIT}")),
        }
    };
    let query = match name.as_str() {
        "records.list" => Query::Records {
            kind: match obj.get("kind") {
                None => None,
                Some(k) => Some(
                    k.as_str()
                        .and_then(parse_kind)
                        .ok_or("records.list: \"kind\" is meeting, dictation or file_import")?,
                ),
            },
            before: match obj.get("before") {
                None => None,
                Some(b) => Some(cursor(b)?),
            },
            limit: limit(DEFAULT_LIMIT)?,
        },
        "records.search" => Query::Search {
            query: text("query")?,
            limit: limit(DEFAULT_LIMIT)?,
        },
        "record.open" => Query::Open {
            record: RecordId(text("record")?),
        },
        "commitments.open" => Query::Owed {
            limit: limit(DEFAULT_LIMIT)?,
        },
        "commitment.set_done" => Query::SetDone {
            commitment: CommitmentId(text("commitment")?),
            done: obj
                .get("done")
                .and_then(Value::as_bool)
                .ok_or("commitment.set_done: needs a boolean \"done\"")?,
        },
        "library.stats" => Query::Stats {
            since_unix_ms: obj
                .get("since_unix_ms")
                .and_then(Value::as_i64)
                .ok_or("library.stats: needs an integer \"since_unix_ms\"")?,
        },
        _ => Query::Permissions,
    };
    Ok(Some(Request { name, id, query }))
}

fn cursor(v: &Value) -> Result<RecordCursor, String> {
    let bad =
        || "records.list: \"before\" is {\"started_at_unix_ms\": <integer>, \"id\": <string>}";
    let obj = v.as_object().ok_or_else(bad)?;
    only_fields(
        obj,
        &["started_at_unix_ms", "id"],
        "records.list: \"before\"",
    )?;
    Ok(RecordCursor {
        started_at_unix_ms: obj
            .get("started_at_unix_ms")
            .and_then(Value::as_i64)
            .ok_or_else(bad)?,
        id: RecordId(
            obj.get("id")
                .and_then(Value::as_str)
                .ok_or_else(bad)?
                .to_owned(),
        ),
    })
}

/// The library's thread. See the module docs.
pub(crate) struct Library {
    tx: Sender<Request>,
    thread: JoinHandle<()>,
}

impl Library {
    /// Starts the thread.
    pub(crate) fn start(shared: Arc<Shared>) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Request>();
        let thread = thread::Builder::new()
            .name("ink-library".into())
            .spawn(move || {
                while let Ok(request) = rx.recv() {
                    guarded(&shared, request);
                }
            })?;
        Ok(Self { tx, thread })
    }

    /// Queues a query.
    pub(crate) fn send(&self, request: Request) -> Result<(), String> {
        self.tx
            .send(request)
            .map_err(|_| "the library thread has stopped".to_owned())
    }

    /// Answers what is queued, then ends the thread.
    pub(crate) fn stop(self) {
        drop(self.tx);
        if self.thread.join().is_err() {
            log::error!("the library thread panicked outside its boundary");
        }
    }
}

/// Runs one query behind a panic boundary, as the command thread runs commands.
fn guarded(shared: &Shared, request: Request) {
    let (name, id) = (request.name.clone(), request.id.clone());
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| answer(shared, request)));
    let failure = match ran {
        Ok(Ok(())) => return,
        Ok(Err(message)) => message,
        Err(_) => {
            // The payload is not logged: it could hold the library's words (I5).
            log::error!("query {name} panicked; the next query still runs");
            "a bug in the core stopped this query".to_owned()
        }
    };
    log::warn!("query {name} failed: {failure}");
    shared
        .events
        .emit(events::command_failed(&name, id.as_deref(), &failure));
}

fn answer(shared: &Shared, request: Request) -> Result<(), String> {
    let Request { name: _, id, query } = request;
    let store = shared.store.as_ref();
    let request_field = ("request", id.map(Value::from));
    let e = |err: ink_core::StoreError| err.to_string();
    let answer = match query {
        Query::Records {
            kind,
            before,
            limit,
        } => {
            // One more than asked, to say whether there is a next page.
            let mut records = store
                .records(&RecordQuery {
                    kind,
                    before,
                    limit: limit + 1,
                })
                .map_err(e)?;
            let more = records.len() > limit;
            records.truncate(limit);
            let rows = records
                .iter()
                .map(|r| record_row(shared, r))
                .collect::<Result<Vec<_>, _>>()?;
            event(
                "library.records",
                &[
                    request_field,
                    ("kind", kind.map(|k| kind_name(k).into())),
                    ("records", Some(Value::Array(rows))),
                    ("more", Some(more.into())),
                ],
            )
        }
        Query::Search { query, limit } => {
            let hits = store.search(&query, limit).map_err(e)?;
            let hits = hits
                .iter()
                .map(|h| {
                    event_object(&[
                        ("record", Some(h.record.0.as_str().into())),
                        ("title", h.title.as_deref().map(Value::from)),
                        ("started_at_unix_ms", Some(h.started_at_unix_ms.into())),
                        ("start_ms", Some(h.start_ms.into())),
                        ("snippet", Some(h.snippet.as_str().into())),
                    ])
                })
                .collect();
            event(
                "library.search",
                &[
                    request_field,
                    ("query", Some(query.into())),
                    ("hits", Some(Value::Array(hits))),
                ],
            )
        }
        Query::Open { record } => {
            let Some(found) = store.record(&record).map_err(e)? else {
                return Err(format!("there is no record {}", record.0));
            };
            open(shared, request_field, &found)?
        }
        Query::Owed { limit } => {
            // Every open one, to count them; the answer carries the first `limit`.
            let all = store.open_commitments(usize::MAX).map_err(e)?;
            let total = all.len();
            let mut titles: Vec<(RecordId, Option<Record>)> = Vec::new();
            let mut items = Vec::new();
            for c in all.into_iter().take(limit) {
                let record = match titles.iter().find(|(id, _)| *id == c.record) {
                    Some((_, r)) => r.clone(),
                    None => {
                        let r = store.record(&c.record).map_err(e)?;
                        titles.push((c.record.clone(), r.clone()));
                        r
                    }
                };
                items.push(event_object(&[
                    ("commitment", Some(commitment(&c))),
                    (
                        "record_title",
                        record
                            .as_ref()
                            .and_then(|r| r.title.as_deref())
                            .map(Value::from),
                    ),
                    (
                        "record_started_at_unix_ms",
                        record.as_ref().map(|r| r.started_at_unix_ms.into()),
                    ),
                ]));
            }
            event(
                "library.owed",
                &[
                    request_field,
                    ("commitments", Some(Value::Array(items))),
                    ("total", Some(total.into())),
                ],
            )
        }
        Query::SetDone { commitment, done } => {
            store.set_commitment_done(&commitment, done).map_err(e)?;
            event(
                "library.commitment_done",
                &[
                    request_field,
                    ("commitment", Some(commitment.0.into())),
                    ("done", Some(done.into())),
                ],
            )
        }
        Query::Stats { since_unix_ms } => stats(shared, request_field, since_unix_ms)?,
        Query::Permissions => {
            let state = |p: Permission| -> Option<Value> {
                let s = match &shared.permissions {
                    Some(probe) => probe.check(p),
                    None => PermissionState::Unknown,
                };
                Some(permission_state(s).into())
            };
            event(
                "permissions.checked",
                &[
                    request_field,
                    ("microphone", state(Permission::Microphone)),
                    ("system_audio", state(Permission::SystemAudio)),
                    ("accessibility", state(Permission::Accessibility)),
                    ("input_monitoring", state(Permission::InputMonitoring)),
                ],
            )
        }
    };
    shared.events.emit(answer);
    Ok(())
}

/// A permission state's schema name.
pub fn permission_state(state: PermissionState) -> &'static str {
    match state {
        PermissionState::Granted => "granted",
        PermissionState::Denied => "denied",
        PermissionState::NotDetermined => "not_determined",
        PermissionState::Unknown => "unknown",
    }
}

/// An object of the fields that are present (as [`event`] builds, without a type).
fn event_object(fields: &[(&str, Option<Value>)]) -> Value {
    let mut map = Map::new();
    for (name, value) in fields {
        if let Some(value) = value {
            map.insert((*name).into(), value.clone());
        }
    }
    Value::Object(map)
}

/// A record as the list shows it. An untitled record carries the start of its words instead.
fn record_row(shared: &Shared, r: &Record) -> Result<Value, String> {
    let preview = match &r.title {
        Some(_) => None,
        None => {
            let segments = shared.store.segments(&r.id).map_err(|e| e.to_string())?;
            preview(&segments).map(Value::from)
        }
    };
    Ok(event_object(&[
        ("record", Some(r.id.0.as_str().into())),
        ("kind", Some(kind_name(r.kind).into())),
        ("title", r.title.as_deref().map(Value::from)),
        ("started_at_unix_ms", Some(r.started_at_unix_ms.into())),
        ("ended_at_unix_ms", r.ended_at_unix_ms.map(Value::from)),
        ("source_app", r.source_app.as_deref().map(Value::from)),
        ("revision", Some(r.revision.into())),
        ("has_audio", Some(r.audio_dir.is_some().into())),
        ("preview", preview),
    ]))
}

/// The first words of a transcript, at most [`PREVIEW_CHARS`], cut at a word.
fn preview(segments: &[Segment]) -> Option<String> {
    let mut out = String::new();
    for word in segments.iter().flat_map(|s| s.text.split_whitespace()) {
        let extra = usize::from(!out.is_empty()) + word.chars().count();
        if out.chars().count() + extra > PREVIEW_CHARS {
            out.push('…');
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    (!out.is_empty()).then_some(out)
}

fn span(s: &Span) -> Value {
    json!({"channel": events::channel(s.channel), "start_ms": s.start_ms, "end_ms": s.end_ms})
}

fn commitment(c: &Commitment) -> Value {
    event_object(&[
        ("commitment", Some(c.id.0.as_str().into())),
        ("record", Some(c.record.0.as_str().into())),
        ("text", Some(c.text.as_str().into())),
        ("owner", c.owner.as_deref().map(Value::from)),
        ("due", c.due.as_deref().map(Value::from)),
        ("due_at_unix_ms", c.due_at_unix_ms.map(Value::from)),
        (
            "provenance",
            Some(Value::Array(c.provenance.iter().map(span).collect())),
        ),
        (
            "merged_into",
            c.merged_into.as_ref().map(|m| m.0.as_str().into()),
        ),
        ("done", Some(c.done.into())),
    ])
}

fn open(
    shared: &Shared,
    request_field: (&str, Option<Value>),
    r: &Record,
) -> Result<Value, String> {
    let store = shared.store.as_ref();
    let e = |err: ink_core::StoreError| err.to_string();
    let segments = store.segments(&r.id).map_err(e)?;
    let segments: Vec<Value> = segments
        .iter()
        .map(|s| {
            event_object(&[
                ("channel", Some(events::channel(s.channel).into())),
                ("start_ms", Some(s.start_ms.into())),
                ("end_ms", Some(s.end_ms.into())),
                ("text", Some(s.text.as_str().into())),
                ("speaker", s.speaker.as_ref().map(|sp| sp.0.as_str().into())),
            ])
        })
        .collect();
    let notes: Vec<Value> = store
        .notes(&r.id)
        .map_err(e)?
        .iter()
        .map(|n| json!({"note": n.id.0, "at_ms": n.at_ms, "text": n.text}))
        .collect();
    let summary = store.summary(&r.id).map_err(e)?.map(
        |s| json!({"text": s.text, "model": s.model, "created_at_unix_ms": s.created_at_unix_ms}),
    );
    let commitments: Vec<Value> = store
        .commitments(&r.id)
        .map_err(e)?
        .iter()
        .map(commitment)
        .collect();
    let speakers: Vec<Value> = store
        .speaker_names(&r.id)
        .map_err(e)?
        .iter()
        .map(|(id, name)| json!({"speaker": id.0, "name": name}))
        .collect();
    let audio = match &r.audio_dir {
        Some(dir) => audio(&shared.data_dir, dir)?,
        None => None,
    };
    Ok(event(
        "library.record",
        &[
            request_field,
            ("record", Some(record_row(shared, r)?)),
            ("segments", Some(Value::Array(segments))),
            ("notes", Some(Value::Array(notes))),
            ("summary", summary),
            ("commitments", Some(Value::Array(commitments))),
            ("speakers", Some(Value::Array(speakers))),
            ("audio", audio),
        ],
    ))
}

/// Writes where a meeting's timeline starts (the host time of its sample 0) beside its chunks.
/// Written to a temporary name and renamed, so a reader never sees half of it.
pub fn write_timeline(dir: &Path, start_host_ns: u64) -> io::Result<()> {
    let tmp = dir.join(format!("{TIMELINE_FILE}.tmp"));
    std::fs::write(&tmp, json!({"start_host_ns": start_host_ns}).to_string())?;
    std::fs::rename(&tmp, dir.join(TIMELINE_FILE))
}

/// Where the timeline starts, as [`write_timeline`] wrote it; `None` when it did not.
pub fn read_timeline(dir: &Path) -> Option<u64> {
    let text = std::fs::read_to_string(dir.join(TIMELINE_FILE)).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v.get("start_host_ns").and_then(Value::as_u64)
}

/// A record's chunks on its timeline. `None` when its directory is gone (a record whose audio was
/// removed): the record stands without a player.
fn audio(data_dir: &Path, relative: &str) -> Result<Option<Value>, String> {
    let dir: PathBuf = data_dir.join(relative);
    // Never created here: a query only reads.
    if !dir.is_dir() {
        return Ok(None);
    }
    let store = ChunkStore::open(&dir).map_err(|e| format!("the record's audio: {e}"))?;
    let mut chunks = Vec::new();
    for channel in [Channel::Mic, Channel::Far] {
        let list = store
            .chunks(channel)
            .map_err(|e| format!("the record's audio: {e}"))?;
        if !list.unreadable.is_empty() {
            log::warn!(
                "record audio: {} unreadable {} chunk(s) left out of the player",
                list.unreadable.len(),
                events::channel(channel)
            );
        }
        // A chunk whose format recovery could only guess cannot be played as frames.
        chunks.extend(list.chunks.into_iter().filter(|c| !c.format_estimated));
    }
    let recorded = read_timeline(&dir);
    let Some(start) = recorded.or_else(|| chunks.iter().map(|c| c.host_time_ns).min()) else {
        return Ok(Some(json!({"timeline": "estimated", "chunks": []})));
    };
    let chunks: Vec<Value> = chunks
        .iter()
        .map(|c| {
            let offset_ns = i128::from(c.host_time_ns) - i128::from(start);
            let start_ms = i64::try_from(offset_ns / 1_000_000).unwrap_or(i64::MAX);
            json!({
                "channel": events::channel(c.channel),
                "path": c.path.to_string_lossy(),
                "start_ms": start_ms,
                "frames": c.frames,
                "sample_rate": c.format.sample_rate,
                "channels": c.format.channels,
                "data_offset": ink_audio::chunk::HEADER_LEN,
            })
        })
        .collect();
    let timeline = if recorded.is_some() {
        "recorded"
    } else {
        "estimated"
    };
    Ok(Some(json!({"timeline": timeline, "chunks": chunks})))
}

fn duration_ms(r: &Record) -> u64 {
    r.ended_at_unix_ms.map_or(0, |end| {
        end.saturating_sub(r.started_at_unix_ms).max(0) as u64
    })
}

fn stats(
    shared: &Shared,
    request_field: (&str, Option<Value>),
    since_unix_ms: i64,
) -> Result<Value, String> {
    let store = shared.store.as_ref();
    let e = |err: ink_core::StoreError| err.to_string();
    let mut kinds = Vec::new();
    for kind in [
        RecordKind::Dictation,
        RecordKind::Meeting,
        RecordKind::FileImport,
    ] {
        let (mut records, mut duration, mut words) = (0u64, 0u64, 0u64);
        let mut before = None;
        'pages: loop {
            let page = store
                .records(&RecordQuery {
                    kind: Some(kind),
                    before: before.clone(),
                    limit: MAX_LIMIT,
                })
                .map_err(e)?;
            for r in &page {
                // Newest first: the first one before the moment ends the count.
                if r.started_at_unix_ms < since_unix_ms {
                    break 'pages;
                }
                records += 1;
                duration += duration_ms(r);
                words += word_count(&store.segments(&r.id).map_err(e)?) as u64;
            }
            match page.last() {
                Some(last) if page.len() == MAX_LIMIT => before = Some(RecordCursor::from(last)),
                _ => break,
            }
        }
        kinds.push(json!({
            "kind": kind_name(kind),
            "records": records,
            "duration_ms": duration,
            "words": words,
        }));
    }
    // The newest finished meetings in a row that kept your words and none of the far end's: the
    // sign that system audio was not being heard (the canvas's "can't hear the other side").
    let meetings = store
        .records(&RecordQuery {
            kind: Some(RecordKind::Meeting),
            before: None,
            limit: FAR_SILENT_WINDOW,
        })
        .map_err(e)?;
    let (mut silent, mut silent_since) = (0u64, None);
    for r in meetings.iter().filter(|r| r.ended_at_unix_ms.is_some()) {
        let segments = store.segments(&r.id).map_err(e)?;
        let words = |c: Channel| {
            segments
                .iter()
                .filter(|s| s.channel == c)
                .map(|s| s.text.split_whitespace().count())
                .sum::<usize>()
        };
        if words(Channel::Far) > 0 || words(Channel::Mic) == 0 {
            break;
        }
        silent += 1;
        silent_since = Some(r.started_at_unix_ms);
    }
    Ok(event(
        "library.stats",
        &[
            request_field,
            ("since_unix_ms", Some(since_unix_ms.into())),
            ("kinds", Some(Value::Array(kinds))),
            ("far_silent_meetings", Some(silent.into())),
            ("far_silent_since_unix_ms", silent_since.map(Value::from)),
        ],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_commands_parse_or_say_why_not() {
        assert_eq!(parse(r#"{"cmd":"model.warm","job":"x"}"#), Ok(None));
        let r = parse(r#"{"cmd":"records.list","kind":"meeting","limit":3,"id":"q1"}"#)
            .unwrap()
            .unwrap();
        assert_eq!(r.id.as_deref(), Some("q1"));
        assert_eq!(
            r.query,
            Query::Records {
                kind: Some(RecordKind::Meeting),
                before: None,
                limit: 3
            }
        );
        let r = parse(r#"{"cmd":"records.list","before":{"started_at_unix_ms":5,"id":"b"}}"#)
            .unwrap()
            .unwrap();
        assert_eq!(
            r.query,
            Query::Records {
                kind: None,
                before: Some(RecordCursor {
                    started_at_unix_ms: 5,
                    id: RecordId("b".into())
                }),
                limit: DEFAULT_LIMIT
            }
        );
        assert_eq!(
            parse(r#"{"cmd":"commitment.set_done","commitment":"c","done":true}"#)
                .unwrap()
                .unwrap()
                .query,
            Query::SetDone {
                commitment: CommitmentId("c".into()),
                done: true
            }
        );
        assert_eq!(
            parse(r#"{"cmd":"permissions.check"}"#)
                .unwrap()
                .unwrap()
                .query,
            Query::Permissions
        );
        for bad in [
            r#"{"cmd":"records.list","kind":"memo"}"#,
            r#"{"cmd":"records.list","limit":0}"#,
            r#"{"cmd":"records.list","limit":501}"#,
            r#"{"cmd":"records.list","before":{"id":"b"}}"#,
            r#"{"cmd":"records.list","before":{"started_at_unix_ms":5,"id":"b","x":1}}"#,
            r#"{"cmd":"records.list","sort":"title"}"#,
            r#"{"cmd":"records.search"}"#,
            r#"{"cmd":"record.open","record":7}"#,
            r#"{"cmd":"commitment.set_done","commitment":"c"}"#,
            r#"{"cmd":"library.stats","since_unix_ms":"today"}"#,
            r#"{"cmd":"permissions.check","permission":"microphone"}"#,
            r#"{"cmd":"record.open","record":"r","id":3}"#,
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_preview_is_the_first_words_cut_at_a_word() {
        let seg = |text: &str| Segment {
            channel: Channel::Mic,
            start_ms: 0,
            end_ms: 1,
            text: text.into(),
            speaker: None,
        };
        assert_eq!(preview(&[]), None);
        assert_eq!(
            preview(&[seg("  hello  "), seg("there")]).as_deref(),
            Some("hello there")
        );
        let long = "word ".repeat(100);
        let p = preview(&[seg(&long)]).unwrap();
        assert!(p.ends_with("word…"), "{p}");
        assert!(p.chars().count() <= PREVIEW_CHARS + 1, "{}", p.len());
    }

    #[test]
    fn the_timeline_round_trips_and_is_absent_until_written() {
        let dir = std::env::temp_dir().join(format!("ink-ffi-timeline-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_timeline(&dir), None);
        write_timeline(&dir, 12_345_678_901).unwrap();
        assert_eq!(read_timeline(&dir), Some(12_345_678_901));
        assert!(!dir.join("timeline.json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
