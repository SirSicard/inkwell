//! Retention: the user's setting for how long the library keeps records, and the sweep that
//! deletes what is older.
//!
//! The setting, [`RETENTION_KEY`], is one of [`RETENTION_VALUES`]: `forever` (the default: nothing
//! is ever deleted on its own) or a number of days. A record older than that, by its start, goes
//! whole: its transcript, notes, summary, commitments and speakers through the store's delete,
//! then its recorded audio.
//!
//! **Secure delete.** The store's delete overwrites the deleted text in the database file (SQLite's
//! `secure_delete`, and the search index's own) and then checkpoints the write-ahead log with
//! TRUNCATE, so no copy of the text is left in the log either; when another process holds the log
//! open, it says so and tries again on every later call (`Store::unscrubbed`). The audio files
//! are removed; a file system that copies on write (APFS) may keep their blocks until they are
//! reused, which only disk encryption (FileVault) covers. The record goes first: if its audio
//! then cannot be removed, the words are already gone and the failure is counted.
//!
//! **What.** Meetings and dictations made here. Nothing an import brought in is ever swept,
//! however old: an imported file ([`RecordKind::FileImport`]), nor a record an importer wrote
//! ([`Record::imported`]: Inkwell 0.2's dictations, another app's meetings). The user brought it
//! in on purpose, and the library may hold its only copy (the Settings copy says "meetings and
//! dictations", and that what was imported is kept).
//!
//! **When.** Never on a timer (nothing ticks while idle): at launch, after each meeting's final
//! pass (a recovered meeting's too), and when the setting changes.
//!
//! **Never a meeting that is not finished.** A record without an end (a meeting live now, or one
//! a crash interrupted) is never swept; nor is one whose final pass has not finished, although
//! its record already reads as ended (a meeting is marked ended when its live phase stops, and a
//! recovered one when recovery takes it up, before the pass runs): one this process is finishing
//! ([`Hold`]), or one with a crash-recovery marker beside its audio (its pass has not run to the
//! end; recovery finishes it, and the sweep after that takes it). Otherwise a sweep that happened
//! to run in that window (the launch's, running late) would delete the record under its own pass.
//!
//! **One record, by hand.** `record.delete` ([`delete_one`]) deletes one record the user chose,
//! of any kind (imports included: the user asked), the same way and with the same checks: never
//! one without an end that this app made, nor one whose final pass has not finished.
//!
//! **Stop and delete.** A meeting its user stopped to delete in its first minute
//! (`meeting.discard`) is deleted by its own worker ([`discard`]), or by recovery after a crash,
//! the same way: the record through the store's secure delete, then its audio directory. It has
//! no final pass to wait for; the worker's hold becomes the delete's claim under one lock.
//!
//! **Where.** On its own thread, `ink-retention` ([`Sweeper`]), which sleeps until a sweep is
//! asked for: never on a meeting's worker (a new meeting must be able to start the moment the last
//! one's final pass is over) nor on the screens' thread. Asks that arrive during a sweep are one
//! more sweep, not one each.

use std::collections::BTreeSet;
use std::io;
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use ink_core::{Record, RecordCursor, RecordId, RecordKind, RecordQuery, StoreError};

use crate::events::event;
use crate::runtime::{Shared, lock};

/// The setting: how long records are kept.
pub const RETENTION_KEY: &str = "retention.days";

/// Its values: `forever`, or days.
pub const RETENTION_VALUES: &[&str] = &["forever", "7", "30", "90", "365"];

/// Records read per page while sweeping.
const PAGE: usize = 200;

const DAY_MS: i64 = 86_400_000;

/// One sweep at a time, in this process: the retention thread runs them, and a test may too.
static SWEEPING: Mutex<()> = Mutex::new(());

/// A message to the retention thread.
pub(crate) enum Ask {
    /// Sweep now (or once more, after the sweep running).
    Sweep,
    /// Stop the thread.
    Quit,
}

/// The retention thread, `ink-retention`.
pub struct Sweeper {
    tx: Sender<Ask>,
    thread: JoinHandle<()>,
}

impl Sweeper {
    /// Starts `ink-retention`, idle until [`Shared::sweep_soon`] asks.
    pub fn start(shared: Arc<Shared>) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("ink-retention".into())
            .spawn(move || {
                while let Ok(Ask::Sweep) = rx.recv() {
                    // What was asked meanwhile is covered by this sweep; a quit among it wins.
                    if rx.try_iter().any(|ask| matches!(ask, Ask::Quit))
                        || shared.shutdown.is_cancelled()
                    {
                        break;
                    }
                    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _ = sweep(&shared);
                    }));
                    if ran.is_err() {
                        log::error!("the retention sweep panicked; the next one still runs");
                    }
                }
            })?;
        Ok(Self { tx, thread })
    }

    /// A sender for asks (the meetings' workers, recovery and the settings reach it through one).
    pub(crate) fn sender(&self) -> Sender<Ask> {
        self.tx.clone()
    }

    /// Stops the thread: a sweep in progress stops at its next record (the shutdown's cancel).
    pub fn stop(self) {
        let _ = self.tx.send(Ask::Quit);
        if self.thread.join().is_err() {
            log::error!("the retention thread panicked outside its boundary");
        }
    }
}

impl Shared {
    /// **Any thread.** Asks the retention thread for a sweep and returns at once. Before the
    /// thread has started, or after it has stopped (the core is shutting down), the ask is
    /// dropped: the next launch sweeps anyway.
    pub(crate) fn sweep_soon(&self) {
        match self.sweeps.get() {
            Some(tx) => {
                if lock(tx).send(Ask::Sweep).is_err() && !self.shutdown.is_cancelled() {
                    log::warn!("retention: the sweep thread has stopped; the next launch sweeps");
                }
            }
            None => log::warn!("retention: no sweep thread yet; the next launch sweeps"),
        }
    }
}

/// The records retention must leave alone, and the ones it is deleting now, under one lock
/// (`Shared::finishing`) that is never held across I/O.
#[derive(Default)]
pub(crate) struct Holds {
    /// Records this process is finishing ([`Hold`]).
    held: BTreeSet<RecordId>,
    /// Records a sweep has checked and is deleting now (marked under the lock, deleted outside it).
    sweeping: BTreeSet<RecordId>,
}

/// A record this process is finishing (a meeting from its start to the end of its final pass, a
/// recovered meeting for the whole of its recovery, from before any path can mark it ended):
/// retention leaves it alone while this is held. Dropping it lets go.
///
/// Taking a hold and the sweep's decision to delete a record never interleave: both run under the
/// same lock, and the sweep marks the record as being swept before it lets go of the lock to
/// delete it. A hold taken first keeps the record; a hold on a record being swept is refused
/// ([`BeingSwept`]); a hold taken after the delete finds the record gone. No hold ever waits on a
/// delete: the lock is only held to read and change the two sets.
pub struct Hold<'a> {
    shared: &'a Shared,
    record: RecordId,
}

/// A hold refused: retention is deleting that record now. Record ids are never reused and a
/// record is only swept once its pass is done, so this means a bug, and it is logged by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeingSwept(pub RecordId);

impl std::fmt::Display for BeingSwept {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "retention is deleting record {} now", self.0.0)
    }
}

impl Shared {
    /// **Worker.** Keeps retention away from `record` until the hold is dropped. Taken before the
    /// record can read as ended. Never waits on a sweep's delete. Refused, and logged, when a
    /// sweep is deleting that very record.
    pub fn hold_from_sweep(&self, record: &RecordId) -> Result<Hold<'_>, BeingSwept> {
        let mut holds = lock(&self.finishing);
        if holds.sweeping.contains(record) {
            log::error!(
                "retention: a hold on record {} was refused: it is being deleted",
                record.0
            );
            return Err(BeingSwept(record.clone()));
        }
        holds.held.insert(record.clone());
        Ok(Hold {
            shared: self,
            record: record.clone(),
        })
    }

    /// Whether this process holds `record` from retention now ([`Shared::hold_from_sweep`]).
    pub fn held_from_sweep(&self, record: &RecordId) -> bool {
        lock(&self.finishing).held.contains(record)
    }
}

impl Drop for Hold<'_> {
    fn drop(&mut self) {
        lock(&self.shared.finishing).held.remove(&self.record);
    }
}

/// Whether `record`'s crash-recovery marker says its pass has not finished: the marker is there,
/// or cannot be looked for (a record kept one sweep too long costs nothing; one deleted under its
/// pass costs the meeting). Read outside the holds' lock: a marker only goes (its pass is done),
/// and a new one is only ever written for a new record.
fn marker(shared: &Shared, record: &Record) -> Option<&'static str> {
    let relative = record.audio_dir.as_ref()?;
    match crate::library::audio_dir(&shared.data_dir, relative) {
        Ok(Some(dir)) => match std::fs::symlink_metadata(dir.join(crate::recovery::LIVE_FILE)) {
            Ok(_) => Some("its crash-recovery marker is there"),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => {
                log::warn!("retention: a meeting's crash marker could not be looked for: {e}");
                Some("its crash-recovery marker could not be looked for")
            }
        },
        // No audio directory on disk: no marker either.
        Ok(None) => None,
        Err(e) => {
            log::warn!("retention: a meeting's audio directory could not be checked: {e}");
            Some("its audio directory could not be checked")
        }
    }
}

/// Under the holds' lock: `record` is held, or already being deleted (by a sweep, or by the user
/// at the same moment), and is left alone; or it is marked as being deleted (returns true).
fn claim(shared: &Shared, record: &RecordId) -> bool {
    let mut holds = lock(&shared.finishing);
    if holds.held.contains(record) || holds.sweeping.contains(record) {
        return false;
    }
    holds.sweeping.insert(record.clone());
    true
}

/// What deleting a claimed record did to its audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Audio {
    /// Removed, or there was none.
    Gone,
    /// It could not be removed (logged): the record's words are gone already.
    Failed,
    /// Its directory is outside the library, and was left alone (logged).
    Outside,
}

/// Deletes a record [`claim`]ed for it: the record first, through the store's secure delete
/// (a record already gone is no error), then its claim is let go, then its audio. `who` starts the
/// log lines. Errors are the store's: the record is still there, and so is its audio.
fn remove(shared: &Shared, record: &Record, who: &str) -> Result<Audio, StoreError> {
    let deleted = shared.store.delete_record(&record.id);
    lock(&shared.finishing).sweeping.remove(&record.id);
    match deleted {
        Ok(()) | Err(StoreError::NotFound) => {}
        Err(e) => return Err(e),
    }
    let Some(relative) = &record.audio_dir else {
        return Ok(Audio::Gone);
    };
    Ok(
        match crate::library::audio_dir(&shared.data_dir, relative) {
            Ok(Some(dir)) => match std::fs::remove_dir_all(&dir) {
                Ok(()) => Audio::Gone,
                Err(e) => {
                    log::warn!("{who}: a deleted record's audio could not be removed: {e}");
                    Audio::Failed
                }
            },
            Ok(None) => Audio::Gone,
            Err(e) => {
                // Never deleted outside the library, whatever a record says.
                log::warn!("{who}: a deleted record's audio was left alone: {e}");
                Audio::Outside
            }
        },
    )
}

/// What a sweep did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Swept {
    /// Records deleted.
    pub deleted: usize,
    /// Records whose delete, or whose audio's removal, failed (each logged by what failed).
    pub failed: usize,
    /// The cut-off: records that started before this, Unix ms, were due.
    pub before_unix_ms: i64,
}

/// The days the setting keeps, or `None` for forever (or no setting).
fn days(shared: &Shared) -> Result<Option<i64>, StoreError> {
    Ok(match shared.store.setting(RETENTION_KEY)?.as_deref() {
        None | Some("forever") => None,
        Some(days) => match days.parse::<i64>() {
            Ok(d) if d > 0 => Some(d),
            _ => {
                // Only the whitelisted values can be set; anything else is never taken as a
                // number of days to delete by.
                log::error!(
                    "retention: the stored setting is not one this build knows; kept everything"
                );
                None
            }
        },
    })
}

/// **Worker** (the retention thread's). Deletes every ended meeting and dictation made here that
/// started before the setting's cut-off, except a meeting whose final pass has not finished (see
/// the module docs). `None` when the setting keeps everything; otherwise what it did,
/// also sent as `library.swept` when it did anything. It stops early at shutdown.
pub fn sweep(shared: &Shared) -> Option<Swept> {
    let _one = SWEEPING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let days = match days(shared) {
        Ok(days) => days?,
        Err(e) => {
            log::warn!("retention: the setting could not be read: {e}; nothing swept");
            return None;
        }
    };
    let before = shared
        .clock
        .unix_ms()
        .saturating_sub(days.saturating_mul(DAY_MS));
    let mut swept = Swept {
        before_unix_ms: before,
        ..Swept::default()
    };
    // Strictly before the cut-off in the listing order (start, then id, both descending): an
    // empty id sorts before every real one, so the cursor at the cut-off itself excludes it.
    let mut cursor = RecordCursor {
        started_at_unix_ms: before,
        id: RecordId(String::new()),
    };
    'pages: loop {
        let page = match shared.store.records(&RecordQuery {
            kind: None,
            before: Some(cursor.clone()),
            limit: PAGE,
        }) {
            Ok(page) => page,
            Err(e) => {
                log::warn!("retention: the library could not be listed: {e}");
                swept.failed += 1;
                break;
            }
        };
        let Some(last) = page.last() else { break };
        cursor = RecordCursor::from(last);
        for record in page {
            if shared.shutdown.is_cancelled() {
                break 'pages;
            }
            // Never a record without an end, nor an import (see What. in the module docs).
            if record.ended_at_unix_ms.is_none()
                || record.kind == RecordKind::FileImport
                || record.imported
            {
                continue;
            }
            // Checked and deleted under the holds' lock, so no hold is taken in between (see
            // Hold). Read after the listing: a record that listed as ended was ended under a hold,
            // which is still there unless its pass is done.
            // By id only: a record kept sweep after sweep (a pass that hangs) shows here.
            if let Some(why) = marker(shared, &record) {
                log::info!("retention: meeting {} kept: {why}", record.id.0);
                continue;
            }
            // Checked and marked under the holds' lock, deleted outside it (see Hold). Read after
            // the listing: a record that listed as ended was ended under a hold, which is still
            // there unless its pass is done.
            if !claim(shared, &record.id) {
                log::info!(
                    "retention: meeting {} kept: this process is finishing it, or it is being deleted",
                    record.id.0
                );
                continue;
            }
            match remove(shared, &record, "retention") {
                Ok(audio) => {
                    swept.deleted += 1;
                    if audio == Audio::Failed {
                        swept.failed += 1;
                    }
                }
                Err(e) => {
                    log::warn!("retention: a record could not be deleted: {e}");
                    swept.failed += 1;
                }
            }
        }
    }
    if swept.deleted > 0 || swept.failed > 0 {
        log::info!(
            "retention: {} records deleted, {} failures",
            swept.deleted,
            swept.failed
        );
        shared.events.emit(event(
            "library.swept",
            &[
                ("deleted", Some(swept.deleted.into())),
                ("failed", Some(swept.failed.into())),
                ("before_unix_ms", Some(swept.before_unix_ms.into())),
            ],
        ));
    }
    Some(swept)
}

/// What `record.delete` did ([`delete_one`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deleted {
    /// The record's kind.
    pub kind: RecordKind,
    /// Its audio is still on disk: it could not be removed, or its directory is outside the
    /// library and was left alone.
    pub audio_left: bool,
    /// The library's files hold no copy of its words: false while another process reading the
    /// database keeps them in the log (the store keeps trying, `Store::unscrubbed`).
    pub scrubbed: bool,
}

/// **Worker** (the screens' thread). Deletes the record `id` whole, as a sweep deletes one (the
/// store's secure delete, then its audio), for the user who asked: any kind, imports included.
/// Refused, and kept, while it is live: a record this app made that has no end yet (a meeting
/// being recorded, an import being written), or a meeting whose final pass has not finished (this
/// process is finishing it, or its crash-recovery marker is there). Errors name the record by id,
/// never its words.
pub fn delete_one(shared: &Shared, id: &RecordId) -> Result<Deleted, String> {
    let record = shared
        .store
        .record(id)
        .map_err(|e| format!("record {}: {e}", id.0))?
        .ok_or_else(|| format!("there is no record {}", id.0))?;
    // An importer's record may have no end: it was never live here.
    if record.ended_at_unix_ms.is_none() && !record.imported {
        return Err(format!("record {} is still being recorded", id.0));
    }
    if let Some(why) = marker(shared, &record) {
        return Err(format!("record {} is still being finished: {why}", id.0));
    }
    if !claim(shared, id) {
        return Err(format!(
            "record {} is still being finished, or is being deleted",
            id.0
        ));
    }
    let audio = remove(shared, &record, "delete").map_err(|e| {
        log::warn!("delete: record {} could not be deleted: {e}", id.0);
        format!("record {} could not be deleted: {e}", id.0)
    })?;
    // This answer reports the scrub's state, so it takes the change too: each change reaches the
    // shell once, through whichever sees it first (a chain's deleted_text_* warning, or this), and
    // a clear after a failure here is still reported by the next chain that asks.
    let _ = shared.store.scrub_change();
    Ok(Deleted {
        kind: record.kind,
        audio_left: audio != Audio::Gone,
        scrubbed: !shared.store.unscrubbed(),
    })
}

/// What Stop and delete did ([`discard`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Discarded {
    /// Its audio is still on disk: it could not be removed, or its directory is outside the
    /// library and was left alone (logged). The record is gone.
    pub audio_left: bool,
    /// The library's files hold no copy of its words (as [`Deleted::scrubbed`]).
    pub scrubbed: bool,
}

/// `meeting.discarded`: `record` is gone, as [`discard`] left it.
pub(crate) fn discarded(record: &RecordId, gone: Discarded) -> serde_json::Value {
    event(
        "meeting.discarded",
        &[
            ("record", Some(record.0.as_str().into())),
            ("audio_left", Some(gone.audio_left.into())),
            ("scrubbed", Some(gone.scrubbed.into())),
        ],
    )
}

/// **Worker** (the meeting's, or recovery's). Deletes the meeting `record`, whose chunks are in
/// `dir` (a directory under the library's `meetings`), for "Stop and delete": as `record.delete`
/// deletes, but with no final pass to wait for and with or without an end. `hold`, this process's
/// hold on it, becomes the delete's claim under one lock, so no sweep or `record.delete` takes it
/// in between; without one, a record held or being deleted by another path is left to it. The
/// record goes first (the store's secure delete; already gone is no error), then the directory,
/// with its markers and chunks. Errors are the store's, or a claim refused: the record is still
/// there, and so is its audio, marker and all, for recovery to delete at the next launch.
pub(crate) fn discard(
    shared: &Shared,
    hold: Option<Hold<'_>>,
    record: &RecordId,
    dir: &Path,
) -> Result<Discarded, String> {
    {
        let mut holds = lock(&shared.finishing);
        if hold.is_none() && (holds.held.contains(record) || holds.sweeping.contains(record)) {
            return Err(format!(
                "record {} is being finished or deleted by another path",
                record.0
            ));
        }
        holds.sweeping.insert(record.clone());
    }
    // After the claim: the record is never unheld and unclaimed at once.
    drop(hold);
    let deleted = shared.store.delete_record(record);
    lock(&shared.finishing).sweeping.remove(record);
    match deleted {
        Ok(()) | Err(StoreError::NotFound) => {}
        Err(e) => return Err(format!("record {}: {e}", record.0)),
    }
    // Through the library's own check, as every delete: never outside it, links resolved.
    let relative = dir
        .file_name()
        .map(|name| format!("meetings/{}", name.to_string_lossy()));
    let audio_left = match relative.map(|r| crate::library::audio_dir(&shared.data_dir, &r)) {
        Some(Ok(Some(dir))) => match std::fs::remove_dir_all(&dir) {
            Ok(()) => false,
            Err(e) if e.kind() == io::ErrorKind::NotFound => false,
            Err(e) => {
                log::warn!("discard: a deleted meeting's audio could not be removed: {e}");
                true
            }
        },
        Some(Ok(None)) => false,
        Some(Err(e)) => {
            log::warn!("discard: a deleted meeting's audio was left alone: {e}");
            true
        }
        None => {
            log::warn!("discard: a deleted meeting's audio directory has no name; left alone");
            true
        }
    };
    // As record.delete: this answer reports the scrub's state, so it takes the change too.
    let _ = shared.store.scrub_change();
    Ok(Discarded {
        audio_left,
        scrubbed: !shared.store.unscrubbed(),
    })
}
