//! The adapters for the meeting's helpers, the VAD and the diarizer, callable in every build.
//!
//! A cargo feature compiles each adapter in (`engine-silero`, `engine-nemo`), and a crate above
//! this one cannot name a feature of this crate in its own `cfg`: the release enables them as
//! `ink-engines/engine-silero` and `ink-engines/engine-nemo`. So these two functions exist in
//! every build and say, by their error, when this one has no adapter for a row's runtime. The
//! registry lists a row only in builds with its adapter, so a caller that routes first only asks
//! for what can run.

use std::sync::Arc;

use ink_audio::SpeechProbability;
use ink_core::{Diarizer, EngineError};

use crate::{EngineRow, ModelDir};

/// A loaded VAD model: each [`vad`](Self::vad) is one stream's state over it.
pub trait VadModel: Send + Sync {
    /// **Worker.** A fresh speech-probability source with its own recurrent state.
    fn vad(&self) -> Result<Box<dyn SpeechProbability>, EngineError>;
}

#[cfg(feature = "engine-silero")]
impl VadModel for crate::SileroModel {
    fn vad(&self) -> Result<Box<dyn SpeechProbability>, EngineError> {
        crate::SileroModel::vad(self).map(|v| Box::new(v) as Box<dyn SpeechProbability>)
    }
}

/// **Worker.** Loads the VAD row installed under `models`. Without its adapter in this build,
/// [`EngineError::ModelMissing`], naming the row.
pub fn load_vad(models: &ModelDir, row: &EngineRow) -> Result<Arc<dyn VadModel>, EngineError> {
    #[cfg(feature = "engine-silero")]
    if row.runtime == crate::Runtime::Tract {
        use crate::Loader;
        let model = crate::SileroLoader::new(models.clone()).load(row)?;
        return Ok(Arc::new(model));
    }
    let _ = models;
    Err(EngineError::ModelMissing(format!(
        "{}: this build has no voice-detection adapter for its runtime",
        row.id
    )))
}

/// **Worker.** Loads the diarizer row installed under `models`, on the GPU where there is one.
/// Without its adapter in this build, [`EngineError::ModelMissing`], naming the row.
pub fn load_diarizer(models: &ModelDir, row: &EngineRow) -> Result<Arc<dyn Diarizer>, EngineError> {
    #[cfg(feature = "engine-nemo")]
    if row.runtime == crate::Runtime::NemoSpeechCpp {
        use crate::Loader;
        let device = if cfg!(target_os = "macos") {
            crate::NemoDevice::Gpu(0)
        } else {
            crate::NemoDevice::Cpu
        };
        let diarizer = crate::NemoLoader::new(models.clone(), device).load(row)?;
        return Ok(Arc::new(diarizer));
    }
    let _ = models;
    Err(EngineError::ModelMissing(format!(
        "{}: this build has no diarizer adapter for its runtime",
        row.id
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without the adapters (CI's default build), each says so by name instead of pretending.
    #[test]
    #[cfg(not(any(feature = "engine-silero", feature = "engine-nemo")))]
    fn a_build_without_the_adapters_says_so() {
        let dir = ModelDir::new(std::env::temp_dir().join("ink-engines-no-adapters"));
        let vad = load_vad(&dir, &crate::silero_vad()).err().unwrap();
        assert!(
            matches!(&vad, EngineError::ModelMissing(m) if m.contains("silero")),
            "{vad}"
        );
        let diar = load_diarizer(&dir, &crate::nemotron_3_diarization())
            .err()
            .unwrap();
        assert!(
            matches!(&diar, EngineError::ModelMissing(m) if m.contains("diarization")),
            "{diar}"
        );
    }

    /// With the VAD's adapter, a row whose file is not installed is missing, not a crash.
    #[test]
    #[cfg(feature = "engine-silero")]
    fn a_vad_row_not_installed_is_missing() {
        let dir = ModelDir::new(std::env::temp_dir().join("ink-engines-vad-not-installed"));
        let err = load_vad(&dir, &crate::silero_vad()).err().unwrap();
        assert!(matches!(err, EngineError::ModelMissing(_)), "{err}");
    }

    /// With the diarizer's adapter, a row whose file is not installed fails as an error, never
    /// a crash (NeMo reports the missing file when it opens it).
    #[test]
    #[cfg(feature = "engine-nemo")]
    fn a_diarizer_row_not_installed_is_an_error() {
        let dir = ModelDir::new(std::env::temp_dir().join("ink-engines-diar-not-installed"));
        assert!(load_diarizer(&dir, &crate::nemotron_3_diarization()).is_err());
    }
}
