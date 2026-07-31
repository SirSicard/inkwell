//! Owns the speech engine on a dedicated thread.
//!
//! The engine used to sit in `Mutex<Option<SpeechEngine>>` inside `AppState`,
//! which caused three separate problems:
//!
//! - `switch_model` loaded a ~650 MB ONNX encoder synchronously on the main
//!   thread, freezing the entire UI until it finished.
//! - `transcribe_file` held the engine lock for a whole file, so hotkey
//!   dictation blocked until the file was done.
//! - `SpeechEngine` carried `unsafe impl Sync` with no real safety argument. It
//!   happened to be sound only because non-async Tauri commands serialise on
//!   the main thread, a property that would have silently stopped holding the
//!   moment any command became async.
//!
//! Confining the engine to one thread fixes all three. Work is queued as
//! messages, so a long file transcription yields between chunks instead of
//! blocking dictation, and `Sync` is no longer needed at all: the engine is
//! moved into the thread once and never shared.

use crate::engine::SpeechEngine;
use crate::models::ModelSpec;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

enum Request {
    Load {
        spec: &'static ModelSpec,
        models_dir: PathBuf,
        reply: Sender<Result<(), String>>,
    },
    Transcribe {
        samples: Vec<f32>,
        hotwords: Option<String>,
        reply: Sender<Result<String, String>>,
    },
}

pub struct EngineService {
    tx: Sender<Request>,
    /// Display name of the loaded model, or empty when none is loaded. Cached
    /// so status reads do not need a round trip through the worker.
    name: Arc<Mutex<String>>,
    /// What was last loaded, so the engine can rebuild itself.
    ///
    /// Needed because not every model takes its bias phrases the same way.
    /// Parakeet accepts them per utterance, so a dictionary edit applies to the
    /// next dictation for free. Qwen3 takes them in its model config, at
    /// construction, so the only way to apply an edit is to build the engine
    /// again. Without this the dictionary would appear to save and then quietly
    /// do nothing until the next restart.
    loaded: Arc<Mutex<Option<(&'static ModelSpec, PathBuf)>>>,
}

impl EngineService {
    pub fn start() -> Self {
        let (tx, rx) = channel::<Request>();
        let name = Arc::new(Mutex::new(String::new()));

        std::thread::Builder::new()
            .name("speech-engine".into())
            .spawn(move || {
                // The engine lives here and nowhere else.
                let mut engine: Option<SpeechEngine> = None;
                while let Ok(req) = rx.recv() {
                    match req {
                        Request::Load {
                            spec,
                            models_dir,
                            reply,
                        } => {
                            let result = spec.load(&models_dir).map(|e| {
                                engine = Some(e);
                            });
                            if result.is_err() {
                                // A failed load must not leave the previous
                                // model half-replaced.
                                log::warn!("Engine load failed, keeping previous model");
                            }
                            let _ = reply.send(result);
                        }
                        Request::Transcribe { samples, hotwords, reply } => {
                            let result = match engine.as_ref() {
                                Some(e) => e.transcribe(&samples, hotwords.as_deref()),
                                None => Err(
                                    "No speech engine loaded. Download a model first.".to_string()
                                ),
                            };
                            let _ = reply.send(result);
                        }
                    }
                }
                log::info!("Engine thread shutting down");
            })
            .expect("failed to spawn engine thread");

        Self { tx, name, loaded: Arc::new(Mutex::new(None)) }
    }

    /// Load a model, blocking until it is ready. Callers on the main thread
    /// must go through `spawn_blocking`, since this can take seconds.
    pub fn load(&self, spec: &'static ModelSpec, models_dir: PathBuf) -> Result<String, String> {
        let models_dir_for_reload = models_dir.clone();
        let (reply, wait) = channel();
        self.tx
            .send(Request::Load {
                spec,
                models_dir,
                reply,
            })
            .map_err(|_| "Engine thread is gone".to_string())?;
        wait.recv()
            .map_err(|_| "Engine thread stopped responding".to_string())??;

        *self.name.lock().unwrap() = spec.display.to_string();
        *self.loaded.lock().unwrap() = Some((spec, models_dir_for_reload));
        Ok(spec.display.to_string())
    }

    /// Rebuild the current model in place, picking up anything it reads at
    /// construction. No-op when nothing is loaded, or when the loaded model
    /// takes its bias phrases per utterance and so has nothing to re-read.
    ///
    /// Blocking and slow, the same as `load`, so main-thread callers must go
    /// through `spawn_blocking`.
    pub fn reload_for_hotwords(&self) -> Result<(), String> {
        let current = self.loaded.lock().unwrap().clone();
        let Some((spec, models_dir)) = current else {
            return Ok(());
        };
        if !spec.hotwords_at_load() {
            return Ok(());
        }
        log::info!("Rebuilding {} to apply dictionary changes", spec.display);
        self.load(spec, models_dir).map(|_| ())
    }

    /// Transcribe 16 kHz mono samples, blocking until the result is ready.
    /// `hotwords`: newline-separated bias phrases, or None for plain decoding.
    pub fn transcribe(&self, samples: Vec<f32>, hotwords: Option<String>) -> Result<String, String> {
        let (reply, wait) = channel();
        self.tx
            .send(Request::Transcribe { samples, hotwords, reply })
            .map_err(|_| "Engine thread is gone".to_string())?;
        wait.recv()
            .map_err(|_| "Engine thread stopped responding".to_string())?
    }

    /// Display name of the loaded model, or "No model loaded".
    pub fn name(&self) -> String {
        let n = self.name.lock().unwrap();
        if n.is_empty() {
            "No model loaded".to_string()
        } else {
            n.clone()
        }
    }

    pub fn is_loaded(&self) -> bool {
        !self.name.lock().unwrap().is_empty()
    }
}
