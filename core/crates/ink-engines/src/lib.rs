//! Engines: which model does each job, getting its files, and keeping it loaded.
//!
//! | Piece | Does |
//! |---|---|
//! | [`Registry`] | Rows of data: id, jobs with measured error rates, files (URL, sha256, size), pinned revision, licence, OSes, runtime. A row that is not pinned to a commit is refused. |
//! | [`Downloader`] | Fetches a row's files: resumes part files, checks size and SHA-256 before a file is moved into place, stops on a [`CancelToken`](ink_core::CancelToken), reports progress. |
//! | [`Router`] | Job → the installed engine for this OS with the lowest measured error rate. Engines the shell registers over the C ABI compete on the same terms. |
//! | [`Residency`] | Keeps the dictation model warm, loads others on demand, unloads what has been idle for five minutes, never loads two copies. |
//!
//! How the pipeline uses them: [`Router::route`] a job; a [`Route::External`] engine is called
//! directly, a [`Route::Model`] is loaded with [`Residency::acquire`] and called through the
//! [`Lease`]. Adapters (llama.cpp, sherpa-onnx, NeMo-Speech.cpp) implement [`Loader`] behind cargo
//! features; the mock engine for tests is `ink_core::mock::MockEngine`.
//!
//! Everything here runs on worker threads, is `Send + Sync`, and holds no lock across a load, a
//! download or an engine call.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod download;
#[cfg(feature = "http")]
mod http;
mod model_dir;
mod registry;
mod residency;
mod router;

pub use download::{DownloadError, DownloadProgress, Downloader, Fetch, FetchError, Fetched};
#[cfg(feature = "http")]
pub use http::HttpFetch;
pub use model_dir::{
    MAX_RELATIVE_PATH_LEN, ModelDir, PART_SUFFIX, REVISION_DIR_LEN, REVISION_MARKER,
};
pub use registry::{
    ALLOWED_WEIGHT_LICENCES, EngineRow, JobScore, MAX_NAME_LEN, ModelFile, Os, Registry,
    RegistryError, Runtime, builtin_rows,
};
pub use residency::{IDLE_UNLOAD, Lease, Loader, Residency};
pub use router::{ExternalEngine, Route, RouteError, Router};

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Locks, ignoring poison. Every critical section in this crate is a few map or set operations
/// that leave the data consistent at each step, so a panic elsewhere while the lock was held does
/// not make the data wrong, and wedging every later caller would turn one failure into many.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
