//! Sets `cfg(ink_silero)` when ink-engines was built with Silero (`engine-silero`), which it
//! reports through its `links` metadata (`DEP_NEMO_SPEECH_ASR_C_SILERO`). See `src/vad.rs`.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=DEP_NEMO_SPEECH_ASR_C_SILERO");
    println!("cargo::rustc-check-cfg=cfg(ink_silero)");
    if std::env::var_os("DEP_NEMO_SPEECH_ASR_C_SILERO").is_some() {
        println!("cargo:rustc-cfg=ink_silero");
    }
}
