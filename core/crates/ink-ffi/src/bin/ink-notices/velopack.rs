//! The crates of Velopack's Setup.exe and Update.exe (`ink-notices --velopack`).
//!
//! Inkwell's Windows installer and updater are Velopack's own prebuilt programs: vpk, the dotnet
//! tool `windows/.config/dotnet-tools.json` pins, packs its release's `setup_x64.exe` or
//! `setup_arm64.exe` as the installer and its `update_*.exe` into the app
//! (`windows/scripts/pack.ps1`). They are Rust programs, so the crates compiled into them need
//! their notices as the core's do; no cargo-deny run sees them.
//!
//! **What is in it.** velopack_bins' normal dependencies (its `setup` and `update` binaries share
//! them), as Velopack's Windows build resolves them: its lock and `--features windows`, for its x64
//! target (its build-rust.yml builds x64 for `x86_64-win7-windows-msvc`, with `-Z build-std`) and
//! ARM64's. Both targets' crates are listed, merged: one About shows them in either app. The inputs
//! are Velopack's files at the release's tag, in [`INPUT`]: its workspace manifest and lock, and the
//! manifests of the three crates the binaries are built from, each verbatim, its name ending in
//! `.upstream` so no tool takes it for a manifest of this repository. [`workspace`] makes a cargo
//! workspace of the three from them in a scratch directory, where cargo resolves them; every crate
//! it resolves must be in the lock at the same version ([`drifted`]).
//!
//! **Network.** Velopack's crates are not in core/Cargo.lock, so a run fetches the ones cargo needs
//! to resolve these from crates.io, as cargo fetches any crate (`CARGO_NET_OFFLINE=true` forbids it).
//!
//! **Licences.** A crate shown under a licence outside the allowlist ([`licence::PREFERENCE`], the
//! one in core/deny.toml) that is not one of [`EXCEPTIONS`] for that crate stops the run.
//!
//! **A new vpk.** [`VERSION`] is the vpk dotnet-tools.json pins and the Velopack package
//! windows/Directory.Packages.props pins (a test holds the three equal, and NoticesTests on
//! Windows): after a bump the tests fail until [`INPUT`] holds the new release's files, VERSION
//! names it and the file is regenerated. `ink-notices --velopack --check` (win.yml) fails on any
//! difference from a fresh run.

use std::path::Path;

use crate::graph;
use crate::licence;
use crate::overrides;
use crate::swift::{self, CrateNotice};

/// The Velopack release whose Setup.exe and Update.exe Inkwell ships.
pub const VERSION: &str = "1.2.161";
/// Its files, from the repository root: `Cargo.toml.upstream`, `Cargo.lock.upstream` and
/// `src/<member>/Cargo.toml.upstream`, from velopack/velopack at the tag 1.2.161 (commit
/// 92d6a1c91716729d449034df5c50307dcce39493), and `overrides.txt`.
pub const INPUT: &str = "core/crates/ink-ffi/notices/velopack";
/// The generated C#, from the repository root.
pub const OUT: &str = "windows/Inkwell.Core/Screens/About/VelopackNotices.g.cs";
/// The package whose binaries ship.
const PACKAGE: &str = "velopack_bins";
/// The feature Velopack's Windows build turns on (`--features windows`).
pub const FEATURES: &str = "windows";
/// Velopack's x64 and ARM64 Windows targets, as its build-rust.yml builds them at the tag.
pub const TARGETS: [&str; 2] = ["x86_64-win7-windows-msvc", "aarch64-pc-windows-msvc"];
/// [`TARGETS`], as the generated file records them (a test holds them equal).
pub const TARGET_LIST: &str = "x86_64-win7-windows-msvc,aarch64-pc-windows-msvc";
/// The tool manifest that pins vpk, from the repository root.
const TOOLS: &str = "windows/.config/dotnet-tools.json";
/// The package versions of the Windows solution, Velopack's among them.
const PROPS: &str = "windows/Directory.Packages.props";
/// The workspace members the binaries are built from: velopack_bins, and velopack_l18n and velopack
/// (lib-rust), which it depends on.
const MEMBERS: [&str; 3] = ["src/bins", "src/l18n", "src/lib-rust"];

/// The licences outside the allowlist that Velopack's crates may be shown under, each for the
/// crates named: the maintainer's scoped exceptions (2026-09-30), recorded in THIRD_PARTY.md.
/// Unicode-3.0 is ICU4X's (under `url`'s international domain names and fluent's locales);
/// CDLA-Permissive-2.0 is webpki-roots', Mozilla's root certificates as data.
pub const EXCEPTIONS: [(&str, &[&str]); 2] = [
    (
        "Unicode-3.0",
        &[
            "icu_collections",
            "icu_locale_core",
            "icu_locid",
            "icu_locid_transform",
            "icu_locid_transform_data",
            "icu_normalizer",
            "icu_normalizer_data",
            "icu_properties",
            "icu_properties_data",
            "icu_provider",
            "litemap",
            "potential_utf",
            "tinystr",
            "writeable",
            "yoke",
            "zerofrom",
            "zerotrie",
            "zerovec",
        ],
    ),
    ("CDLA-Permissive-2.0", &["webpki-roots"]),
];

/// Whether `tools` (the text of [`TOOLS`]) pins vpk at [`VERSION`].
fn vpk_is_ours(tools: &str) -> Result<(), String> {
    let v: serde_json::Value = serde_json::from_str(tools).map_err(|e| format!("{TOOLS}: {e}"))?;
    let pinned = v["tools"]["vpk"]["version"]
        .as_str()
        .ok_or(format!("{TOOLS} pins no vpk version"))?;
    if pinned != VERSION {
        return Err(format!(
            "{TOOLS} pins vpk {pinned}, but these notices are Velopack {VERSION}'s: put {pinned}'s files in {INPUT}, set VERSION in core/crates/ink-ffi/src/bin/ink-notices/velopack.rs and run `cargo run -p ink-ffi --bin ink-notices -- --velopack`"
        ));
    }
    Ok(())
}

/// Whether the Velopack the Windows build uses is [`VERSION`]: the vpk that packs the installer
/// ([`TOOLS`]) and the package the app links ([`PROPS`]).
fn pinned(root: &Path) -> Result<(), String> {
    let read = |p: &str| std::fs::read_to_string(root.join(p)).map_err(|e| format!("{p}: {e}"));
    vpk_is_ours(&read(TOOLS)?)?;
    let props = read(PROPS)?;
    let line = props
        .lines()
        .find(|l| l.contains("Include=\"Velopack\""))
        .ok_or(format!("{PROPS} pins no Velopack package"))?;
    if !line.contains(&format!("Version=\"{VERSION}\"")) {
        return Err(format!(
            "{PROPS} pins another Velopack than {VERSION}: {}",
            line.trim()
        ));
    }
    Ok(())
}

/// An input file's text, line ends normalised.
fn read(input: &Path, name: &str) -> Result<String, String> {
    std::fs::read_to_string(input.join(name))
        .map(|t| t.replace("\r\n", "\n"))
        .map_err(|e| format!("{INPUT}/{name}: {e}"))
}

/// Velopack's workspace manifest with its members narrowed to [`MEMBERS`]. An error if any of them
/// is no longer a member.
fn narrow_members(root: &str) -> Result<String, String> {
    let start = root
        .find("members = [")
        .ok_or("Velopack's Cargo.toml has no `members = [`")?;
    let end = root[start..]
        .find(']')
        .map(|i| start + i + 1)
        .ok_or("Velopack's Cargo.toml: its `members = [` is not closed")?;
    let listed = &root[start..end];
    if let Some(gone) = MEMBERS
        .iter()
        .find(|m| !listed.contains(&format!("\"{m}\"")))
    {
        return Err(format!(
            "Velopack's workspace no longer has {gone}: its binaries' crates need a new look"
        ));
    }
    let members: Vec<String> = MEMBERS.iter().map(|m| format!("\"{m}\"")).collect();
    Ok(format!(
        "{}members = [{}]{}",
        &root[..start],
        members.join(", "),
        &root[end..]
    ))
}

/// A manifest without its dev-dependency tables (`[dev-dependencies]`, and a target's): test-only
/// crates never reach the binaries, and `cargo metadata` would fetch them.
fn without_dev_dependencies(manifest: &str) -> String {
    let mut out = Vec::new();
    let mut skipping = false;
    for line in manifest.lines() {
        if let Some(header) = line.trim().strip_prefix('[') {
            skipping = header.trim_end_matches(']').ends_with("dev-dependencies");
        }
        if !skipping {
            out.push(line);
        }
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// The source files a member's manifest names (`path = "..."`), and `src/lib.rs`: empty ones are
/// written, so cargo reads the targets; nothing is compiled.
fn sources(manifest: &str) -> Vec<String> {
    let mut out = vec!["src/lib.rs".to_string()];
    for line in manifest.lines() {
        if let Some(path) = line
            .trim()
            .strip_prefix("path = \"")
            .and_then(|l| l.strip_suffix('"'))
            && path.ends_with(".rs")
            && !out.iter().any(|p| p == path)
        {
            out.push(path.to_string());
        }
    }
    out
}

/// Writes the cargo workspace of [`MEMBERS`] into `dir` from the files in `input`: the workspace
/// manifest narrowed to them ([`narrow_members`]), the lock as it is, and each member's manifest
/// without its dev-dependencies, with empty sources.
pub fn workspace(input: &Path, dir: &Path) -> Result<(), String> {
    let write = |name: &str, text: &str| {
        let path = dir.join(name);
        path.parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, text))
            .map_err(|e| format!("the scratch workspace's {name}: {e}"))
    };
    write(
        "Cargo.toml",
        &narrow_members(&read(input, "Cargo.toml.upstream")?)?,
    )?;
    write("Cargo.lock", &read(input, "Cargo.lock.upstream")?)?;
    for member in MEMBERS {
        let manifest =
            without_dev_dependencies(&read(input, &format!("{member}/Cargo.toml.upstream"))?);
        for source in sources(&manifest) {
            write(&format!("{member}/{source}"), "")?;
        }
        write(&format!("{member}/Cargo.toml"), &manifest)?;
    }
    Ok(())
}

/// The crates of `resolved` (the lock cargo left in the scratch workspace) that Velopack's own lock
/// does not have at that version: cargo resolved them afresh, so they are not what Velopack built.
pub fn drifted(resolved: &str, upstream: &str) -> Vec<String> {
    let locked = crate::locked(upstream);
    crate::locked(resolved)
        .into_iter()
        .filter(|c| !locked.contains(c))
        .map(|(name, version)| {
            format!("{name} {version}: not in Velopack's Cargo.lock at that version; cargo resolved it afresh")
        })
        .collect()
}

/// The crates shown under a licence that is neither on the allowlist nor one of [`EXCEPTIONS`]
/// for that crate.
pub fn not_allowed(notices: &[CrateNotice]) -> Vec<String> {
    let mut out = Vec::new();
    for n in notices {
        for id in n.shown.split(" AND ") {
            let excepted = EXCEPTIONS
                .iter()
                .any(|(licence, crates)| *licence == id && crates.contains(&n.name.as_str()));
            if !licence::PREFERENCE.contains(&id) && !excepted {
                out.push(format!(
                    "{} {}: shown under {id}, which is neither allowed nor one of Velopack's exceptions for it; its notice needs a decision",
                    n.name, n.version
                ));
            }
        }
    }
    out
}

/// The generated file, from the files in [`INPUT`] and cargo's resolution of them.
pub fn generate(root: &Path) -> Result<String, Vec<String>> {
    pinned(root).map_err(|e| vec![e])?;
    let input = root.join(INPUT);
    let dir = std::env::temp_dir().join(format!("ink-notices-velopack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let out = workspace(&input, &dir)
        .map_err(|e| vec![e])
        .and_then(|()| resolve(root, &input, &dir));
    let _ = std::fs::remove_dir_all(&dir);
    out
}

fn resolve(root: &Path, input: &Path, dir: &Path) -> Result<String, Vec<String>> {
    let mut tree: Vec<(String, String)> = Vec::new();
    for target in TARGETS {
        for c in crate::crate_tree(dir, PACKAGE, target, FEATURES, false).map_err(|e| vec![e])? {
            if !tree.contains(&c) {
                tree.push(c);
            }
        }
    }
    let upstream = read(input, "Cargo.lock.upstream").map_err(|e| vec![e])?;
    let resolved = std::fs::read_to_string(dir.join("Cargo.lock"))
        .map_err(|e| vec![format!("the scratch workspace's Cargo.lock: {e}")])?
        .replace("\r\n", "\n");
    let drift = drifted(&resolved, &upstream);
    if !drift.is_empty() {
        return Err(drift);
    }
    let features = format!("{PACKAGE}/{FEATURES}");
    let mut args = vec!["metadata", "--format-version", "1"];
    for target in TARGETS {
        args.extend(["--filter-platform", target]);
    }
    args.extend(["--features", features.as_str()]);
    let packages = crate::run_cargo(dir, &args)
        .and_then(|m| graph::packages(&m))
        .map_err(|e| vec![e])?;
    let crates = graph::release_crates(&tree, &packages)?;
    let table = read(input, "overrides.txt").map_err(|e| vec![e])?;
    let overrides =
        overrides::parse(&table).map_err(|e| vec![format!("{INPUT}/overrides.txt: {e}")])?;
    let notices = crate::notices(
        &crates,
        &overrides,
        &root.join(crate::NOTICES_DIR).join("texts"),
    )?;
    let refused = not_allowed(&notices);
    if !refused.is_empty() {
        return Err(refused);
    }
    let rendered = crate::csharp::render_velopack(
        VERSION,
        FEATURES,
        TARGET_LIST,
        &swift::fingerprint(FEATURES, TARGET_LIST, &upstream),
        &notices,
    );
    let mut local = crate::machine_paths(&crates, root, |k| std::env::var(k).ok());
    local.push(dir.display().to_string());
    if let Some(found) = local.iter().find(|p| rendered.contains(p.as_str())) {
        return Err(vec![format!(
            "the output names a path of this machine ({found})"
        )]);
    }
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo_root;

    /// The Velopack the notices are for is the one the Windows build uses: the vpk that packs the
    /// installer, and the library the app links.
    #[test]
    fn the_version_is_the_vpk_and_the_velopack_package_the_windows_build_pins() {
        if let Err(e) = pinned(&repo_root()) {
            panic!("{e}");
        }
        assert_eq!(TARGET_LIST, TARGETS.join(","));
    }

    #[test]
    fn another_vpk_is_refused_by_name() {
        let tools = |v: &str| {
            format!(
                "{{\n  \"version\": 1,\n  \"tools\": {{\n    \"vpk\": {{\n      \"version\": \"{v}\"\n    }}\n  }}\n}}\n"
            )
        };
        assert!(vpk_is_ours(&tools(VERSION)).is_ok());
        let e = vpk_is_ours(&tools("9.9.9")).unwrap_err();
        assert!(
            e.contains("9.9.9") && e.contains(VERSION) && e.contains(INPUT),
            "{e}"
        );
        assert!(vpk_is_ours("{\"tools\": {}}").is_err());
    }

    #[test]
    fn the_workspace_keeps_only_the_members_the_binaries_are_built_from() {
        let root = "[workspace]\nresolver = \"2\"\nmembers = [\n    \"src/bins\",\n    \"src/l18n\",\n    \"src/lib-rust\",\n    \"src/lib-python\",\n]\nexclude = [\"x\"]\n";
        assert_eq!(
            narrow_members(root).unwrap(),
            "[workspace]\nresolver = \"2\"\nmembers = [\"src/bins\", \"src/l18n\", \"src/lib-rust\"]\nexclude = [\"x\"]\n"
        );
        let gone = "[workspace]\nmembers = [\"src/bins\", \"src/lib-rust\"]\n";
        assert!(narrow_members(gone).unwrap_err().contains("src/l18n"));
        assert!(narrow_members("[workspace]\n").is_err());
    }

    #[test]
    fn dev_dependencies_are_dropped_and_nothing_else() {
        let manifest = "[package]\nname = \"a\"\n\n[dependencies]\nlog = \"0.4\"\n\n[dev-dependencies]\ntempfile = \"3\"\n\n[target.'cfg(windows)'.dev-dependencies]\nntest = \"0.9\"\n\n[build-dependencies]\nsemver = \"1\"\n\n[target.'cfg(windows)'.dependencies]\nwinreg = \"0.56\"\n";
        let out = without_dev_dependencies(manifest);
        assert!(!out.contains("tempfile") && !out.contains("ntest"), "{out}");
        for kept in ["log = ", "semver = ", "winreg = ", "[build-dependencies]"] {
            assert!(out.contains(kept), "{kept} is kept: {out}");
        }
    }

    #[test]
    fn a_members_sources_are_its_library_and_the_paths_it_names() {
        let manifest = "[lib]\nname = \"b\"\npath = \"src/lib.rs\"\n\n[[bin]]\nname = \"setup\"\npath = \"src/setup.rs\"\n\n[[bin]]\nname = \"update\"\npath = \"src/update.rs\"\n";
        assert_eq!(
            sources(manifest),
            ["src/lib.rs", "src/setup.rs", "src/update.rs"]
        );
        assert_eq!(sources("[package]\nname = \"c\"\n"), ["src/lib.rs"]);
    }

    /// The committed inputs make a workspace of the three members, with Velopack's own lock.
    #[test]
    fn the_committed_files_make_the_workspace() {
        let dir =
            std::env::temp_dir().join(format!("ink-notices-velopack-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let input = repo_root().join(INPUT);
        workspace(&input, &dir).unwrap();
        let root = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        assert!(
            root.contains("members = [\"src/bins\", \"src/l18n\", \"src/lib-rust\"]"),
            "{root}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("Cargo.lock")).unwrap(),
            read(&input, "Cargo.lock.upstream").unwrap()
        );
        let bins = std::fs::read_to_string(dir.join("src/bins/Cargo.toml")).unwrap();
        assert!(bins.contains("name = \"velopack_bins\"") && !bins.contains("dev-dependencies"));
        for source in [
            "src/bins/src/setup.rs",
            "src/bins/src/update.rs",
            "src/l18n/src/lib.rs",
        ] {
            assert!(dir.join(source).is_file(), "{source}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_crate_cargo_resolved_afresh_is_drift() {
        let upstream = "[[package]]\nname = \"a\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"b\"\nversion = \"2.0.0\"\n";
        assert!(drifted("[[package]]\nname = \"b\"\nversion = \"2.0.0\"\n", upstream).is_empty());
        assert_eq!(
            drifted("[[package]]\nname = \"a\"\nversion = \"1.0.1\"\n", upstream),
            ["a 1.0.1: not in Velopack's Cargo.lock at that version; cargo resolved it afresh"]
        );
    }

    #[test]
    fn a_licence_off_the_allowlist_ships_only_as_the_exception_for_its_crates() {
        let n = |name: &str, shown: &str| CrateNotice {
            name: name.into(),
            version: "1.0.0".into(),
            licence: shown.into(),
            shown: shown.into(),
            text: String::new(),
        };
        let fine = [
            n("anyhow", "MIT"),
            n("ring", "Apache-2.0 AND ISC"),
            n("zerovec", "Unicode-3.0"),
            n("icu_collections", "MIT AND Unicode-3.0"),
            n("webpki-roots", "CDLA-Permissive-2.0"),
        ];
        assert!(not_allowed(&fine).is_empty(), "{:?}", not_allowed(&fine));
        let refused = not_allowed(&[
            n("serde", "Unicode-3.0"),
            n("webpki-roots", "MPL-2.0"),
            n("x", "Apache-2.0 WITH LLVM-exception"),
        ]);
        assert_eq!(refused.len(), 3, "{refused:?}");
        assert!(refused[0].starts_with("serde 1.0.0: shown under Unicode-3.0"));
    }
}
