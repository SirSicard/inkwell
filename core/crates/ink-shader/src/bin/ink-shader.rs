//! Writes the shells' shaders from `shaders/ink.wgsl`: the Mac app's MSL and the Windows shell's
//! HLSL.
//!
//! ```text
//! cargo run -p ink-shader --bin ink-shader             # regenerate
//! cargo run -p ink-shader --bin ink-shader -- --check  # fail if either is stale
//! ```
//!
//! The paths are [`ink_shader::MSL_OUT`] and [`ink_shader::HLSL_OUT`], from the repository root.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ink_shader::{HLSL_OUT, INK_WGSL, MSL_OUT, ShaderError, hlsl_file, msl_file};

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
    // Both, even when the first fails: each failure is printed.
    let msl_ok = write_or_check(MSL_OUT, msl_file(&wgsl), check);
    let hlsl_ok = write_or_check(HLSL_OUT, hlsl_file(&wgsl), check);
    let ok = msl_ok && hlsl_ok;
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Writes (or with `check`, compares) one generated file; false on any failure, which it prints.
fn write_or_check(out: &str, generated: Result<String, ShaderError>, check: bool) -> bool {
    let generated = match generated {
        Ok(s) => s,
        Err(e) => {
            eprintln!("shaders/ink.wgsl: {e}");
            return false;
        }
    };
    let path = repo_root().join(out);
    if check {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current.replace("\r\n", "\n") != generated {
            eprintln!("{out} is stale: run `cargo run -p ink-shader --bin ink-shader`");
            return false;
        }
        return true;
    }
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        eprintln!("{}: {e}", dir.display());
        return false;
    }
    if let Err(e) = std::fs::write(&path, generated) {
        eprintln!("{out}: {e}");
        return false;
    }
    println!("wrote {out}");
    true
}
