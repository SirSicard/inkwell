//! What the own-key provider tests share: a key store in memory and a transport that records what
//! it was sent and answers as scripted. Every key and text here is synthetic.

#![allow(dead_code)] // each test binary uses a different subset

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ink_engines::{EngineRow, ModelDir, Registry};
use ink_ffi::runtime::{Core, Parts};
use ink_llm::{
    ApiKey, HttpRequest, HttpResponse, KeyStore, KeyStoreError, Transport, TransportError,
};
use serde_json::Value;

use super::common::{
    Behaviour, Gate, MockInstaller, MockLoader, Recorder, TempDir, clock, install, start_parts,
};

/// How long an answer may take.
pub const WAIT: Duration = Duration::from_secs(5);

/// A synthetic key, looked for everywhere it must not be.
pub const KEY: &str = "sk-synthetic-canary-5d1e";

/// A core over a library on disk, with `rows` installed, own-key providers over [`MemoryKeys`]
/// and a [`FakeTransport`] answering OK.
pub struct Rig {
    pub core: Core,
    pub events: Arc<Recorder>,
    pub keys: Arc<MemoryKeys>,
    pub net: Arc<FakeTransport>,
    pub db: PathBuf,
    pub dir: TempDir,
}

impl Rig {
    pub fn new(label: &str, rows: &[EngineRow]) -> Self {
        let dir = TempDir::new(label);
        let db = dir.path().join("library.sqlite");
        let keys = Arc::new(MemoryKeys::default());
        let net = FakeTransport::answering(200, &openai_answer("OK"));
        let (core, events) = Self::start(&dir, &db, rows);
        core.set_cloud_services(keys.clone(), net.clone());
        Self {
            core,
            events,
            keys,
            net,
            db,
            dir,
        }
    }

    fn start(dir: &TempDir, db: &PathBuf, rows: &[EngineRow]) -> (Core, Arc<Recorder>) {
        let models = ModelDir::new(dir.path().join("models"));
        for row in rows {
            install(&models, row);
        }
        let loader = MockLoader::new(Behaviour::Say("synthetic words".into()));
        let parts = Parts {
            store: Arc::new(ink_store::SqliteStore::open(db).unwrap()),
            clock: clock(),
            registry: Registry::new(rows.to_vec()).unwrap(),
            models,
            installer: Arc::new(MockInstaller {
                generation: loader.generation.clone(),
                gate: None,
                installs: AtomicUsize::new(0),
            }),
            loader,
            data_dir: dir.path().to_owned(),
            permissions: Arc::new(ink_ffi::queries::NoPermissionProbe),
            meetings: Default::default(),
        };
        start_parts(parts)
    }

    /// Stops the core and starts another over the same library, keys and transport.
    pub fn restart(self) -> Self {
        let Self {
            core,
            keys,
            net,
            db,
            dir,
            ..
        } = self;
        core.shutdown();
        let (core, events) = Self::start(&dir, &db, &[]);
        core.set_cloud_services(keys.clone(), net.clone());
        Self {
            core,
            events,
            keys,
            net,
            db,
            dir,
        }
    }

    /// Sends `cmd` with `id` and returns its answer: the event with that ref, or the
    /// command.failed with that id.
    pub fn ask(&self, mut cmd: Value, id: &str) -> Value {
        cmd["id"] = id.into();
        self.core.command(&cmd.to_string()).unwrap();
        self.events
            .wait_for(WAIT, |v| {
                v["ref"] == id || (v["type"] == "command.failed" && v["id"] == id)
            })
            .unwrap_or_else(|| panic!("no answer to {id}; got {:?}", self.events.types()))
    }

    /// `setting.set llm.local_only`, as the shell's own switch would (answered without a ref).
    pub fn set_local_only(&self, value: &str) {
        let before = self.events.count("setting.value");
        self.core
            .command(
                &serde_json::json!({"cmd": "setting.set", "key": "llm.local_only", "value": value})
                    .to_string(),
            )
            .unwrap();
        assert!(self.events.wait_count("setting.value", before + 1, WAIT));
    }

    /// Stores [`KEY`] for `provider`.
    pub fn save_key(&self, provider: &str, id: &str) -> Value {
        let saved = self.ask(
            serde_json::json!({"cmd": "llm.key.save", "provider": provider, "key": KEY}),
            id,
        );
        assert_eq!(saved["type"], "llm.providers", "{saved}");
        saved
    }

    /// Chooses OpenAI, turning local-only mode off.
    pub fn choose_openai(&self, id: &str) -> Value {
        let chosen = self.ask(
            serde_json::json!({"cmd": "llm.choose", "provider": "openai", "local_only": "off"}),
            id,
        );
        assert_eq!(chosen["type"], "llm.providers", "{chosen}");
        chosen
    }
}

/// `provider`'s entry in an `llm.providers` event.
pub fn entry<'a>(listed: &'a Value, provider: &str) -> &'a Value {
    listed["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == provider)
        .unwrap()
}

/// Whether any file under `dir` holds `needle`'s bytes.
pub fn any_file_holds(dir: &std::path::Path, needle: &str) -> bool {
    std::fs::read_dir(dir).unwrap().flatten().any(|e| {
        let path = e.path();
        if path.is_dir() {
            any_file_holds(&path, needle)
        } else {
            std::fs::read(&path)
                .unwrap_or_default()
                .windows(needle.len())
                .any(|w| w == needle.as_bytes())
        }
    })
}

/// Keys in memory, by provider. `refuse` makes every call fail as a locked store would.
#[derive(Default)]
pub struct MemoryKeys {
    pub keys: Mutex<HashMap<String, String>>,
    pub refuse: Mutex<Option<KeyStoreError>>,
    pub reads: AtomicUsize,
}

impl MemoryKeys {
    fn check(&self) -> Result<(), KeyStoreError> {
        match *self.refuse.lock().unwrap() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl KeyStore for MemoryKeys {
    fn has_key(&self, provider: &str) -> Result<bool, KeyStoreError> {
        self.check()?;
        Ok(self.keys.lock().unwrap().contains_key(provider))
    }

    fn read_key(&self, provider: &str) -> Result<ApiKey, KeyStoreError> {
        self.check()?;
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.keys
            .lock()
            .unwrap()
            .get(provider)
            .map(|k| ApiKey::new(k.clone()))
            .ok_or(KeyStoreError::NotFound)
    }

    fn save_key(&self, provider: &str, key: &str) -> Result<(), KeyStoreError> {
        self.check()?;
        self.keys
            .lock()
            .unwrap()
            .insert(provider.to_owned(), key.to_owned());
        Ok(())
    }

    fn delete_key(&self, provider: &str) -> Result<(), KeyStoreError> {
        self.check()?;
        self.keys.lock().unwrap().remove(provider);
        Ok(())
    }
}

/// One request as the transport saw it.
#[derive(Clone, Debug)]
pub struct Seen {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub loopback_only: bool,
    pub deadline: Option<Instant>,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Records every request and answers with `answer`; waits at `gate` first when one is set. With
/// `stall` set it is a provider that takes the request and never answers: it gives up at the
/// request's deadline, as the real client does, and says it timed out (without a deadline, after
/// [`WAIT`]).
pub struct FakeTransport {
    pub seen: Mutex<Vec<Seen>>,
    pub answer: Mutex<Result<(u16, String), TransportError>>,
    pub gate: Mutex<Option<Arc<Gate>>>,
    pub stall: AtomicBool,
}

/// An OpenAI-shaped answer holding `text`.
pub fn openai_answer(text: &str) -> String {
    serde_json::json!({"choices": [{"message": {"role": "assistant", "content": text}}]})
        .to_string()
}

impl FakeTransport {
    pub fn answering(status: u16, body: &str) -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::default(),
            answer: Mutex::new(Ok((status, body.to_owned()))),
            gate: Mutex::new(None),
            stall: AtomicBool::new(false),
        })
    }

    pub fn calls(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    pub fn set(&self, answer: Result<(u16, String), TransportError>) {
        *self.answer.lock().unwrap() = answer;
    }
}

impl Transport for FakeTransport {
    fn post(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
        self.seen.lock().unwrap().push(Seen {
            url: request.url.clone(),
            headers: request
                .headers
                .iter()
                .map(|(n, v)| ((*n).to_owned(), v.clone()))
                .collect(),
            body: String::from_utf8(request.body.clone()).unwrap(),
            loopback_only: request.loopback_only,
            deadline: request.deadline,
        });
        if self.stall.load(Ordering::SeqCst) {
            let until = request.deadline.map_or_else(
                || Instant::now() + WAIT,
                |d| d + ink_llm::transport::DEADLINE_GRACE,
            );
            std::thread::sleep(until.saturating_duration_since(Instant::now()));
            return Err(TransportError::Timeout);
        }
        let gate = self.gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.wait();
        }
        match &*self.answer.lock().unwrap() {
            Ok((status, body)) => Ok(HttpResponse {
                status: *status,
                body: if (200..300).contains(status) {
                    body.clone().into_bytes()
                } else {
                    Vec::new()
                },
            }),
            Err(e) => Err(*e),
        }
    }
}
