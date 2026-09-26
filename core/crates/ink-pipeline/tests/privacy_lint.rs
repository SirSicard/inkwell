//! I5, as a source lint: no log line and no error message in this crate formats transcript text.
//!
//! Ported from Inkwell 0.2 (`pipeline.rs`, `log_privacy_tests`). A source lint rather than only a
//! behaviour test, because that is where 0.2's bug lived: `redact()` existed, was tested, and was
//! used on every log line but one, which put two full copies of a dictation into a log file that
//! outlived the delete button. What 0.2's lint learned, kept here: a statement is joined across
//! lines before it is judged (a multi-line call has no string on its first line), and the lint is
//! proven on planted leaks. What is new:
//!
//! - **Every source file of the crate** is scanned, found at test time, so a new file cannot be
//!   forgotten (0.2 listed its files by hand).
//! - **Inline format arguments** (`"{text}"`, `"{raw:?}"`) are checked. 0.2 looked only after the
//!   format string, so a leak written the modern way passed.
//! - **Each argument** is judged on its own, where 0.2 let one `len()` anywhere in the statement
//!   excuse every other argument.
//! - **Error messages** (`write!(f, …)` in `Display` impls) are checked like log lines: errors end
//!   up in logs.

use std::path::Path;

/// Names that hold dictated text somewhere in this crate (and the names 0.2's lint watched).
const TEXT_BINDINGS: &[&str] = &[
    "text",
    "cleaned",
    "styled",
    "polished",
    "transcript",
    "raw",
    "instruction",
    "selection",
    "sel",
    "expanded",
    "final_text",
    "hypothesis",
    "partial",
    "content",
    "body",
    "written",
    "spoken",
    "insert",
    "words",
    "corrected",
    "segments",
    "segment",
    "prompt",
    "answer",
    "reply",
    "take",
];

/// Macros whose output goes to a log or into an error message.
const SINKS: &[&str] = &[
    "log::",
    "info!(",
    "warn!(",
    "error!(",
    "debug!(",
    "trace!(",
    "println!(",
    "eprintln!(",
    "print!(",
    "eprint!(",
    "panic!(",
    "write!(f",
    "writeln!(f",
];

/// Wrappers that make an argument safe: it becomes a count.
const SAFE: &[&str] = &["redact(", ".len()", ".chars().count()", ".count()"];

fn is_text_binding(word: &str) -> bool {
    TEXT_BINDINGS.contains(&word)
}

/// Every identifier in `expr`.
fn identifiers(expr: &str) -> impl Iterator<Item = &str> {
    expr.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
}

/// Splits a macro's argument list (after the format string) at top-level commas.
fn arguments(rest: &str) -> Vec<&str> {
    let mut args = Vec::new();
    let (mut depth, mut start) = (0i32, 0);
    for (i, c) in rest.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                args.push(&rest[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    args.push(&rest[start..]);
    args
}

/// The leaks in one joined statement: inline captures of text bindings, and arguments that
/// mention one without a safe wrapper.
fn leaks(statement: &str) -> Vec<String> {
    let mut found = Vec::new();
    let (Some(open), Some(close)) = (statement.find('"'), statement.rfind('"')) else {
        return found;
    };
    if close <= open {
        return found;
    }
    let format = &statement[open + 1..close];
    // Inline captures: `{name}`, `{name:?}`, `{name:>8}`. `{{` is a literal brace.
    for piece in format.split('{').skip(1) {
        let name = piece.split(['}', ':']).next().unwrap_or("").trim();
        if is_text_binding(name) {
            found.push(format!("inline {{{name}}}"));
        }
    }
    let rest = statement[close + 1..].trim_start_matches(',');
    for arg in arguments(rest) {
        if SAFE.iter().any(|s| arg.contains(s)) {
            continue;
        }
        if let Some(word) = identifiers(arg).find(|w| is_text_binding(w)) {
            found.push(format!("argument `{}` ({word})", arg.trim()));
        }
    }
    found
}

/// Every sink statement in `source`, joined across lines, with the line it starts on.
fn statements(source: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if t.starts_with("//") || !SINKS.iter().any(|s| t.contains(s)) {
            i += 1;
            continue;
        }
        let start = i;
        let mut statement = String::new();
        let mut depth = 0i32;
        loop {
            for c in lines[i].chars() {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
            }
            statement.push_str(lines[i].trim());
            statement.push(' ');
            if depth <= 0 || i + 1 >= lines.len() {
                break;
            }
            i += 1;
        }
        out.push((start + 1, statement));
        i += 1;
    }
    out
}

fn lint(name: &str, source: &str) -> Vec<String> {
    statements(source)
        .into_iter()
        .flat_map(|(line, s)| {
            leaks(&s)
                .into_iter()
                .map(move |l| format!("{name}:{line}: {l}"))
        })
        .collect()
}

/// Every `.rs` file under `dir`, named relative to the crate.
fn sources(dir: &Path, out: &mut Vec<(String, String)>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for entry in std::fs::read_dir(dir).expect("the source directory is readable") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).expect("a source file is readable");
            let name = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            out.push((name, text));
        }
    }
}

#[test]
fn no_log_line_formats_transcript_text_directly() {
    let mut files = Vec::new();
    sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(files.len() >= 10, "found only {} source files", files.len());
    let sinks: usize = files.iter().map(|(_, s)| statements(s).len()).sum();
    assert!(
        sinks >= 10,
        "the scan found only {sinks} log and error statements"
    );
    let offenders: Vec<String> = files.iter().flat_map(|(n, s)| lint(n, s)).collect();
    assert!(
        offenders.is_empty(),
        "log or error statement(s) format transcript text without redact():\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_lint_catches_planted_leaks() {
    let planted = r#"
fn f() {
    log::info!("raw: {}", text);
    log::warn!(
        "Disfluencies removed: {:?} -> {:?}",
        raw,
        cleaned
    );
    log::info!("inline: {text}");
    log::debug!("debug inline: {written:?}");
    log::info!("safe: {} and leak {}", redact(&text), styled);
    write!(f, "engine failed on {}", transcript)
}
"#;
    let found = lint("planted.rs", planted);
    assert_eq!(found.len(), 7, "{found:#?}");
    for needle in [
        "(text)",
        "(raw)",
        "(cleaned)",
        "{text}",
        "{written}",
        "(styled)",
        "(transcript)",
    ] {
        assert!(
            found.iter().any(|f| f.contains(needle)),
            "missed {needle}: {found:#?}"
        );
    }
}

#[test]
fn the_lint_passes_counts_and_unrelated_values() {
    let clean = r#"
fn f() {
    log::info!("inserted: {} ({outcome:?})", redact(&written));
    log::info!("{} chars, {} samples", text.chars().count(), audio.len());
    log::warn!("matcher failed to build: {e}");
    // log::info!("commented out: {}", text);
    write!(f, "mic format changed from {} Hz", expected.sample_rate)
}
"#;
    assert_eq!(lint("clean.rs", clean), Vec::<String>::new());
}
