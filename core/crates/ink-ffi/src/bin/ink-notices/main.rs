//! Writes the licence notices of the third-party Rust crates the release links into the core
//! (`mac/Sources/Inkwell/Generated/RustNotices.swift`, shown in Settings > About).
//!
//! ```text
//! cargo run -p ink-ffi --bin ink-notices                  # regenerate
//! cargo run -p ink-ffi --bin ink-notices -- --check       # fail if the file differs from a fresh run
//! cargo run -p ink-ffi --bin ink-notices -- --check-lock  # fail if it was made from another lock
//! cargo run -p ink-ffi --bin ink-notices -- --windows [--check | --check-lock]
//! cargo run -p ink-ffi --bin ink-notices -- --velopack [--check | --check-lock]
//! ```
//!
//! `--windows` does the same for the Windows shell: the crates of the Windows target
//! ([`WINDOWS`]), as C# (`windows/Inkwell.Core/Screens/About/RustNotices.g.cs`, [`csharp`]).
//! `--velopack` does it for the crates in Velopack's Setup.exe and Update.exe, which the Windows
//! installer ships, from Velopack's own lock and manifests ([`velopack`], with its own rules for
//! the network and licences), as C# (`windows/Inkwell.Core/Screens/About/VelopackNotices.g.cs`).
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

mod csharp;
mod graph;
mod licence;
mod overrides;
mod swift;
mod velopack;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::graph::Package;
use crate::licence::LicenceFile;
use crate::overrides::{Override, Overrides};
use crate::swift::CrateNotice;

/// The features the release builds the core with: `release_features` in
/// `mac/scripts/build-mac.sh` (a test holds them equal).
const RELEASE_FEATURES: &str = "engine-llama,ink-engines/engine-silero,ink-engines/engine-nemo";
/// The release's one target: the app is Apple silicon only.
const TARGET: &str = "aarch64-apple-darwin";
/// The generated file, from the repository root.
const SWIFT_OUT: &str = "mac/Sources/Inkwell/Generated/RustNotices.swift";
/// The Windows release's target, whose crates the Windows file lists.
const WINDOWS_TARGET: &str = "x86_64-pc-windows-msvc";
/// Windows on ARM64 ships the same file, so every crate it links must be among
/// [`WINDOWS_TARGET`]'s ([`not_covered`]). It links a subset of them: the same crates less two
/// x86-only CPU-feature detection crates.
const WINDOWS_ARM64_TARGET: &str = "aarch64-pc-windows-msvc";
/// The features the Windows release builds the core with. It has no build script yet to hold
/// them to, as the Mac's has (a test does); its engines are the Mac's (S3.2: llama.cpp, whose
/// Vulkan backend, `ink-engines/engine-llama-vulkan`, adds no crate, Silero and NeMo-Speech.cpp).
const WINDOWS_RELEASE_FEATURES: &str = RELEASE_FEATURES;
/// The generated C# file, from the repository root.
const CSHARP_OUT: &str = "windows/Inkwell.Core/Screens/About/RustNotices.g.cs";
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

/// A shell the notices are generated for: the release's target and features, and its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shell {
    target: &'static str,
    features: &'static str,
    out: &'static str,
    /// The file's language: C# (the Windows shell) or Swift (the Mac's).
    csharp: bool,
    /// Velopack's binaries' crates ([`velopack`]), not the core's.
    velopack: bool,
}

/// The Mac app: Swift, for Apple silicon.
const MAC: Shell = Shell {
    target: TARGET,
    features: RELEASE_FEATURES,
    out: SWIFT_OUT,
    csharp: false,
    velopack: false,
};

/// The Windows app: C#, for x64 and ARM64 (from x64's crates).
const WINDOWS: Shell = Shell {
    target: WINDOWS_TARGET,
    features: WINDOWS_RELEASE_FEATURES,
    out: CSHARP_OUT,
    csharp: true,
    velopack: false,
};

/// The crates in Velopack's Setup.exe and Update.exe: C#, for both of Velopack's Windows targets.
const VELOPACK: Shell = Shell {
    target: velopack::TARGET_LIST,
    features: velopack::FEATURES,
    out: velopack::OUT,
    csharp: true,
    velopack: true,
};

impl Shell {
    fn render(self, fingerprint: &str, notices: &[CrateNotice]) -> String {
        if self.velopack {
            csharp::render_velopack(
                velopack::VERSION,
                self.features,
                self.target,
                fingerprint,
                notices,
            )
        } else if self.csharp {
            csharp::render(self.features, self.target, fingerprint, notices)
        } else {
            swift::render(self.features, self.target, fingerprint, notices)
        }
    }

    /// A value the file records, by its Swift name (`lockFingerprint`; C# capitalises it).
    fn recorded(self, file: &str, name: &str) -> Option<String> {
        if self.csharp {
            let mut chars = name.chars();
            let upper: String = chars
                .next()
                .map(|c| c.to_ascii_uppercase())
                .into_iter()
                .chain(chars)
                .collect();
            csharp::recorded(file, &upper)
        } else {
            swift::recorded(file, name)
        }
    }

    fn listed(self, file: &str) -> Vec<String> {
        if self.csharp {
            csharp::listed(file)
        } else {
            listed(file)
        }
    }

    /// The command that regenerates the file.
    fn regenerate(self) -> &'static str {
        if self.velopack {
            "cargo run -p ink-ffi --bin ink-notices -- --velopack"
        } else if self.csharp {
            "cargo run -p ink-ffi --bin ink-notices -- --windows"
        } else {
            "cargo run -p ink-ffi --bin ink-notices"
        }
    }
}

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
        return Err(format!(
            "cargo {} failed:\n{}{}",
            args[0],
            stderr.trim_end(),
            fetch_hint(args, &stderr)
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("cargo {}: {e}", args[0]))
}

/// What to do when cargo (run with `args`) could not find or fetch a crate's package: the core's
/// runs are offline (`--offline`), and rust-notices.sh fetches their crates first; Velopack's are
/// not, and fetch Velopack's crates themselves.
fn fetch_hint(args: &[&str], stderr: &str) -> &'static str {
    if !(stderr.contains("offline") || stderr.contains("download")) {
        ""
    } else if args.contains(&"--offline") {
        "\n(a crate's package is not in the local registry: run mac/scripts/rust-notices.sh, which fetches them)"
    } else {
        "\n(cargo could not fetch a crate: the Velopack run fetches Velopack's crates from crates.io, so it needs the network, with CARGO_NET_OFFLINE unset)"
    }
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
        if !LICENCE_FILE_PREFIXES.iter().any(|p| lower.starts_with(p)) {
            continue;
        }
        // The entry itself, not what a link points at: a published package holds no links
        // (cargo package stores their targets), so one here is not followed out of the package.
        let kind = entry.file_type().map_err(|e| format!("{name}: {e}"))?;
        if kind.is_symlink() {
            return Err(format!(
                "{name} is a symbolic link, which a published package never holds: not followed; its notice needs a decision"
            ));
        }
        if !kind.is_file() {
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
    for c in crates {
        let over = overrides.get(&(c.name.clone(), c.version.clone()));
        match notice(c, over, texts) {
            Ok(n) => out.push(n),
            Err(e) => errors.push(format!("{} {}: {e}", c.name, c.version)),
        }
    }
    for (name, version) in overrides.keys() {
        if !crates
            .iter()
            .any(|c| (&c.name, &c.version) == (name, version))
        {
            errors.push(format!(
                "{name} {version}: overridden, but not in the release"
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

/// One crate's notice: its own licence files, or with its override (`over`) where they carry no
/// text of any licence it offers. An override the package no longer needs is an error.
fn notice(c: &Package, over: Option<&Override>, texts: &Path) -> Result<CrateNotice, String> {
    let licence = c
        .licence
        .as_ref()
        .ok_or("declares no SPDX licence (only a licence file); its notice needs a decision")?;
    let expr = licence::parse(licence)?;
    let mut files = licence_files(&c.dir)?;
    let selection = match (licence::select(&expr, &files, &c.authors), over) {
        (Ok(_), Some(o)) => {
            return Err(format!(
                "its package now carries its licence, so the override ({}) must go",
                o.text
            ));
        }
        (Ok(s), None) => s,
        (Err(e), None) => return Err(e),
        (Err(_), Some(o)) => {
            let text = std::fs::read_to_string(texts.join(&o.text))
                .map_err(|e| format!("notices/texts/{}: {e}", o.text))?;
            let text = swift::clean(&text).map_err(|e| format!("notices/texts/{} {e}", o.text))?;
            files.push(LicenceFile {
                name: o.text.trim_end_matches(".txt").to_string(),
                text,
                supplied: Some(o.reason.clone()),
            });
            sort_files(&mut files);
            licence::select(&expr, &files, &c.authors)
                .map_err(|e| format!("with its override, {e}"))?
        }
    };
    Ok(CrateNotice {
        name: c.name.clone(),
        version: c.version.clone(),
        licence: licence.clone(),
        shown: selection.shown,
        text: swift::compose(&selection.files),
    })
}

/// The paths of this machine the output must not name: each package's registry directory, the
/// checkout, and the home directory (`HOME`, and `USERPROFILE` on Windows). `var` reads the
/// environment.
fn machine_paths(
    crates: &[Package],
    root: &Path,
    var: impl Fn(&str) -> Option<String>,
) -> Vec<String> {
    let mut paths: Vec<String> = crates
        .iter()
        .filter_map(|c| c.dir.parent().map(|p| p.display().to_string()))
        .collect();
    paths.push(
        root.canonicalize()
            .unwrap_or_else(|_| root.to_path_buf())
            .display()
            .to_string(),
    );
    // A home of "/" or "C:\\" would match every path in every text.
    paths.extend(
        ["HOME", "USERPROFILE"]
            .into_iter()
            .filter_map(&var)
            .filter(|h| h.trim_end_matches(['/', '\\']).len() > 3),
    );
    paths
}

/// The crates a lock records, as (name, version).
fn locked(lock: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut name = None;
    for line in lock.lines() {
        if let Some(n) = line
            .strip_prefix("name = \"")
            .and_then(|l| l.strip_suffix('"'))
        {
            name = Some(n.to_string());
        } else if let Some(v) = line
            .strip_prefix("version = \"")
            .and_then(|l| l.strip_suffix('"'))
            && let Some(n) = name.take()
        {
            out.push((n, v.to_string()));
        }
    }
    out
}

/// The overrides the Windows run checks: all but those for a crate the lock has that the Windows
/// release does not link (another target's, which the Mac run checks, and refuses where no
/// release needs it). One for a crate the lock no longer has stays, and is an error.
fn windows_overrides(overrides: Overrides, crates: &[Package], lock: &str) -> Overrides {
    let locked = locked(lock);
    overrides
        .into_iter()
        .filter(|(key, _)| {
            crates
                .iter()
                .any(|c| (&c.name, &c.version) == (&key.0, &key.1))
                || !locked.contains(key)
        })
        .collect()
}

/// ink-ffi's normal dependencies for `target` with `features`, as (name, version).
fn release_tree(
    core: &Path,
    target: &str,
    features: &str,
) -> Result<Vec<(String, String)>, String> {
    crate_tree(core, "ink-ffi", target, features, true)
}

/// `package`'s normal dependencies in the workspace at `dir` for `target` with `features`, as
/// (name, version). `locked`: offline, from the lock as it is (the core's); otherwise cargo may
/// fetch and narrow the lock to the workspace (Velopack's, whose drift [`velopack`] checks).
fn crate_tree(
    dir: &Path,
    package: &str,
    target: &str,
    features: &str,
    locked: bool,
) -> Result<Vec<(String, String)>, String> {
    let mut args = vec!["tree"];
    if locked {
        args.extend(["--offline", "--locked"]);
    }
    args.extend([
        "-p",
        package,
        "-e",
        "normal,no-proc-macro",
        "--target",
        target,
        "--features",
        features,
        "--prefix",
        "none",
        "--format",
        "{p}",
    ]);
    run_cargo(dir, &args).and_then(|t| graph::tree(&t))
}

/// The crates of `other` (another target's tree) that `listed` (the tree a file is made from)
/// lacks: that target's app would link them with no notice.
fn not_covered(listed: &[(String, String)], other: &[(String, String)]) -> Vec<String> {
    other
        .iter()
        .filter(|c| !listed.contains(c))
        .map(|(name, version)| format!("{name} {version}"))
        .collect()
}

/// The shell's file, from cargo's resolution and the packages on this machine.
fn generate(root: &Path, shell: Shell) -> Result<String, Vec<String>> {
    if shell.velopack {
        return velopack::generate(root);
    }
    let core = root.join("core");
    let tree = release_tree(&core, shell.target, shell.features).map_err(|e| vec![e])?;
    if shell.csharp {
        let arm64 =
            release_tree(&core, WINDOWS_ARM64_TARGET, shell.features).map_err(|e| vec![e])?;
        let missing = not_covered(&tree, &arm64);
        if !missing.is_empty() {
            return Err(missing
                .into_iter()
                .map(|c| {
                    format!(
                        "{c}: linked on {WINDOWS_ARM64_TARGET} but not on {}, whose crates the \
                         Windows file lists; its notice needs a decision",
                        shell.target
                    )
                })
                .collect());
        }
    }
    let packages = run_cargo(
        &core,
        &[
            "metadata",
            "--offline",
            "--locked",
            "--format-version",
            "1",
            "--filter-platform",
            shell.target,
            "--features",
            shell.features,
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
    let lock = std::fs::read_to_string(core.join("Cargo.lock"))
        .map_err(|e| vec![format!("core/Cargo.lock: {e}")])?;
    let overrides = if shell.csharp {
        windows_overrides(overrides, &crates, &lock)
    } else {
        overrides
    };
    let notices = notices(&crates, &overrides, &notices_dir.join("texts"))?;
    let rendered = shell.render(
        &swift::fingerprint(shell.features, shell.target, &lock),
        &notices,
    );
    // Nothing of this machine: no path of the checkout, the registry or the home directory.
    let local = machine_paths(&crates, root, |k| std::env::var(k).ok());
    if let Some(found) = local.iter().find(|p| rendered.contains(p.as_str())) {
        return Err(vec![format!(
            "the output names a path of this machine ({found})"
        )]);
    }
    Ok(rendered)
}

/// The crates a generated file lists, as `name version`: the header line of each
/// `RustCrateNotice(`, read outside the licence texts (a raw literal from its opening line,
/// `text: #"""`, to its closing one, `"""#),`), so no text can add or hide a crate.
fn listed(swift: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut closing: Option<String> = None;
    let mut after_open = false;
    for line in swift.lines() {
        if let Some(end) = &closing {
            if line == end {
                closing = None;
            }
            continue;
        }
        if let Some(hashes) = line
            .strip_prefix("            text: ")
            .and_then(|l| l.strip_suffix("\"\"\""))
        {
            closing = Some(format!("\"\"\"{hashes}),"));
            continue;
        }
        if after_open
            && let Some(rest) = line.strip_prefix("            name: \"")
            && let Some((name, rest)) = rest.split_once('"')
            && let Some(version) = rest
                .strip_prefix(", version: \"")
                .and_then(|r| r.split('"').next())
        {
            out.push(format!("{name} {version}"));
        }
        after_open = line == "        RustCrateNotice(";
    }
    out
}

/// Compares the file with a fresh run.
fn check(root: &Path, shell: Shell, generated: &str) -> Result<(), String> {
    let current = std::fs::read_to_string(root.join(shell.out))
        .unwrap_or_default()
        .replace("\r\n", "\n");
    if current == generated {
        return Ok(());
    }
    let (was, now) = (shell.listed(&current), shell.listed(generated));
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
        "{} is stale:{why}\nRegenerate: {}",
        shell.out,
        shell.regenerate()
    ))
}

/// Compares the fingerprint the file records with the lock as it is now. Needs no package.
fn check_lock(root: &Path, shell: Shell) -> Result<(), String> {
    let out = shell.out;
    let file = std::fs::read_to_string(root.join(out)).map_err(|e| format!("{out}: {e}"))?;
    let lock_path = if shell.velopack {
        format!("{}/Cargo.lock.upstream", velopack::INPUT)
    } else {
        "core/Cargo.lock".to_string()
    };
    let lock =
        std::fs::read_to_string(root.join(&lock_path)).map_err(|e| format!("{lock_path}: {e}"))?;
    let recorded = shell
        .recorded(&file, "lockFingerprint")
        .ok_or(format!("{out} records no lockFingerprint"))?;
    let features = shell.recorded(&file, "features").unwrap_or_default();
    let target = shell.recorded(&file, "target").unwrap_or_default();
    let regenerate = shell.regenerate();
    if (features.as_str(), target.as_str()) != (shell.features, shell.target) {
        return Err(format!(
            "{out} was made for {target} with {features}; the release builds {} with {}.\n\
             Regenerate: {regenerate}",
            shell.target, shell.features
        ));
    }
    if shell.velopack {
        let version = shell.recorded(&file, "version").unwrap_or_default();
        if version != velopack::VERSION {
            return Err(format!(
                "{out} was made for Velopack {version}, not {}.\nRegenerate: {regenerate}",
                velopack::VERSION
            ));
        }
    }
    if recorded != swift::fingerprint(shell.features, shell.target, &lock) {
        let fetch = if shell.csharp {
            ""
        } else {
            " (or mac/scripts/rust-notices.sh, which fetches the packages first)"
        };
        return Err(format!(
            "{out} was made from another {lock_path}: a dependency changed and its notice may be missing.\n\
             Regenerate: {regenerate}{fetch}"
        ));
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = repo_root();
    let (shell, args) = match args.split_first() {
        Some((first, rest)) if first == "--windows" => (WINDOWS, rest.to_vec()),
        Some((first, rest)) if first == "--velopack" => (VELOPACK, rest.to_vec()),
        _ => (MAC, args),
    };
    let out = shell.out;
    let result = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--check-lock"] => check_lock(&root, shell),
        [] | ["--check"] => match generate(&root, shell) {
            Err(errors) => {
                eprintln!("ink-notices: {} problem(s), nothing written:", errors.len());
                for e in &errors {
                    eprintln!("  {e}");
                }
                if errors.iter().any(|e| e.contains("carries")) {
                    let table = if shell.velopack {
                        velopack::INPUT
                    } else {
                        NOTICES_DIR
                    };
                    eprintln!(
                        "A crate without its licence text needs {table}/overrides.txt: add a line only after reading its upstream licence."
                    );
                }
                return ExitCode::FAILURE;
            }
            Ok(generated) if args.is_empty() => {
                let path = root.join(out);
                let written = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(&path, &generated));
                written
                    .map(|()| println!("wrote {out} ({} crates)", shell.listed(&generated).len()))
                    .map_err(|e| format!("{out}: {e}"))
            }
            Ok(generated) => check(&root, shell, &generated),
        },
        _ => {
            Err("usage: ink-notices [--windows | --velopack] [--check | --check-lock]".to_string())
        }
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

    /// The Windows file is made from x64's crates and ships on ARM64 too: a crate only ARM64
    /// links is named, one ARM64 does not link is fine.
    #[test]
    fn a_crate_only_the_other_target_links_is_not_covered() {
        let key = |n: &str, v: &str| (n.to_string(), v.to_string());
        let x64 = [key("shared", "1.0.0"), key("x86-only", "0.3.1")];
        let arm64 = [key("shared", "1.0.0"), key("arm-only", "2.0.0")];
        assert_eq!(not_covered(&x64, &arm64), ["arm-only 2.0.0"]);
        assert!(not_covered(&x64, &x64[..1]).is_empty());
        // The same name at another version is another crate, with its own notice.
        assert_eq!(
            not_covered(&x64, &[key("shared", "1.0.1")]),
            ["shared 1.0.1"]
        );
    }

    fn committed(shell: Shell) -> String {
        std::fs::read_to_string(repo_root().join(shell.out))
            .expect("the generated notices are checked in")
            .replace("\r\n", "\n")
    }

    /// The staleness check every CI run makes, with no package needed: the checked-in notices were
    /// generated from this Cargo.lock. `ink-notices --check` (mac/scripts/rust-notices.sh) compares
    /// the whole file.
    #[test]
    fn the_checked_in_notices_were_generated_from_the_current_lock() {
        if let Err(e) = check_lock(&repo_root(), MAC) {
            panic!("{e}");
        }
    }

    /// The same for the Windows shell's (`ink-notices --windows --check` compares the whole file).
    #[test]
    fn the_checked_in_windows_notices_were_generated_from_the_current_lock() {
        if let Err(e) = check_lock(&repo_root(), WINDOWS) {
            panic!("{e}");
        }
    }

    /// The same for Velopack's binaries' (`ink-notices --velopack --check` compares the whole file):
    /// made from Velopack's lock in core/crates/ink-ffi/notices/velopack/, for the Velopack the
    /// Windows build pins.
    #[test]
    fn the_checked_in_velopack_notices_were_generated_from_its_lock_for_the_pinned_release() {
        if let Err(e) = check_lock(&repo_root(), VELOPACK) {
            panic!("{e}");
        }
    }

    #[test]
    fn the_windows_notices_are_the_windows_targets_crates() {
        let cs = committed(WINDOWS);
        let crates = WINDOWS.listed(&cs);
        let names: Vec<&str> = crates.iter().filter_map(|c| c.split(' ').next()).collect();
        for mac_only in ["objc2", "security-framework", "core-foundation", "block2"] {
            assert!(!names.contains(&mac_only), "{mac_only} is the Mac's");
        }
        assert!(names.contains(&"windows-sys"), "{names:?}");
        assert_eq!(
            WINDOWS.recorded(&cs, "target").as_deref(),
            Some("x86_64-pc-windows-msvc")
        );
    }

    #[test]
    fn the_windows_run_leaves_another_targets_overrides_to_the_mac_run() {
        let lock = "[[package]]\nname = \"objc2\"\nversion = \"0.6.4\"\n\n[[package]]\nname = \"zeta\"\nversion = \"1.0.0\"\n";
        assert_eq!(
            locked(lock),
            [
                ("objc2".to_string(), "0.6.4".to_string()),
                ("zeta".to_string(), "1.0.0".to_string())
            ]
        );
        let registry = scratch();
        let crates = [package(&registry, "zeta", "MIT", &[])];
        let o = || overrides::Override {
            text: "MIT.txt".into(),
            verified: None,
            reason: "why".into(),
        };
        let mut table = Overrides::new();
        table.insert(("objc2".into(), "0.6.4".into()), o());
        table.insert(("zeta".into(), "1.0.0".into()), o());
        table.insert(("gone".into(), "2.0.0".into()), o());
        let kept: Vec<String> = windows_overrides(table, &crates, lock)
            .into_keys()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(kept, ["gone", "zeta"], "a crate no lock has stays, to fail");
        let _ = std::fs::remove_dir_all(registry);
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
        for shell in [MAC, WINDOWS, VELOPACK] {
            let file = committed(shell);
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
                assert!(
                    !file.contains(local),
                    "{}: the notices hold `{local}`",
                    shell.out
                );
            }
            assert!(
                shell.listed(&file).len() > 100,
                "{}: the release links over a hundred crates",
                shell.out
            );
            assert!(
                shell.listed(&file).iter().all(|c| !c.starts_with("ink-")),
                "{}: no workspace crate",
                shell.out
            );
        }
    }

    #[test]
    fn a_crate_cargo_could_not_fetch_gets_the_hint_of_its_run() {
        // The core's runs are offline; Velopack's fetch (crate_tree unlocked, its metadata).
        let core = ["tree", "--offline", "--locked", "-p", "ink-ffi"];
        let velopack = ["metadata", "--format-version", "1"];
        let missing = "error: failed to download `foo v1.0.0`";
        let forbidden = "error: attempting to make an HTTP request, but --offline was specified";
        for stderr in [missing, forbidden] {
            assert!(
                fetch_hint(&core, stderr).contains("rust-notices.sh"),
                "{stderr}"
            );
            let hint = fetch_hint(&velopack, stderr);
            assert!(hint.contains("needs the network"), "{stderr}");
            assert!(!hint.contains("rust-notices.sh"), "{stderr}");
        }
        for args in [&core[..], &velopack[..]] {
            assert_eq!(fetch_hint(args, "error: package `foo` not found"), "");
        }
    }

    #[test]
    fn every_override_line_names_a_text_that_is_a_licence() {
        // The core's table and Velopack's (whose run needs the network), both over texts/.
        let dir = repo_root().join(NOTICES_DIR);
        for table in [
            dir.join("overrides.txt"),
            repo_root().join(velopack::INPUT).join("overrides.txt"),
        ] {
            let at = table.display();
            let table = overrides::parse(&std::fs::read_to_string(&table).unwrap())
                .unwrap_or_else(|e| panic!("{at}: {e}"));
            assert!(!table.is_empty(), "{at}");
            for ((name, version), o) in &table {
                let text = std::fs::read_to_string(dir.join("texts").join(&o.text))
                    .unwrap_or_else(|e| panic!("{at}: {name} {version}: texts/{}: {e}", o.text));
                assert!(
                    !licence::classify(&text).is_empty(),
                    "{at}: {name} {version}: texts/{} is no licence text",
                    o.text
                );
            }
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
                verified: None,
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
            verified: None,
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
    fn listed_reads_only_the_crate_headers_never_a_licence_text() {
        let tricky = "        RustCrateNotice(\n            name: \"fake\", version: \"9.9.9\", licence: \"MIT\", shown: \"MIT\",";
        let swift = swift::render(
            "f",
            "t",
            "0123456789abcdef",
            &[CrateNotice {
                name: "real".into(),
                version: "1.0.0".into(),
                licence: "MIT".into(),
                shown: "MIT".into(),
                text: format!("a text quoting a header:\n{tricky}"),
            }],
        );
        assert!(swift.contains("name: \"fake\""), "the trap is in the file");
        assert_eq!(listed(&swift), ["real 1.0.0"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_in_a_package_is_an_error_naming_it_not_followed() {
        let registry = scratch();
        let outside = registry.join("outside-LICENSE");
        std::fs::write(&outside, MIT).unwrap();
        let mut c = package(&registry, "linked", "MIT", &[]);
        std::os::unix::fs::symlink(&outside, c.dir.join("LICENSE")).unwrap();
        let errors = notices(std::slice::from_ref(&c), &Overrides::new(), &registry).unwrap_err();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].starts_with("linked 1.0.0: LICENSE is a symbolic link"),
            "{errors:?}"
        );
        // A directory named like a licence file is not read, and not an error.
        std::fs::remove_file(c.dir.join("LICENSE")).unwrap();
        std::fs::create_dir(c.dir.join("LICENSES")).unwrap();
        std::fs::write(c.dir.join("LICENSE-MIT"), MIT).unwrap();
        c.licence = Some("MIT".into());
        assert!(notices(&[c], &Overrides::new(), &registry).is_ok());
        let _ = std::fs::remove_dir_all(registry);
    }

    #[test]
    fn the_paths_of_this_machine_include_both_home_directories() {
        let env = |k: &str| match k {
            "HOME" => Some("/home/someone".to_string()),
            "USERPROFILE" => Some("C:\\Users\\someone".to_string()),
            _ => None,
        };
        let paths = machine_paths(&[], Path::new("/checkout"), env);
        assert!(paths.contains(&"/home/someone".to_string()), "{paths:?}");
        assert!(
            paths.contains(&"C:\\Users\\someone".to_string()),
            "{paths:?}"
        );
        assert!(
            machine_paths(&[], Path::new("/checkout"), |_| Some("/".into()))
                .iter()
                .all(|p| p != "/"),
            "a root home would match everything"
        );
    }

    #[test]
    fn versions_sort_by_number() {
        let mut v = vec!["1.10.0", "1.9.0", "0.2.0-alpha"];
        v.sort_by_key(|s| version_key(s));
        assert_eq!(v, ["0.2.0-alpha", "1.9.0", "1.10.0"]);
    }
}
