//! The registry rows of the diarizer and the VAD.
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

/// The VAD's registry id.
pub const SILERO_VAD_ID: &str = "silero-vad-v6-16k";

/// Silero VAD v6.2.3, the 16 kHz opset-15 ONNX export, on tract (`engine-silero`): voice
/// activity for the gain stages and trimming.
///
/// - **Pinned:** the commit tag `v6.2.3` of `snakers4/silero-vad` points at, and the file's size
///   and SHA-256 (the file in `src/silero_vad/data/`).
/// - **Measured:** against an energy oracle on AMI headset speech (windows at or above −40 dBFS
///   are speech; windows deep in pauses below −60 dBFS are not), the share of those windows it
///   misjudges. `tests/silero.rs` reproduces it.
/// - **Licence:** MIT. Ship Silero's copyright and licence notice, and credit it in About.
/// - **Both OSes:** tract is pure Rust.
pub fn silero_vad() -> EngineRow {
    const REVISION: &str = "5cd7945676eb32225748052e2e6a0580e4686a08";
    const FILE: &str = "silero_vad_16k_op15.onnx";
    EngineRow {
        id: SILERO_VAD_ID.into(),
        scores: vec![JobScore {
            job: Job::VoiceActivity,
            wer: 1.5,
        }],
        files: vec![ModelFile {
            name: FILE.into(),
            url: format!(
                "https://raw.githubusercontent.com/snakers4/silero-vad/{REVISION}/src/silero_vad/data/{FILE}"
            ),
            sha256: "7ed98ddbad84ccac4cd0aeb3099049280713df825c610a8ed34543318f1b2c49".into(),
            size: 1_289_603,
        }],
        revision: REVISION.into(),
        licence: "MIT".into(),
        oses: vec![Os::MacOs, Os::Windows],
        runtime: Runtime::Tract,
    }
}

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
    fn the_vad_row_is_valid_pinned_and_does_voice_activity_only() {
        let row = silero_vad();
        row.validate().unwrap();
        Registry::new(vec![row.clone(), nemotron_3_diarization()]).unwrap();
        assert_eq!(row.info().jobs, [Job::VoiceActivity]);
        assert_eq!(row.revision, "5cd7945676eb32225748052e2e6a0580e4686a08");
        let [file] = row.files.as_slice() else {
            panic!("{:?}", row.files)
        };
        assert_eq!(file.name, "silero_vad_16k_op15.onnx");
        assert_eq!(
            file.url,
            "https://raw.githubusercontent.com/snakers4/silero-vad/5cd7945676eb32225748052e2e6a0580e4686a08/src/silero_vad/data/silero_vad_16k_op15.onnx"
        );
        assert_eq!(
            file.sha256,
            "7ed98ddbad84ccac4cd0aeb3099049280713df825c610a8ed34543318f1b2c49"
        );
        assert_eq!(file.size, 1_289_603);
        assert_eq!(row.licence, "MIT");
        assert!(ALLOWED_WEIGHT_LICENCES.contains(&row.licence.as_str()));
        // Pure Rust: it runs wherever the core does.
        assert!(row.runs_on(Os::MacOs) && row.runs_on(Os::Windows));
        assert_eq!(row.runtime, Runtime::Tract);
    }
}
