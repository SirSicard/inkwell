//! The downloader, against an in-memory server: no network.

mod common;

use std::fs;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{
    Gate, MemFetch, REV, RangeMode, Scratch, Served, file, row_with, sha256_hex, weights,
};
use ink_core::{CancelToken, EventSink, Job};
use ink_engines::{
    DownloadError, DownloadProgress, Downloader, EngineRow, ModelDir, ModelFile, Os,
    REVISION_DIR_LEN, REVISION_MARKER, RegistryError,
};

const LEN: usize = 10_000;

fn one_file_row(id: &str, bytes: &[u8]) -> EngineRow {
    row_with(
        id,
        &[(Job::DictationFinal, 5.0)],
        &[Os::MacOs, Os::Windows],
        bytes,
    )
}

fn no_progress() -> EventSink<DownloadProgress> {
    Arc::new(|_| {})
}

fn recorder() -> (
    EventSink<DownloadProgress>,
    Arc<Mutex<Vec<DownloadProgress>>>,
) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink_seen = Arc::clone(&seen);
    (Arc::new(move |p| sink_seen.lock().unwrap().push(p)), seen)
}

struct Setup {
    _scratch: Scratch,
    dir: ModelDir,
    fetch: Arc<MemFetch>,
    dl: Downloader,
    row: EngineRow,
    bytes: Vec<u8>,
}

fn setup(tag: &str) -> Setup {
    let scratch = Scratch::new(tag);
    let dir = scratch.model_dir();
    let bytes = weights(7, LEN);
    let row = one_file_row("synthetic-asr", &bytes);
    let fetch = Arc::new(MemFetch::new());
    fetch.serve(&row.files[0].url, &bytes);
    let dl = Downloader::new(fetch.clone(), dir.clone());
    Setup {
        _scratch: scratch,
        dir,
        fetch,
        dl,
        row,
        bytes,
    }
}

impl Setup {
    fn final_path(&self) -> std::path::PathBuf {
        self.dir.file_path(&self.row, &self.row.files[0])
    }
    fn part_path(&self) -> std::path::PathBuf {
        self.dir.part_path(&self.row, &self.row.files[0])
    }
    fn write_part(&self, bytes: &[u8]) {
        fs::create_dir_all(self.dir.row_dir(&self.row)).unwrap();
        fs::write(self.part_path(), bytes).unwrap();
    }
    fn run(&self) -> Result<(), DownloadError> {
        self.dl
            .download(&self.row, &CancelToken::new(), no_progress())
    }
}

#[test]
fn a_fresh_download_verifies_and_installs() {
    let s = setup("fresh");
    let (sink, seen) = recorder();
    s.dl.download(&s.row, &CancelToken::new(), sink).unwrap();

    assert_eq!(fs::read(s.final_path()).unwrap(), s.bytes);
    assert!(!s.part_path().exists());
    assert!(s.dir.is_installed(&s.row));
    assert_eq!(s.fetch.requests(), vec![(s.row.files[0].url.clone(), 0)]);
    assert_eq!(
        fs::read_to_string(s.dir.marker_path(&s.row)).unwrap(),
        REV,
        "the marker holds the full revision"
    );

    let seen = seen.lock().unwrap();
    let last = seen.last().expect("progress was reported");
    assert_eq!((last.done, last.total), (LEN as u64, LEN as u64));
    assert!(seen.iter().all(|p| p.id == "synthetic-asr"));
    assert!(seen.windows(2).all(|w| w[0].done <= w[1].done));
}

#[test]
fn a_partial_file_resumes_from_its_length() {
    let s = setup("resume");
    s.write_part(&s.bytes[..4_321]);
    s.run().unwrap();

    assert_eq!(
        s.fetch.requests(),
        vec![(s.row.files[0].url.clone(), 4_321)],
        "asked for the rest only"
    );
    assert_eq!(fs::read(s.final_path()).unwrap(), s.bytes);
    assert!(!s.part_path().exists());
}

#[test]
fn a_hash_mismatch_is_rejected_and_the_part_file_deleted() {
    let s = setup("hash");
    let wrong = weights(8, LEN); // right size, wrong content
    s.fetch.serve(&s.row.files[0].url, &wrong);

    let err = s.run().unwrap_err();
    assert_eq!(
        err,
        DownloadError::HashMismatch {
            file: "weights.bin".into(),
            expected: s.row.files[0].sha256.clone(),
            actual: sha256_hex(&wrong),
        }
    );
    let msg = err.to_string();
    assert!(msg.contains(&s.row.files[0].sha256) && msg.contains(&sha256_hex(&wrong)));
    assert!(!s.part_path().exists(), "a bad part must not be resumed");
    assert!(!s.final_path().exists());
    assert!(!s.dir.is_installed(&s.row));
}

#[test]
fn a_corrupt_partial_file_fails_the_hash_and_the_retry_starts_over() {
    let s = setup("corrupt-part");
    s.write_part(&[0xAA; 3_000]);
    assert!(matches!(s.run(), Err(DownloadError::HashMismatch { .. })));
    assert!(!s.part_path().exists());

    s.run().unwrap();
    assert_eq!(fs::read(s.final_path()).unwrap(), s.bytes);
    assert_eq!(
        s.fetch.requests().last().unwrap().1,
        0,
        "retry started over"
    );
}

#[test]
fn a_body_longer_than_the_registry_size_is_rejected() {
    let s = setup("too-long");
    let mut longer = s.bytes.clone();
    longer.extend_from_slice(&[1, 2, 3]);
    // The server does not declare a size, so only the body can give it away.
    s.fetch.serve_with(
        &s.row.files[0].url,
        Served {
            bytes: longer,
            declared_total: Some(None),
            cut_at: None,
        },
    );
    let err = s.run().unwrap_err();
    match err {
        DownloadError::SizeMismatch {
            ref file,
            expected,
            actual,
        } => {
            assert_eq!(file, "weights.bin");
            assert_eq!(expected, LEN as u64);
            assert!(actual > LEN as u64, "{actual}");
        }
        other => panic!("expected a size mismatch, got {other:?}"),
    }
    assert!(!s.part_path().exists());
    assert!(!s.final_path().exists());
}

#[test]
fn a_declared_size_that_disagrees_is_rejected_before_writing() {
    let s = setup("declared");
    s.write_part(&s.bytes[..1_000]);
    s.fetch.serve_with(
        &s.row.files[0].url,
        Served {
            bytes: s.bytes.clone(),
            declared_total: Some(Some(LEN as u64 + 1)),
            cut_at: None,
        },
    );
    assert_eq!(
        s.run().unwrap_err(),
        DownloadError::SizeMismatch {
            file: "weights.bin".into(),
            expected: LEN as u64,
            actual: LEN as u64 + 1,
        }
    );
    // Nothing the mismatched server sent was written; the earlier part is untouched.
    assert_eq!(fs::read(s.part_path()).unwrap(), &s.bytes[..1_000]);
}

#[test]
fn a_body_that_ends_early_keeps_the_part_for_a_resume() {
    let s = setup("short");
    s.fetch.serve_with(
        &s.row.files[0].url,
        Served {
            bytes: s.bytes.clone(),
            declared_total: None,
            cut_at: Some(6_000),
        },
    );
    assert_eq!(
        s.run().unwrap_err(),
        DownloadError::Incomplete {
            file: "weights.bin".into(),
            expected: LEN as u64,
            received: 6_000,
        }
    );
    assert_eq!(fs::read(s.part_path()).unwrap(), &s.bytes[..6_000]);

    s.fetch.serve(&s.row.files[0].url, &s.bytes);
    s.run().unwrap();
    assert_eq!(s.fetch.requests().last().unwrap().1, 6_000);
    assert_eq!(fs::read(s.final_path()).unwrap(), s.bytes);
}

#[test]
fn cancelling_mid_download_leaves_a_resumable_part_file() {
    let s = setup("cancel");
    let cancel = CancelToken::new();
    *s.fetch.cancel_after.lock().unwrap() = Some((3_500, cancel.clone()));

    assert_eq!(
        s.dl.download(&s.row, &cancel, no_progress()),
        Err(DownloadError::Cancelled)
    );
    let part = fs::read(s.part_path()).unwrap();
    assert!(
        !part.is_empty() && part.len() < LEN,
        "part has {} bytes",
        part.len()
    );
    assert_eq!(
        part,
        &s.bytes[..part.len()],
        "the part holds a clean prefix"
    );
    assert!(!s.final_path().exists());

    *s.fetch.cancel_after.lock().unwrap() = None;
    s.run().unwrap();
    assert_eq!(
        s.fetch.requests().last().unwrap(),
        &(s.row.files[0].url.clone(), part.len() as u64),
        "resumed where the cancel stopped"
    );
    assert_eq!(fs::read(s.final_path()).unwrap(), s.bytes);
}

#[test]
fn a_cancelled_token_stops_before_fetching() {
    let s = setup("pre-cancel");
    let cancel = CancelToken::new();
    cancel.cancel();
    assert_eq!(
        s.dl.download(&s.row, &cancel, no_progress()),
        Err(DownloadError::Cancelled)
    );
    assert!(s.fetch.requests().is_empty());
}

#[test]
fn a_server_that_ignores_range_restarts_the_file_cleanly() {
    let s = setup("ignore-range");
    *s.fetch.mode.lock().unwrap() = RangeMode::Ignore;
    s.write_part(&s.bytes[..4_000]);

    s.run().unwrap();
    assert_eq!(
        s.fetch.requests(),
        vec![(s.row.files[0].url.clone(), 4_000)]
    );
    // Not the old prefix followed by the whole file again.
    assert_eq!(fs::read(s.final_path()).unwrap(), s.bytes);
}

#[test]
fn installed_files_are_not_fetched_again() {
    let s = setup("installed");
    s.run().unwrap();
    s.run().unwrap();
    assert_eq!(s.fetch.requests().len(), 1);
}

#[test]
fn a_part_file_longer_than_the_file_is_discarded() {
    let s = setup("long-part");
    s.write_part(&weights(1, LEN + 10));
    s.run().unwrap();
    assert_eq!(s.fetch.requests().last().unwrap().1, 0);
    assert_eq!(fs::read(s.final_path()).unwrap(), s.bytes);
}

#[test]
fn a_complete_part_file_is_verified_without_fetching() {
    let s = setup("complete-part");
    s.write_part(&s.bytes);
    s.run().unwrap();
    assert!(s.fetch.requests().is_empty());
    assert_eq!(fs::read(s.final_path()).unwrap(), s.bytes);
}

#[test]
fn every_file_of_a_row_is_downloaded_and_progress_covers_the_row() {
    let scratch = Scratch::new("multi");
    let dir = scratch.model_dir();
    let (a, b) = (weights(1, 2_500), weights(2, 3_700));
    let mut row = one_file_row("synthetic-two-files", &a);
    row.files = vec![
        file("synthetic-two-files", "model.bin", &a),
        file("synthetic-two-files", "projector.bin", &b),
    ];
    let fetch = Arc::new(MemFetch::new());
    fetch.serve(&row.files[0].url, &a);
    fetch.serve(&row.files[1].url, &b);
    let dl = Downloader::new(fetch.clone(), dir.clone());
    let (sink, seen) = recorder();

    dl.download(&row, &CancelToken::new(), sink).unwrap();
    assert_eq!(fs::read(dir.file_path(&row, &row.files[0])).unwrap(), a);
    assert_eq!(fs::read(dir.file_path(&row, &row.files[1])).unwrap(), b);
    assert!(dir.is_installed(&row));
    let last = seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!((last.done, last.total), (6_200, 6_200));
}

#[test]
fn an_unpinned_row_is_refused_before_any_fetch() {
    let mut s = setup("unpinned");
    s.row.files[0].url = "https://models.example/synthetic/x/resolve/main/weights.bin".into();
    assert!(matches!(
        s.run(),
        Err(DownloadError::InvalidRow(RegistryError::Unpinned { .. }))
    ));
    assert!(s.fetch.requests().is_empty());
}

#[test]
fn the_same_row_cannot_download_twice_at_once_but_others_can() {
    let s = Arc::new(setup("concurrent"));
    let gate = Arc::new(Gate::default());
    *s.fetch.gate.lock().unwrap() = Some(Arc::clone(&gate));

    // Thread 1 starts the row and blocks inside the transfer.
    let s1 = Arc::clone(&s);
    let first = std::thread::spawn(move || s1.run());
    // Wait until its request is in flight (bounded, so a regression fails instead of hanging).
    let deadline = Instant::now() + Duration::from_secs(20);
    while s.fetch.requests().is_empty() {
        assert!(
            Instant::now() < deadline,
            "the first download never fetched"
        );
        std::thread::yield_now();
    }
    *s.fetch.gate.lock().unwrap() = None;

    assert_eq!(
        s.run(),
        Err(DownloadError::AlreadyRunning {
            id: "synthetic-asr".into()
        })
    );

    // Another row downloads while the first is still blocked: no lock is held across a transfer.
    let other_bytes = weights(3, 1_234);
    let other = one_file_row("synthetic-other", &other_bytes);
    s.fetch.serve(&other.files[0].url, &other_bytes);
    let (tx, rx) = mpsc::channel();
    let s2 = Arc::clone(&s);
    let other2 = other.clone();
    std::thread::spawn(move || {
        let _ = tx.send(s2.dl.download(&other2, &CancelToken::new(), no_progress()));
    });
    let result = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("a second row blocked behind the first: a lock is held across a download");
    result.unwrap();
    assert!(s.dir.is_installed(&other));

    gate.open();
    first.join().unwrap().unwrap();
    assert!(s.dir.is_installed(&s.row));
    // Once finished, the row can be downloaded (a no-op) again.
    s.run().unwrap();
}

#[test]
fn the_downloader_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Downloader>();
}

/// Another commit of the same repository whose first 12 hex digits match [`REV`], so both
/// revisions of a row share one directory.
const REV_SAME_PREFIX: &str = "0123456789abffffffffffffffffffffffffffff";

/// `row` moved to `revision`, with one file (same name) whose content is now `bytes`.
fn at_revision(row: &EngineRow, revision: &str, bytes: &[u8]) -> EngineRow {
    let mut moved = row.clone();
    moved.revision = revision.into();
    let name = &row.files[0].name;
    moved.files = vec![ModelFile {
        name: name.clone(),
        url: format!(
            "https://models.example/synthetic/{}/resolve/{revision}/{name}",
            row.id
        ),
        sha256: sha256_hex(bytes),
        size: bytes.len() as u64,
    }];
    moved.validate().unwrap();
    moved
}

#[test]
fn a_new_revision_sharing_the_directory_prefix_is_fetched_and_verified() {
    let s = setup("new-revision");
    s.run().unwrap();
    let new_bytes = weights(9, LEN); // same name and size, different content
    let moved = at_revision(&s.row, REV_SAME_PREFIX, &new_bytes);
    assert_eq!(REV[..REVISION_DIR_LEN], REV_SAME_PREFIX[..REVISION_DIR_LEN]);
    assert_eq!(
        s.dir.row_dir(&moved),
        s.dir.row_dir(&s.row),
        "one directory"
    );

    assert!(
        !s.dir.is_installed(&moved),
        "the old revision's files must not pass for the new one"
    );
    s.fetch.serve(&moved.files[0].url, &new_bytes);
    s.dl.download(&moved, &CancelToken::new(), no_progress())
        .unwrap();

    assert_eq!(
        s.fetch.requests().last().unwrap(),
        &(moved.files[0].url.clone(), 0),
        "the new revision's file was fetched from the start"
    );
    assert_eq!(fs::read(s.final_path()).unwrap(), new_bytes);
    assert_eq!(
        fs::read_to_string(s.dir.marker_path(&moved)).unwrap(),
        REV_SAME_PREFIX
    );
    assert!(s.dir.is_installed(&moved));
    assert!(!s.dir.is_installed(&s.row), "and the old revision is gone");
}

#[test]
fn a_new_revision_with_identical_files_is_verified_locally_not_refetched() {
    let s = setup("same-bytes");
    s.run().unwrap();
    // A new commit that did not change this file: same bytes, same hash.
    let moved = at_revision(&s.row, REV_SAME_PREFIX, &s.bytes);
    assert!(!s.dir.is_installed(&moved));

    s.dl.download(&moved, &CancelToken::new(), no_progress())
        .unwrap();
    assert_eq!(
        s.fetch.requests().len(),
        1,
        "hashed on disk, not fetched again"
    );
    assert_eq!(
        fs::read_to_string(s.dir.marker_path(&moved)).unwrap(),
        REV_SAME_PREFIX
    );
    assert!(s.dir.is_installed(&moved));
}

#[test]
fn a_missing_or_foreign_marker_is_not_installed() {
    let s = setup("marker");
    s.run().unwrap();
    let marker = s.dir.marker_path(&s.row);
    assert!(s.dir.is_installed(&s.row));

    let foreign = [
        REV_SAME_PREFIX.to_string(),
        format!("{REV}\n"),
        REV.to_uppercase(),
        REV[..REVISION_DIR_LEN].to_string(),
        String::new(),
    ];
    for content in &foreign {
        fs::write(&marker, content).unwrap();
        assert!(
            !s.dir.is_installed(&s.row),
            "marker {content:?} was accepted"
        );
    }
    fs::remove_file(&marker).unwrap();
    assert!(!s.dir.is_installed(&s.row), "a missing marker was accepted");

    fs::write(&marker, REV).unwrap();
    assert!(s.dir.is_installed(&s.row));
}

#[test]
fn a_crash_before_the_marker_is_written_leaves_the_row_uninstalled_and_the_retry_rehashes() {
    let s = setup("crash");
    // Make the marker write fail after every file is verified and renamed: a directory where
    // the marker's temporary file goes. This is the state a crash between the last rename and
    // the marker write leaves on disk.
    let marker = s.dir.marker_path(&s.row);
    let blocker = marker.with_file_name(format!("{REVISION_MARKER}.tmp"));
    fs::create_dir_all(&blocker).unwrap();

    let err = s.run().unwrap_err();
    assert!(
        matches!(&err, DownloadError::Io { file, .. } if file == REVISION_MARKER),
        "expected the marker write to fail, got {err:?}"
    );
    assert_eq!(
        fs::read(s.final_path()).unwrap(),
        s.bytes,
        "the file is in place"
    );
    assert!(!marker.exists());
    assert!(!s.dir.is_installed(&s.row), "no marker, not installed");

    fs::remove_dir(&blocker).unwrap();
    s.run().unwrap();
    assert_eq!(
        s.fetch.requests().len(),
        1,
        "the verified file was hashed on disk, not fetched again"
    );
    assert_eq!(fs::read_to_string(&marker).unwrap(), REV);
    assert!(s.dir.is_installed(&s.row));
}

#[test]
fn an_unmarked_file_with_the_right_size_but_wrong_bytes_is_replaced() {
    let s = setup("unmarked-wrong");
    fs::create_dir_all(s.dir.row_dir(&s.row)).unwrap();
    fs::write(s.final_path(), weights(99, LEN)).unwrap();
    assert!(!s.dir.is_installed(&s.row));

    s.run().unwrap();
    assert_eq!(s.fetch.requests(), vec![(s.row.files[0].url.clone(), 0)]);
    assert_eq!(fs::read(s.final_path()).unwrap(), s.bytes);
    assert!(s.dir.is_installed(&s.row));
}

#[test]
fn a_file_missing_from_an_installed_row_is_fetched_alone() {
    let scratch = Scratch::new("missing-one");
    let dir = scratch.model_dir();
    let (a, b) = (weights(1, 2_500), weights(2, 3_700));
    let mut row = one_file_row("synthetic-two-files", &a);
    row.files = vec![
        file("synthetic-two-files", "model.bin", &a),
        file("synthetic-two-files", "projector.bin", &b),
    ];
    let fetch = Arc::new(MemFetch::new());
    fetch.serve(&row.files[0].url, &a);
    fetch.serve(&row.files[1].url, &b);
    let dl = Downloader::new(fetch.clone(), dir.clone());
    dl.download(&row, &CancelToken::new(), no_progress())
        .unwrap();

    fs::remove_file(dir.file_path(&row, &row.files[1])).unwrap();
    assert!(!dir.is_installed(&row));
    dl.download(&row, &CancelToken::new(), no_progress())
        .unwrap();
    assert_eq!(
        fetch.requests()[2..],
        [(row.files[1].url.clone(), 0)],
        "only the missing file"
    );
    assert!(dir.is_installed(&row));
}
