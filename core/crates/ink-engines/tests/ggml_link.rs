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
//! With the diarizer's feature on as well (`engine-nemo`; it links NeMo-Speech.cpp, so this runs
//! locally, with `NEMO_SPEECH_DIR` set):
//!
//! 4. 1 and 2 still hold, and the only engine library the core loads is the diarizer's C API: its
//!    adapter never links ggml by name, because `-lggml` would bind to whichever ggml the linker
//!    meets first.
//! 5. The diarizer's libraries are two-level and import each ggml symbol from their own `libggml*`
//!    (`nm -m` shows `from libggml…`, never `dynamically looked up`), found beside them through
//!    `@loader_path` (`#[ignore]`: reads the installed libraries).
//! 6. In one process, llama.cpp transcribes and the diarizer diarizes, in turn, while the ggml the
//!    core calls still reports llama.cpp's version and the diarizer's loaded copy reports its own
//!    (`#[ignore]`: needs both models).

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

/// Points 4 to 6 of the module docs: the diarizer's ggml beside llama.cpp's.
#[cfg(feature = "engine-nemo")]
mod diarizer {
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::path::{Path, PathBuf};

    use ink_core::{CancelToken, Channel, Diarizer, OfflineEngine, TranscribeOptions};
    use ink_engines::llama::QwenAsr;
    use ink_engines::{NemoDevice, NemoDiarizer, Registry, nemotron_3_diarization};

    use super::{LLAMA_CPP_GGML, this_binary, tool};

    /// The ggml NeMo-Speech.cpp 97a15af ships (its libraries' `0.12.0` version suffix).
    const NEMO_GGML: &str = "0.12";

    /// The installed NeMo-Speech.cpp's library directory.
    fn nemo_lib() -> PathBuf {
        let dir = std::env::var_os("NEMO_SPEECH_DIR")
            .expect("NEMO_SPEECH_DIR must name the installed NeMo-Speech.cpp");
        PathBuf::from(dir).join("lib")
    }

    #[test]
    fn the_core_links_the_diarizers_c_api_and_no_ggml() {
        let libs = tool("otool", &["-L"], &this_binary());
        let direct: Vec<&str> = libs
            .lines()
            .skip(1)
            .filter(|l| l.contains("nemo_speech") || l.contains("ggml"))
            .map(str::trim)
            .collect();
        assert_eq!(direct.len(), 1, "{direct:?}");
        assert!(
            direct[0].starts_with("@rpath/libnemo_speech_asr_c."),
            "{direct:?}"
        );
    }

    #[test]
    #[ignore = "reads the installed NeMo-Speech.cpp libraries under NEMO_SPEECH_DIR; run locally"]
    fn the_diarizers_libraries_import_ggml_from_their_own_libggml() {
        let lib = nemo_lib();
        let mut ggml_imports = 0;
        for name in ["libnemo_speech_asr_c.dylib", "libnemo_speech_asr.dylib"] {
            let path = lib.join(name);
            let header = tool("otool", &["-hv"], &path);
            assert!(header.contains("TWOLEVEL"), "{name} is not two-level");

            // Every ggml import names the library it comes from, and that is one of NeMo's own.
            let symbols = tool("nm", &["-m"], &path);
            assert!(
                !symbols.contains("dynamically looked up"),
                "{name} looks symbols up dynamically"
            );
            for line in symbols.lines().filter(|l| l.contains("(undefined)")) {
                let symbol = line.split_whitespace().nth(2).unwrap_or_default();
                if symbol.starts_with("_ggml_") || symbol.starts_with("_gguf_") {
                    ggml_imports += 1;
                    assert!(line.contains("(from libggml"), "{name}: {line}");
                }
            }

            // Those libraries are loaded by @rpath, and the only rpath is beside the library.
            let loads = tool("otool", &["-L"], &path);
            for dep in loads.lines().skip(1).map(str::trim) {
                if dep.contains("libggml") {
                    let file = dep
                        .strip_prefix("@rpath/")
                        .and_then(|d| d.split_whitespace().next())
                        .unwrap_or_else(|| panic!("{name} loads ggml by a fixed path: {dep}"));
                    assert!(lib.join(file).exists(), "{name}: {file} is not beside it");
                }
            }
            let commands = tool("otool", &["-l"], &path);
            let rpaths: Vec<&str> = commands
                .lines()
                .map(str::trim)
                .filter_map(|l| l.strip_prefix("path "))
                .collect();
            assert!(!rpaths.is_empty(), "{name} has no rpath");
            assert!(
                rpaths.iter().all(|r| r.starts_with("@loader_path")),
                "{name}: {rpaths:?}"
            );
        }
        assert!(ggml_imports > 100, "only {ggml_imports} ggml imports");
    }

    unsafe extern "C" {
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> c_int;
    }

    /// `RTLD_LAZY | RTLD_NOLOAD` on macOS: a handle only if the library is already loaded.
    const LOADED_ONLY: c_int = 0x1 | 0x10;

    /// The version the diarizer's loaded ggml reports, asked of that library by name.
    fn nemo_ggml_version(lib: &Path) -> String {
        let path = CString::new(lib.join("libggml-base.0.dylib").to_str().unwrap()).unwrap();
        // SAFETY: `path` is a NUL-terminated string; with RTLD_NOLOAD dlopen only returns a
        // handle to a library that is already loaded (a reference count, released below).
        let handle = unsafe { dlopen(path.as_ptr(), LOADED_ONLY) };
        assert!(
            !handle.is_null(),
            "NeMo's libggml-base is not loaded from {}",
            lib.display()
        );
        // SAFETY: `handle` is live; the name is NUL-terminated.
        let symbol = unsafe { dlsym(handle, c"ggml_version".as_ptr()) };
        assert!(!symbol.is_null(), "NeMo's libggml-base has no ggml_version");
        // SAFETY: the symbol is ggml's `const char *ggml_version(void)`, which returns a static
        // NUL-terminated string.
        let version = unsafe {
            let f: unsafe extern "C" fn() -> *const c_char = std::mem::transmute(symbol);
            CStr::from_ptr(f()).to_string_lossy().into_owned()
        };
        // SAFETY: the handle came from dlopen above and is released once.
        unsafe { dlclose(handle) };
        version
    }

    unsafe extern "C" {
        fn ggml_version() -> *const c_char;
    }

    fn bench() -> PathBuf {
        PathBuf::from(std::env::var_os("INK_BENCH_DIR").expect("INK_BENCH_DIR is not set"))
    }

    /// The first `seconds` of an AMI headset clip (16 kHz float).
    fn ami(seconds: usize) -> Vec<f32> {
        let path = bench().join("ami-ihm/ami-ihm-00.wav");
        let mut reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().sample_rate, 16_000);
        reader
            .samples::<f32>()
            .take(seconds * 16_000)
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    #[ignore = "needs the Qwen3-ASR and Nemotron models under INK_BENCH_DIR, and NEMO_SPEECH_DIR"]
    fn llama_cpp_and_the_diarizer_both_answer_in_one_process() {
        let audio = ami(20);
        let qwen = Registry::builtin()
            .unwrap()
            .get("qwen3-asr-1.7b-q8")
            .unwrap()
            .clone();
        let qwen_dir = bench().join("models/qwen3-asr-1.7b-gguf");
        let asr = QwenAsr::load(
            &qwen_dir.join(&qwen.files[0].name),
            &qwen_dir.join(&qwen.files[1].name),
            qwen.info(),
        )
        .unwrap();
        let row = nemotron_3_diarization();
        let diarizer = NemoDiarizer::new(
            &bench()
                .join("models/nemotron-3-diarization")
                .join(&row.files[0].name),
            row.info(),
            NemoDevice::Gpu(0),
        )
        .unwrap();
        let options = TranscribeOptions {
            channel: Channel::Mic,
            context: None,
            cancel: CancelToken::new(),
        };
        // In turn, twice: each engine answers with the other one loaded and used.
        for round in 0..2 {
            let text = asr.transcribe(&audio, &options).unwrap().text();
            let turns = diarizer.diarize(&audio, &CancelToken::new()).unwrap();
            println!(
                "round {round}: {} words transcribed, {} turns diarized",
                text.split_whitespace().count(),
                turns.len()
            );
            assert!(
                text.split_whitespace().count() > 10,
                "round {round}: {text:?}"
            );
            assert!(!turns.is_empty(), "round {round}: no turns");
        }
        // SAFETY: `ggml_version` takes no arguments and returns a static, NUL-terminated string.
        let core = unsafe { CStr::from_ptr(ggml_version()) }
            .to_str()
            .unwrap()
            .to_owned();
        let nemo = nemo_ggml_version(&nemo_lib());
        println!("the core's ggml {core}; the diarizer's ggml {nemo}");
        assert_eq!(core, LLAMA_CPP_GGML);
        assert!(
            nemo.starts_with(NEMO_GGML),
            "the diarizer's ggml reports {nemo}"
        );
        // llama.cpp's Metal backend aborts if a model is still loaded at exit.
        drop(asr);
        drop(diarizer);
    }
}
