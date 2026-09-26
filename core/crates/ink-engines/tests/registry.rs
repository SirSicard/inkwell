//! Registry rows are validated before anything downloads, routes or loads them.

mod common;

use common::{REV, row, sha256_hex};
use ink_core::Job;
use ink_engines::{
    ALLOWED_WEIGHT_LICENCES, EngineRow, JobScore, MAX_RELATIVE_PATH_LEN, ModelDir, ModelFile, Os,
    REVISION_MARKER, Registry, RegistryError, builtin_rows,
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
        "sub/w.bin",
        "sub\\w.bin",
        "w.bin.part",
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
}
