//! Sets `cfg(ink_silero)` when ink-engines was built with Silero (`engine-silero`), which it
//! reports through its `links` metadata (`DEP_NEMO_SPEECH_ASR_C_SILERO`). See `src/vad.rs`.
//!
//! On Windows, links the core's DLL with up to two DLLs delay-loaded, each reported the same way:
//! - with Vulkan (`ink-engines/engine-llama-vulkan`, `DEP_NEMO_SPEECH_ASR_C_VULKAN_DELAYLOAD`), the
//!   Vulkan loader, so the DLL loads on a PC without one and llama.cpp runs on the CPU there
//!   (ink-engines' `src/llama/no_vulkan.rs`);
//! - with the diarizer (`ink-engines/engine-nemo`, `DEP_NEMO_SPEECH_ASR_C_NEMO_DELAYLOAD`),
//!   NeMo-Speech.cpp's DLL, whose Vulkan backend needs that loader as soon as it loads (ink-engines'
//!   `src/nemo.rs` loads it before its first call).

/// The metadata that names a DLL to delay-load.
const DELAYLOADS: [&str; 2] = [
    "DEP_NEMO_SPEECH_ASR_C_VULKAN_DELAYLOAD",
    "DEP_NEMO_SPEECH_ASR_C_NEMO_DELAYLOAD",
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=DEP_NEMO_SPEECH_ASR_C_SILERO");
    println!("cargo::rustc-check-cfg=cfg(ink_silero)");
    if std::env::var_os("DEP_NEMO_SPEECH_ASR_C_SILERO").is_some() {
        println!("cargo:rustc-cfg=ink_silero");
    }
    let mut delayed = false;
    for var in DELAYLOADS {
        println!("cargo:rerun-if-env-changed={var}");
        if let Ok(dll) = std::env::var(var) {
            println!("cargo:rustc-link-arg-cdylib=/DELAYLOAD:{dll}");
            delayed = true;
        }
    }
    if delayed {
        println!("cargo:rustc-link-arg-cdylib=delayimp.lib");
    }
}
