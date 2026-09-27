//! With `engine-nemo`, this package's test binaries link NeMo-Speech.cpp's library, which
//! ink-engines links from where it was installed (`NEMO_SPEECH_DIR`). Cargo gives only
//! ink-engines' own binaries its rpath, so without this the tests here would not start unless
//! `DYLD_LIBRARY_PATH` named the library. ink-engines declares `links`, and hands the directory
//! over as `DEP_NEMO_SPEECH_ASR_C_LIB_DIR` whenever it links the library.
//!
//! The flag reaches every binary this package links, which are only test binaries (the library's
//! unit tests and `tests/`; there are no bins, examples or cdylibs). A shipped app bundles the
//! library and sets its own rpath.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=DEP_NEMO_SPEECH_ASR_C_LIB_DIR");
    let Ok(dir) = std::env::var("DEP_NEMO_SPEECH_ASR_C_LIB_DIR") else {
        return;
    };
    // As ink-engines does: `-rpath` is the macOS and Linux linkers' flag.
    if matches!(
        std::env::var("CARGO_CFG_TARGET_OS").as_deref(),
        Ok("macos" | "linux")
    ) {
        // Not `rustc-link-arg-tests`: that one misses the library's unit-test binary.
        println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
    }
}
