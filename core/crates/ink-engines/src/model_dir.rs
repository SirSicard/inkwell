//! Where model files live on disk.

use std::fs;
use std::path::{Path, PathBuf};

use crate::registry::{EngineRow, ModelFile};

/// Suffix of a file being downloaded. A registry file name never ends in it, so a part file can
/// never be mistaken for, or collide with, a finished one.
pub const PART_SUFFIX: &str = ".part";

/// How many leading hex digits of a row's revision name its directory. Unique enough within one
/// id (a row changes revision a handful of times), and short enough for Windows paths. The row
/// itself keeps, and is validated against, the full revision.
pub const REVISION_DIR_LEN: usize = 12;

/// The longest path below a [`ModelDir`] root, in characters: id, revision directory and file
/// name at their limits plus separators and [`PART_SUFFIX`] is 64 + 1 + 12 + 1 + 64 + 5 = 147.
///
/// Windows' classic `MAX_PATH` is 260 characters including the drive and the terminating NUL, so a
/// root of up to about 100 characters keeps every model path inside it. The Windows shell picks a
/// short root under its local app data. As a second line of defence the Windows app manifest
/// declares `longPathAware` (set when the WinUI solution is created), which lifts the limit where
/// the OS allows it.
pub const MAX_RELATIVE_PATH_LEN: usize = 150;

/// The directory models are installed in: `<root>/<row id>/<revision prefix>/<file name>`.
///
/// The revision (its first [`REVISION_DIR_LEN`] hex digits) is part of the path so a row that
/// moves to a new revision downloads fresh files instead of trusting same-named files from the
/// old one.
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

    /// Where a finished file lives.
    pub fn file_path(&self, row: &EngineRow, file: &ModelFile) -> PathBuf {
        self.row_dir(row).join(&file.name)
    }

    /// Where a file lives while it downloads.
    pub fn part_path(&self, row: &EngineRow, file: &ModelFile) -> PathBuf {
        self.row_dir(row)
            .join(format!("{}{PART_SUFFIX}", file.name))
    }

    /// **Worker.** Whether every file of `row` is in place with its registry size.
    ///
    /// Size, not hash: a file only reaches its final name after its hash was checked, and hashing
    /// gigabytes on every routing decision would stall the caller.
    pub fn is_installed(&self, row: &EngineRow) -> bool {
        row.files.iter().all(|f| {
            fs::metadata(self.file_path(row, f)).is_ok_and(|m| m.is_file() && m.len() == f.size)
        })
    }
}
