//! Writes the shells' design tokens from `design/tokens.json`: the Mac app's Swift, and the
//! Windows shell's XAML resources and C#.
//!
//! ```text
//! cargo run -p ink-shader --bin ink-tokens             # regenerate
//! cargo run -p ink-shader --bin ink-tokens -- --check  # fail if any is stale
//! ```
//!
//! The paths are [`ink_shader::tokens::SWIFT_OUT`], [`ink_shader::tokens::XAML_OUT`] and
//! [`ink_shader::tokens::CSHARP_OUT`], from the repository root.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ink_shader::tokens::{
    CSHARP_OUT, SWIFT_OUT, TOKENS_JSON, TOKENS_PATH, XAML_OUT, csharp, parse, swift, xaml,
};

fn repo_root() -> PathBuf {
    // core/crates/ink-shader -> the repository root.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn main() -> ExitCode {
    let check = std::env::args().any(|a| a == "--check");
    // The tokens as they are on disk now, not as this binary was built with them: a regenerate
    // right after an edit must use the edit.
    let json = match std::fs::read_to_string(repo_root().join(TOKENS_PATH)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{TOKENS_PATH}: {e}");
            return ExitCode::FAILURE;
        }
    };
    if json.replace("\r\n", "\n") != TOKENS_JSON.replace("\r\n", "\n") {
        // Harmless for the output (it is made from the file read above), but the tests check the
        // built-in copy: say so rather than let them disagree.
        eprintln!(
            "note: {TOKENS_PATH} changed since this binary was built; cargo rebuilds it next time"
        );
    }
    let tokens = match parse(&json) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    // Every target, so one stale file does not hide another.
    let mut ok = true;
    for (out, generated) in [
        (SWIFT_OUT, swift(&tokens)),
        (XAML_OUT, xaml(&tokens)),
        (CSHARP_OUT, csharp(&tokens)),
    ] {
        ok &= write_or_check(out, &generated, check);
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Writes (or with `check`, compares) one generated file; false on any failure, which it prints.
fn write_or_check(out: &str, generated: &str, check: bool) -> bool {
    let path = repo_root().join(out);
    if check {
        // A Windows checkout may turn line ends into CRLF; the content is what must match.
        let current = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        if current != generated {
            eprintln!("{out} is stale: run `cargo run -p ink-shader --bin ink-tokens`");
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
