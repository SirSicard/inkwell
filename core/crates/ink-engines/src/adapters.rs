//! The adapters for the meeting's helpers, the VAD and the diarizer, and for Windows' Parakeet,
//! callable in every build.
//!
//! A cargo feature compiles each adapter in (`engine-silero`, `engine-nemo`, `engine-sherpa`), and
//! a crate above this one cannot name a feature of this crate in its own `cfg`: the release enables
//! them as `ink-engines/engine-silero` and so on. So these functions exist in every build and say,
//! by their error, when this one has no adapter for a row's runtime. The registry lists a row only
//! in builds with its adapter, so a caller that routes first only asks for what can run.

use std::sync::Arc;

use ink_audio::SpeechProbability;
use ink_core::{Diarizer, EngineError, OfflineEngine};

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

/// **Worker.** Loads the diarizer row installed under `models`, on the GPU where there is one: on
/// the Mac its GPU; on Windows GPU 0 (Vulkan), and the CPU when the model does not load there (no
/// Vulkan GPU, or too little memory on it). The model loads at the diarizer's first use, so that
/// is where the CPU is tried (`NemoDiarizer::with_fallback`). Without its adapter in this build,
/// [`EngineError::ModelMissing`], naming the row.
pub fn load_diarizer(models: &ModelDir, row: &EngineRow) -> Result<Arc<dyn Diarizer>, EngineError> {
    #[cfg(feature = "engine-nemo")]
    if row.runtime == crate::Runtime::NemoSpeechCpp {
        use crate::Loader;
        let diarizer =
            crate::NemoLoader::new(models.clone(), crate::NemoDevice::Gpu(0)).load(row)?;
        let diarizer = if cfg!(target_os = "macos") {
            diarizer
        } else {
            diarizer.with_fallback(crate::NemoDevice::Cpu)
        };
        return Ok(Arc::new(diarizer));
    }
    let _ = models;
    Err(EngineError::ModelMissing(format!(
        "{}: this build has no diarizer adapter for its runtime",
        row.id
    )))
}

/// Loads with each of `devices` in turn and returns the first that loads (the diarizer's model,
/// `nemo.rs`). A missing model, or a cancel, ends it at once. When no device loads, the one
/// device's error, or an error naming each device's.
#[cfg_attr(not(feature = "engine-nemo"), allow(dead_code))]
pub(crate) fn first_that_loads<D: Copy + std::fmt::Debug, T>(
    devices: &[D],
    mut load: impl FnMut(D) -> Result<T, EngineError>,
) -> Result<T, EngineError> {
    let mut failures = Vec::new();
    for &device in devices {
        match load(device) {
            Ok(loaded) => return Ok(loaded),
            Err(e @ (EngineError::ModelMissing(_) | EngineError::Cancelled)) => return Err(e),
            Err(e) => failures.push((device, e)),
        }
    }
    match failures.len() {
        0 => Err(EngineError::Failed("no device to load on".into())),
        1 => Err(failures.remove(0).1),
        _ => Err(EngineError::Failed(format!(
            "loaded on no device ({})",
            failures
                .iter()
                .map(|(device, e)| format!("{device:?}: {e}"))
                .collect::<Vec<_>>()
                .join("; ")
        ))),
    }
}

/// **Worker.** Loads a speech row installed under `models` that this crate's adapters run
/// directly: Windows' Parakeet on sherpa-onnx (`engine-sherpa`), for dictation and, through
/// [`TrailingWindow`](crate::TrailingWindow), live partials. (llama.cpp's rows load through their
/// own loader, `llama::QwenAsrLoader`.) Without its adapter in this build,
/// [`EngineError::ModelMissing`], naming the row.
pub fn load_speech(
    models: &ModelDir,
    row: &EngineRow,
) -> Result<Box<dyn OfflineEngine>, EngineError> {
    #[cfg(feature = "engine-sherpa")]
    if row.runtime == crate::Runtime::SherpaOnnx {
        if !models.is_installed(row) {
            return Err(EngineError::ModelMissing(format!(
                "{}: not installed",
                row.id
            )));
        }
        let engine = crate::sherpa::SherpaParakeet::load(&models.row_dir(row), row.info())?;
        return Ok(Box::new(engine));
    }
    let _ = models;
    Err(EngineError::ModelMissing(format!(
        "{}: this build has no adapter for its runtime",
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

    /// Without sherpa-onnx's adapter (CI's builds), Windows' Parakeet says so by name.
    #[test]
    #[cfg(not(feature = "engine-sherpa"))]
    fn a_build_without_sherpa_says_so() {
        let dir = ModelDir::new(std::env::temp_dir().join("ink-engines-no-sherpa"));
        let err = load_speech(&dir, &crate::parakeet_tdt_v3_int8())
            .err()
            .unwrap();
        assert!(
            matches!(&err, EngineError::ModelMissing(m) if m.contains("parakeet")),
            "{err}"
        );
    }

    /// With sherpa-onnx's adapter, a row that is not installed is missing, before any load.
    #[test]
    #[cfg(feature = "engine-sherpa")]
    fn a_parakeet_row_not_installed_is_missing() {
        let dir = ModelDir::new(std::env::temp_dir().join("ink-engines-parakeet-not-installed"));
        let err = load_speech(&dir, &crate::parakeet_tdt_v3_int8())
            .err()
            .unwrap();
        assert!(matches!(err, EngineError::ModelMissing(_)), "{err}");
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Device {
        Gpu,
        Cpu,
    }

    /// Windows' diarizer: the GPU first, the CPU when the GPU does not load.
    #[test]
    fn the_first_device_that_loads_is_used() {
        let mut tried = Vec::new();
        let loaded = first_that_loads(&[Device::Gpu, Device::Cpu], |d| {
            tried.push(d);
            match d {
                Device::Gpu => Err(EngineError::Failed("no matching GPU device found".into())),
                Device::Cpu => Ok("on the CPU"),
            }
        });
        assert_eq!(loaded, Ok("on the CPU"));
        assert_eq!(tried, [Device::Gpu, Device::Cpu]);
        // A GPU that loads is the only one tried.
        let mut tried = Vec::new();
        let loaded = first_that_loads(&[Device::Gpu, Device::Cpu], |d| {
            tried.push(d);
            Ok(d)
        });
        assert_eq!((loaded, tried), (Ok(Device::Gpu), vec![Device::Gpu]));
    }

    #[test]
    fn no_device_that_loads_is_an_error_naming_each() {
        let err = first_that_loads(&[Device::Gpu, Device::Cpu], |d| {
            Err::<(), _>(EngineError::Failed(format!("{d:?} refused")))
        })
        .unwrap_err();
        let EngineError::Failed(msg) = &err else {
            panic!("{err}")
        };
        assert!(
            msg.contains("Gpu refused") && msg.contains("Cpu refused"),
            "{msg}"
        );
        // One device: its own error, as it was.
        let err = first_that_loads(&[Device::Gpu], |_| {
            Err::<(), _>(EngineError::Failed("the GPU's own error".into()))
        });
        assert_eq!(err, Err(EngineError::Failed("the GPU's own error".into())));
        // A missing model is missing on every device: nothing more is tried.
        let mut tried = 0;
        let err = first_that_loads(&[Device::Gpu, Device::Cpu], |_| {
            tried += 1;
            Err::<(), _>(EngineError::ModelMissing("m".into()))
        });
        assert_eq!(
            (err, tried),
            (Err(EngineError::ModelMissing("m".into())), 1)
        );
    }
}
