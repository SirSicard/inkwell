//! The registry rows of the diarizer, the VAD, Windows' Parakeet and the Mac's Parakeet.
//!
//! Plain data, compiled whatever the features, so CI validates it. A row reaches
//! [`builtin_rows`](crate::builtin_rows) only when its adapter is built: a build without the
//! adapter must not offer a download it cannot use. The Mac's Parakeet has no adapter in the core
//! (the shell runs it), so it is in every build.

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
/// - **Both OSes.** On Windows (Vulkan, a 12-core desktop with an RTX 3090) EN2002a and b came
///   within 0.2 of those, and EN2002c at 26.6, with one cluster more than the Mac found; accepted
///   by the maintainer for 1.0 (2026-09-30).
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
        oses: vec![Os::MacOs, Os::Windows],
        runtime: Runtime::NemoSpeechCpp,
    }
}

/// Windows' Parakeet's registry id.
pub const PARAKEET_INT8_ID: &str = "parakeet-tdt-0.6b-v3-int8";

/// Parakeet TDT 0.6B v3, the int8 ONNX conversion for sherpa-onnx, on the CPU (`engine-sherpa`):
/// Windows' live partials (through [`TrailingWindow`](crate::TrailingWindow)), and the dictation
/// final on a PC without a GPU, where Qwen3-ASR takes seconds ([`Router`](crate::Router)).
///
/// - **Pinned:** the Hugging Face revision Inkwell 0.2 downloads it from, and each file's size and
///   SHA-256 as downloaded from it (`tests/sherpa.rs` checks the ONNX files on disk).
/// - **Measured**, the whole of each clip in one pass (`tests/sherpa.rs`): 27.93 on AMI IHM (the
///   Mac's Core ML build of the same weights: 23.4) and 16.36 on FLEURS English dev as published,
///   where 21 of the quietest utterances come back empty. With the level normalised first, as the
///   app's gain stage does, FLEURS is 7.17 (Qwen3-ASR 1.7B: 4.3). A 5 s utterance took a median
///   0.30-0.36 s on a 12-core desktop's CPU, in two runs while other builds shared the machine:
///   not a measurement, and a PC with no usable GPU is usually far weaker.
/// - **Licence:** CC-BY-4.0 (NVIDIA's weights; the conversion adds none). Credited in About.
/// - **Windows only:** the Mac runs Parakeet in FluidAudio, on the Neural Engine.
pub fn parakeet_tdt_v3_int8() -> EngineRow {
    const REVISION: &str = "2bda32ec70b097a55adaa07d9a7173915b43cc78";
    let file = |name: &str, sha256: &str, size: u64| ModelFile {
        name: name.into(),
        url: format!(
            "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/{REVISION}/{name}"
        ),
        sha256: sha256.into(),
        size,
    };
    EngineRow {
        id: PARAKEET_INT8_ID.into(),
        scores: vec![
            JobScore {
                job: Job::LivePartials,
                wer: 27.93,
            },
            JobScore {
                job: Job::DictationFinal,
                wer: 16.36,
            },
        ],
        files: vec![
            file(
                "encoder.int8.onnx",
                "acfc2b4456377e15d04f0243af540b7fe7c992f8d898d751cf134c3a55fd2247",
                652_184_281,
            ),
            file(
                "decoder.int8.onnx",
                "179e50c43d1a9de79c8a24149a2f9bac6eb5981823f2a2ed88d655b24248db4e",
                11_845_275,
            ),
            file(
                "joiner.int8.onnx",
                "3164c13fc2821009440d20fcb5fdc78bff28b4db2f8d0f0b329101719c0948b3",
                6_355_277,
            ),
            file(
                "tokens.txt",
                "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
                93_939,
            ),
        ],
        revision: REVISION.into(),
        licence: "CC-BY-4.0".into(),
        oses: vec![Os::Windows],
        runtime: Runtime::SherpaOnnx,
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

/// The Mac's Parakeet's registry id.
pub const PARAKEET_COREML_ID: &str = "parakeet-tdt-0.6b-v3-coreml";

/// The folder the Mac's Parakeet files sit in, inside its row's directory. FluidAudio 0.15.5
/// reads v3 from a folder of this name (its repository's name without `-coreml`) next to the
/// directory it is given, so the shell passes `<row directory>/parakeet-tdt-0.6b-v3` to
/// `AsrModels.load(from:version:)` and `AsrModels.modelsExist(at:version:)` (see
/// [`ModelDir::row_dir`](crate::ModelDir)).
pub const PARAKEET_COREML_FOLDER: &str = "parakeet-tdt-0.6b-v3";

/// Parakeet TDT 0.6B v3 for Core ML, as FluidAudio 0.15.5 loads it: the Mac's live partials, and
/// the finals' fallback while Qwen3-ASR is not installed.
///
/// - **Only downloaded here:** the Mac shell loads it with FluidAudio on the Neural Engine and
///   registers it over the C ABI (architecture rule 2); the core has no Core ML runtime. The row
///   fills no job ([`Runtime::CoreMl`]), so the router never picks it and residency never loads
///   it. `model.update` installs it and `models.listed` shows it like any other row.
/// - **Files:** exactly those FluidAudio 0.15.5 loads for v3 at int8 encoder precision, the
///   app's (its default): the `Preprocessor`, `Encoder`, `Decoder` and `JointDecisionv3` bundles,
///   every file in each, and `parakeet_vocab.json`. They sit under [`PARAKEET_COREML_FOLDER`].
/// - **Pinned:** the Hugging Face revision. The LFS files' sizes and SHA-256 are Hugging Face's
///   file listing at that revision; the other files' are those of the files fetched from it,
///   whose git blob ids match that listing.
/// - **Licence:** CC-BY-4.0 (NVIDIA's weights; the conversion adds none). Credited in About.
/// - **macOS only.**
pub fn parakeet_tdt_v3_coreml() -> EngineRow {
    const REVISION: &str = "7dd20fe6b1797d35f5e3307e8b1732d9a178edfe";
    let file = |path: &str, sha256: &str, size: u64| ModelFile {
        name: format!("{PARAKEET_COREML_FOLDER}/{path}"),
        url: format!(
            "https://huggingface.co/FluidInference/parakeet-tdt-0.6b-v3-coreml/resolve/{REVISION}/{path}"
        ),
        sha256: sha256.into(),
        size,
    };
    EngineRow {
        id: PARAKEET_COREML_ID.into(),
        scores: Vec::new(),
        files: vec![
            file(
                "Preprocessor.mlmodelc/coremldata.bin",
                "dbde3f2300842c1fd51ef3ff948a0bcffe65ffd2dca10707f2509f32c1d65b1d",
                486,
            ),
            file(
                "Preprocessor.mlmodelc/analytics/coremldata.bin",
                "c9beeb989c8d66f8be11df59bc6df277ec76cee404f6865b46243835ef562f6d",
                243,
            ),
            file(
                "Preprocessor.mlmodelc/metadata.json",
                "2a98699e22d279dd37fa1d238aeb1c6db1df0d6fad687775324157689d8f3acf",
                2_841,
            ),
            file(
                "Preprocessor.mlmodelc/model.mil",
                "4b8518a956450fec57f06c2a21bdffc26973f7f1fa6842fb38fe917f896b6b93",
                28_181,
            ),
            file(
                "Preprocessor.mlmodelc/weights/weight.bin",
                "129b76e3aeafa8afa3ea76d995b964b145fe83700d579f6ff42c4c38fa0968ea",
                491_072,
            ),
            file(
                "Encoder.mlmodelc/coremldata.bin",
                "d48034a167a82e88fc3df64f60af963ab3983538271175b8319e7d5720a0fb86",
                485,
            ),
            file(
                "Encoder.mlmodelc/analytics/coremldata.bin",
                "42e638870d73f26b332918a3496ce36793fbb413a81cbd3d16ba01328637a105",
                243,
            ),
            file(
                "Encoder.mlmodelc/metadata.json",
                "da24da9cca943fb29d7fa8e376d57fca7cb3aa08ca51b956b0b0e56813f087e9",
                2_921,
            ),
            file(
                "Encoder.mlmodelc/model.mil",
                "ed7b19156ca29fa7dfd6891deb9fda4b0e8893f68597c985d135736546a43808",
                959_769,
            ),
            file(
                "Encoder.mlmodelc/weights/weight.bin",
                "e2020f323703477a5b21d7c2d282c403e371afb5962e79877e3033e73ba6f421",
                445_187_200,
            ),
            file(
                "Decoder.mlmodelc/coremldata.bin",
                "18647af085d87bd8f3121c8a9b4d4564c1ede038dab63d295b4e745cf2d7fb99",
                554,
            ),
            file(
                "Decoder.mlmodelc/analytics/coremldata.bin",
                "4238c4e81ecd0dc94bd7dfbb60f7e2cc824107c1ffe0387b8607b72833dba350",
                243,
            ),
            file(
                "Decoder.mlmodelc/metadata.json",
                "a39e93cd8371b8ded92635c7804fcd0590f0d1dd9415c6d19a0484be073077d9",
                3_427,
            ),
            file(
                "Decoder.mlmodelc/model.mil",
                "ef2a0a281695398a62fde86ac269c68f73d5b578d7ed3b31f2ba91a2d1ea1f35",
                13_110,
            ),
            file(
                "Decoder.mlmodelc/weights/weight.bin",
                "48adf0f0d47c406c8253d4f7fef967436a39da14f5a65e66d5a4b407be355d41",
                23_604_992,
            ),
            file(
                "JointDecisionv3.mlmodelc/coremldata.bin",
                "f5fc08b741400f0088492c9e839418b1e18522f19cba28d361dd030c5f398342",
                521,
            ),
            file(
                "JointDecisionv3.mlmodelc/analytics/coremldata.bin",
                "26def4bf73dd56d29dee21c8ef97cb8969e62f6120ed1adc91e46828e2737b6c",
                243,
            ),
            file(
                "JointDecisionv3.mlmodelc/metadata.json",
                "d9307211b9a37e0f0ac260c7660b1571a3de25841035cfdf9b58fd40425f890f",
                3_453,
            ),
            file(
                "JointDecisionv3.mlmodelc/model.mil",
                "be60732943389a047175111a83f8839f3eb39d4803adafa828a0871b2f39818d",
                11_775,
            ),
            file(
                "JointDecisionv3.mlmodelc/weights/weight.bin",
                "4e0e63d840032f7f07ddb1d64446051166281e5491bf22da8a945c41f6eedb3e",
                12_642_764,
            ),
            file(
                "parakeet_vocab.json",
                "7ec60e05f1b24480736ec0eed40900f4626bce1fa9a60fd700ec7e2a59198735",
                151_122,
            ),
        ],
        revision: REVISION.into(),
        licence: "CC-BY-4.0".into(),
        oses: vec![Os::MacOs],
        runtime: Runtime::CoreMl,
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
    fn the_diarizer_runs_on_both_oses() {
        let row = nemotron_3_diarization();
        assert!(row.runs_on(Os::MacOs) && row.runs_on(Os::Windows));
    }

    #[test]
    fn windows_parakeet_row_is_valid_pinned_and_windows_only() {
        let row = parakeet_tdt_v3_int8();
        row.validate().unwrap();
        Registry::new(vec![row.clone(), nemotron_3_diarization(), silero_vad()]).unwrap();
        assert_eq!(row.id, PARAKEET_INT8_ID);
        assert_eq!(row.info().jobs, [Job::LivePartials, Job::DictationFinal]);
        assert_eq!(row.wer(Job::MeetingFinal), None, "Qwen3-ASR does meetings");
        assert_eq!(row.revision, "2bda32ec70b097a55adaa07d9a7173915b43cc78");
        let names: Vec<&str> = row.files.iter().map(|f| f.name.as_str()).collect();
        // The four files the sherpa-onnx adapter loads, by the names it looks for.
        assert_eq!(
            names,
            [
                "encoder.int8.onnx",
                "decoder.int8.onnx",
                "joiner.int8.onnx",
                "tokens.txt"
            ]
        );
        for f in &row.files {
            assert!(
                f.url.contains(&format!("/resolve/{}/", row.revision)),
                "{}",
                f.url
            );
            assert!(f.url.ends_with(&format!("/{}", f.name)), "{}", f.url);
        }
        assert_eq!(row.total_size(), 670_478_772);
        assert_eq!(row.licence, "CC-BY-4.0");
        assert!(ALLOWED_WEIGHT_LICENCES.contains(&row.licence.as_str()));
        assert!(row.runs_on(Os::Windows) && !row.runs_on(Os::MacOs));
        assert_eq!(row.runtime, Runtime::SherpaOnnx);
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

    #[test]
    fn the_macs_parakeet_row_is_valid_and_only_downloaded() {
        let row = parakeet_tdt_v3_coreml();
        row.validate().unwrap();
        Registry::new(vec![row.clone()]).unwrap();
        assert_eq!(row.id, PARAKEET_COREML_ID);
        assert_eq!(row.revision, "7dd20fe6b1797d35f5e3307e8b1732d9a178edfe");
        assert_eq!(row.licence, "CC-BY-4.0");
        assert_eq!(row.oses, [Os::MacOs]);
        assert_eq!(row.runtime, Runtime::CoreMl);
        // No job: the router never routes to it.
        assert!(row.scores.is_empty());
        assert!(row.info().jobs.is_empty());
        assert_eq!(row.total_size(), 483_105_645);
        // FluidAudio 0.15.5's folder for v3: `Repo.parakeetV3.folderName`.
        assert_eq!(PARAKEET_COREML_FOLDER, "parakeet-tdt-0.6b-v3");
        for f in &row.files {
            let path = f
                .name
                .strip_prefix("parakeet-tdt-0.6b-v3/")
                .unwrap_or_else(|| panic!("{}", f.name));
            assert_eq!(
                f.url,
                format!(
                    "https://huggingface.co/FluidInference/parakeet-tdt-0.6b-v3-coreml/resolve/{}/{path}",
                    row.revision
                )
            );
        }
    }

    #[test]
    fn the_macs_parakeet_row_holds_what_fluidaudio_loads_for_v3_at_int8() {
        // FluidAudio 0.15.5: `ModelNames.ASR.requiredModelsV3(precision: .int8)` and
        // `ModelNames.ASR.vocabularyFile`; each bundle whole, as Hugging Face lists it.
        let bundles = [
            "Preprocessor.mlmodelc",
            "Encoder.mlmodelc",
            "Decoder.mlmodelc",
            "JointDecisionv3.mlmodelc",
        ];
        let inside = [
            "coremldata.bin",
            "analytics/coremldata.bin",
            "metadata.json",
            "model.mil",
            "weights/weight.bin",
        ];
        let mut expected: Vec<String> = bundles
            .iter()
            .flat_map(|b| {
                inside
                    .iter()
                    .map(move |f| format!("{PARAKEET_COREML_FOLDER}/{b}/{f}"))
            })
            .collect();
        expected.push(format!("{PARAKEET_COREML_FOLDER}/parakeet_vocab.json"));
        let names: Vec<String> = parakeet_tdt_v3_coreml()
            .files
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(names, expected);
    }
}
