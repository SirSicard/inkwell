//! Language models the shell registered, and the polish model the dictation chain calls.
//!
//! The router's jobs are speech jobs (`ink_core::Job`), so registered language models are kept
//! here instead. Polish goes to one of them, chosen at each call: the Mac shell registers
//! Foundation Models while Apple Intelligence is available and lets go of it when it is not, so a
//! take polished after such a change uses what is registered then. With several registered, the
//! lowest id wins, so the choice never depends on timing.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use ink_core::{CancelToken, Endpoint, Llm, LlmError, LlmInfo, LlmRequest, LlmResponse};

use crate::external::ExternalLlm;

/// The registered language models, by id. `Send + Sync`; the lock is held only to insert,
/// remove or copy out an `Arc`, never across a call.
#[derive(Default)]
pub struct ShellLlms {
    engines: RwLock<BTreeMap<String, Arc<ExternalLlm>>>,
}

impl ShellLlms {
    /// Adds `llm` under its id. `false` when the id is taken: the model is then dropped without
    /// its release, as the header promises for a refused registration.
    pub fn insert(&self, llm: ExternalLlm) -> bool {
        let mut engines = self.engines.write().unwrap_or_else(PoisonError::into_inner);
        if engines.contains_key(llm.id()) {
            llm.disarm();
            return false;
        }
        engines.insert(llm.id().to_owned(), Arc::new(llm));
        true
    }

    /// Lets go of model `id`. Its release runs once no call holds it. Returns whether it was
    /// registered.
    pub fn remove(&self, id: &str) -> bool {
        let removed = self
            .engines
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id);
        // Dropped here, after the lock: the last reference may release the shell's engine.
        removed.is_some()
    }

    /// The model polish goes to now, if one is registered.
    pub fn pick(&self) -> Option<Arc<ExternalLlm>> {
        self.engines
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .next()
            .cloned()
    }
}

/// The dictation chain's polish model: whichever registered model [`ShellLlms::pick`] gives at
/// each call. With none registered, a call fails (the chain keeps the text as written and says
/// polish failed); it never makes up an answer.
pub struct PolishModel {
    llms: Arc<ShellLlms>,
}

impl PolishModel {
    /// Polish through the models in `llms`.
    pub fn new(llms: Arc<ShellLlms>) -> Self {
        Self { llms }
    }
}

impl Llm for PolishModel {
    fn info(&self) -> LlmInfo {
        match self.llms.pick() {
            Some(llm) => llm.info(),
            None => LlmInfo {
                provider: "shell".into(),
                model: "none registered".into(),
                // Nothing is sent anywhere without a model.
                endpoint: Endpoint::InProcess,
            },
        }
    }

    fn complete(
        &self,
        request: &LlmRequest,
        cancel: &CancelToken,
    ) -> Result<LlmResponse, LlmError> {
        // The Arc is held for the call, so a model let go of meanwhile is released after it.
        let Some(llm) = self.llms.pick() else {
            return Err(LlmError::Engine(
                "no language model is registered for polish".into(),
            ));
        };
        llm.complete(request, cancel)
    }
}
