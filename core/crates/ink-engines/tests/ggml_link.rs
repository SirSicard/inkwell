//! The ggml decision (docs/ARCHITECTURE.md, "ggml"): two copies, kept apart. llama.cpp's ggml is
//! linked statically into the core; the diarizer's ggml stays in its own libraries.
//!
//! What this proves for the llama.cpp side, on the binary cargo links for this test (the same way
//! it links the app's core):
//!
//! 1. It loads no llama, mtmd, gguf or ggml library, so no install name of ours can collide with
//!    the diarizer's `libggml*.dylib`.
//! 2. It imports no `ggml_*`, `gguf_*`, `llama_*`, `mtmd_*` or `clip_*` symbol from any library,
//!    and defines the ones llama.cpp calls: all of them were bound inside the core at link time.
//! 3. At run time the ggml the core calls is the one llama.cpp vendors.
//!
//! With the diarizer's feature on as well, this file must also show (the diarizer's step adds it):
//! 1 and 2 still hold (its adapter calls only the diarizer's C API and never links ggml by name,
//! because `-lggml` would bind to whichever ggml the linker meets first); its library imports each
//! ggml symbol from its own `libggml*` under the two-level namespace (`nm -m` shows `from
//! libggml…`, never `dynamically looked up`); and in one process, 3 still reads llama.cpp's version
//! while the diarizer's copy reports its own.

#![cfg(all(feature = "engine-llama", target_os = "macos"))]

use std::ffi::{CStr, c_char};
use std::path::{Path, PathBuf};
use std::process::Command;

use ink_core::{EngineError, EngineInfo};
use ink_engines::llama::QwenAsr;

unsafe extern "C" {
    /// ggml's version string. Resolved at link time, like every ggml call the core makes.
    fn ggml_version() -> *const c_char;
}

/// The ggml that llama-cpp-sys-2 0.1.157 vendors. A llama-cpp-2 bump changes it, and this test
/// then asks for the decision to be checked again.
const LLAMA_CPP_GGML: &str = "0.24.0";

/// Symbol prefixes of llama.cpp, mtmd and ggml (Mach-O adds the leading underscore).
const ENGINE_PREFIXES: &[&str] = &["_ggml_", "_gguf_", "_llama_", "_mtmd_", "_clip_"];

fn this_binary() -> PathBuf {
    // A call into the adapter, so the linker keeps llama.cpp's loading path in this binary (it
    // strips what nothing here calls). It returns before touching llama.cpp: the file does not
    // exist.
    let info = EngineInfo {
        id: "link-test".into(),
        jobs: Vec::new(),
        licence: "MIT".into(),
    };
    let absent = Path::new("absent.gguf");
    assert!(matches!(
        QwenAsr::load(absent, absent, info),
        Err(EngineError::ModelMissing(_))
    ));
    std::env::current_exe().unwrap()
}

fn tool(name: &str, args: &[&str], file: &Path) -> String {
    let out = Command::new(name)
        .args(args)
        .arg(file)
        .output()
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(out.status.success(), "{name} failed: {:?}", out.status);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn the_core_loads_no_llama_or_ggml_library() {
    let libs = tool("otool", &["-L"], &this_binary());
    let engine_libs: Vec<&str> = libs
        .lines()
        .skip(1) // the binary's own path
        .filter(|l| {
            ["libggml", "libllama", "libmtmd", "libgguf"]
                .iter()
                .any(|n| l.contains(n))
        })
        .collect();
    assert!(
        engine_libs.is_empty(),
        "dynamic engine libraries: {engine_libs:?}"
    );
}

#[test]
fn every_llama_and_ggml_symbol_is_bound_inside_the_core() {
    let symbols = tool("nm", &["-m"], &this_binary());
    let name = |line: &str| {
        line.split_whitespace()
            .nth(3)
            .unwrap_or_default()
            .to_owned()
    };
    let engine = |line: &&str| ENGINE_PREFIXES.iter().any(|p| name(line).starts_with(p));

    // `nm -m` marks an import `(undefined) external _x (from libY)`.
    let imported: Vec<&str> = symbols
        .lines()
        .filter(|l| l.contains("(undefined)"))
        .filter(|l| {
            let n = l.split_whitespace().nth(2).unwrap_or_default();
            ENGINE_PREFIXES.iter().any(|p| n.starts_with(p))
        })
        .collect();
    assert!(imported.is_empty(), "imported engine symbols: {imported:?}");

    let defined: Vec<String> = symbols
        .lines()
        .filter(|l| l.contains("(__TEXT,__text)"))
        .filter(engine)
        .map(name)
        .collect();
    for needed in [
        "_ggml_init",
        "_ggml_version",
        "_gguf_init_from_file",
        "_llama_backend_init",
        "_llama_model_load_from_file",
        "_mtmd_init_from_file",
    ] {
        assert!(
            defined.iter().any(|d| d == needed),
            "{needed} is not defined in the core ({} engine symbols are)",
            defined.len()
        );
    }
}

#[test]
fn the_ggml_the_core_calls_is_llama_cpps_own() {
    let _ = this_binary();
    // SAFETY: `ggml_version` takes no arguments and returns a pointer to a static, NUL-terminated
    // string that lives as long as the program; it is only read here.
    let version = unsafe { CStr::from_ptr(ggml_version()) };
    assert_eq!(version.to_str(), Ok(LLAMA_CPP_GGML));
}
