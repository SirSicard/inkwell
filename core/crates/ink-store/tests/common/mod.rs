//! Helpers shared by the integration tests.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use ink_core::{Channel, NewRecord, RecordId, RecordKind, Segment, Store};
use ink_store::SqliteStore;

/// A fresh directory under the system temp dir, removed on drop. The store expects its directory
/// to exist, the way the app's data directory does.
pub struct TempDb {
    dir: PathBuf,
}

impl TempDb {
    pub fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ink-store-test-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join("inkwell.sqlite")
    }

    pub fn open(&self) -> SqliteStore {
        SqliteStore::open(self.path()).unwrap()
    }

    /// A second, raw connection to the same file, for looking at rows the trait does not expose.
    pub fn raw(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.path()).unwrap()
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub fn seg(channel: Channel, start_ms: u64, text: &str) -> Segment {
    Segment {
        channel,
        start_ms,
        end_ms: start_ms + 1_000,
        text: text.into(),
        speaker: None,
    }
}

pub fn meeting(store: &dyn Store, started: i64) -> RecordId {
    store
        .create_record(NewRecord {
            kind: RecordKind::Meeting,
            title: None,
            started_at_unix_ms: started,
            source_app: Some("com.example.meet".into()),
            audio_dir: None,
        })
        .unwrap()
}
