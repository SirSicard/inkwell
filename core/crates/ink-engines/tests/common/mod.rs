//! Shared test helpers: synthetic rows, a scratch directory, and an in-memory fetcher. Nothing
//! here touches the network or a real model.

#![allow(dead_code)] // Each test binary uses a different subset.

use std::collections::HashMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use ink_core::{CancelToken, Job};
use ink_engines::{
    EngineRow, Fetch, FetchError, Fetched, JobScore, ModelDir, ModelFile, Os, Runtime,
};
use sha2::{Digest, Sha256};

/// A revision that looks like a real commit hash and is not one.
pub const REV: &str = "0123456789abcdef0123456789abcdef01234567";

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Deterministic synthetic "weights": `len` bytes that differ per `seed`.
pub fn weights(seed: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

pub fn url_for(id: &str, name: &str) -> String {
    format!("https://models.example/synthetic/{id}/resolve/{REV}/{name}")
}

pub fn file(id: &str, name: &str, bytes: &[u8]) -> ModelFile {
    ModelFile {
        name: name.into(),
        url: url_for(id, name),
        sha256: sha256_hex(bytes),
        size: bytes.len() as u64,
    }
}

/// A valid synthetic row with one file whose content is `bytes`.
pub fn row_with(id: &str, scores: &[(Job, f32)], oses: &[Os], bytes: &[u8]) -> EngineRow {
    EngineRow {
        id: id.into(),
        scores: scores
            .iter()
            .map(|&(job, wer)| JobScore { job, wer })
            .collect(),
        files: vec![file(id, "weights.bin", bytes)],
        revision: REV.into(),
        licence: "MIT".into(),
        oses: oses.to_vec(),
        runtime: Runtime::LlamaCpp,
    }
}

/// A valid synthetic row for both OSes.
pub fn row(id: &str, scores: &[(Job, f32)]) -> EngineRow {
    row_with(id, scores, &[Os::MacOs, Os::Windows], id.as_bytes())
}

/// A directory under the system temp dir, removed on drop. Works on macOS and Windows.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(tag: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("ink-engines-test-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn model_dir(&self) -> ModelDir {
        ModelDir::new(self.0.join("models"))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Writes `row`'s files into `dir` as if they had been downloaded, so the router sees it
/// installed. `content` is written for every file, padded or cut to the registry size.
pub fn install(dir: &ModelDir, row: &EngineRow) {
    for f in &row.files {
        let path = dir.file_path(row, f);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, vec![0u8; f.size as usize]).unwrap();
    }
}

/// How the in-memory server behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeMode {
    /// Honours `Range` (HTTP 206).
    Honour,
    /// Ignores it and sends the whole file (HTTP 200).
    Ignore,
}

/// Opens when the test says so; a body read blocks on it.
#[derive(Default)]
pub struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
}

impl Gate {
    pub fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.changed.notify_all();
    }

    pub fn wait(&self) {
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.changed.wait(open).unwrap();
        }
    }
}

#[derive(Clone)]
pub struct Served {
    pub bytes: Vec<u8>,
    /// What the server declares as the file's size; `None` declares the real length.
    pub declared_total: Option<Option<u64>>,
    /// Stop the body after this many bytes of the file (a dropped connection).
    pub cut_at: Option<u64>,
}

/// An in-memory server. Records every request as `(url, offset)`.
pub struct MemFetch {
    files: Mutex<HashMap<String, Served>>,
    pub mode: Mutex<RangeMode>,
    pub requests: Mutex<Vec<(String, u64)>>,
    /// Largest read a body returns, so a download takes many chunks.
    pub chunk: usize,
    /// Cancel this token once this many body bytes have been read in total.
    pub cancel_after: Mutex<Option<(u64, CancelToken)>>,
    /// Bodies block before their first byte until this opens, when set.
    pub gate: Mutex<Option<Arc<Gate>>>,
    read_total: Arc<AtomicU64>,
}

impl MemFetch {
    pub fn new() -> Self {
        Self {
            files: Mutex::default(),
            mode: Mutex::new(RangeMode::Honour),
            requests: Mutex::default(),
            chunk: 1000,
            cancel_after: Mutex::default(),
            gate: Mutex::default(),
            read_total: Arc::default(),
        }
    }

    /// Serves `bytes` at the URL `row` uses for its file `name`.
    pub fn serve(&self, url: &str, bytes: &[u8]) {
        self.serve_with(
            url,
            Served {
                bytes: bytes.to_vec(),
                declared_total: None,
                cut_at: None,
            },
        );
    }

    pub fn serve_with(&self, url: &str, served: Served) {
        self.files.lock().unwrap().insert(url.into(), served);
    }

    pub fn requests(&self) -> Vec<(String, u64)> {
        self.requests.lock().unwrap().clone()
    }
}

struct Body {
    data: Vec<u8>,
    pos: usize,
    chunk: usize,
    read_total: Arc<AtomicU64>,
    cancel_after: Option<(u64, CancelToken)>,
    gate: Option<Arc<Gate>>,
}

impl Read for Body {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(gate) = self.gate.take() {
            gate.wait();
        }
        let n = buf.len().min(self.chunk).min(self.data.len() - self.pos);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        let total = self.read_total.fetch_add(n as u64, Ordering::SeqCst) + n as u64;
        if let Some((after, token)) = &self.cancel_after
            && total >= *after
        {
            token.cancel();
        }
        Ok(n)
    }
}

impl Fetch for MemFetch {
    fn get(&self, url: &str, offset: u64) -> Result<Fetched, FetchError> {
        // Taken before the request is recorded, so a test that waits for the request and then
        // clears the gate cannot race this body out of it.
        let gate = self.gate.lock().unwrap().clone();
        self.requests.lock().unwrap().push((url.into(), offset));
        let served = self
            .files
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .ok_or(FetchError::Http { status: 404 })?;
        let len = served.bytes.len() as u64;
        let end = served.cut_at.unwrap_or(len).min(len) as usize;
        let start = match *self.mode.lock().unwrap() {
            RangeMode::Honour => offset,
            RangeMode::Ignore => 0,
        };
        if start > len {
            return Err(FetchError::Http { status: 416 });
        }
        let data = served.bytes[start as usize..end.max(start as usize)].to_vec();
        Ok(Fetched {
            start,
            total: served.declared_total.unwrap_or(Some(len)),
            body: Box::new(Body {
                data,
                pos: 0,
                chunk: self.chunk,
                read_total: Arc::clone(&self.read_total),
                cancel_after: self.cancel_after.lock().unwrap().clone(),
                gate,
            }),
        })
    }
}
