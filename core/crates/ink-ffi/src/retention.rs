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
//! **When.** Never on a timer (nothing ticks while idle): at launch, after each meeting's final
//! pass, and when the setting changes. A record without an end (a meeting live now, or one a crash
//! interrupted, which recovery finishes first) is never swept.

use std::sync::Mutex;

use ink_core::{RecordCursor, RecordId, RecordQuery, StoreError};

use crate::events::event;
use crate::runtime::Shared;

/// The setting: how long records are kept.
pub const RETENTION_KEY: &str = "retention.days";

/// Its values: `forever`, or days.
pub const RETENTION_VALUES: &[&str] = &["forever", "7", "30", "90", "365"];

/// Records read per page while sweeping.
const PAGE: usize = 200;

const DAY_MS: i64 = 86_400_000;

/// One sweep at a time: the meeting worker, the screens' thread and the launch may each start one.
static SWEEPING: Mutex<()> = Mutex::new(());

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

/// **Worker.** Deletes every ended record that started before the setting's cut-off. `None` when
/// the setting keeps everything; otherwise what it did, also sent as `library.swept` when it did
/// anything.
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
    loop {
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
            if record.ended_at_unix_ms.is_none() {
                continue;
            }
            match shared.store.delete_record(&record.id) {
                Ok(()) | Err(StoreError::NotFound) => {}
                Err(e) => {
                    log::warn!("retention: a record could not be deleted: {e}");
                    swept.failed += 1;
                    continue;
                }
            }
            swept.deleted += 1;
            if let Some(relative) = &record.audio_dir {
                match crate::library::audio_dir(&shared.data_dir, relative) {
                    Ok(Some(dir)) => {
                        if let Err(e) = std::fs::remove_dir_all(&dir) {
                            log::warn!(
                                "retention: a deleted record's audio could not be removed: {e}"
                            );
                            swept.failed += 1;
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        // Never deleted outside the library, whatever a record says.
                        log::warn!("retention: a deleted record's audio was left alone: {e}");
                    }
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
