//! Writes the licence notices of the third-party Rust crates the release links into the core
//! (`mac/Sources/Inkwell/Generated/RustNotices.swift`, shown in Settings > About).
//!
//! ```text
//! cargo run -p ink-ffi --bin ink-notices                  # regenerate
//! cargo run -p ink-ffi --bin ink-notices -- --check       # fail if the file differs from a fresh run
//! cargo run -p ink-ffi --bin ink-notices -- --check-lock  # fail if it was made from another lock
//! ```
//!
//! Offline: it reads cargo's resolution (`cargo tree` and `cargo metadata`, both `--offline
//! --locked`) and each crate's unpacked package in the local registry. A crate whose package is
//! not there fails cargo itself; `mac/scripts/rust-notices.sh` fetches them first (`cargo fetch`,
//! the only step that uses the network). `--check-lock` needs neither: it compares the fingerprint
//! the file records with `core/Cargo.lock`, and is what the core's tests run.
//!
//! **What is in it.** ink-ffi's normal dependencies for the release's target and features
//! ([`RELEASE_FEATURES`], the set `mac/scripts/build-mac.sh --engines` builds), as the release
//! build resolves them ([`graph`]). Build scripts, proc macros and dev-dependencies never reach
//! the app, and the workspace's own crates are Inkwell's; all are left out.
//!
//! **Which texts.** Each crate's licence files (`LICENSE*`, `LICENCE*`, `COPYING*`,
//! `COPYRIGHT*`, `NOTICE*`, `UNLICENSE*` at the top of its package), matched to its licence
//! expression by their text ([`licence`]). Where it offers a choice, MIT is taken first, with
//! the crate's own copyright line; an MIT file naming no holder gets the authors from the crate's
//! manifest, marked as such. NOTICE files always ship (Apache-2.0 §4(d)).
//!
//! **Nothing is skipped.** A crate whose package carries no text of any licence it offers is an
//! error naming it, unless `notices/overrides.txt` supplies a text for that exact version
//! ([`overrides`]); an override no crate needs is an error too. Every error is listed before
//! anything is written.

mod graph;
mod licence;
mod overrides;
mod swift;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::graph::Package;
use crate::licence::LicenceFile;
use crate::overrides::Overrides;
use crate::swift::CrateNotice;

/// The features the release builds the core with: `release_features` in
/// `mac/scripts/build-mac.sh` (a test holds them equal).
const RELEASE_FEATURES: &str = "engine-llama,ink-engines/engine-silero,ink-engines/engine-nemo";
/// The release's one target: the app is Apple silicon only.
const TARGET: &str = "aarch64-apple-darwin";
/// The generated file, from the repository root.
const SWIFT_OUT: &str = "mac/Sources/Inkwell/Generated/RustNotices.swift";
/// The overrides table and its texts, from the repository root.
const NOTICES_DIR: &str = "core/crates/ink-ffi/notices";

/// The file names, lower-cased, that hold a crate's licence.
const LICENCE_FILE_PREFIXES: [&str; 6] = [
    "license",
    "licence",
    "copying",
    "copyright",
    "notice",
    "unlicense",
];

fn repo_root() -> PathBuf {
    // core/crates/ink-ffi -> the repository root.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// The cargo that runs this (`cargo run` sets `CARGO`), so the pinned toolchain resolves.
fn cargo() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}

fn run_cargo(core: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cargo())
        .args(args)
        .current_dir(core)
        .output()
        .map_err(|e| format!("cargo {}: {e}", args[0]))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let hint = if stderr.contains("offline") || stderr.contains("download") {
            "\n(a crate's package is not in the local registry: run mac/scripts/rust-notices.sh, which fetches them)"
        } else {
            ""
        };
        return Err(format!(
            "cargo {} failed:\n{}{hint}",
            args[0],
            stderr.trim_end()
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("cargo {}: {e}", args[0]))
}

/// A crate's licence files, sorted by name (case-insensitively), each cleaned ([`swift::clean`]).
fn licence_files(dir: &Path) -> Result<Vec<LicenceFile>, String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("its package is not readable ({e})"))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("its package is not readable ({e})"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let lower = name.to_ascii_lowercase();
        if !LICENCE_FILE_PREFIXES.iter().any(|p| lower.starts_with(p)) || !entry.path().is_file() {
            continue;
        }
        let bytes = std::fs::read(entry.path()).map_err(|e| format!("{name}: {e}"))?;
        let text = String::from_utf8(bytes).map_err(|_| format!("{name} is not UTF-8"))?;
        let text = swift::clean(&text).map_err(|e| format!("{name} {e}"))?;
        files.push(LicenceFile {
            name,
            text,
            supplied: None,
        });
    }
    sort_files(&mut files);
    Ok(files)
}

fn sort_files(files: &mut [LicenceFile]) {
    files.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
            .then(a.name.cmp(&b.name))
    });
}

/// A version's numeric parts, for sorting (`1.10.0` after `1.9.0`).
fn version_key(version: &str) -> Vec<(u64, String)> {
    version
        .split(['.', '-', '+'])
        .map(|part| (part.parse().unwrap_or(u64::MAX), part.to_string()))
        .collect()
}

/// Every crate's notice, sorted by name and version. Errors name every crate that cannot ship
/// its notice, and every override no crate used.
fn notices(
    crates: &[Package],
    overrides: &Overrides,
    texts: &Path,
) -> Result<Vec<CrateNotice>, Vec<String>> {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    let mut used = Vec::new();
    for c in crates {
        let key = (c.name.clone(), c.version.clone());
        let id = format!("{} {}", c.name, c.version);
        let Some(licence) = &c.licence else {
            errors.push(format!(
                "{id}: declares no SPDX licence (only a licence file); its notice needs a decision"
            ));
            continue;
        };
        let expr = match licence::parse(licence) {
            Ok(e) => e,
            Err(e) => {
                errors.push(format!("{id}: {e}"));
                continue;
            }
        };
        let mut files = match licence_files(&c.dir) {
            Ok(f) => f,
            Err(e) => {
                errors.push(format!("{id}: {e}"));
                continue;
            }
        };
        let selection = match (
            licence::select(&expr, &files, &c.authors),
            overrides.get(&key),
        ) {
            (Ok(_), Some(o)) => {
                errors.push(format!(
                    "{id}: its package now carries its licence, so the override ({}) must go",
                    o.text
                ));
                continue;
            }
            (Ok(s), None) => s,
            (Err(e), None) => {
                errors.push(format!("{id}: {e}"));
                continue;
            }
            (Err(_), Some(o)) => {
                used.push(key.clone());
                let path = texts.join(&o.text);
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| format!("notices/texts/{}: {e}", o.text))
                    .and_then(|t| {
                        swift::clean(&t).map_err(|e| format!("notices/texts/{} {e}", o.text))
                    });
                let text = match text {
                    Ok(t) => t,
                    Err(e) => {
                        errors.push(format!("{id}: {e}"));
                        continue;
                    }
                };
                let name = o.text.trim_end_matches(".txt").to_string();
                files.push(LicenceFile {
                    name,
                    text,
                    supplied: Some(o.reason.clone()),
                });
                sort_files(&mut files);
                match licence::select(&expr, &files, &c.authors) {
                    Ok(s) => s,
                    Err(e) => {
                        errors.push(format!("{id}: with its override, {e}"));
                        continue;
                    }
                }
            }
        };
        out.push(CrateNotice {
            name: c.name.clone(),
            version: c.version.clone(),
            licence: licence.clone(),
            shown: selection.shown.clone(),
            text: swift::compose(&selection.files),
        });
    }
    for key in overrides.keys().filter(|k| !used.contains(k)) {
        if !crates
            .iter()
            .any(|c| (&c.name, &c.version) == (&key.0, &key.1))
        {
            errors.push(format!(
                "{} {}: overridden, but not in the release",
                key.0, key.1
            ));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    out.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| version_key(&a.version).cmp(&version_key(&b.version)))
    });
    Ok(out)
}

/// The Swift file, from cargo's resolution and the packages on this machine.
fn generate(root: &Path) -> Result<String, Vec<String>> {
    let core = root.join("core");
    let tree = run_cargo(
        &core,
        &[
            "tree",
            "--offline",
            "--locked",
            "-p",
            "ink-ffi",
            "-e",
            "normal,no-proc-macro",
            "--target",
            TARGET,
            "--features",
            RELEASE_FEATURES,
            "--prefix",
            "none",
            "--format",
            "{p}",
        ],
    )
    .and_then(|t| graph::tree(&t))
    .map_err(|e| vec![e])?;
    let packages = run_cargo(
        &core,
        &[
            "metadata",
            "--offline",
            "--locked",
            "--format-version",
            "1",
            "--filter-platform",
            TARGET,
            "--features",
            RELEASE_FEATURES,
        ],
    )
    .and_then(|m| graph::packages(&m))
    .map_err(|e| vec![e])?;
    let crates = graph::release_crates(&tree, &packages)?;
    let notices_dir = root.join(NOTICES_DIR);
    let table = std::fs::read_to_string(notices_dir.join("overrides.txt"))
        .map_err(|e| vec![format!("{NOTICES_DIR}/overrides.txt: {e}")])?;
    let overrides =
        overrides::parse(&table).map_err(|e| vec![format!("{NOTICES_DIR}/overrides.txt: {e}")])?;
    let notices = notices(&crates, &overrides, &notices_dir.join("texts"))?;
    let lock = std::fs::read_to_string(core.join("Cargo.lock"))
        .map_err(|e| vec![format!("core/Cargo.lock: {e}")])?;
    let rendered = swift::render(
        RELEASE_FEATURES,
        TARGET,
        &swift::fingerprint(RELEASE_FEATURES, TARGET, &lock),
        &notices,
    );
    // Nothing of this machine: no path of the checkout, the registry or the home directory.
    let mut local: Vec<String> = crates
        .iter()
        .filter_map(|c| c.dir.parent().map(|p| p.display().to_string()))
        .collect();
    local.push(
        root.canonicalize()
            .unwrap_or_else(|_| root.to_path_buf())
            .display()
            .to_string(),
    );
    local.extend(std::env::var("HOME").ok().filter(|h| h.len() > 1));
    if let Some(found) = local.iter().find(|p| rendered.contains(p.as_str())) {
        return Err(vec![format!(
            "the output names a path of this machine ({found})"
        )]);
    }
    Ok(rendered)
}

/// The crates a generated file lists, as `name version`.
fn listed(swift: &str) -> Vec<String> {
    swift
        .lines()
        .filter_map(|l| {
            let rest = l.trim_start().strip_prefix("name: \"")?;
            let (name, rest) = rest.split_once('"')?;
            let version = rest.strip_prefix(", version: \"")?.split('"').next()?;
            Some(format!("{name} {version}"))
        })
        .collect()
}

/// Compares the file with a fresh run.
fn check(root: &Path, generated: &str) -> Result<(), String> {
    let current = std::fs::read_to_string(root.join(SWIFT_OUT))
        .unwrap_or_default()
        .replace("\r\n", "\n");
    if current == generated {
        return Ok(());
    }
    let (was, now) = (listed(&current), listed(generated));
    let added: Vec<&String> = now.iter().filter(|c| !was.contains(c)).collect();
    let removed: Vec<&String> = was.iter().filter(|c| !now.contains(c)).collect();
    let mut why = String::new();
    if !added.is_empty() {
        why.push_str(&format!(
            "\n  not in it yet: {}",
            added
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !removed.is_empty() {
        why.push_str(&format!(
            "\n  no longer linked: {}",
            removed
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if why.is_empty() {
        why.push_str("\n  the same crates; a text, a licence or the lock's fingerprint differs");
    }
    Err(format!(
        "{SWIFT_OUT} is stale:{why}\nRegenerate: cargo run -p ink-ffi --bin ink-notices"
    ))
}

/// Compares the fingerprint the file records with the lock as it is now. Needs no package.
fn check_lock(root: &Path) -> Result<(), String> {
    let swift =
        std::fs::read_to_string(root.join(SWIFT_OUT)).map_err(|e| format!("{SWIFT_OUT}: {e}"))?;
    let lock = std::fs::read_to_string(root.join("core/Cargo.lock"))
        .map_err(|e| format!("core/Cargo.lock: {e}"))?;
    let recorded = swift::recorded(&swift, "lockFingerprint")
        .ok_or(format!("{SWIFT_OUT} records no lockFingerprint"))?;
    let features = swift::recorded(&swift, "features").unwrap_or_default();
    let target = swift::recorded(&swift, "target").unwrap_or_default();
    if (features.as_str(), target.as_str()) != (RELEASE_FEATURES, TARGET) {
        return Err(format!(
            "{SWIFT_OUT} was made for {target} with {features}; the release builds {TARGET} with {RELEASE_FEATURES}.\n\
             Regenerate: cargo run -p ink-ffi --bin ink-notices"
        ));
    }
    if recorded != swift::fingerprint(RELEASE_FEATURES, TARGET, &lock) {
        return Err(format!(
            "{SWIFT_OUT} was made from another core/Cargo.lock: a dependency changed and its notice may be missing.\n\
             Regenerate: cargo run -p ink-ffi --bin ink-notices (or mac/scripts/rust-notices.sh, which fetches the packages first)"
        ));
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = repo_root();
    let result = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--check-lock"] => check_lock(&root),
        [] | ["--check"] => match generate(&root) {
            Err(errors) => {
                eprintln!("ink-notices: {} problem(s), nothing written:", errors.len());
                for e in &errors {
                    eprintln!("  {e}");
                }
                if errors.iter().any(|e| e.contains("carries")) {
                    eprintln!(
                        "A crate without its licence text needs {NOTICES_DIR}/overrides.txt: add a line only after reading its upstream licence."
                    );
                }
                return ExitCode::FAILURE;
            }
            Ok(generated) if args.is_empty() => {
                let path = root.join(SWIFT_OUT);
                let written = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(&path, &generated));
                written
                    .map(|()| println!("wrote {SWIFT_OUT} ({} crates)", listed(&generated).len()))
                    .map_err(|e| format!("{SWIFT_OUT}: {e}"))
            }
            Ok(generated) => check(&root, &generated),
        },
        _ => Err("usage: ink-notices [--check | --check-lock]".to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ink-notices: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::graph::CRATES_IO;

    const MIT: &str = "Copyright (c) 2020 Someone\n\nPermission is hereby granted, free of charge, to any person obtaining a copy of this software.\n\nThe above copyright notice and this permission notice shall be included in all copies.\n";
    const APACHE: &str = "Apache License\nVersion 2.0, January 2004\nTERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION\n";

    /// A fresh directory under the system's temporary one.
    fn scratch() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ink-notices-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A crate whose package holds `files`.
    fn package(registry: &Path, name: &str, licence: &str, files: &[(&str, &[u8])]) -> Package {
        let dir = registry.join(format!("{name}-1.0.0"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.join("README.md"), "not a licence").unwrap();
        for (file, text) in files {
            std::fs::write(dir.join(file), text).unwrap();
        }
        Package {
            name: name.into(),
            version: "1.0.0".into(),
            licence: Some(licence.into()),
            authors: vec![],
            source: Some(CRATES_IO.into()),
            dir,
            workspace: false,
        }
    }

    fn committed() -> String {
        std::fs::read_to_string(repo_root().join(SWIFT_OUT))
            .expect("the generated notices are checked in")
            .replace("\r\n", "\n")
    }

    /// The staleness check every CI run makes, with no package needed: the checked-in notices were
    /// generated from this Cargo.lock. `ink-notices --check` (mac/scripts/rust-notices.sh) compares
    /// the whole file.
    #[test]
    fn the_checked_in_notices_were_generated_from_the_current_lock() {
        if let Err(e) = check_lock(&repo_root()) {
            panic!("{e}");
        }
    }

    #[test]
    fn the_release_features_are_the_ones_build_mac_builds() {
        let script = std::fs::read_to_string(repo_root().join("mac/scripts/build-mac.sh")).unwrap();
        let line = script
            .lines()
            .find_map(|l| l.strip_prefix("release_features=\""))
            .and_then(|l| l.strip_suffix('"'))
            .expect("build-mac.sh sets release_features");
        assert_eq!(line, RELEASE_FEATURES);
    }

    #[test]
    fn the_checked_in_notices_name_no_path_of_a_machine() {
        let swift = committed();
        // The macOS home prefix is spelt in two pieces so the pre-push privacy grep, which
        // refuses that path in a diff, does not match this test.
        for local in [
            concat!("/Us", "ers/"),
            "/home/",
            "\\Users\\",
            ".cargo/registry",
            "registry/src/",
            "/private/tmp/",
        ] {
            assert!(!swift.contains(local), "the notices hold `{local}`");
        }
        assert!(
            listed(&swift).len() > 100,
            "the release links over a hundred crates"
        );
        assert!(
            listed(&swift).iter().all(|c| !c.starts_with("ink-")),
            "no workspace crate"
        );
    }

    #[test]
    fn every_override_line_names_a_text_that_is_a_licence() {
        let dir = repo_root().join(NOTICES_DIR);
        let table =
            overrides::parse(&std::fs::read_to_string(dir.join("overrides.txt")).unwrap()).unwrap();
        assert!(!table.is_empty());
        for ((name, version), o) in &table {
            let text = std::fs::read_to_string(dir.join("texts").join(&o.text))
                .unwrap_or_else(|e| panic!("{name} {version}: texts/{}: {e}", o.text));
            assert!(
                !licence::classify(&text).is_empty(),
                "{name} {version}: texts/{} is no licence text",
                o.text
            );
        }
    }

    #[test]
    fn a_crate_without_its_licence_text_is_an_error_naming_it_and_nothing_else_is_skipped() {
        let registry = scratch();
        let crates = [
            package(
                &registry,
                "good",
                "MIT OR Apache-2.0",
                &[
                    ("LICENSE-MIT", MIT.as_bytes()),
                    ("LICENSE-APACHE", APACHE.as_bytes()),
                ],
            ),
            package(&registry, "bare", "MIT", &[]),
            package(
                &registry,
                "statement",
                "MIT OR Apache-2.0",
                &[("LICENSE", b"MIT OR Apache-2.0")],
            ),
            package(
                &registry,
                "binary",
                "MIT",
                &[("LICENSE", &[0xff, 0xfe, 0x00])],
            ),
        ];
        let errors = notices(&crates, &Overrides::new(), &registry).unwrap_err();
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(
            errors[0].starts_with(
                "bare 1.0.0: its licence is `MIT`, but its package carries no licence text"
            ),
            "{errors:?}"
        );
        assert!(
            errors[1].starts_with("statement 1.0.0:") && errors[1].contains("(LICENSE)"),
            "{errors:?}"
        );
        assert!(
            errors[2].starts_with("binary 1.0.0: LICENSE is not UTF-8"),
            "{errors:?}"
        );
        let _ = std::fs::remove_dir_all(registry);
    }

    #[test]
    fn an_override_supplies_a_missing_text_and_says_so() {
        let registry = scratch();
        let texts = registry.join("texts");
        std::fs::create_dir_all(&texts).unwrap();
        std::fs::write(texts.join("Apache-2.0.txt"), APACHE).unwrap();
        let crates = [
            package(&registry, "zeta", "MIT", &[("LICENSE", MIT.as_bytes())]),
            package(
                &registry,
                "bare",
                "Zlib OR Apache-2.0 OR MIT",
                &[("COPYING", b"Copyright 2020 Someone")],
            ),
        ];
        let mut table = Overrides::new();
        table.insert(
            ("bare".into(), "1.0.0".into()),
            overrides::Override {
                text: "Apache-2.0.txt".into(),
                reason: "the standard text".into(),
            },
        );
        let got = notices(&crates, &table, &texts).unwrap();
        assert_eq!(
            got.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(),
            ["bare", "zeta"],
            "sorted"
        );
        assert_eq!(got[0].shown, "Apache-2.0");
        assert_eq!(
            got[0].text,
            "--- Apache-2.0 ---\n\
             [The published crate carries no text of this licence. Supplied by Inkwell: the standard text.]\n\
             Apache License\nVersion 2.0, January 2004\nTERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION\n\n\
             --- COPYING ---\nCopyright 2020 Someone",
            "files in name order, the supplied one among them"
        );
        let _ = std::fs::remove_dir_all(registry);
    }

    #[test]
    fn an_override_no_crate_needs_is_an_error() {
        let registry = scratch();
        let crates = [package(
            &registry,
            "good",
            "MIT",
            &[("LICENSE", MIT.as_bytes())],
        )];
        let mut table = Overrides::new();
        let o = || overrides::Override {
            text: "MIT.txt".into(),
            reason: "why".into(),
        };
        table.insert(("good".into(), "1.0.0".into()), o());
        table.insert(("gone".into(), "2.0.0".into()), o());
        let errors = notices(&crates, &table, &registry).unwrap_err();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(
            errors[0].starts_with("good 1.0.0: its package now carries its licence"),
            "{errors:?}"
        );
        assert!(
            errors[1].starts_with("gone 2.0.0: overridden, but not in the release"),
            "{errors:?}"
        );
        let _ = std::fs::remove_dir_all(registry);
    }

    #[test]
    fn versions_sort_by_number() {
        let mut v = vec!["1.10.0", "1.9.0", "0.2.0-alpha"];
        v.sort_by_key(|s| version_key(s));
        assert_eq!(v, ["0.2.0-alpha", "1.9.0", "1.10.0"]);
    }
}
