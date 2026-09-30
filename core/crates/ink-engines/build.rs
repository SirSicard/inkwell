//! Links NeMo-Speech.cpp's diarization library for `engine-nemo`, and sherpa-onnx's for
//! `engine-sherpa` (`sherpa()` at the end). Without those features it does nothing, but for one
//! line of metadata: whether `engine-silero` is on (for ink-ffi's build
//! script, which loads Silero for dictation when it is).
//!
//! The library is built outside cargo (`native/build-nemo-speech.sh`) and found through
//! `NEMO_SPEECH_DIR`, its install prefix. Before linking, this checks:
//!
//! - the installed C headers against the pinned commit's, by SHA-256: `src/nemo.rs` declares the
//!   C ABI by hand, so a library from any other commit is refused at build time rather than
//!   misread at run time;
//! - the manifest the build script wrote: the pinned commit, and the SHA-256 of every library it
//!   installed, which each file must still match. The library linked must be one of them.
//! - that the manifest records `ggml_native OFF`: ggml built for any Apple silicon Mac, not tuned
//!   to the CPU of the Mac that built it (whose newer instructions would stop an older Mac). A
//!   manifest from before the script recorded it is refused too: that prefix was built native, and
//!   links SentencePiece and Abseil from where Homebrew installed them.
//!
//! On Windows there is no rpath: the manifest's DLLs are copied into this build's `OUT_DIR` and that
//! directory is declared as a native search path, which cargo puts on `PATH` when it runs this
//! package's tests and its dependents' (cargo adds only search paths inside the target directory).
//!
//! `INK_NEMO_CHECK_ONLY=1` is for type-checking (CI's clippy) where the library is not built: it
//! skips the library, the manifest and the link, so a binary or test built that way does not link.
//! It still compiles and lints every line of the adapter, and if `NEMO_SPEECH_DIR` is set as well,
//! the headers are still checked. With the feature on and neither variable set, the build fails.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The NeMo-Speech.cpp commit `src/nemo.rs` is written against.
const NEMO_COMMIT: &str = "97a15afa5caa9bce5baaa86c1184103877af4101";

/// SHA-256 of the headers `src/nemo.rs` is written against, at [`NEMO_COMMIT`].
const PINNED_HEADERS: [(&str, &str); 2] = [
    (
        "include/nemo_speech/diar.h",
        "4839f70ccab30af3ed9949751792a547f3a8a4129848f64c8cd69889bd433afd",
    ),
    (
        "include/nemo_speech/asr.h",
        "12f3e017391ae19daabe59d2d50541f0aabb010228d075426b4a9834337422fa",
    ),
];

/// Where `native/build-nemo-speech.sh` writes its manifest, below the prefix.
const MANIFEST: &str = "share/inkwell/nemo-speech.manifest";

/// The library linked, by the names each OS gives it (the first that exists is used).
const LIBRARY: [&str; 3] = [
    "lib/libnemo_speech_asr_c.dylib",
    "lib/libnemo_speech_asr_c.so",
    "lib/nemo_speech_asr_c.lib",
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Tells dependents' build scripts (through `links`) that Silero is in this build, so ink-ffi
    // loads it for dictation without a feature of its own (its `src/vad.rs`).
    if env::var_os("CARGO_FEATURE_ENGINE_SILERO").is_some() {
        println!("cargo:silero=1");
    }
    if env::var_os("CARGO_FEATURE_ENGINE_SHERPA").is_some() {
        sherpa();
    }
    if env::var_os("CARGO_FEATURE_ENGINE_NEMO").is_none() {
        return;
    }
    println!("cargo:rerun-if-env-changed=NEMO_SPEECH_DIR");
    println!("cargo:rerun-if-env-changed=INK_NEMO_CHECK_ONLY");
    let check_only = match env::var("INK_NEMO_CHECK_ONLY") {
        Err(env::VarError::NotPresent) => false,
        Ok(value) if value == "1" => true,
        _ => fail("INK_NEMO_CHECK_ONLY must be 1 or unset"),
    };
    let dir = env::var_os("NEMO_SPEECH_DIR").map(PathBuf::from);
    match (dir, check_only) {
        (None, false) => fail(
            "engine-nemo needs NEMO_SPEECH_DIR: the install prefix of NeMo-Speech.cpp 97a15af \
             (build it with crates/ink-engines/native/build-nemo-speech.sh). To type-check without \
             it, set INK_NEMO_CHECK_ONLY=1.",
        ),
        (None, true) => warn_check_only(),
        (Some(dir), true) => {
            check_headers(&dir);
            warn_check_only();
        }
        (Some(dir), false) => {
            check_headers(&dir);
            let (library, listed) = check_manifest(&dir);
            link(&dir, &library, &listed);
        }
    }
}

fn warn_check_only() {
    println!(
        "cargo:warning=engine-nemo in check-only mode (INK_NEMO_CHECK_ONLY=1): NeMo-Speech.cpp is \
         not linked, so binaries and tests will not link"
    );
}

fn check_headers(dir: &Path) {
    for (header, pinned) in PINNED_HEADERS {
        let path = dir.join(header);
        println!("cargo:rerun-if-changed={}", path.display());
        let found = sha256_of(&path, &format!("NEMO_SPEECH_DIR has no {header}"));
        if found != pinned {
            fail(&format!(
                "{} is not the pinned NeMo-Speech.cpp 97a15af header (sha256 {found}); \
                 src/nemo.rs declares that commit's C ABI",
                path.display()
            ));
        }
    }
}

/// Checks the manifest and every library it lists, and returns the library to link and every
/// library listed.
fn check_manifest(dir: &Path) -> (PathBuf, Vec<PathBuf>) {
    let path = dir.join(MANIFEST);
    println!("cargo:rerun-if-changed={}", path.display());
    let text = fs::read_to_string(&path).unwrap_or_else(|e| {
        fail(&format!(
            "NEMO_SPEECH_DIR has no {MANIFEST} ({}: {e}); install with \
             crates/ink-engines/native/build-nemo-speech.sh",
            path.display()
        ))
    });
    let mut commit = None;
    let mut ggml_native = None;
    let mut listed = Vec::new();
    for line in text.lines().map(str::trim) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.as_slice() {
            [] => {}
            [first, ..] if first.starts_with('#') => {}
            ["commit", hash] => commit = Some(hash.to_string()),
            ["ggml_native", value] => ggml_native = Some(value.to_string()),
            ["sha256", hash, file] => {
                let file_path = dir.join(file);
                println!("cargo:rerun-if-changed={}", file_path.display());
                let found = sha256_of(&file_path, &format!("{MANIFEST} lists {file}"));
                if found != *hash {
                    fail(&format!(
                        "{} changed since it was installed (sha256 {found}, manifest {hash})",
                        file_path.display()
                    ));
                }
                listed.push(canonical(&file_path));
            }
            _ => fail(&format!("{}: unreadable line {line:?}", path.display())),
        }
    }
    if commit.as_deref() != Some(NEMO_COMMIT) {
        fail(&format!(
            "{} records commit {commit:?}, not the pinned {NEMO_COMMIT}",
            path.display()
        ));
    }
    if ggml_native.as_deref() != Some("OFF") {
        fail(&format!(
            "{} records GGML_NATIVE {ggml_native:?}, not OFF: rebuild the prefix with \
             crates/ink-engines/native/build-nemo-speech.sh, which builds ggml for every Apple \
             silicon Mac and bundles NeMo's dependencies",
            path.display()
        ));
    }
    let library = LIBRARY
        .iter()
        .map(|name| dir.join(name))
        .find(|p| p.exists())
        .unwrap_or_else(|| {
            fail(&format!(
                "no nemo_speech_asr_c library in {}",
                dir.join("lib").display()
            ))
        });
    if !listed.contains(&canonical(&library)) {
        fail(&format!(
            "{} is not among the libraries {MANIFEST} pins",
            library.display()
        ));
    }
    (library, listed)
}

fn link(dir: &Path, library: &Path, listed: &[PathBuf]) {
    let lib = dir.join("lib");
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=dylib=nemo_speech_asr_c");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // The import library links; the DLLs are found at run time, beside the binary or on
        // PATH. Copies of the checked DLLs in OUT_DIR, which cargo puts on PATH for tests.
        let dlls: Vec<&Path> = listed
            .iter()
            .map(PathBuf::as_path)
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")))
            .collect();
        if dlls.is_empty() {
            fail(&format!("{MANIFEST} lists no DLL"));
        }
        let out = copy_dlls(&dlls, "nemo-bin");
        println!("cargo:lib_dir={}", out.display());
        return;
    }
    println!("cargo:lib_dir={}", lib.display());
    // This package's own tests and examples find the library where it was installed. The Mac app
    // bundles the prefix's libraries in Contents/Frameworks and gives itself the rpath there
    // (mac/scripts/build-mac.sh); a static library of the core carries no rpath of its own.
    match env::var("CARGO_CFG_TARGET_OS") {
        Ok(os) if os == "macos" || os == "linux" => {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
        }
        Ok(_) => {}
        Err(_) => println!(
            "cargo:warning=CARGO_CFG_TARGET_OS is not set; no rpath added for {}, so this \
             package's tests may not find it",
            library.display()
        ),
    }
}

/// Copies checked DLLs into `OUT_DIR/<subdir>` and declares it a native search path, which cargo
/// puts on `PATH` when it runs this package's tests and its dependents' (it adds only search paths
/// inside the target directory). Returns the directory.
fn copy_dlls(dlls: &[&Path], subdir: &str) -> PathBuf {
    let out =
        PathBuf::from(env::var_os("OUT_DIR").unwrap_or_else(|| fail("no OUT_DIR"))).join(subdir);
    fs::create_dir_all(&out).unwrap_or_else(|e| fail(&format!("creating {}: {e}", out.display())));
    for dll in dlls {
        let name = dll
            .file_name()
            .unwrap_or_else(|| fail("a DLL path with no name"));
        fs::copy(dll, out.join(name))
            .unwrap_or_else(|e| fail(&format!("copying {}: {e}", dll.display())));
    }
    println!("cargo:rustc-link-search=native={}", out.display());
    out
}

/// sherpa-onnx 1.13.4's `win-x64-shared-MD-Release-no-tts-lib` archive: the files `engine-sherpa`
/// uses, below the unpacked directory, with their SHA-256s (the archive's own is `dec41ab39449…`,
/// as GitHub publishes it). `src/sherpa.rs` declares that release's C API.
const SHERPA_FILES: [(&str, &str); 4] = [
    (
        "lib/sherpa-onnx-c-api.lib",
        "806798a9fa6da0027f50ee6d8c0fe94f62f4a3f0947c3f1a96fe42acbee97d84",
    ),
    (
        "lib/sherpa-onnx-c-api.dll",
        "5319701a29c7b1b82aad8ad8ade890a2590440659cb874d8da400dff32f587c2",
    ),
    (
        "lib/onnxruntime.dll",
        "f4dcddcbe283c19a5046340460c23f64869132a299904d3b62ec5c353f5b56ed",
    ),
    (
        "lib/onnxruntime_providers_shared.dll",
        "92cee8b204ef376f3f5f52be94d3dc620e099aa3aeb8c5e2d17b455d6b912ef4",
    ),
];

/// `engine-sherpa`: checks the unpacked sherpa-onnx libraries named by `SHERPA_ONNX_DIR` against
/// [`SHERPA_FILES`], links the C API's import library, and puts the DLLs where tests find them.
/// Windows only: the Mac runs Parakeet in FluidAudio. `INK_SHERPA_CHECK_ONLY=1` type-checks the
/// adapter without the libraries (binaries and tests then do not link), on any OS.
fn sherpa() {
    println!("cargo:rerun-if-env-changed=SHERPA_ONNX_DIR");
    println!("cargo:rerun-if-env-changed=INK_SHERPA_CHECK_ONLY");
    match env::var("INK_SHERPA_CHECK_ONLY") {
        Err(env::VarError::NotPresent) => {}
        Ok(value) if value == "1" => {
            println!(
                "cargo:warning=engine-sherpa in check-only mode (INK_SHERPA_CHECK_ONLY=1): \
                 sherpa-onnx is not linked, so binaries and tests will not link"
            );
            return;
        }
        _ => fail("INK_SHERPA_CHECK_ONLY must be 1 or unset"),
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        fail(
            "engine-sherpa links sherpa-onnx's prebuilt Windows libraries; on other OSes, \
             type-check it with INK_SHERPA_CHECK_ONLY=1",
        );
    }
    let dir = env::var_os("SHERPA_ONNX_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            fail(
                "engine-sherpa needs SHERPA_ONNX_DIR: the unpacked \
                 sherpa-onnx-v1.13.4-win-x64-shared-MD-Release-no-tts-lib archive. To \
                 type-check without it, set INK_SHERPA_CHECK_ONLY=1.",
            )
        });
    let mut dlls = Vec::new();
    for (file, pinned) in SHERPA_FILES {
        let path = dir.join(file);
        println!("cargo:rerun-if-changed={}", path.display());
        let found = sha256_of(&path, &format!("SHERPA_ONNX_DIR has no {file}"));
        if found != pinned {
            fail(&format!(
                "{} is not sherpa-onnx 1.13.4's no-tts build (sha256 {found})",
                path.display()
            ));
        }
        if file.ends_with(".dll") {
            dlls.push(path);
        }
    }
    println!(
        "cargo:rustc-link-search=native={}",
        dir.join("lib").display()
    );
    println!("cargo:rustc-link-lib=dylib=sherpa-onnx-c-api");
    let dlls: Vec<&Path> = dlls.iter().map(PathBuf::as_path).collect();
    copy_dlls(&dlls, "sherpa-bin");
}

/// The file's SHA-256 as lowercase hex, or a build failure that says `what`.
fn sha256_of(path: &Path, what: &str) -> String {
    let bytes =
        fs::read(path).unwrap_or_else(|e| fail(&format!("{what} ({}: {e})", path.display())));
    Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `path` with symbolic links resolved (the library is a link to its versioned file).
fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|e| fail(&format!("resolving {}: {e}", path.display())))
}

fn fail(message: &str) -> ! {
    // A build script reports a failure by panicking; the message is what cargo prints.
    panic!("{message}");
}
