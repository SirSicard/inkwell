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
//! 4. removes the marker, and asks for a retention sweep, as a live meeting's pass does.
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
/// so a crash leaves it complete or not at all.
pub fn mark_live(dir: &Path, record: &RecordId) -> io::Result<()> {
    use std::io::Write;
    let tmp = dir.join(format!("{LIVE_FILE}.tmp"));
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(json!({"record": record.0}).to_string().as_bytes())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, dir.join(LIVE_FILE))?;
    crate::library::sync_dir(dir)
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

/// The record a marker names, if it is one.
fn read_marker(dir: &Path) -> Option<RecordId> {
    let text = std::fs::read_to_string(dir.join(LIVE_FILE)).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let id = v.get("record")?.as_str()?;
    (!id.is_empty()).then(|| RecordId(id.to_owned()))
}

/// **Worker.** The meeting directories under the data directory that still have a marker, with
/// the record each names, oldest directory first. A directory whose marker cannot be read is
/// left alone and logged: it is never guessed at.
pub fn interrupted(data_dir: &Path) -> Vec<(PathBuf, RecordId)> {
    let Ok(entries) = std::fs::read_dir(data_dir.join("meetings")) else {
        return Vec::new();
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
    found
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
                    if let Err(e) = shared
                        .store
                        .finish_record(record, stored.started_at_unix_ms)
                    {
                        log::warn!("meeting recovery: the record could not be marked ended: {e}");
                    }
                    clear_live(dir);
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
    match ended.finalize(&chunks, cancel) {
        Ok(_) => {
            clear_live(dir);
            // As after a live meeting's pass: the library changed, and the setting applies now.
            shared.sweep_soon();
        }
        // The app is quitting: the marker stays for the next launch.
        Err(ink_pipeline::meeting::FinalizeError::Cancelled) => {}
        Err(e) => {
            clear_live(dir);
            fail(e.to_string());
        }
    }
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
        let found = interrupted(&root);
        assert_eq!(
            found
                .iter()
                .map(|(d, r)| (d.file_name().unwrap().to_str().unwrap(), r.0.as_str()))
                .collect::<Vec<_>>(),
            [("a", "rec-a"), ("b", "rec-b")]
        );
        clear_live(&meetings.join("a"));
        clear_live(&meetings.join("d"));
        assert_eq!(interrupted(&root).len(), 1);
        assert!(!meetings.join("b").join(format!("{LIVE_FILE}.tmp")).exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
