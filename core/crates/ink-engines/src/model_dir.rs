//! Where model files live on disk.

use std::fs;
use std::path::{Path, PathBuf};

use crate::registry::{EngineRow, ModelFile};

/// Suffix of a file being downloaded. A registry file name never ends in it, so a part file can
/// never be mistaken for, or collide with, a finished one.
pub const PART_SUFFIX: &str = ".part";

/// The directory models are installed in: `<root>/<row id>/<revision>/<file name>`.
///
/// The revision is part of the path so a row that moves to a new revision downloads fresh files
/// instead of trusting same-named files from the old one.
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
        self.root.join(&row.id).join(&row.revision)
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
