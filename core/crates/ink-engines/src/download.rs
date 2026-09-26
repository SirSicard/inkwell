//! The model downloader: resumable, hash-checked, cancellable.
//!
//! Each file downloads to `<name>.part` next to its final place. The part is appended to on a
//! retry (an HTTP Range request from its length), its SHA-256 is checked against the registry once
//! it has every byte, and only then is it renamed into place. A finished file is therefore always a
//! verified one, which is what lets [`ModelDir::is_installed`] check sizes only.

use std::collections::HashSet;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

use ink_core::{CancelToken, EventSink};
use sha2::{Digest, Sha256};

use crate::lock;
use crate::model_dir::ModelDir;
use crate::registry::{EngineRow, ModelFile, RegistryError};

/// The transport: a ranged GET. The real one is [`HttpFetch`](crate::HttpFetch) (feature `http`);
/// tests use an in-memory one.
pub trait Fetch: Send + Sync {
    /// **Worker.** Starts a GET of `url`, asking for the bytes from `offset` on when `offset > 0`
    /// (`Range: bytes=<offset>-`). A server may ignore the range and send the whole file; the
    /// answer's [`start`](Fetched::start) says which happened.
    fn get(&self, url: &str, offset: u64) -> Result<Fetched, FetchError>;
}

/// A response body and where it starts.
pub struct Fetched {
    /// The file offset of the body's first byte: the requested offset when the range was honoured
    /// (HTTP 206), 0 when the server sent the whole file (HTTP 200).
    pub start: u64,
    /// The whole remote file's size, when the server said (Content-Length of a 200, the total of
    /// a 206's Content-Range).
    pub total: Option<u64>,
    /// The body.
    pub body: Box<dyn Read + Send>,
}

/// Why a fetch failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FetchError {
    /// The server answered with an error status.
    Http {
        /// The status code.
        status: u16,
    },
    /// No usable answer: DNS, TLS, a dropped connection, a malformed header.
    Network(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http { status } => write!(f, "server returned HTTP {status}"),
            Self::Network(msg) => write!(f, "network: {msg}"),
        }
    }
}

impl std::error::Error for FetchError {}

/// Progress of one row's download, for the UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadProgress {
    /// The row.
    pub id: String,
    /// Bytes on disk so far across the row's files, finished or partial. It can go down: when a
    /// server ignores a resume request the part file starts over.
    pub done: u64,
    /// The row's total size.
    pub total: u64,
}

/// Why a download stopped. Whether the part file was kept is part of each variant's contract,
/// because it decides whether the next attempt resumes or starts over.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DownloadError {
    /// The row failed validation; nothing was fetched.
    InvalidRow(RegistryError),
    /// This row is already downloading on another thread; nothing was touched.
    AlreadyRunning {
        /// The row.
        id: String,
    },
    /// The transport failed. The part file is kept for a resume.
    Fetch {
        /// The file.
        file: String,
        /// What went wrong.
        error: FetchError,
    },
    /// The server's file is not the size the registry says. Nothing it sent is kept.
    SizeMismatch {
        /// The file.
        file: String,
        /// The registry's size.
        expected: u64,
        /// What the server declared or sent (when it sent too much, the bytes seen before
        /// stopping).
        actual: u64,
    },
    /// The body ended before the file was complete. The part file is kept for a resume.
    Incomplete {
        /// The file.
        file: String,
        /// The registry's size.
        expected: u64,
        /// Bytes on disk.
        received: u64,
    },
    /// The finished file's SHA-256 is not the registry's. The part file was deleted, so the next
    /// attempt starts over.
    HashMismatch {
        /// The file.
        file: String,
        /// The registry's hash.
        expected: String,
        /// The downloaded file's hash.
        actual: String,
    },
    /// The [`CancelToken`] was set. The part file is kept for a resume.
    Cancelled,
    /// A local file operation failed.
    Io {
        /// The file.
        file: String,
        /// What failed.
        message: String,
    },
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRow(e) => write!(f, "refused to download: {e}"),
            Self::AlreadyRunning { id } => write!(f, "{id} is already downloading"),
            Self::Fetch { file, error } => write!(f, "downloading {file}: {error}"),
            Self::SizeMismatch {
                file,
                expected,
                actual,
            } => write!(
                f,
                "{file}: size mismatch, expected {expected} bytes, server has {actual}"
            ),
            Self::Incomplete {
                file,
                expected,
                received,
            } => write!(
                f,
                "{file}: download ended at {received} of {expected} bytes; it resumes next time"
            ),
            Self::HashMismatch {
                file,
                expected,
                actual,
            } => write!(
                f,
                "{file}: sha256 mismatch, expected {expected}, got {actual}; the partial file was deleted"
            ),
            Self::Cancelled => f.write_str("download cancelled"),
            Self::Io { file, message } => write!(f, "{file}: {message}"),
        }
    }
}

impl std::error::Error for DownloadError {}

/// Downloads registry rows into a [`ModelDir`].
///
/// `Send + Sync`: several worker threads may download different rows at once. The only lock guards
/// the set of rows in flight and is never held across I/O.
pub struct Downloader {
    fetch: Arc<dyn Fetch>,
    dir: ModelDir,
    running: Mutex<HashSet<String>>,
}

impl Downloader {
    /// A downloader writing into `dir` through `fetch`.
    pub fn new(fetch: Arc<dyn Fetch>, dir: ModelDir) -> Self {
        Self {
            fetch,
            dir,
            running: Mutex::default(),
        }
    }

    /// The directory it installs into.
    pub fn dir(&self) -> &ModelDir {
        &self.dir
    }

    /// **Worker.** Downloads every file of `row` that is not installed yet, resuming part files,
    /// and returns once all are verified and in place. Blocks for as long as the transfer takes;
    /// `cancel` is checked between chunks. `progress` runs on this thread and must not block.
    pub fn download(
        &self,
        row: &EngineRow,
        cancel: &CancelToken,
        progress: EventSink<DownloadProgress>,
    ) -> Result<(), DownloadError> {
        row.validate().map_err(DownloadError::InvalidRow)?;
        let _claim = self.claim(&row.id)?;
        let total = row.total_size();
        let mut base = 0;
        for file in &row.files {
            let file_base = base;
            let report = |on_disk: u64| {
                progress(DownloadProgress {
                    id: row.id.clone(),
                    done: file_base + on_disk,
                    total,
                })
            };
            self.download_file(row, file, cancel, &report)?;
            report(file.size);
            base += file.size;
        }
        Ok(())
    }

    /// Marks `id` as in flight until the returned claim drops.
    fn claim(&self, id: &str) -> Result<Claim<'_>, DownloadError> {
        if !lock(&self.running).insert(id.to_string()) {
            return Err(DownloadError::AlreadyRunning { id: id.to_string() });
        }
        Ok(Claim {
            running: &self.running,
            id: id.to_string(),
        })
    }

    fn download_file(
        &self,
        row: &EngineRow,
        file: &ModelFile,
        cancel: &CancelToken,
        report: &dyn Fn(u64),
    ) -> Result<(), DownloadError> {
        let final_path = self.dir.file_path(row, file);
        if fs::metadata(&final_path).is_ok_and(|m| m.is_file() && m.len() == file.size) {
            return Ok(());
        }
        let io = |what: &str, e: io::Error| DownloadError::Io {
            file: file.name.clone(),
            message: format!("{what}: {e}"),
        };
        let part_path = self.dir.part_path(row, file);
        fs::create_dir_all(self.dir.row_dir(row))
            .map_err(|e| io("creating the model directory", e))?;
        let mut part = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&part_path)
            .map_err(|e| io("opening the partial file", e))?;
        let mut have = part
            .metadata()
            .map_err(|e| io("reading the partial file", e))?
            .len();
        if have > file.size {
            // Longer than the file can be: not a prefix of it.
            part.set_len(0)
                .map_err(|e| io("truncating the partial file", e))?;
            have = 0;
        }

        // Hash what is already on disk before asking for the rest, so the connection is not left
        // idle while gigabytes are re-read.
        let mut hasher = Sha256::new();
        hash_prefix(&mut part, have, &mut hasher, cancel).map_err(|e| match e {
            PrefixError::Cancelled => DownloadError::Cancelled,
            PrefixError::Io(e) => io("reading the partial file", e),
        })?;
        report(have);

        if have < file.size {
            if cancel.is_cancelled() {
                return Err(DownloadError::Cancelled);
            }
            let fetched =
                self.fetch
                    .get(&file.url, have)
                    .map_err(|error| DownloadError::Fetch {
                        file: file.name.clone(),
                        error,
                    })?;
            if let Some(actual) = fetched.total.filter(|&t| t != file.size) {
                return Err(DownloadError::SizeMismatch {
                    file: file.name.clone(),
                    expected: file.size,
                    actual,
                });
            }
            if fetched.start == 0 && have > 0 {
                // The server ignored the range and sent the whole file: start the part over.
                part.set_len(0)
                    .map_err(|e| io("truncating the partial file", e))?;
                hasher = Sha256::new();
                have = 0;
                report(0);
            } else if fetched.start != have {
                return Err(DownloadError::Fetch {
                    file: file.name.clone(),
                    error: FetchError::Network(format!(
                        "asked for bytes from {have}, the server sent bytes from {}",
                        fetched.start
                    )),
                });
            }
            part.seek(SeekFrom::Start(have))
                .map_err(|e| io("seeking in the partial file", e))?;

            let mut body = fetched.body;
            let mut buf = vec![0u8; CHUNK];
            let mut reported = have;
            loop {
                if cancel.is_cancelled() {
                    return Err(DownloadError::Cancelled);
                }
                let n = match body.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        return Err(DownloadError::Fetch {
                            file: file.name.clone(),
                            error: FetchError::Network(e.to_string()),
                        });
                    }
                };
                let after = have + n as u64;
                if after > file.size {
                    // More bytes than the registry's size: this is some other file. Stop reading
                    // (a server could send forever) and keep none of it.
                    drop(part);
                    fs::remove_file(&part_path)
                        .map_err(|e| io("deleting the oversized partial file", e))?;
                    return Err(DownloadError::SizeMismatch {
                        file: file.name.clone(),
                        expected: file.size,
                        actual: after,
                    });
                }
                part.write_all(&buf[..n])
                    .map_err(|e| io("writing the partial file", e))?;
                hasher.update(&buf[..n]);
                have = after;
                if have - reported >= PROGRESS_STEP {
                    report(have);
                    reported = have;
                }
            }
            if have < file.size {
                return Err(DownloadError::Incomplete {
                    file: file.name.clone(),
                    expected: file.size,
                    received: have,
                });
            }
        }

        part.sync_all()
            .map_err(|e| io("flushing the partial file", e))?;
        // Closed before it is renamed or deleted: Windows refuses either on an open file.
        drop(part);
        let actual = hex(&hasher.finalize());
        if actual != file.sha256 {
            if let Err(e) = fs::remove_file(&part_path) {
                return Err(io(
                    &format!(
                        "sha256 mismatch (expected {}, got {actual}), and deleting the partial file failed",
                        file.sha256
                    ),
                    e,
                ));
            }
            return Err(DownloadError::HashMismatch {
                file: file.name.clone(),
                expected: file.sha256.clone(),
                actual,
            });
        }
        fs::rename(&part_path, &final_path)
            .map_err(|e| io("moving the verified file into place", e))?;
        Ok(())
    }
}

/// Bytes read or written per step; cancellation is checked between steps.
const CHUNK: usize = 64 * 1024;

/// Progress is reported at most once per this many bytes, plus at the start and end of each file.
const PROGRESS_STEP: u64 = 1 << 20;

/// Removes a row from the in-flight set when its download returns, however it returns.
struct Claim<'a> {
    running: &'a Mutex<HashSet<String>>,
    id: String,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        lock(self.running).remove(&self.id);
    }
}

enum PrefixError {
    Cancelled,
    Io(io::Error),
}

/// Feeds the first `len` bytes of `file` to `hasher`.
fn hash_prefix(
    file: &mut File,
    len: u64,
    hasher: &mut Sha256,
    cancel: &CancelToken,
) -> Result<(), PrefixError> {
    if len == 0 {
        return Ok(());
    }
    file.seek(SeekFrom::Start(0)).map_err(PrefixError::Io)?;
    let mut prefix = file.take(len);
    let mut buf = vec![0u8; CHUNK];
    let mut read = 0;
    while read < len {
        if cancel.is_cancelled() {
            return Err(PrefixError::Cancelled);
        }
        let n = match prefix.read(&mut buf) {
            Ok(0) => {
                return Err(PrefixError::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the partial file shrank while it was read",
                )));
            }
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(PrefixError::Io(e)),
        };
        hasher.update(&buf[..n]);
        read += n as u64;
    }
    Ok(())
}

/// Lowercase hex, the form the registry stores hashes in.
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_published_vectors() {
        assert_eq!(
            hex(&Sha256::digest(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
