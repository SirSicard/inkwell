//! Sets `cfg(ink_silero)` when ink-engines was built with Silero (`engine-silero`), which it
//! reports through its `links` metadata (`DEP_NEMO_SPEECH_ASR_C_SILERO`). See `src/vad.rs`.
//!
//! On Windows with Vulkan (`ink-engines/engine-llama-vulkan`, reported the same way as
//! `DEP_NEMO_SPEECH_ASR_C_VULKAN_DELAYLOAD`), links the core's DLL with the Vulkan loader
//! delay-loaded, so the DLL loads on a PC without one and llama.cpp runs on the CPU there
//! (ink-engines' `src/llama/no_vulkan.rs`).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=DEP_NEMO_SPEECH_ASR_C_SILERO");
    println!("cargo:rerun-if-env-changed=DEP_NEMO_SPEECH_ASR_C_VULKAN_DELAYLOAD");
    println!("cargo::rustc-check-cfg=cfg(ink_silero)");
    if std::env::var_os("DEP_NEMO_SPEECH_ASR_C_SILERO").is_some() {
        println!("cargo:rustc-cfg=ink_silero");
    }
    if let Ok(dll) = std::env::var("DEP_NEMO_SPEECH_ASR_C_VULKAN_DELAYLOAD") {
        println!("cargo:rustc-link-arg-cdylib=/DELAYLOAD:{dll}");
        println!("cargo:rustc-link-arg-cdylib=delayimp.lib");
    }
}
