//! Registry rows are validated before anything downloads, routes or loads them.

mod common;

use common::{REV, row, sha256_hex};
use ink_core::Job;
use ink_engines::{
    ALLOWED_WEIGHT_LICENCES, ChatQuirks, EngineRow, JobScore, LanguageRow, LanguageSize,
    MAX_RELATIVE_PATH_LEN, ModelDir, ModelFile, Os, REVISION_MARKER, Registry, RegistryError,
    RowKind, Runtime, SMALL_BELOW_MEMORY, builtin_rows, suggested_language,
};

fn valid() -> EngineRow {
    row(
        "synthetic-asr",
        &[(Job::DictationFinal, 7.5), (Job::MeetingFinal, 12.25)],
    )
}

fn refused(row: EngineRow) -> RegistryError {
    Registry::new(vec![row]).expect_err("row should be refused")
}

#[test]
fn a_valid_row_is_accepted_and_describes_itself() {
    let reg = Registry::new(vec![valid()]).unwrap();
    let row = reg.get("synthetic-asr").unwrap();
    assert_eq!(row.wer(Job::MeetingFinal), Some(12.25));
    assert_eq!(row.wer(Job::LivePartials), None);
    assert!(row.does(Job::DictationFinal));
    assert!(row.runs_on(Os::Windows));
    let info = row.info();
    assert_eq!(info.id, "synthetic-asr");
    assert_eq!(info.jobs, vec![Job::DictationFinal, Job::MeetingFinal]);
    assert_eq!(info.licence, "MIT");
}

#[test]
fn a_resolve_main_url_is_rejected() {
    let mut r = valid();
    r.files[0].url = "https://models.example/synthetic/x/resolve/main/weights.bin".into();
    assert!(matches!(refused(r), RegistryError::Unpinned { .. }));
}

#[test]
fn a_url_on_another_branch_or_revision_is_rejected() {
    // The revision appears elsewhere in the URL, but what is resolved is a branch.
    let mut r = valid();
    r.files[0].url = format!("https://models.example/{REV}/x/resolve/dev/weights.bin");
    assert!(matches!(refused(r), RegistryError::Unpinned { .. }));

    // Pinned, but to a different commit than the row says.
    let mut r = valid();
    r.files[0].url = format!(
        "https://models.example/synthetic/x/resolve/{}/weights.bin",
        "f".repeat(40)
    );
    assert!(matches!(refused(r), RegistryError::Unpinned { .. }));

    // No revision in the URL at all (a release tag, say).
    let mut r = valid();
    r.files[0].url = "https://models.example/releases/download/v1.0/weights.bin".into();
    assert!(matches!(refused(r), RegistryError::Unpinned { .. }));
}

#[test]
fn a_branch_tag_or_short_hash_revision_is_rejected() {
    for rev in ["main", "v1.0", "0123456", &REV.to_uppercase(), ""] {
        let mut r = valid();
        r.revision = rev.to_string();
        r.files[0].url = format!("https://models.example/synthetic/x/resolve/{rev}/weights.bin");
        assert!(
            matches!(refused(r), RegistryError::Unpinned { .. }),
            "revision {rev:?} was accepted"
        );
    }
    // A 64-digit (SHA-256 repository) commit is a pinned revision too.
    let mut r = valid();
    r.revision = "a".repeat(64);
    r.files[0].url = format!(
        "https://models.example/synthetic/x/resolve/{}/weights.bin",
        r.revision
    );
    Registry::new(vec![r]).unwrap();
}

#[test]
fn a_plain_http_url_is_rejected() {
    let mut r = valid();
    r.files[0].url = r.files[0].url.replacen("https://", "http://", 1);
    assert!(matches!(refused(r), RegistryError::Invalid { .. }));
}

#[test]
fn a_hash_that_is_not_64_lowercase_hex_is_rejected() {
    for bad in [
        "abc".to_string(),
        "g".repeat(64),
        "A".repeat(64),
        "a".repeat(63),
    ] {
        let mut r = valid();
        r.files[0].sha256 = bad.clone();
        assert!(
            matches!(refused(r), RegistryError::Invalid { .. }),
            "hash {bad:?} was accepted"
        );
    }
}

#[test]
fn a_licence_outside_the_weights_allowlist_is_rejected() {
    for licence in ["CC-BY-NC-4.0", "other", "NVIDIA Open Model License", ""] {
        let mut r = valid();
        r.licence = licence.into();
        assert_eq!(
            refused(r),
            RegistryError::Licence {
                id: "synthetic-asr".into(),
                licence: licence.into()
            }
        );
    }
    for licence in ALLOWED_WEIGHT_LICENCES {
        let mut r = valid();
        r.licence = (*licence).into();
        Registry::new(vec![r]).unwrap();
    }
}

#[test]
fn duplicate_ids_are_rejected() {
    assert_eq!(
        Registry::new(vec![valid(), valid()]).unwrap_err(),
        RegistryError::Duplicate {
            id: "synthetic-asr".into()
        }
    );
}

#[test]
fn ids_and_file_names_cannot_leave_the_model_directory() {
    for id in [
        "",
        "..",
        "../escape",
        "trailing.",
        "con",
        "LPT1.x",
        "a/b",
        "a\\b",
        "C:",
        ".hidden",
        "sp ace",
    ] {
        let mut r = valid();
        r.id = id.into();
        assert!(
            matches!(refused(r), RegistryError::Invalid { .. }),
            "id {id:?} was accepted"
        );
    }
    for name in [
        "",
        "..",
        "../w.bin",
        "w.bin.",
        "nul.bin",
        REVISION_MARKER,
        "sub\\w.bin",
        "w.bin.part",
        // In a subdirectory: every name in the path is held to the same rules.
        "/w.bin",
        "/etc/w.bin",
        "C:/w.bin",
        "sub/",
        "sub//w.bin",
        "sub/../w.bin",
        "sub/../../w.bin",
        "./w.bin",
        "sub/./w.bin",
        "sub/.hidden",
        "sub./w.bin",
        "aux/w.bin",
        "sub.part/w.bin",
        "sub/w.bin.part",
    ] {
        let mut r = valid();
        r.files[0].name = name.into();
        assert!(
            matches!(refused(r), RegistryError::Invalid { .. }),
            "file name {name:?} was accepted"
        );
    }
}

#[test]
fn file_names_may_name_subdirectories_of_the_row() {
    let mut r = valid();
    let bytes = b"weights";
    r.files = [
        "Encoder.mlmodelc/weights/weight.bin",
        "Encoder.mlmodelc/model.mil",
        "vocab.json",
    ]
    .iter()
    .map(|name| ModelFile {
        name: (*name).into(),
        url: format!("https://models.example/synthetic/x/resolve/{REV}/{name}"),
        sha256: sha256_hex(bytes),
        size: bytes.len() as u64,
    })
    .collect();
    let reg = Registry::new(vec![r]).expect("names below the row's directory are valid");
    let r = &reg.rows()[0];

    // Each lands below the row's directory, split into the OS's own path components.
    let dir = ModelDir::new(std::env::temp_dir().join("ink-engines-subdirectories"));
    let row_dir = dir.row_dir(r);
    assert_eq!(
        dir.file_path(r, &r.files[0]),
        row_dir
            .join("Encoder.mlmodelc")
            .join("weights")
            .join("weight.bin")
    );
    assert_eq!(
        dir.part_path(r, &r.files[0]),
        row_dir
            .join("Encoder.mlmodelc")
            .join("weights")
            .join("weight.bin.part")
    );
    assert_eq!(dir.file_path(r, &r.files[2]), row_dir.join("vocab.json"));
    for f in &r.files {
        let path = dir.file_path(r, f);
        assert!(path.starts_with(&row_dir), "{path:?}");
        assert!(
            path.components()
                .all(|c| !matches!(c, std::path::Component::ParentDir)),
            "{path:?}"
        );
    }
}

#[test]
fn a_file_name_that_is_also_another_files_directory_is_rejected() {
    let mut r = valid();
    let mut below = r.files[0].clone();
    below.name = format!("{}/inner.bin", r.files[0].name);
    below.url = format!(
        "https://models.example/synthetic/x/resolve/{REV}/{}",
        below.name
    );
    r.files.push(below);
    assert!(matches!(refused(r.clone()), RegistryError::Invalid { .. }));
    // Either order.
    r.files.reverse();
    assert!(matches!(refused(r), RegistryError::Invalid { .. }));
}

#[test]
fn two_files_with_one_name_are_rejected() {
    let mut r = valid();
    r.files.push(r.files[0].clone());
    assert!(matches!(refused(r), RegistryError::Invalid { .. }));
}

#[test]
fn rows_need_jobs_files_an_os_a_size_and_finite_error_rates() {
    let mut r = valid();
    r.scores.clear();
    assert!(matches!(refused(r), RegistryError::Invalid { .. }));

    let mut r = valid();
    r.files.clear();
    assert!(matches!(refused(r), RegistryError::Invalid { .. }));

    let mut r = valid();
    r.oses.clear();
    assert!(matches!(refused(r), RegistryError::Invalid { .. }));

    let mut r = valid();
    r.files[0].size = 0;
    assert!(matches!(refused(r), RegistryError::Invalid { .. }));

    for wer in [f32::NAN, f32::INFINITY, -1.0] {
        let mut r = valid();
        r.scores[0].wer = wer;
        assert!(
            matches!(refused(r), RegistryError::Invalid { .. }),
            "error rate {wer} was accepted"
        );
    }

    let mut r = valid();
    r.scores.push(JobScore {
        job: Job::DictationFinal,
        wer: 1.0,
    });
    assert!(matches!(refused(r), RegistryError::Invalid { .. }));
}

#[test]
fn a_row_the_shell_runs_fills_no_job_and_every_other_row_fills_one() {
    // Core ML runs in the Mac shell: the core only downloads such a row, so a job on it would
    // route to a model the core cannot load.
    let mut r = valid();
    r.runtime = Runtime::CoreMl;
    assert!(matches!(refused(r.clone()), RegistryError::Invalid { .. }));
    r.scores.clear();
    let reg = Registry::new(vec![r]).expect("a download-only row");
    assert!(reg.rows()[0].info().jobs.is_empty());
}

#[test]
fn builtin_rows_pass_validation() {
    let reg = Registry::builtin().unwrap();
    assert_eq!(reg.rows().len(), builtin_rows().len());
}

#[test]
fn paths_at_the_name_limits_fit_the_windows_path_budget() {
    // The limits: ids and file names up to 64 characters; the revision directory is the first 12
    // hex digits of the revision. The budget for everything below the root is 150 characters.
    const MAX: usize = 64;
    const BUDGET: usize = 150;
    assert_eq!(MAX_RELATIVE_PATH_LEN, BUDGET);

    let id = "i".repeat(MAX);
    let name = "n".repeat(MAX);
    let revision = "b".repeat(64); // the longer (SHA-256) commit form
    let mut r = row("placeholder", &[(Job::DictationFinal, 5.0)]);
    r.id = id.clone();
    r.revision = revision.clone();
    r.files = vec![ModelFile {
        name: name.clone(),
        url: format!("https://models.example/synthetic/{id}/resolve/{revision}/{name}"),
        sha256: sha256_hex(b"x"),
        size: 1,
    }];
    let reg = Registry::new(vec![r]).expect("a row at the limits is valid");
    let r = &reg.rows()[0];
    assert_eq!(r.revision, revision, "the row keeps the full revision");

    let root = std::env::temp_dir().join("ink-engines-path-budget");
    let dir = ModelDir::new(&root);
    assert_eq!(
        dir.row_dir(r).file_name().unwrap().to_str().unwrap(),
        &revision[..12]
    );
    let marker = dir.marker_path(r);
    let marker_tmp = marker.with_file_name(format!("{REVISION_MARKER}.tmp"));
    for path in [
        dir.file_path(r, &r.files[0]),
        dir.part_path(r, &r.files[0]),
        marker,
        marker_tmp,
    ] {
        let relative = path.strip_prefix(&root).unwrap().as_os_str().len();
        assert!(relative <= BUDGET, "{relative} characters below the root");
    }

    // One character over either limit is refused.
    let mut long_id = reg.rows()[0].clone();
    long_id.id.push('i');
    assert!(matches!(refused(long_id), RegistryError::Invalid { .. }));
    let mut long_name = reg.rows()[0].clone();
    long_name.files[0].name.push('n');
    assert!(matches!(refused(long_name), RegistryError::Invalid { .. }));

    // A name with subdirectories is held to the whole budget: with a 64-character id, 70
    // characters of name make 70 + 64 + 1 + 12 + 1 + 5 = 153 below the root, and are refused;
    // 67 make exactly 150 and are accepted.
    for (len, ok) in [(70, false), (67, true)] {
        let mut deep = reg.rows()[0].clone();
        let name = format!("{}/{}", "d".repeat(40), "f".repeat(len - 41));
        assert_eq!(name.len(), len);
        deep.files[0].url =
            format!("https://models.example/synthetic/{id}/resolve/{revision}/{name}");
        deep.files[0].name = name;
        match ok {
            true => {
                let reg = Registry::new(vec![deep]).unwrap();
                let r = &reg.rows()[0];
                let part = dir.part_path(r, &r.files[0]);
                let relative = part.strip_prefix(&root).unwrap().as_os_str().len();
                assert_eq!(relative, BUDGET);
            }
            false => assert!(matches!(refused(deep), RegistryError::Invalid { .. })),
        }
    }
}

fn language(size: LanguageSize) -> RowKind {
    RowKind::Language(LanguageRow {
        name: "Synthetic Chat".into(),
        size,
        chat: ChatQuirks::default(),
    })
}

#[test]
fn a_language_row_fills_no_speech_job_and_is_one_gguf_for_llama_cpp() {
    let mut chat = valid();
    chat.scores.clear();
    chat.files[0].name = "chat.gguf".into();
    chat.files[0].url = chat.files[0].url.replace("weights.bin", "chat.gguf");
    chat.kind = language(LanguageSize::Default);
    Registry::new(vec![chat.clone()]).expect("a language row");
    assert!(chat.info().jobs.is_empty());

    let mut scored = chat.clone();
    scored.scores = valid().scores;
    assert!(matches!(refused(scored), RegistryError::Invalid { .. }));
    let mut other_runtime = chat.clone();
    other_runtime.runtime = Runtime::SherpaOnnx;
    assert!(matches!(
        refused(other_runtime),
        RegistryError::Invalid { .. }
    ));
    let mut two_files = chat.clone();
    let mut second = two_files.files[0].clone();
    second.name = "more.gguf".into();
    second.url = second.url.replace("chat.gguf", "more.gguf");
    two_files.files.push(second);
    assert!(matches!(refused(two_files), RegistryError::Invalid { .. }));
    let mut not_gguf = valid();
    not_gguf.scores.clear();
    not_gguf.kind = language(LanguageSize::Default);
    assert!(matches!(refused(not_gguf), RegistryError::Invalid { .. }));
    for name in ["", " Synthetic", "Synthetic\n", &"x".repeat(65)] {
        let mut named = chat.clone();
        named.kind = RowKind::Language(LanguageRow {
            name: name.into(),
            size: LanguageSize::Default,
            chat: ChatQuirks::default(),
        });
        assert!(
            matches!(refused(named), RegistryError::Invalid { .. }),
            "{name:?}"
        );
    }
}

#[test]
fn the_suggested_language_model_is_the_default_unless_memory_is_under_12_gb() {
    let row = |id: &str, size: LanguageSize, oses: &[Os]| {
        let mut r = valid();
        r.id = id.into();
        r.scores.clear();
        r.files[0].name = "chat.gguf".into();
        r.files[0].url = r.files[0].url.replace("weights.bin", "chat.gguf");
        r.oses = oses.to_vec();
        r.kind = language(size);
        r
    };
    let rows = [
        valid(),
        row("synthetic-small", LanguageSize::Small, &[Os::Windows]),
        row("synthetic-default", LanguageSize::Default, &[Os::Windows]),
    ];
    let pick =
        |memory: Option<u64>| suggested_language(&rows, Os::Windows, memory).map(|r| r.id.as_str());
    assert_eq!(pick(Some(16 << 30)), Some("synthetic-default"));
    // A PC fitted with 12 GB reports a little under 12 GiB.
    assert_eq!(pick(Some(12_500_000_000)), Some("synthetic-default"));
    assert_eq!(pick(Some(SMALL_BELOW_MEMORY)), Some("synthetic-default"));
    assert_eq!(pick(Some(SMALL_BELOW_MEMORY - 1)), Some("synthetic-small"));
    assert_eq!(pick(Some(8 << 30)), Some("synthetic-small"));
    // Memory that could not be read: the Default.
    assert_eq!(pick(None), Some("synthetic-default"));
    // No language row for the OS (the Mac): none.
    assert_eq!(suggested_language(&rows, Os::MacOs, Some(8 << 30)), None);
    // With only one size there, that one, whatever the memory.
    assert_eq!(
        suggested_language(&rows[..2], Os::Windows, Some(64 << 30)).map(|r| r.id.as_str()),
        Some("synthetic-small")
    );
}
