//! The library as the Today, Library and Record screens read it: records newest first, full-text
//! search, one record whole with its audio on disk, and counts since a moment. Each is a command
//! answered by one event that echoes the command's id as `ref`; a failure is `command.failed` with
//! that id, so a screen can tell "could not load" from an empty library.
//!
//! | Command | Answer |
//! |---|---|
//! | `records.list` | `library.records`: records newest first (by start, then id), one kind or all, paged by a keyset cursor |
//! | `records.search` | `library.search`: full-text hits across every record's current transcript |
//! | `record.open` | `library.record`: one record whole: transcript, notes, summary, commitments, speakers, and where its audio is on disk |
//! | `library.stats` | `library.stats`: counts, time and words per kind since a moment, and the run of recent meetings that kept no far end |
//!
//! They run on the screens' thread, `ink-queries` ([`queries`](crate::queries)), in order with the
//! screens' other commands, so they never wait behind a model download on the command thread.
//!
//! **Words travel in these answers** (the transcript, notes, summaries, search snippets): they are
//! the library the screens show. As for every event that carries them, never log them. A failure
//! names what failed (a record id, a store error), never the text.
//!
//! **The meeting's timeline.** A record's segments are in ms from the meeting's start, which is a
//! host time on the capture clock. Its chunks carry host times too, so a chunk's place on the
//! timeline is its first frame's host time minus that start. The meeting worker writes the start
//! beside the chunks ([`write_timeline`]); a record without it (one recorded before it existed)
//! falls back to its earliest chunk, and says the timeline is estimated.

use std::io;
use std::path::{Path, PathBuf};

use ink_audio::{ChunkInfo, ChunkStore};
use ink_core::store::word_count;
use ink_core::{
    Channel, Commitment, Record, RecordCursor, RecordId, RecordKind, RecordQuery, Segment, Span,
};
use serde_json::{Map, Value, json};

use crate::events::{self, event};
use crate::runtime::Shared;

/// The file beside a meeting's chunks that holds where its timeline starts.
pub const TIMELINE_FILE: &str = "timeline.json";

/// Records per `records.list` when the command names no limit.
pub const DEFAULT_LIMIT: usize = 50;
/// The most records or hits one answer carries.
pub const MAX_LIMIT: usize = 500;
/// How many of the newest meetings `library.stats` looks at for a far end that kept nothing.
pub const FAR_SILENT_WINDOW: usize = 20;
/// The furthest back `library.stats` counts: it reads every transcript in its window to count
/// words, so the window is bounded here rather than by what a shell happens to ask (Today asks for
/// the day and the week). An older moment is refused, never quietly cut.
pub const STATS_MAX_DAYS: i64 = 31;
/// The longest preview of an untitled record's words.
pub const PREVIEW_CHARS: usize = 140;

/// A library query, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryQuery {
    /// `records.list`.
    Records {
        /// Only this kind, or every kind.
        kind: Option<RecordKind>,
        /// Only records after this one in the listing order.
        before: Option<RecordCursor>,
        /// At most this many.
        limit: usize,
    },
    /// `records.search`.
    Search {
        /// The words.
        query: String,
        /// At most this many hits.
        limit: usize,
    },
    /// `record.open`.
    Open {
        /// The record.
        record: RecordId,
    },
    /// `library.stats`.
    Stats {
        /// Counted from this moment, Unix ms.
        since_unix_ms: i64,
    },
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

/// Reads `v` as the library query named `name`: `None` when `name` is not one, else the query or
/// why it cannot be read. Unknown fields are refused, as for every other command.
pub fn parse(name: &str, v: &Value) -> Option<Result<LibraryQuery, String>> {
    let fields: &[&str] = match name {
        "records.list" => &["kind", "before", "limit"],
        "records.search" => &["query", "limit"],
        "record.open" => &["record"],
        "library.stats" => &["since_unix_ms"],
        _ => return None,
    };
    Some(parse_known(name, fields, v))
}

fn parse_known(name: &str, fields: &[&str], v: &Value) -> Result<LibraryQuery, String> {
    let obj = v.as_object().ok_or("command: not an object")?;
    if let Some(k) = obj
        .keys()
        .find(|k| !["cmd", "id"].contains(&k.as_str()) && !fields.contains(&k.as_str()))
    {
        return Err(format!("{name}: unknown field \"{k}\""));
    }
    let text = |k: &str| -> Result<String, String> {
        obj.get(k)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("{name}: needs a string \"{k}\""))
    };
    let limit = || -> Result<usize, String> {
        match obj.get("limit") {
            None => Ok(DEFAULT_LIMIT),
            Some(l) => l
                .as_u64()
                .filter(|l| (1..=MAX_LIMIT as u64).contains(l))
                .map(|l| l as usize)
                .ok_or_else(|| format!("{name}: \"limit\" must be 1 to {MAX_LIMIT}")),
        }
    };
    Ok(match name {
        "records.list" => LibraryQuery::Records {
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
            limit: limit()?,
        },
        "records.search" => LibraryQuery::Search {
            query: text("query")?,
            limit: limit()?,
        },
        "record.open" => LibraryQuery::Open {
            record: RecordId(text("record")?),
        },
        _ => LibraryQuery::Stats {
            since_unix_ms: obj
                .get("since_unix_ms")
                .and_then(Value::as_i64)
                .ok_or("library.stats: needs an integer \"since_unix_ms\"")?,
        },
    })
}

fn cursor(v: &Value) -> Result<RecordCursor, String> {
    let bad =
        || "records.list: \"before\" is {\"started_at_unix_ms\": <integer>, \"id\": <string>}";
    let obj = v.as_object().ok_or_else(bad)?;
    if obj.keys().any(|k| k != "started_at_unix_ms" && k != "id") {
        return Err(bad().into());
    }
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

/// **Worker** (the screens' thread). Answers `query`, echoing `id` as `ref`: the event, or why it
/// could not (the caller sends that as `command.failed`).
pub fn answer(shared: &Shared, query: LibraryQuery, id: Option<&str>) -> Result<Value, String> {
    let store = shared.store.as_ref();
    let ref_field = ("ref", id.map(Value::from));
    let e = |err: ink_core::StoreError| err.to_string();
    Ok(match query {
        LibraryQuery::Records {
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
                .map(|r| record_row(shared, r, None))
                .collect::<Result<Vec<_>, _>>()?;
            event(
                "library.records",
                &[
                    ref_field,
                    ("kind", kind.map(|k| kind_name(k).into())),
                    ("records", Some(Value::Array(rows))),
                    ("more", Some(more.into())),
                ],
            )
        }
        LibraryQuery::Search { query, limit } => {
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
                    ref_field,
                    ("query", Some(query.into())),
                    ("hits", Some(Value::Array(hits))),
                ],
            )
        }
        LibraryQuery::Open { record } => {
            let Some(found) = store.record(&record).map_err(e)? else {
                return Err(format!("there is no record {}", record.0));
            };
            open(shared, ref_field, &found)?
        }
        LibraryQuery::Stats { since_unix_ms } => stats(shared, ref_field, since_unix_ms)?,
    })
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

/// A record as the list shows it. An untitled record carries the start of its words instead:
/// from `segments` when the caller already read them (`record.open`), else read here.
///
/// For a list that is one `segments()` per untitled row, at most a page (`MAX_LIMIT`, 500) of
/// them. The store has no "first segments" query, and a dictation (the usual untitled record) is
/// a segment or two; an untitled meeting is rare, since its summary's headline names it.
fn record_row(shared: &Shared, r: &Record, segments: Option<&[Segment]>) -> Result<Value, String> {
    let preview = match (&r.title, segments) {
        (Some(_), _) => None,
        (None, Some(segments)) => preview(segments).map(Value::from),
        (None, None) => {
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

fn open(shared: &Shared, ref_field: (&str, Option<Value>), r: &Record) -> Result<Value, String> {
    let store = shared.store.as_ref();
    let e = |err: ink_core::StoreError| err.to_string();
    let read = store.segments(&r.id).map_err(e)?;
    let segments: Vec<Value> = read
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
            ref_field,
            // The segments just read give an untitled record's preview: one read, not two.
            ("record", Some(record_row(shared, r, Some(&read))?)),
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
/// Written to a temporary name, synced, and renamed, then the directory synced (as ink-engines
/// writes its revision marker): after a power cut the file is there whole, or not at all, and
/// never a torn one that would place the chunks wrongly.
pub fn write_timeline(dir: &Path, start_host_ns: u64) -> io::Result<()> {
    use std::io::Write;
    let tmp = dir.join(format!("{TIMELINE_FILE}.tmp"));
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(
        json!({"start_host_ns": start_host_ns})
            .to_string()
            .as_bytes(),
    )?;
    file.sync_all()?;
    // Closed before the rename: Windows refuses to rename an open file.
    drop(file);
    std::fs::rename(&tmp, dir.join(TIMELINE_FILE))?;
    sync_dir(dir)
}

/// Syncs a directory's entries, so a rename in it survives a power cut. On Windows std cannot
/// open a directory, and NTFS journals the entry.
fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

/// Where the timeline starts, as [`write_timeline`] wrote it; `None` when it did not.
pub fn read_timeline(dir: &Path) -> Option<u64> {
    let text = std::fs::read_to_string(dir.join(TIMELINE_FILE)).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v.get("start_host_ns").and_then(Value::as_u64)
}

/// The record's audio directory, `relative` under `data_dir`, or why it is refused: it must be a
/// plain relative path (no root, no `..`), and once links are resolved it must still be inside the
/// library. `Ok(None)` when it does not exist (a record whose audio was removed).
pub fn audio_dir(data_dir: &Path, relative: &str) -> Result<Option<PathBuf>, String> {
    const OUTSIDE: &str = "the record's audio directory is outside the library";
    let rel = Path::new(relative);
    if relative.is_empty()
        || rel
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(OUTSIDE.into());
    }
    let dir = data_dir.join(rel);
    // Never created here: a query only reads.
    if !dir.is_dir() {
        return Ok(None);
    }
    let root = data_dir
        .canonicalize()
        .map_err(|e| format!("the library's directory: {e}"))?;
    let resolved = dir
        .canonicalize()
        .map_err(|e| format!("the record's audio: {e}"))?;
    if !resolved.starts_with(&root) {
        return Err(OUTSIDE.into());
    }
    Ok(Some(resolved))
}

/// Where the chunks go on the timeline, and how many are left out of the player: unreadable ones
/// (counted by the caller), a format recovery could only guess (their bytes cannot be read as
/// frames), and a host time recovery could only guess (0 or extrapolated: their place is not
/// known, and a wrong one would put speech against the wrong lines). The start is the recorded
/// one, else the earliest chunk whose host time is real.
fn place(chunks: Vec<ChunkInfo>, recorded: Option<u64>) -> (Option<u64>, Vec<ChunkInfo>, usize) {
    let total = chunks.len();
    let kept: Vec<ChunkInfo> = chunks
        .into_iter()
        .filter(|c| !c.format_estimated && !c.host_time_estimated)
        .collect();
    let left_out = total - kept.len();
    let start = recorded.or_else(|| kept.iter().map(|c| c.host_time_ns).min());
    (start, kept, left_out)
}

/// A record's chunks on its timeline. `None` when its directory is gone (a record whose audio was
/// removed): the record stands without a player.
fn audio(data_dir: &Path, relative: &str) -> Result<Option<Value>, String> {
    let Some(dir) = audio_dir(data_dir, relative)? else {
        return Ok(None);
    };
    let store = ChunkStore::open(&dir).map_err(|e| format!("the record's audio: {e}"))?;
    let mut chunks = Vec::new();
    let mut unreadable = 0;
    for channel in [Channel::Mic, Channel::Far] {
        let list = store
            .chunks(channel)
            .map_err(|e| format!("the record's audio: {e}"))?;
        unreadable += list.unreadable.len();
        chunks.extend(list.chunks);
    }
    let recorded = read_timeline(&dir);
    let (start, kept, guessed) = place(chunks, recorded);
    let left_out = unreadable + guessed;
    if left_out > 0 {
        log::warn!(
            "record audio: {left_out} chunk(s) left out of the player ({unreadable} unreadable, {guessed} of unknown format or place)"
        );
    }
    let timeline = if recorded.is_some() {
        "recorded"
    } else {
        "estimated"
    };
    let Some(start) = start else {
        return Ok(Some(
            json!({"timeline": timeline, "chunks": [], "left_out": left_out}),
        ));
    };
    let chunks: Vec<Value> = kept
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
    Ok(Some(
        json!({"timeline": timeline, "chunks": chunks, "left_out": left_out}),
    ))
}

fn duration_ms(r: &Record) -> u64 {
    r.ended_at_unix_ms.map_or(0, |end| {
        end.saturating_sub(r.started_at_unix_ms).max(0) as u64
    })
}

fn stats(
    shared: &Shared,
    ref_field: (&str, Option<Value>),
    since_unix_ms: i64,
) -> Result<Value, String> {
    let store = shared.store.as_ref();
    let e = |err: ink_core::StoreError| err.to_string();
    let oldest = shared
        .clock
        .unix_ms()
        .saturating_sub(STATS_MAX_DAYS * 24 * 3_600_000);
    if since_unix_ms < oldest {
        return Err(format!(
            "library.stats counts at most the last {STATS_MAX_DAYS} days"
        ));
    }
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
            ref_field,
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

    fn p(json: &str) -> Option<Result<LibraryQuery, String>> {
        let v: Value = serde_json::from_str(json).unwrap();
        parse(v["cmd"].as_str().unwrap(), &v)
    }

    #[test]
    fn library_commands_parse_or_say_why_not() {
        assert!(p(r#"{"cmd":"model.warm","job":"x"}"#).is_none());
        assert!(
            p(r#"{"cmd":"permissions.check"}"#).is_none(),
            "the screens' own"
        );
        assert_eq!(
            p(r#"{"cmd":"records.list","kind":"meeting","limit":3,"id":"q1"}"#),
            Some(Ok(LibraryQuery::Records {
                kind: Some(RecordKind::Meeting),
                before: None,
                limit: 3
            }))
        );
        assert_eq!(
            p(r#"{"cmd":"records.list","before":{"started_at_unix_ms":5,"id":"b"}}"#),
            Some(Ok(LibraryQuery::Records {
                kind: None,
                before: Some(RecordCursor {
                    started_at_unix_ms: 5,
                    id: RecordId("b".into())
                }),
                limit: DEFAULT_LIMIT
            }))
        );
        assert_eq!(
            p(r#"{"cmd":"library.stats","since_unix_ms":-5}"#),
            Some(Ok(LibraryQuery::Stats { since_unix_ms: -5 }))
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
            r#"{"cmd":"library.stats","since_unix_ms":"today"}"#,
        ] {
            assert!(matches!(p(bad), Some(Err(_))), "{bad}");
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

    fn chunk(channel: Channel, host_time_ns: u64) -> ChunkInfo {
        ChunkInfo {
            channel,
            index: 0,
            format: ink_core::StreamFormat::CANONICAL,
            host_time_ns,
            frames: 16_000,
            after_gap: false,
            host_time_estimated: false,
            format_estimated: false,
            path: PathBuf::from("c.pcm"),
        }
    }

    /// A chunk whose host time recovery could only guess (0, or extrapolated) is left out and
    /// counted, like one whose format it guessed: its place on the timeline is not known, and a
    /// 0 would otherwise become the timeline's start and push every real chunk minutes late.
    #[test]
    fn chunks_of_unknown_place_or_format_are_left_out_and_counted() {
        let guessed_time = ChunkInfo {
            host_time_estimated: true,
            ..chunk(Channel::Mic, 0)
        };
        let guessed_format = ChunkInfo {
            format_estimated: true,
            ..chunk(Channel::Far, 7_000_000_000)
        };
        let chunks = vec![
            guessed_time,
            chunk(Channel::Mic, 5_000_000_000),
            guessed_format,
            chunk(Channel::Far, 6_000_000_000),
        ];
        let (start, kept, left_out) = place(chunks.clone(), None);
        assert_eq!(start, Some(5_000_000_000), "the earliest real host time");
        assert_eq!(left_out, 2);
        assert!(
            kept.iter()
                .all(|c| !c.host_time_estimated && !c.format_estimated)
        );
        let (start, _, _) = place(chunks, Some(4_900_000_000));
        assert_eq!(start, Some(4_900_000_000), "the recorded start wins");
        let (start, kept, left_out) = place(
            vec![ChunkInfo {
                host_time_estimated: true,
                ..chunk(Channel::Mic, 0)
            }],
            None,
        );
        assert_eq!((start, kept.len(), left_out), (None, 0, 1));
    }

    /// The audio directory is a plain relative path inside the library, links resolved.
    #[test]
    fn an_audio_directory_outside_the_library_is_refused() {
        let root = std::env::temp_dir().join(format!("ink-ffi-audio-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let data = root.join("data");
        std::fs::create_dir_all(data.join("meetings/m1")).unwrap();
        std::fs::create_dir_all(root.join("elsewhere")).unwrap();
        assert_eq!(
            audio_dir(&data, "meetings/m1").unwrap(),
            Some(data.join("meetings/m1").canonicalize().unwrap())
        );
        assert_eq!(audio_dir(&data, "meetings/gone").unwrap(), None);
        for bad in [
            "../elsewhere",
            "meetings/../../elsewhere",
            "/tmp",
            "",
            "./meetings/m1",
        ] {
            let refused = audio_dir(&data, bad).unwrap_err();
            assert!(refused.contains("outside the library"), "{bad}: {refused}");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("elsewhere"), data.join("meetings/link")).unwrap();
            assert!(
                audio_dir(&data, "meetings/link").is_err(),
                "a link out of the library"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
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
