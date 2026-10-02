//! The shells' design tokens are generated from `design/tokens.json`; this test fails while any
//! generated file is stale. Regenerate with `cargo run -p ink-shader --bin ink-tokens`.

use std::path::Path;

use ink_shader::tokens::{
    CSHARP_OUT, SWIFT_OUT, TOKENS_JSON, TOKENS_PATH, XAML_OUT, csharp, parse, swift, xaml,
};

fn repo_root() -> &'static Path {
    // core/crates/ink-shader -> the repository root.
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."))
}

#[test]
fn the_generated_tokens_are_current() {
    let tokens = parse(TOKENS_JSON).expect("design/tokens.json reads");
    for (out, generated) in [
        (SWIFT_OUT, swift(&tokens)),
        (XAML_OUT, xaml(&tokens)),
        (CSHARP_OUT, csharp(&tokens)),
    ] {
        let committed = std::fs::read_to_string(repo_root().join(out)).unwrap_or_default();
        // Line endings are normalised: a Windows checkout may turn LF into CRLF.
        assert!(
            committed.replace("\r\n", "\n") == generated,
            "{out} is stale: run `cargo run -p ink-shader --bin ink-tokens` in core/"
        );
    }
}

#[test]
fn the_tokens_file_is_the_one_built_in() {
    // TOKENS_JSON is include_str!'d at build time; a stale build would check old tokens.
    let on_disk = std::fs::read_to_string(repo_root().join(TOKENS_PATH)).expect(TOKENS_PATH);
    assert_eq!(
        on_disk.replace("\r\n", "\n"),
        TOKENS_JSON.replace("\r\n", "\n")
    );
}
