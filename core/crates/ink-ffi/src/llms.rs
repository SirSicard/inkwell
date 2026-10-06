//! Language models the shell registered, and the polish model the dictation chain calls.
//!
//! The router's jobs are speech jobs (`ink_core::Job`), so registered language models are kept
//! here instead. Polish goes to one of them, chosen at each call: the Mac shell registers
//! Foundation Models while Apple Intelligence is available and lets go of it when it is not, so a
//! take polished after such a change uses what is registered then. With several registered, the
//! lowest id wins, so the choice never depends on timing.
//!
//! The user's own-key provider ([`cloud`](crate::cloud), chosen in Settings > AI) is kept here too,
//! and the core's own model on this machine ([`local`](crate::local), Windows' Qwen3, once one is
//! downloaded). The user's choice comes first: while a provider is chosen it is used; while none
//! is, the core's own model is, and a model the shell registered (the Mac's Foundation Models)
//! while there is neither. The Mac has no model of the core's own, so its order is unchanged.
//!
//! # A mode's own model
//!
//! A dictation mode may name the model it is polished on ([`ModelPin`]: a [`ModelRef`], one the
//! shell registered or the chosen own-key provider, and for a provider optionally a model of its
//! own at it). Those are every model the core holds, so they are what a mode can pick
//! ([`ShellLlms::choices`]); a model named at the provider is the same client asked for that
//! model ([`ByokLlm::with_model`]): same endpoint, key and guard. The mode's model is found again
//! at each take ([`mode_models`]) and at the call, behind local-only mode like every other call,
//! and polish checks the user's consents on it as on the AI setting's model. A mode whose model is
//! gone, or sends elsewhere than when the mode was saved ([`ModelPin::to`]), gets none.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use ink_core::{CancelToken, Endpoint, Llm, LlmError, LlmInfo, LlmRequest, LlmResponse};
use ink_llm::guard::{GuardedLlm, LocalOnly};
use ink_llm::{ByokLlm, Provider};
use ink_pipeline::consent::Destination;
use ink_pipeline::modes::ModelPin;

use crate::external::ExternalLlm;
use crate::local::{LOCAL_ID, LocalLlm};

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
/// the shell registered under that id (on the Mac, `engine:apple-foundation-models`), or the
/// core's own model on this machine (`engine:local`, whichever size is installed), or
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

/// The registered language models, by id, the chosen own-key provider, and the core's own model
/// on this machine. `Send + Sync`; the locks are held only to insert, remove or copy out an `Arc`,
/// never across a call.
#[derive(Default)]
pub struct ShellLlms {
    engines: RwLock<BTreeMap<String, Arc<ExternalLlm>>>,
    cloud: RwLock<Option<Arc<ByokLlm>>>,
    /// The core's own model installed on this machine, `engine:local`.
    local: RwLock<Option<Arc<LocalLlm>>>,
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

    /// The model polish goes to now: the chosen own-key provider, else the core's own model on
    /// this machine, else a model the shell registered, if any. The user's choice wins.
    pub fn pick(&self) -> Option<Arc<dyn Llm>> {
        if let Some(cloud) = self.cloud() {
            return Some(cloud as Arc<dyn Llm>);
        }
        if let Some(local) = self.local() {
            return Some(local as Arc<dyn Llm>);
        }
        self.pick_shell().map(|shell| shell as Arc<dyn Llm>)
    }

    /// **Any thread.** What [`pick`](Self::pick) gives, as a mode names it: the AI setting's
    /// model.
    pub fn pick_ref(&self) -> Option<ModelRef> {
        if let Some(info) = self
            .cloud
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|c| c.info())
        {
            return Provider::from_id(&info.provider).map(ModelRef::Provider);
        }
        if self.local().is_some() {
            return Some(ModelRef::Engine(LOCAL_ID.into()));
        }
        self.engines
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .next()
            .map(|id| ModelRef::Engine(id.clone()))
    }

    /// The core's own model on this machine, if one is installed.
    pub fn local(&self) -> Option<Arc<LocalLlm>> {
        self.local
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Sets the core's own model (`None`: none is installed). A call already holding the previous
    /// one finishes with it.
    pub fn set_local(&self, llm: Option<Arc<LocalLlm>>) {
        let previous = std::mem::replace(
            &mut *self.local.write().unwrap_or_else(PoisonError::into_inner),
            llm,
        );
        drop(previous);
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

    /// **Any thread.** The model `r` names, if the core holds it now: the engine registered under
    /// its id, or the chosen own-key provider when it is `r`'s provider, asked for `model` when a
    /// name is given (a provider's only: an engine has no model to pick, so `None`).
    pub fn get(&self, r: &ModelRef, model: Option<&str>) -> Option<Arc<dyn Llm>> {
        match (r, model) {
            (ModelRef::Engine(id), None) if id == LOCAL_ID => {
                self.local().map(|local| local as Arc<dyn Llm>)
            }
            (ModelRef::Engine(id), None) => self
                .engines
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .get(id)
                .cloned()
                .map(|llm| llm as Arc<dyn Llm>),
            (ModelRef::Engine(_), Some(_)) => None,
            (ModelRef::Provider(p), model) => {
                let cloud = self
                    .cloud()
                    .filter(|cloud| cloud.info().provider == p.id())?;
                Some(match model {
                    None => cloud as Arc<dyn Llm>,
                    Some(model) => Arc::new(cloud.with_model(model)) as Arc<dyn Llm>,
                })
            }
        }
    }

    /// **Any thread.** What [`get`](Self::get) would reach, described: its info, read under the
    /// lock, so no engine is held (and none can be released on this thread) to answer.
    pub fn info_of(&self, r: &ModelRef, model: Option<&str>) -> Option<LlmInfo> {
        match (r, model) {
            (ModelRef::Engine(id), None) if id == LOCAL_ID => self
                .local
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .as_ref()
                .map(|local| local.info()),
            (ModelRef::Engine(id), None) => self
                .engines
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .get(id)
                .map(|llm| llm.info()),
            (ModelRef::Engine(_), Some(_)) => None,
            (ModelRef::Provider(p), model) => {
                let cloud = self.cloud.read().unwrap_or_else(PoisonError::into_inner);
                let mut info = cloud.as_ref().map(|c| c.info())?;
                if info.provider != p.id() {
                    return None;
                }
                if let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) {
                    model.clone_into(&mut info.model);
                }
                Some(info)
            }
        }
    }

    /// **Any thread.** Whether the core holds the model `r` names now.
    pub fn contains(&self, r: &ModelRef) -> bool {
        self.info_of(r, None).is_some()
    }

    /// **Any thread.** Every model a mode can pick now, as [`pick`](Self::pick) orders them (so
    /// the first is the AI setting's): the chosen own-key provider, then the core's own model
    /// (`engine:local`), then the registered models by id. Described under the locks, holding
    /// none of them.
    pub fn choices(&self) -> Vec<(ModelRef, LlmInfo)> {
        let cloud = self
            .cloud
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|cloud| {
                let info = cloud.info();
                Some((ModelRef::Provider(Provider::from_id(&info.provider)?), info))
            });
        let local = self
            .local
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|local| (ModelRef::Engine(LOCAL_ID.into()), local.info()));
        let engines = self.engines.read().unwrap_or_else(PoisonError::into_inner);
        cloud
            .into_iter()
            .chain(local)
            .chain(
                engines
                    .iter()
                    .map(|(id, llm)| (ModelRef::Engine(id.clone()), llm.info())),
            )
            .collect()
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

/// **Worker.** The dictation chain's lookup for a mode's own model ([`ModelPin`]): none when the
/// core holds no model by that id as the take is polished, else a [`PolishModel`] on it, which
/// finds it again at the call, refuses it when it sends elsewhere than the pin recorded, and
/// checks local-only mode and the polish consents on the model the call reaches, exactly as for
/// the AI setting's model.
pub fn mode_models(llms: Arc<ShellLlms>, local_only: LocalOnly) -> ink_pipeline::chain::ModeModels {
    Arc::new(move |pin: &ModelPin| {
        let model = ModelRef::parse(&pin.id)?;
        if !llms.contains(&model) {
            return None;
        }
        Some(Arc::new(PolishModel::for_mode(
            llms.clone(),
            local_only.clone(),
            model,
            pin,
        )) as Arc<dyn Llm>)
    })
}

/// A mode's own model, as [`PolishModel`] finds it at each call.
struct ModeModel {
    model: ModelRef,
    /// A model named at the provider.
    name: Option<String>,
    /// Where it sent when the mode was saved; `None`: never recorded, so it is never called.
    to: Option<Destination>,
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
    model: Option<ModeModel>,
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

    /// Calls the model `pin` names in `llms` (`model`, as parsed from it), never another, and only
    /// while it sends where the pin recorded; behind the `local_only` switch.
    pub fn for_mode(
        llms: Arc<ShellLlms>,
        local_only: LocalOnly,
        model: ModelRef,
        pin: &ModelPin,
    ) -> Self {
        Self {
            llms,
            local_only,
            model: Some(ModeModel {
                model,
                name: pin.model.clone(),
                to: pin.to.clone(),
            }),
        }
    }

    /// The model this call goes to, if there is one now. A mode's own model that sends elsewhere
    /// than its pin recorded is none.
    fn reach(&self) -> Option<Arc<dyn Llm>> {
        match &self.model {
            None => self.llms.pick(),
            Some(own) => self
                .llms
                .get(&own.model, own.name.as_deref())
                .filter(|llm| own.to.as_ref().is_some_and(|to| to.covers(&llm.info()))),
        }
    }

    fn none(&self) -> LlmError {
        match self.model {
            None => LlmError::Engine("no language model is registered for polish".into()),
            // The chain says polish_model_missing: nothing was sent, and no other model is used.
            Some(_) => LlmError::Unavailable,
        }
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
    /// looked can never receive text `allow` would refuse (dictation polish's consent). Local-only
    /// mode is asked first: a model it refuses is refused as such, never as a consent the user
    /// could give and local-only would refuse anyway.
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
        self.local_only.check(&info.endpoint)?;
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
        assert!(
            llms.get(&ModelRef::Provider(Provider::OpenAi), None)
                .is_none()
        );
        assert!(!llms.contains(&ModelRef::Engine("apple-foundation-models".into())));
        let find = mode_models(Arc::new(llms), LocalOnly::new(true));
        let pin = |id: &str| ModelPin {
            id: id.into(),
            model: None,
            to: Some(Destination::OnDevice),
        };
        assert!(find(&pin("engine:apple-foundation-models")).is_none());
        assert!(find(&pin("not a ref")).is_none());
        assert!(find(&pin("")).is_none(), "blank never resolves");
    }
}
