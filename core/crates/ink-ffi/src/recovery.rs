//! Crash recovery: a meeting whose app was killed while it recorded, or while its final pass ran,
//! is finished at the next launch.
//!
//! What a crash leaves: the chunks on disk (each block written straight to its file, so at most
//! what was still in the capture ring is lost, and the open chunk's torn tail), the live finals in
//! the library (revision 1), and a marker beside the chunks, [`LIVE_FILE`], written when the chain
//! starts and removed when its final pass has run. Recovery:
//!
//! 1. finds every meeting directory that still has a marker ([`interrupted`]);
//! 2. repairs its chunks ([`ChunkStore::recover`]: torn tails trimmed, torn headers rebuilt;
//!    chunks whose time or format could only be guessed are left out of the pass and counted,
//!    as the library already counts them);
//! 3. marks the record ended where its audio ends, and runs the final pass over what is on disk
//!    ([`EndedMeeting::interrupted`]), with the same engines a live meeting would get;
//! 4. removes the marker once the record reads as ended (when the store refused to end it, the
//!    marker stays and the next launch tries again), and asks for a retention sweep, as a live
//!    meeting's pass does.
//!
//! A marker that says `"discard": true` ([`mark_discard`]) is a meeting its user stopped to
//! delete ("Stop and delete"); a crash came before the delete was done. It is never finished:
//! its record and its audio are deleted ([`crate::retention::discard`]), and `meeting.discarded`
//! says so.
//!
//! The shell asks for it (`meetings.recover`) once its own engines are registered, so a
//! recovered meeting gets the live partials' fallback and the language model a live one would.
//! It runs on its own thread, one meeting at a time, and never while that meeting's record is
//! live in this process (a marker is only ever left by a process that is gone).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use ink_audio::{ChunkStore, Repair};
use ink_core::{CancelToken, Channel, RecordId};
use ink_pipeline::meeting::{EndedMeeting, Interrupted};
use serde_json::{Value, json};

use crate::events::event;
use crate::meeting::{MeetingInfo, meeting_sink, services};
use crate::runtime::Shared;

/// The marker beside a meeting's chunks while its final pass has not run.
pub const LIVE_FILE: &str = "live.json";

/// **Worker.** Marks the meeting in `dir` live: written whole (a temporary name, synced, renamed),
/// so a crash leaves it complete or not at all. Readable by its owner only, as the library is.
pub fn mark_live(dir: &Path, record: &RecordId) -> io::Result<()> {
    write_marker(dir, &json!({"record": record.0}))
}

/// **Worker.** Marks the meeting in `dir` to be deleted, not finished (Stop and delete), written
/// whole as [`mark_live`] writes: recovery deletes a meeting marked so.
pub fn mark_discard(dir: &Path, record: &RecordId) -> io::Result<()> {
    write_marker(dir, &json!({"record": record.0, "discard": true}))
}

/// The marker in `dir`, written whole: a temporary name, synced, renamed, the directory synced.
fn write_marker(dir: &Path, marker: &Value) -> io::Result<()> {
    use std::io::Write;
    let tmp = dir.join(format!("{LIVE_FILE}.tmp"));
    let mut file = owner_only(&tmp)?;
    file.write_all(marker.to_string().as_bytes())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, dir.join(LIVE_FILE))?;
    crate::library::sync_dir(dir)
}

/// `path`, created or emptied, readable and writable by its owner only (0600 on Unix).
fn owner_only(path: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options.open(path)?;
    // The mode applies only to a new file: one a crash left behind is narrowed here.
    #[cfg(unix)]
    file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    Ok(file)
}

/// **Worker.** The meeting in `dir` is done: its marker goes. A failure is logged; the next
/// launch would run the pass again, which costs time, not data.
pub fn clear_live(dir: &Path) {
    if let Err(e) = std::fs::remove_file(dir.join(LIVE_FILE))
        && e.kind() != io::ErrorKind::NotFound
    {
        log::warn!("meeting: the crash-recovery marker could not be removed: {e}");
    }
}

/// Whether the marker in `dir` says the meeting is to be deleted ([`mark_discard`]).
fn marked_discard(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(LIVE_FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|v| v.get("discard").and_then(Value::as_bool))
        == Some(true)
}

/// The record a marker names, if it is one.
fn read_marker(dir: &Path) -> Option<RecordId> {
    let text = std::fs::read_to_string(dir.join(LIVE_FILE)).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let id = v.get("record")?.as_str()?;
    (!id.is_empty()).then(|| RecordId(id.to_owned()))
}

/// **Worker.** The meeting directories under the data directory that still have a marker, with
/// the record each names, oldest directory first. A directory whose marker cannot be read is
/// left alone and logged: it is never guessed at. No meetings directory is none found; one that
/// cannot be listed is an error (the caller says so: an interrupted meeting may be waiting).
pub fn interrupted(data_dir: &Path) -> io::Result<Vec<(PathBuf, RecordId)>> {
    let entries = match std::fs::read_dir(data_dir.join("meetings")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut found: Vec<(PathBuf, RecordId)> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join(LIVE_FILE).is_file())
        .filter_map(|dir| match read_marker(&dir) {
            Some(record) => Some((dir, record)),
            None => {
                log::warn!("meeting recovery: a marker could not be read; left alone");
                None
            }
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(found)
}

/// What recovery repaired in one meeting's chunks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Repaired {
    /// Torn partial frames cut from a chunk's end.
    pub trimmed: usize,
    /// Headers rebuilt.
    pub rebuilt: usize,
    /// Chunks left exactly as they were because nothing could place them.
    pub unrecoverable: usize,
}

/// Where the recorded audio ends, ms after the timeline's start: the latest end of a chunk
/// whose host time is real, on either side.
fn audio_end_ms(chunks: &ChunkStore, t0_ns: u64) -> u64 {
    [Channel::Mic, Channel::Far]
        .into_iter()
        .filter_map(|c| chunks.chunks(c).ok())
        .flat_map(|list| list.chunks)
        .filter(|c| !c.host_time_estimated && !c.format_estimated && c.format.sample_rate > 0)
        .map(|c| {
            let end =
                c.host_time_ns + c.frames * 1_000_000_000 / u64::from(c.format.sample_rate.max(1));
            end.saturating_sub(t0_ns) / 1_000_000
        })
        .max()
        .unwrap_or(0)
}

/// **Worker.** Recovers the interrupted meeting in `dir` (see the module docs). Its events:
/// `meeting.recovered`, then the final pass's, ending in `meeting.finished` or `meeting.failed`.
pub fn recover(shared: &Arc<Shared>, dir: &Path, record: &RecordId, cancel: &CancelToken) {
    let fail = |message: String| {
        log::warn!("meeting recovery: {message}");
        shared.events.emit(event(
            "meeting.failed",
            &[
                ("record", Some(record.0.as_str().into())),
                ("message", Some(message.into())),
            ],
        ));
    };
    // For the whole recovery, before any path below can mark the record ended: retention must not
    // take it until its pass (or its early end) is done. Released on every return, and before
    // the sweep that follows the pass is asked for (a hold still there would keep it from it).
    let hold = match shared.hold_from_sweep(record) {
        Ok(hold) => hold,
        // Logged by name there. Retention is deleting the record: nothing to finish, and its
        // marker goes with its audio.
        Err(e) => return fail(format!("the meeting could not be recovered: {e}")),
    };
    if marked_discard(dir) {
        // Its user stopped it to delete it: deleted, never finished (nothing is transcribed
        // again, summarised or sent anywhere).
        match crate::retention::discard(shared, Some(hold), record, dir) {
            Ok(gone) => {
                log::info!("meeting recovery: a meeting stopped to be deleted was deleted");
                shared
                    .events
                    .emit(crate::retention::discarded(record, gone));
            }
            Err(e) => fail(format!(
                "a meeting stopped to be deleted could not be deleted: {e}; the next launch tries again"
            )),
        }
        return;
    }
    let stored = match shared.store.record(record) {
        Ok(Some(r)) => r,
        Ok(None) => {
            // Its record was deleted (by the user, or retention): nothing to finish.
            log::info!("meeting recovery: the record is gone; the marker goes");
            clear_live(dir);
            return;
        }
        Err(e) => return fail(format!("the record could not be read: {e}")),
    };
    let chunks = match ChunkStore::open(dir) {
        Ok(c) => c,
        Err(e) => return fail(format!("the meeting's audio: {e}")),
    };
    let report = match chunks.recover() {
        Ok(r) => r,
        Err(e) => return fail(format!("the meeting's audio could not be repaired: {e}")),
    };
    let mut repaired = Repaired::default();
    for repair in &report.repairs {
        match repair {
            Repair::TrimmedPartialFrame { .. } => repaired.trimmed += 1,
            Repair::RebuiltHeader { .. } => repaired.rebuilt += 1,
            Repair::Unrecoverable { .. } => repaired.unrecoverable += 1,
        }
    }
    // The timeline's start as the worker wrote it, else the earliest chunk whose time is real
    // (as the library's player estimates it).
    let t0_ns = match crate::library::read_timeline(dir) {
        Some(t0) => t0,
        None => {
            let earliest = [Channel::Mic, Channel::Far]
                .into_iter()
                .filter_map(|c| chunks.chunks(c).ok())
                .flat_map(|list| list.chunks)
                .filter(|c| !c.host_time_estimated)
                .map(|c| c.host_time_ns)
                .min();
            match earliest {
                Some(t0) => t0,
                None => {
                    // No audio can be placed: the live transcript stands, the record is ended.
                    // Unless the store refuses: then the marker stays, and the next launch
                    // tries again (the record still reads as live).
                    match shared
                        .store
                        .finish_record(record, stored.started_at_unix_ms)
                    {
                        Ok(()) => clear_live(dir),
                        Err(e) => {
                            log::warn!(
                                "meeting recovery: the record could not be marked ended: {e}; the marker stays"
                            );
                        }
                    }
                    return fail(
                        "no recorded audio could be placed; the live transcript stands".into(),
                    );
                }
            }
        }
    };
    let end_ms = audio_end_ms(&chunks, t0_ns);
    shared.events.emit(event(
        "meeting.recovered",
        &[
            ("record", Some(record.0.as_str().into())),
            ("trimmed", Some(repaired.trimmed.into())),
            ("rebuilt", Some(repaired.rebuilt.into())),
            ("unrecoverable", Some(repaired.unrecoverable.into())),
            ("recorded_ms", Some(end_ms.into())),
        ],
    ));
    let slot: Arc<OnceLock<RecordId>> = Arc::new(OnceLock::from(record.clone()));
    let info = MeetingInfo {
        title: stored.title.clone(),
        ..MeetingInfo::default()
    };
    let (services, settings) = services(shared);
    let vad = crate::engines::vad_source(shared);
    let ended = EndedMeeting::interrupted(
        services,
        settings,
        vad,
        meeting_sink(shared, &slot, &info),
        Interrupted {
            record: record.clone(),
            started_unix_ms: stored.started_at_unix_ms,
            t0_ns,
            ended_unix_ms: stored
                .started_at_unix_ms
                .saturating_add(i64::try_from(end_ms).unwrap_or(i64::MAX)),
        },
    );
    let result = ended.finalize(&chunks, cancel);
    let cancelled = matches!(result, Err(ink_pipeline::meeting::FinalizeError::Cancelled));
    // Cancelled: the app is quitting, and the marker stays for the next launch. A record the
    // store could not end keeps it too, so the next launch ends it.
    if marker_goes(ended.record_ended(), cancelled) {
        clear_live(dir);
    } else if !cancelled {
        log::warn!("meeting recovery: the record could not be marked ended; the marker stays");
    }
    drop(hold);
    match result {
        // As after a live meeting's pass: the library changed, and the setting applies now.
        Ok(_) => shared.sweep_soon(),
        Err(_) if cancelled => {}
        Err(e) => fail(e.to_string()),
    }
}

/// Whether a meeting's crash marker goes after its final pass: only when the record reads as
/// ended and the pass was not cancelled (the app quitting). The live path and recovery share it.
pub(crate) fn marker_goes(record_ended: bool, cancelled: bool) -> bool {
    record_ended && !cancelled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marker_round_trips_and_only_readable_markers_are_found() {
        let root = std::env::temp_dir().join(format!("ink-ffi-recovery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let meetings = root.join("meetings");
        for name in ["b", "a", "c", "d"] {
            std::fs::create_dir_all(meetings.join(name)).unwrap();
        }
        mark_live(&meetings.join("b"), &RecordId("rec-b".into())).unwrap();
        mark_live(&meetings.join("a"), &RecordId("rec-a".into())).unwrap();
        std::fs::write(meetings.join("c").join(LIVE_FILE), "not json").unwrap();
        let found = interrupted(&root).unwrap();
        assert_eq!(
            found
                .iter()
                .map(|(d, r)| (d.file_name().unwrap().to_str().unwrap(), r.0.as_str()))
                .collect::<Vec<_>>(),
            [("a", "rec-a"), ("b", "rec-b")]
        );
        clear_live(&meetings.join("a"));
        clear_live(&meetings.join("d"));
        assert_eq!(interrupted(&root).unwrap().len(), 1);
        assert!(!meetings.join("b").join(format!("{LIVE_FILE}.tmp")).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Review (S2.8): the marker names a record, so it is the owner's alone, as the library's
    /// files are; a temporary one a crash left with a wider mode is narrowed.
    #[cfg(unix)]
    #[test]
    fn a_marker_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("ink-ffi-marker-mode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let stale = root.join(format!("{LIVE_FILE}.tmp"));
        std::fs::write(&stale, "stale").unwrap();
        std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o644)).unwrap();
        mark_live(&root, &RecordId("rec".into())).unwrap();
        let mode = std::fs::metadata(root.join(LIVE_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Review (S2.8): no meetings directory is nothing to recover; one that cannot be listed is
    /// an error, never an empty list (a meeting may be waiting).
    #[test]
    fn a_meetings_directory_that_cannot_be_listed_is_an_error() {
        let root = std::env::temp_dir().join(format!("ink-ffi-unlisted-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert!(interrupted(&root).unwrap().is_empty(), "none yet");
        std::fs::write(root.join("meetings"), "not a directory").unwrap();
        assert!(interrupted(&root).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_marker_goes_only_for_an_ended_record_and_a_pass_that_was_not_cancelled() {
        assert!(marker_goes(true, false));
        assert!(!marker_goes(false, false), "the next launch ends it");
        assert!(!marker_goes(true, true), "the next launch runs the pass");
        assert!(!marker_goes(false, true));
    }

    #[test]
    fn a_marker_says_to_delete_only_when_written_so() {
        let dir =
            std::env::temp_dir().join(format!("ink-ffi-discard-marker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let record = RecordId("r1".into());
        assert!(!marked_discard(&dir), "no marker");
        mark_live(&dir, &record).unwrap();
        assert!(!marked_discard(&dir));
        mark_discard(&dir, &record).unwrap();
        assert!(marked_discard(&dir));
        assert_eq!(read_marker(&dir), Some(record), "still names its record");
        std::fs::write(dir.join(LIVE_FILE), "not json").unwrap();
        assert!(!marked_discard(&dir), "unreadable is not a delete");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
