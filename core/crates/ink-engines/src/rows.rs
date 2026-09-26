//! The models S1.4c's adapters load: the diarizer's registry row, and the VAD's pinned file.
//!
//! Plain data, compiled whatever the features, so CI validates it. A row reaches
//! [`builtin_rows`](crate::builtin_rows) only when its adapter is built: a build without the
//! adapter must not offer a download it cannot use.

use ink_core::Job;

use crate::registry::{EngineRow, JobScore, ModelFile, Os, Runtime};

/// The diarizer's registry id.
pub const NEMOTRON_DIARIZATION_ID: &str = "nemotron-3-diarization-q8";

/// Nemotron-3-Diarization, q8_0 GGUF, for NeMo-Speech.cpp (`engine-nemo`): far-end diarization
/// (architecture rule 10).
///
/// - **Pinned:** the Hugging Face revision, size and SHA-256 are the ones NeMo-Speech.cpp's own
///   model index lists at the commit the adapter builds against; the file checked on disk has
///   that size and hash.
/// - **Measured:** DER in percent at ±0.25 s, overlap scored, on AMI test meetings EN2002a, b and
///   c (15-minute single-distant-mic stretches): 15.3, 22.6 and 22.8 with the `v3-offline`
///   preset. The row carries their mean. `tests/nemo.rs` reproduces them.
/// - **Licence:** OpenMDW-1.1 (weights). The code that runs them is Apache-2.0.
/// - **macOS only for now:** the Windows build of NeMo-Speech.cpp is not verified yet.
pub fn nemotron_3_diarization() -> EngineRow {
    const REVISION: &str = "f667ed73aee57d40cc39428eb768b4fd87a0a29e";
    const FILE: &str = "Nemotron-3-Diarization.q8_0.gguf";
    EngineRow {
        id: NEMOTRON_DIARIZATION_ID.into(),
        scores: vec![JobScore {
            job: Job::Diarization,
            wer: 20.2,
        }],
        files: vec![ModelFile {
            name: FILE.into(),
            url: format!(
                "https://huggingface.co/nvidia/Nemotron-3-Diarization/resolve/{REVISION}/{FILE}"
            ),
            sha256: "08456d9e22cd9a323c0364d98375f3746d6e68507ebb705cd46438c534c7a3a1".into(),
            size: 107_012_128,
        }],
        revision: REVISION.into(),
        licence: "OpenMDW-1.1".into(),
        oses: vec![Os::MacOs],
        runtime: Runtime::NemoSpeechCpp,
    }
}

/// A model file pinned by content, whose registry row is not written yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PinnedFile {
    /// The file name, also its name on disk.
    pub name: &'static str,
    /// SHA-256 of the whole file, lowercase hex.
    pub sha256: &'static str,
    /// Size in bytes.
    pub size: u64,
    /// The weights' licence (SPDX).
    pub licence: &'static str,
    /// What the licence asks of us.
    pub attribution: &'static str,
}

/// Silero VAD v6.2.3, the 16 kHz opset-15 ONNX export (`engine-silero`). Downloaded at runtime,
/// never bundled.
///
/// **No registry row yet**, for two reasons a row cannot paper over:
/// - Registry rows are pinned to a full commit hash, never a tag, and the file is published under
///   the `v6.2.3` tag of `snakers4/silero-vad` (`src/silero_vad/data/`). The tag's commit has to
///   be looked up before a URL can be pinned.
/// - A row must fill a [`Job`], and voice activity is not one of ink-core's jobs.
pub const SILERO_VAD_FILE: PinnedFile = PinnedFile {
    name: "silero_vad_16k_op15.onnx",
    sha256: "7ed98ddbad84ccac4cd0aeb3099049280713df825c610a8ed34543318f1b2c49",
    size: 1_289_603,
    licence: "MIT",
    attribution: "Silero VAD (MIT): ship its copyright and licence notice, and credit it in About",
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{ALLOWED_WEIGHT_LICENCES, Registry};

    #[test]
    fn the_diarizer_row_is_valid_and_pinned() {
        let row = nemotron_3_diarization();
        row.validate().unwrap();
        Registry::new(vec![row.clone()]).unwrap();
        assert!(row.does(Job::Diarization));
        assert!(!row.does(Job::LivePartials));
        assert!(
            row.files[0]
                .url
                .contains(&format!("/resolve/{}/", row.revision))
        );
    }

    #[test]
    fn the_diarizer_row_carries_the_mean_of_the_measured_ders() {
        let mean = (15.3f32 + 22.6 + 22.8) / 3.0;
        let wer = nemotron_3_diarization().wer(Job::Diarization).unwrap();
        assert!((wer - mean).abs() < 0.05, "{wer} against {mean}");
    }

    #[test]
    fn the_vad_file_is_pinned_and_allowed() {
        let f = SILERO_VAD_FILE;
        assert_eq!(f.sha256.len(), 64);
        assert!(
            f.sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert!(f.size > 0);
        assert!(ALLOWED_WEIGHT_LICENCES.contains(&f.licence));
    }
}
