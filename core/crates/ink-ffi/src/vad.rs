//! The voice-activity detector dictation levels its takes with (architecture rule 11): Silero,
//! when this build has it and its model is installed.
//!
//! Whether the build has it is ink-engines' `engine-silero` feature, which the release enables as
//! `ink-engines/engine-silero`. ink-engines tells this crate's build script so (its `links`
//! metadata), which sets `cfg(ink_silero)`: this crate needs no feature of its own, so the
//! release's feature list stays what it is.

use ink_engines::ModelDir;
use ink_pipeline::events::VadUnavailable;
use ink_pipeline::gain_stage::Vad;

use crate::runtime::Shared;

/// **Queries thread** (at `dictation.enable`; any worker would do). Silero, loaded from `models`,
/// or why there is none: a read of its 1.3 MB model file, once per enable. The chain says so to
/// the shell (`dictation.voice_detection`), and levels takes with the fallback meanwhile.
pub fn installed(shared: &Shared, models: &ModelDir) -> Vad {
    #[cfg(ink_silero)]
    {
        use ink_engines::Loader;
        let Some(row) = shared.registry.get(ink_engines::SILERO_VAD_ID) else {
            return Vad::Unavailable(VadUnavailable::ModelMissing);
        };
        if !models.is_installed(row) {
            return Vad::Unavailable(VadUnavailable::ModelMissing);
        }
        match ink_engines::SileroLoader::new(models.clone())
            .load(row)
            .and_then(|model| model.vad())
        {
            Ok(vad) => Vad::Installed(Box::new(vad)),
            Err(e) => {
                log::warn!("voice detection could not be loaded: {e}");
                Vad::Unavailable(VadUnavailable::LoadFailed)
            }
        }
    }
    #[cfg(not(ink_silero))]
    {
        let _ = (shared, models);
        Vad::Unavailable(VadUnavailable::ModelMissing)
    }
}

#[cfg(test)]
mod tests {
    /// Silero is in this build exactly when ink-engines has it (the release's
    /// `ink-engines/engine-silero`), which is when the built-in registry lists its row.
    #[test]
    fn silero_is_loaded_exactly_when_ink_engines_has_it() {
        let listed = ink_engines::Registry::builtin()
            .unwrap()
            .get(ink_engines::SILERO_VAD_ID)
            .is_some();
        assert_eq!(cfg!(ink_silero), listed);
    }
}
