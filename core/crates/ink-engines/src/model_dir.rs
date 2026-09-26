//! Where model files live on disk.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::registry::{EngineRow, ModelFile};

/// Suffix of a file being downloaded. A registry file name never ends in it, so a part file can
/// never be mistaken for, or collide with, a finished one.
pub const PART_SUFFIX: &str = ".part";

/// The file in a row's directory that holds the row's full revision. It starts with `.`, which
/// registry file names may not, so it can never collide with a model file.
pub const REVISION_MARKER: &str = ".revision";

/// How many leading hex digits of a row's revision name its directory. Unique enough within one
/// id (a row changes revision a handful of times), and short enough for Windows paths. The row
/// itself keeps, and is validated against, the full revision.
pub const REVISION_DIR_LEN: usize = 12;

/// The longest path below a [`ModelDir`] root, in characters: id, revision directory and file
/// name at their limits plus separators and [`PART_SUFFIX`] is 64 + 1 + 12 + 1 + 64 + 5 = 147.
/// The revision marker and its temporary file (`.revision.tmp`, 13 characters) are shorter.
///
/// Windows' classic `MAX_PATH` is 260 characters including the drive and the terminating NUL, so a
/// root of up to about 100 characters keeps every model path inside it. The Windows shell picks a
/// short root under its local app data. As a second line of defence the Windows app manifest
/// declares `longPathAware` (set when the WinUI solution is created), which lifts the limit where
/// the OS allows it.
pub const MAX_RELATIVE_PATH_LEN: usize = 150;

/// The directory models are installed in: `<root>/<row id>/<revision prefix>/<file name>`.
///
/// The directory is named by the revision's first [`REVISION_DIR_LEN`] hex digits, which keeps
/// paths short but does not by itself tell two revisions apart. What does is the row's
/// [`REVISION_MARKER`]: the full revision, written after every file of the row has been verified
/// and moved into place, and removed before a download changes anything in the directory. A row is
/// installed only when its marker holds exactly its revision, so files left by another revision
/// that shares the prefix, or by an interrupted install, are never taken for this one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelDir {
    root: PathBuf,
}

impl ModelDir {
    /// Models under `root` (the app's data directory picks it; the core does not guess).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The directory holding one row's files.
    pub fn row_dir(&self, row: &EngineRow) -> PathBuf {
        // A validated revision is 40 or 64 ASCII hex digits, so the prefix always exists; an
        // unvalidated shorter or non-ASCII one is used whole rather than panicking here.
        let revision = row
            .revision
            .get(..REVISION_DIR_LEN)
            .unwrap_or(&row.revision);
        self.root.join(&row.id).join(revision)
    }

    /// Where the row's revision marker lives.
    pub fn marker_path(&self, row: &EngineRow) -> PathBuf {
        self.row_dir(row).join(REVISION_MARKER)
    }

    /// Where a finished file lives.
    pub fn file_path(&self, row: &EngineRow, file: &ModelFile) -> PathBuf {
        self.row_dir(row).join(&file.name)
    }

    /// Where a file lives while it downloads.
    pub fn part_path(&self, row: &EngineRow, file: &ModelFile) -> PathBuf {
        self.row_dir(row)
            .join(format!("{}{PART_SUFFIX}", file.name))
    }

    /// **Worker.** Whether `row` is installed: its marker holds exactly its revision, and every
    /// file is in place with its registry size.
    ///
    /// Size, not hash, once the marker matches: the marker is only written after every file's
    /// hash was checked, and hashing gigabytes on every routing decision would stall the caller.
    pub fn is_installed(&self, row: &EngineRow) -> bool {
        self.marker_matches(row)
            && row.files.iter().all(|f| {
                fs::metadata(self.file_path(row, f)).is_ok_and(|m| m.is_file() && m.len() == f.size)
            })
    }

    /// Whether the row's marker holds exactly its revision. A missing, unreadable or different
    /// marker (including one with trailing whitespace) does not.
    pub(crate) fn marker_matches(&self, row: &EngineRow) -> bool {
        // Read at most one byte past the longest valid revision, so a stray large file is not
        // read whole on every routing decision.
        let mut marker = Vec::new();
        File::open(self.marker_path(row))
            .and_then(|f| f.take(MAX_MARKER_LEN + 1).read_to_end(&mut marker))
            .is_ok_and(|_| marker == row.revision.as_bytes())
    }

    /// Removes the row's marker; a missing one is fine.
    pub(crate) fn remove_marker(&self, row: &EngineRow) -> io::Result<()> {
        match fs::remove_file(self.marker_path(row)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Writes the row's marker atomically: a temporary file, flushed, then renamed over the marker,
    /// so a crash leaves either no marker or a complete one.
    pub(crate) fn write_marker(&self, row: &EngineRow) -> io::Result<()> {
        let marker = self.marker_path(row);
        let temp = marker.with_file_name(format!("{REVISION_MARKER}{MARKER_TEMP_SUFFIX}"));
        let mut file = File::create(&temp)?;
        file.write_all(row.revision.as_bytes())?;
        file.sync_all()?;
        // Closed before the rename: Windows refuses to rename an open file.
        drop(file);
        fs::rename(&temp, &marker)
    }
}

/// The longest revision a valid row has (a SHA-256 commit hash).
const MAX_MARKER_LEN: u64 = 64;

/// Suffix of the marker's temporary file while it is written.
const MARKER_TEMP_SUFFIX: &str = ".tmp";
