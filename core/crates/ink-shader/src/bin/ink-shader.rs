//! Writes the Mac app's MSL from `shaders/ink.wgsl`.
//!
//! ```text
//! cargo run -p ink-shader --bin ink-shader             # regenerate
//! cargo run -p ink-shader --bin ink-shader -- --check  # fail if it is stale
//! ```
//!
//! The path is [`ink_shader::MSL_OUT`], from the repository root. The Windows shell's HLSL joins in
//! S3.4.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ink_shader::{INK_WGSL, MSL_OUT, msl_file};

fn repo_root() -> PathBuf {
    // core/crates/ink-shader -> the repository root.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn main() -> ExitCode {
    let check = std::env::args().any(|a| a == "--check");
    // The shader as it is on disk now, not as this binary was built with it: a regenerate right
    // after an edit must use the edit.
    let wgsl_path = repo_root().join("shaders/ink.wgsl");
    let wgsl = match std::fs::read_to_string(&wgsl_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("shaders/ink.wgsl: {e}");
            return ExitCode::FAILURE;
        }
    };
    if wgsl.replace("\r\n", "\n") != INK_WGSL.replace("\r\n", "\n") {
        // Harmless for the output (it is made from the file read above), but the tests check
        // the built-in copy: say so rather than let them disagree.
        eprintln!(
            "note: shaders/ink.wgsl changed since this binary was built; cargo rebuilds it next time"
        );
    }
    let generated = match msl_file(&wgsl) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("shaders/ink.wgsl: {e}");
            return ExitCode::FAILURE;
        }
    };
    let path = repo_root().join(MSL_OUT);
    if check {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current.replace("\r\n", "\n") != generated {
            eprintln!("{MSL_OUT} is stale: run `cargo run -p ink-shader --bin ink-shader`");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        eprintln!("{}: {e}", dir.display());
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(&path, generated) {
        eprintln!("{MSL_OUT}: {e}");
        return ExitCode::FAILURE;
    }
    println!("wrote {MSL_OUT}");
    ExitCode::SUCCESS
}
