//! Language models the shell registered, and the polish model the dictation chain calls.
//!
//! The router's jobs are speech jobs (`ink_core::Job`), so registered language models are kept
//! here instead. Polish goes to one of them, chosen at each call: the Mac shell registers
//! Foundation Models while Apple Intelligence is available and lets go of it when it is not, so a
//! take polished after such a change uses what is registered then. With several registered, the
//! lowest id wins, so the choice never depends on timing.
//!
//! The user's own-key provider ([`cloud`](crate::cloud), chosen in Settings > AI) is kept here too.
//! The user's choice comes first: while a provider is chosen it is used, and a model the shell
//! registered (the Mac's Foundation Models) is used while none is.
//!
//! # A mode's own model
//!
//! A dictation mode may name the model it is polished on ([`ModelRef`]): one the shell registered,
//! or the chosen own-key provider. Those are every model the core holds, so they are what a mode
//! can pick ([`ShellLlms::choices`]); there is no second client. The mode's model is found again
//! at each take ([`mode_models`]), behind local-only mode like every other call, and polish checks
//! the user's consent on it as on the AI setting's model. A mode whose model is gone gets none.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use ink_core::{CancelToken, Endpoint, Llm, LlmError, LlmInfo, LlmRequest, LlmResponse};
use ink_llm::guard::{GuardedLlm, LocalOnly};
use ink_llm::{ByokLlm, Provider};

use crate::external::ExternalLlm;

/// The setting for local-only mode: `on` (the default: a model that is not on this machine is
/// refused) or `off`.
pub const LOCAL_ONLY_KEY: &str = "llm.local_only";

/// **Worker.** Local-only mode as the setting says: on unless it says `off`, and on when it
/// cannot be read (refusing a remote model by mistake costs a summary; sending a meeting away by
/// mistake cannot be undone).
pub fn local_only_setting(store: &dyn ink_core::Store) -> bool {
    match store.setting(LOCAL_ONLY_KEY) {
        Ok(v) => v.as_deref() != Some("off"),
        Err(e) => {
            log::warn!("the local-only setting could not be read ({e}); local-only stays on");
            true
        }
    }
}

/// A language model as a mode names it (`polish_model` in the stored modes): `engine:<id>`, a model
/// the shell registered under that id (on the Mac, `engine:apple-foundation-models`), or
/// `provider:<id>`, the own-key provider chosen in Settings > AI with the model chosen there
/// (`provider:anthropic`). A provider not chosen now is not one the core holds: a mode naming it
/// is not polished until it is chosen again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelRef {
    /// A model the shell registered, by its id.
    Engine(String),
    /// The chosen own-key provider, when it is this one.
    Provider(Provider),
}

impl ModelRef {
    /// The ref `id` spells, if any.
    pub fn parse(id: &str) -> Option<Self> {
        if let Some(engine) = id.strip_prefix("engine:") {
            return (!engine.trim().is_empty()).then(|| Self::Engine(engine.to_owned()));
        }
        Provider::from_id(id.strip_prefix("provider:")?).map(Self::Provider)
    }

    /// Its spelling: what a mode stores and `modes.listed` names.
    pub fn id(&self) -> String {
        match self {
            Self::Engine(id) => format!("engine:{id}"),
            Self::Provider(p) => format!("provider:{}", p.id()),
        }
    }
}

/// The registered language models, by id, and the chosen own-key provider. `Send + Sync`; the
/// locks are held only to insert, remove or copy out an `Arc`, never across a call.
#[derive(Default)]
pub struct ShellLlms {
    engines: RwLock<BTreeMap<String, Arc<ExternalLlm>>>,
    cloud: RwLock<Option<Arc<ByokLlm>>>,
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

    /// The model polish goes to now: the chosen own-key provider, else a model the shell
    /// registered, if either. The user's choice wins over the shell's model.
    pub fn pick(&self) -> Option<Arc<dyn Llm>> {
        match self.cloud() {
            Some(cloud) => Some(cloud as Arc<dyn Llm>),
            None => self.pick_shell().map(|shell| shell as Arc<dyn Llm>),
        }
    }

    /// The model the shell registered that [`pick`](Self::pick) gives, if one is registered.
    pub fn pick_shell(&self) -> Option<Arc<ExternalLlm>> {
        self.engines
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .next()
            .cloned()
    }

    /// The model `r` names, if the core holds it now: the engine registered under its id, or the
    /// chosen own-key provider when it is `r`'s provider.
    pub fn get(&self, r: &ModelRef) -> Option<Arc<dyn Llm>> {
        match r {
            ModelRef::Engine(id) => self
                .engines
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .get(id)
                .cloned()
                .map(|llm| llm as Arc<dyn Llm>),
            ModelRef::Provider(p) => self
                .cloud()
                .filter(|cloud| cloud.info().provider == p.id())
                .map(|cloud| cloud as Arc<dyn Llm>),
        }
    }

    /// Every model a mode can pick now, as [`pick`](Self::pick) orders them: the chosen own-key
    /// provider, then the registered models by id.
    pub fn choices(&self) -> Vec<(ModelRef, Arc<dyn Llm>)> {
        let cloud = self.cloud().and_then(|cloud| {
            let provider = Provider::from_id(&cloud.info().provider)?;
            Some((ModelRef::Provider(provider), cloud as Arc<dyn Llm>))
        });
        let engines = self.engines.read().unwrap_or_else(PoisonError::into_inner);
        cloud
            .into_iter()
            .chain(
                engines
                    .iter()
                    .map(|(id, llm)| (ModelRef::Engine(id.clone()), llm.clone() as Arc<dyn Llm>)),
            )
            .collect()
    }

    /// What [`pick`](Self::pick) gives now, as a mode would name it: the model a mode without one
    /// of its own is polished on.
    pub fn setting_ref(&self) -> Option<ModelRef> {
        self.choices().into_iter().next().map(|(r, _)| r)
    }

    /// The chosen own-key provider, if one is chosen.
    pub fn cloud(&self) -> Option<Arc<ByokLlm>> {
        self.cloud
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Chooses the own-key provider (`None`: none). A call already holding the previous one
    /// finishes with it.
    pub fn set_cloud(&self, llm: Option<Arc<ByokLlm>>) {
        let previous = std::mem::replace(
            &mut *self.cloud.write().unwrap_or_else(PoisonError::into_inner),
            llm,
        );
        drop(previous);
    }
}

/// The dictation chain's lookup for a mode's own model ([`ModelRef`]): none when the core holds
/// no model by that id as the take is polished, else a [`PolishModel`] on it, which checks the
/// polish consent and local-only mode on the model each call reaches, exactly as for the AI
/// setting's model.
pub fn mode_models(llms: Arc<ShellLlms>, local_only: LocalOnly) -> ink_pipeline::chain::ModeModels {
    Arc::new(move |id: &str| {
        let model = ModelRef::parse(id)?;
        llms.get(&model)?;
        Some(Arc::new(PolishModel::for_mode(
            llms.clone(),
            local_only.clone(),
            model,
        )) as Arc<dyn Llm>)
    })
}

/// The dictation chain's polish model, and every other use of a registered language model (a
/// meeting's summary and commitments, Ask): whichever registered model [`ShellLlms::pick`] gives
/// at each call. With none registered, a call fails (the chain keeps the text as written and says
/// polish failed); it never makes up an answer.
///
/// **Local-only mode** (architecture rule 6) is enforced here, in code: each call goes through
/// [`GuardedLlm`] around the model picked for it, so a model whose info says it is not on this
/// machine is refused while the switch is on ([`LlmError::LocalOnly`]), before anything is sent.
/// The check is on the model called, never on one picked a moment earlier.
///
/// A mode's own model ([`for_mode`](Self::for_mode)) is called the same way, found by its
/// [`ModelRef`] at each call instead of picked.
pub struct PolishModel {
    llms: Arc<ShellLlms>,
    local_only: LocalOnly,
    /// A mode's own model; `None` for whatever [`ShellLlms::pick`] gives.
    model: Option<ModelRef>,
}

impl PolishModel {
    /// Calls through the models in `llms`, behind the `local_only` switch.
    pub fn new(llms: Arc<ShellLlms>, local_only: LocalOnly) -> Self {
        Self {
            llms,
            local_only,
            model: None,
        }
    }

    /// Calls the model `model` names in `llms` (never another), behind the `local_only` switch.
    pub fn for_mode(llms: Arc<ShellLlms>, local_only: LocalOnly, model: ModelRef) -> Self {
        Self {
            llms,
            local_only,
            model: Some(model),
        }
    }

    /// The model this call goes to, if there is one now.
    fn reach(&self) -> Option<Arc<dyn Llm>> {
        match &self.model {
            None => self.llms.pick(),
            Some(model) => self.llms.get(model),
        }
    }

    fn none(&self) -> LlmError {
        LlmError::Engine(match self.model {
            None => "no language model is registered for polish".into(),
            Some(_) => "this mode's language model is not set up now".into(),
        })
    }
}

impl Llm for PolishModel {
    fn info(&self) -> LlmInfo {
        match self.reach() {
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
        let Some(llm) = self.reach() else {
            return Err(self.none());
        };
        GuardedLlm::new(llm, self.local_only.clone()).complete(request, cancel)
    }

    /// Checks the model picked for this call: a model registered or let go of since the caller
    /// looked can never receive text `allow` would refuse (dictation polish's consent).
    fn complete_if(
        &self,
        request: &LlmRequest,
        cancel: &CancelToken,
        allow: &dyn Fn(&LlmInfo) -> bool,
    ) -> Result<LlmResponse, LlmError> {
        let Some(llm) = self.reach() else {
            return Err(self.none());
        };
        let info = llm.info();
        if !allow(&info) {
            return Err(LlmError::NotAllowed { refused: info });
        }
        GuardedLlm::new(llm, self.local_only.clone()).complete(request, cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_ref_reads_back_its_own_spelling_and_nothing_else() {
        for r in [
            ModelRef::Engine("apple-foundation-models".into()),
            ModelRef::Provider(Provider::Anthropic),
            ModelRef::Provider(Provider::Custom),
        ] {
            assert_eq!(ModelRef::parse(&r.id()), Some(r));
        }
        for bad in [
            "",
            "apple-foundation-models",
            "anthropic",
            "engine:",
            "engine: ",
            "provider:",
            "provider:somebody",
            "Provider:anthropic",
        ] {
            assert_eq!(ModelRef::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn with_nothing_registered_there_is_nothing_to_pick() {
        let llms = ShellLlms::default();
        assert!(llms.choices().is_empty());
        assert_eq!(llms.setting_ref(), None);
        assert!(llms.get(&ModelRef::Provider(Provider::OpenAi)).is_none());
        let find = mode_models(Arc::new(llms), LocalOnly::new(true));
        assert!(find("engine:apple-foundation-models").is_none());
        assert!(find("not a ref").is_none());
    }
}
