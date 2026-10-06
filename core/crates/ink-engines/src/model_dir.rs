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

/// The longest path below a [`ModelDir`] root, in characters: id, revision directory and a file
/// name of one component at their limits plus separators and [`PART_SUFFIX`] is
/// 64 + 1 + 12 + 1 + 64 + 5 = 147. A file name with subdirectories may be longer than one
/// component; registry validation refuses a row whose paths would pass this. The revision marker
/// and its temporary file (`.revision.tmp`, 13 characters) are shorter.
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

    /// Where a finished file lives: in a subdirectory of the row's when its name has `/`.
    pub fn file_path(&self, row: &EngineRow, file: &ModelFile) -> PathBuf {
        below(self.row_dir(row), &file.name)
    }

    /// Where a file lives while it downloads, next to where it will be.
    pub fn part_path(&self, row: &EngineRow, file: &ModelFile) -> PathBuf {
        below(self.row_dir(row), &format!("{}{PART_SUFFIX}", file.name))
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

    /// **Worker.** How many of `row`'s bytes are on disk already: each finished file at its
    /// registry size, else its part file, up to that size. What a download still needs is the
    /// row's total less this. (A finished file another revision left is counted, though the
    /// download may find its hash wrong and fetch it again.)
    pub fn bytes_on_disk(&self, row: &EngineRow) -> u64 {
        row.files
            .iter()
            .map(|f| {
                let len = |path: PathBuf| {
                    fs::metadata(path)
                        .ok()
                        .filter(fs::Metadata::is_file)
                        .map(|m| m.len())
                };
                match len(self.file_path(row, f)) {
                    Some(n) if n == f.size => f.size,
                    _ => len(self.part_path(row, f)).map_or(0, |n| n.min(f.size)),
                }
            })
            .sum()
    }

    /// **Worker.** Deletes everything installed for row `row.id`: its directory under the root,
    /// with every revision's files, part files and markers. Nothing there is fine.
    ///
    /// The row is validated first, so its id is one safe name below the root. The directory must
    /// be a real directory directly inside the root: a link (or a Windows junction) in its place,
    /// or a path that resolves anywhere else, is refused and nothing is deleted. The markers go
    /// first, so a delete that fails half way leaves the row not installed rather than installed
    /// with files missing. A loaded model must be unloaded first: Windows refuses to delete an
    /// open file.
    pub fn remove_row(&self, row: &EngineRow) -> io::Result<()> {
        row.validate()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
        let dir = self.root.join(&row.id);
        let meta = match fs::symlink_metadata(&dir) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{}'s model directory is not a plain directory", row.id),
            ));
        }
        let (root, resolved) = (fs::canonicalize(&self.root)?, fs::canonicalize(&dir)?);
        if resolved.parent() != Some(root.as_path()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{}'s model directory resolves outside the models root",
                    row.id
                ),
            ));
        }
        for revision in fs::read_dir(&resolved)? {
            let revision = revision?;
            if revision.file_type()?.is_dir() {
                match fs::remove_file(revision.path().join(REVISION_MARKER)) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                    _ => {}
                }
            }
        }
        // Does not follow links inside it: a link is removed, never what it points to.
        fs::remove_dir_all(&resolved)
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

/// `dir` joined with each `/`-separated name of `name`, so the path uses the OS's own separator.
/// Registry validation keeps every name inside the row's directory.
fn below(dir: PathBuf, name: &str) -> PathBuf {
    name.split('/').fold(dir, |path, part| path.join(part))
}

/// The longest revision a valid row has (a SHA-256 commit hash).
const MAX_MARKER_LEN: u64 = 64;

/// Suffix of the marker's temporary file while it is written.
const MARKER_TEMP_SUFFIX: &str = ".tmp";
