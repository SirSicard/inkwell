//! Links NeMo-Speech.cpp's diarization library for `engine-nemo`. Without that feature it does
//! nothing.
//!
//! The library is built outside cargo (`native/build-nemo-speech.sh`) and found through
//! `NEMO_SPEECH_DIR`, its install prefix. Before linking, the installed C headers are compared
//! with the pinned commit's by SHA-256: `src/nemo.rs` declares the C ABI by hand, so a library
//! from any other commit is refused at build time rather than misread at run time.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// SHA-256 of the headers `src/nemo.rs` is written against, at NeMo-Speech.cpp `97a15af`.
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

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var_os("CARGO_FEATURE_ENGINE_NEMO").is_none() {
        return;
    }
    println!("cargo:rerun-if-env-changed=NEMO_SPEECH_DIR");
    let Some(dir) = env::var_os("NEMO_SPEECH_DIR").map(PathBuf::from) else {
        fail(
            "engine-nemo needs NEMO_SPEECH_DIR: the install prefix of NeMo-Speech.cpp 97a15af \
             (build it with crates/ink-engines/native/build-nemo-speech.sh)",
        );
    };
    for (header, pinned) in PINNED_HEADERS {
        let path = dir.join(header);
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes = fs::read(&path).unwrap_or_else(|e| {
            fail(&format!(
                "NEMO_SPEECH_DIR has no {header} ({}: {e})",
                path.display()
            ))
        });
        let found = hex(&Sha256::digest(&bytes));
        if found != pinned {
            fail(&format!(
                "{} is not the pinned NeMo-Speech.cpp 97a15af header (sha256 {found}); \
                 src/nemo.rs declares that commit's C ABI",
                path.display()
            ));
        }
    }
    let lib = dir.join("lib");
    if !has_library(&lib) {
        fail(&format!(
            "no nemo_speech_asr_c library in {}",
            lib.display()
        ));
    }
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=dylib=nemo_speech_asr_c");
    // This package's own tests and examples find the library where it was installed. A shipped
    // app bundles it next to the binary and sets its own rpath.
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os == "macos" || os == "linux" {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
    }
}

fn has_library(lib: &Path) -> bool {
    [
        "libnemo_speech_asr_c.dylib",
        "libnemo_speech_asr_c.so",
        "nemo_speech_asr_c.lib",
    ]
    .iter()
    .any(|name| lib.join(name).is_file())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn fail(message: &str) -> ! {
    // A build script reports a failure by panicking; the message is what cargo prints.
    panic!("{message}");
}
