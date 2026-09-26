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
//! - **Error messages** are checked like log lines, because errors end up in logs: `write!` and
//!   `writeln!` (`Display` impls), `format!` (the strings inside error variants) and `panic!`.
//! - **A statement is recognised by its joined text**, not its first line: rustfmt puts `write!(`
//!   on one line and the formatter and the format string on the next ones.
//! - **Building the dictation itself is not a log**: the text to insert, an export. Such a
//!   `format!` is exempt only with `// i5-allow: <reason>` on the line above it, so every exemption
//!   is written down and reviewed.

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

/// Macros whose output goes to a log or into an error message, as they read with whitespace
/// removed.
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
    "write!(",
    "writeln!(",
    "format!(",
    "format_args!(",
];

/// The comment that exempts the statement below it, with a reason after the colon.
const ALLOW: &str = "// i5-allow:";

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
    let Some(open) = statement.find('"') else {
        return found;
    };
    let Some(close) = string_end(statement, open) else {
        return found;
    };
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

/// The change in parenthesis depth over `line`, ignoring parentheses inside string and character
/// literals (a `"("` in a format string must not swallow the lines after it).
fn depth_change(line: &str) -> i32 {
    let mut depth = 0;
    let mut in_string = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' if in_string => {
                chars.next();
            }
            '"' => in_string = !in_string,
            '\'' if !in_string => {
                // A character literal such as '(' : skip it whole.
                let mut ahead = chars.clone();
                if let (Some(_), Some('\'')) = (ahead.next(), ahead.next()) {
                    chars.next();
                    chars.next();
                }
            }
            '(' if !in_string => depth += 1,
            ')' if !in_string => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// Every sink statement in `source`, joined across lines, with the line it starts on. A statement
/// starts at any line that opens a macro call or names `log::`; it is joined until its
/// parentheses balance, and it is a sink when the joined text, whitespace removed, contains one.
/// Statements under an [`ALLOW`] comment are left out.
fn statements(source: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if t.starts_with("//") || !(t.contains("!(") || t.contains("log::")) {
            i += 1;
            continue;
        }
        let start = i;
        let mut statement = String::new();
        let mut depth = 0i32;
        loop {
            depth += depth_change(lines[i]);
            statement.push_str(lines[i].trim());
            statement.push(' ');
            if depth <= 0 || i + 1 >= lines.len() {
                break;
            }
            i += 1;
        }
        let compact: String = statement.chars().filter(|c| !c.is_whitespace()).collect();
        let allowed = start > 0 && lines[start - 1].trim().starts_with(ALLOW);
        if !allowed && SINKS.iter().any(|sink| compact.contains(sink)) {
            out.push((start + 1, statement));
        }
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

/// The index of the quote that closes the string literal opening at `open`, skipping escapes. The
/// format string ends there, not at the statement's last quote: an argument may hold a string.
fn string_end(statement: &str, open: usize) -> Option<usize> {
    let mut escaped = false;
    for (i, c) in statement[open + 1..].char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => return Some(open + 1 + i),
            _ => {}
        }
    }
    None
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

/// The forms rustfmt produces once a call is too long for one line: the macro on one line, the
/// formatter or the format string on the next. A lint that judged the first line alone missed
/// every one of these.
#[test]
fn the_lint_catches_leaks_split_across_lines_by_rustfmt() {
    let planted = r#"
impl fmt::Display for E {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "engine failed on {}",
            transcript
        )
    }
}
fn g() -> EngineError {
    EngineError::Failed(format!(
        "decode failed after {}",
        raw
    ))
}
fn h(out: &mut String) {
    writeln!(
        out,
        "{text}"
    )
    .ok();
    let e = EngineError::Failed(
        format!("inline {spoken}"),
    );
    panic!(
        "unexpected {:?} (at {})",
        written, "("
    );
}
"#;
    let found = lint("split.rs", planted);
    for needle in ["(transcript)", "(raw)", "{text}", "{spoken}", "(written)"] {
        assert!(
            found.iter().any(|f| f.contains(needle)),
            "missed {needle}: {found:#?}"
        );
    }
    assert_eq!(found.len(), 5, "{found:#?}");
}

/// Building the dictation itself (the text to insert, an export) is not a log or an error. Such a
/// `format!` is exempt only with a reason on the line above it.
#[test]
fn a_marked_format_is_exempt_and_an_unmarked_one_is_not() {
    let source = r#"
fn f() {
    // i5-allow: the dictation itself, on its way into the focused app
    let insert = format!("{written} ");
    let other = format!("{written} ");
}
"#;
    let found = lint("marked.rs", source);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].starts_with("marked.rs:5:"), "{found:#?}");
}
