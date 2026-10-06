//! What a meeting runs on besides its speech engines: voice detection, the far end's diarizer,
//! and the language model for its summary, commitments and Ask.
//!
//! | Helper | Where it comes from | Without it |
//! |---|---|---|
//! | VAD | the router's [`Job::VoiceActivity`] model (Silero), loaded at the meeting's start | the fallbacks level everything, and the shell says voice detection is unavailable |
//! | Diarizer | the router's [`Job::Diarization`] model (Nemotron), loaded only for the final pass and let go of after it | the far end stays one voice, "Them" |
//! | Language model | the AI setting's: the chosen own-key provider, the core's own on this machine (Windows), or the one the shell registered (Foundation Models on the Mac), asked at each call | no summary, and the pass says so (`summary_unavailable`) |
//!
//! Each is looked up when a meeting starts, never assumed: a build without the VAD's or the
//! diarizer's adapter lists neither in its registry, and a Mac without Apple Intelligence
//! registers no language model.

use std::sync::Arc;

use ink_core::{
    CancelToken, DiarizeInput, Diarizer, EngineError, EngineInfo, EngineStream, EventSink, Job,
    Llm, SpeakerTurn,
};
use ink_engines::{Route, load_diarizer, load_vad};
use ink_llm::tasks::ask::AskOptions;
use ink_llm::tasks::summary::SummaryOptions;
use ink_pipeline::events::VadUnavailable;
use ink_pipeline::speech::VadSource;

use crate::llms::PolishModel;
use crate::local::LOCAL_CONTEXT_TOKENS;
use crate::runtime::Shared;

/// The context the shell's language model is taken to hold when it does not say: the on-device
/// model's on macOS 26, the smallest a shell registers.
pub const DEFAULT_CONTEXT_TOKENS: u32 = 4_096;

/// **Worker.** Voice detection for a meeting starting now: the installed VAD model, loaded, or why
/// there is none.
pub fn vad_source(shared: &Shared) -> VadSource {
    let row = match shared.router.route(Job::VoiceActivity) {
        Ok(Route::Model(row)) => row,
        Ok(other) => {
            // A shell engine for voice activity: the ABI has no such kind, so this is a bug.
            log::error!(
                "voice activity routes to {}, which is not a model",
                other.id()
            );
            return VadSource::Unavailable(VadUnavailable::LoadFailed);
        }
        Err(_) => return VadSource::Unavailable(VadUnavailable::ModelMissing),
    };
    match load_vad(&shared.models, &row) {
        Ok(model) => VadSource::Installed(Arc::new(move || model.vad())),
        Err(EngineError::ModelMissing(why)) => {
            log::warn!("meeting: no voice detection: {why}");
            VadSource::Unavailable(VadUnavailable::ModelMissing)
        }
        Err(e) => {
            log::warn!("meeting: the voice detection model did not load: {e}");
            VadSource::Unavailable(VadUnavailable::LoadFailed)
        }
    }
}

/// **Worker.** The far end's diarizer for a meeting starting now, when one is installed: it loads
/// its model only when the final pass asks, and lets go of it after (RAM holds seconds, never a
/// session: the model is no reason to hold a hundred megabytes through a meeting).
pub fn diarizer(shared: &Arc<Shared>) -> Option<Arc<dyn Diarizer>> {
    match shared.router.route(Job::Diarization) {
        Ok(Route::Model(row)) => Some(Arc::new(RoutedDiarizer {
            shared: shared.clone(),
            info: row.info(),
        })),
        Ok(other) => {
            log::error!("diarization routes to {}, which is not a model", other.id());
            None
        }
        Err(_) => None,
    }
}

/// See [`diarizer`].
struct RoutedDiarizer {
    shared: Arc<Shared>,
    info: EngineInfo,
}

impl Diarizer for RoutedDiarizer {
    fn info(&self) -> EngineInfo {
        self.info.clone()
    }

    fn diarize(
        &self,
        audio: &mut dyn DiarizeInput,
        cancel: &CancelToken,
    ) -> Result<Vec<SpeakerTurn>, EngineError> {
        let row = self
            .shared
            .registry
            .get(&self.info.id)
            .ok_or_else(|| EngineError::ModelMissing(self.info.id.clone()))?;
        // Loaded for this call and dropped with it.
        let model = load_diarizer(&self.shared.models, row)?;
        model.diarize(audio, cancel)
    }

    fn open_stream(&self, _: EventSink<SpeakerTurn>) -> Result<Box<dyn EngineStream>, EngineError> {
        Err(EngineError::Unsupported(
            "live labels are not wired into the meeting yet",
        ))
    }
}

/// **Worker.** The language model for a meeting's summary, commitments and Ask: the one the shell
/// registered, picked at each call, if one is registered now.
pub fn llm(shared: &Shared) -> Option<Arc<dyn Llm>> {
    shared.llms.pick().map(|_| {
        Arc::new(PolishModel::new(
            shared.llms.clone(),
            shared.local_only.clone(),
        )) as Arc<dyn Llm>
    })
}

/// The context the language model a meeting uses holds, in tokens: the chosen own-key provider's,
/// else the core's own model's ([`LOCAL_CONTEXT_TOKENS`]), else the registered model's (the order
/// [`ShellLlms::pick`](crate::llms::ShellLlms::pick) follows), else [`DEFAULT_CONTEXT_TOKENS`].
pub fn context_tokens(shared: &Shared) -> u32 {
    if let Some(cloud) = shared.llms.cloud() {
        return cloud.context_tokens().unwrap_or(DEFAULT_CONTEXT_TOKENS);
    }
    if shared.llms.local().is_some() {
        return LOCAL_CONTEXT_TOKENS;
    }
    if shared.llms.on_device() {
        return DEFAULT_CONTEXT_TOKENS;
    }
    shared
        .llms
        .pick_shell()
        .and_then(|shell| shell.context_tokens())
        .unwrap_or(DEFAULT_CONTEXT_TOKENS)
}

/// How a meeting's summary is sized for the language model it uses.
pub fn summary_options(shared: &Shared) -> SummaryOptions {
    SummaryOptions::for_context(context_tokens(shared))
}

/// How an Ask is sized for the language model it uses.
pub fn ask_options(shared: &Shared) -> AskOptions {
    AskOptions::for_context(context_tokens(shared))
}
