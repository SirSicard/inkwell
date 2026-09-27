//! Licence texts for crates whose published package carries none (`notices/overrides.txt`).
//!
//! Each line names one crate version, the text file under `notices/texts/` that stands in for the
//! licence file its package lacks, and why and where that text comes from. The generator uses a
//! line only for a crate whose own files cannot ship any licence it offers, and refuses a line that
//! no crate needs, so a version bump or a crate that starts shipping its files forces a new look.

use std::collections::BTreeMap;

/// One override.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Override {
    /// The text file under `notices/texts/`.
    pub text: String,
    /// Why, and where the text comes from; shown with the text.
    pub reason: String,
}

/// The table, by crate name and version.
pub type Overrides = BTreeMap<(String, String), Override>;

/// Reads the table: `<crate> <version> <text file> <reason...>` per line; blank lines and lines
/// starting with `#` are comments.
pub fn parse(table: &str) -> Result<Overrides, String> {
    let mut out = Overrides::new();
    for (n, line) in table.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut words = line.split_whitespace();
        let (Some(name), Some(version), Some(text)) = (words.next(), words.next(), words.next())
        else {
            return Err(format!(
                "line {}: expected `<crate> <version> <text file> <reason>`",
                n + 1
            ));
        };
        let reason = words.collect::<Vec<_>>().join(" ");
        if reason.is_empty() {
            return Err(format!("line {}: {name} {version} gives no reason", n + 1));
        }
        if text.contains(['/', '\\']) || text.starts_with('.') || !text.ends_with(".txt") {
            return Err(format!(
                "line {}: `{text}` is not a .txt file name under notices/texts/",
                n + 1
            ));
        }
        let key = (name.to_string(), version.to_string());
        if out.contains_key(&key) {
            return Err(format!("line {}: {name} {version} is listed twice", n + 1));
        }
        out.insert(
            key,
            Override {
                text: text.to_string(),
                reason,
            },
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_reads_one_override_per_line() {
        let table =
            "# comment\n\nfoo 1.2.3 Apache-2.0.txt the standard text; the package has none\n";
        let o = parse(table).unwrap();
        let got = &o[&("foo".to_string(), "1.2.3".to_string())];
        assert_eq!(got.text, "Apache-2.0.txt");
        assert_eq!(got.reason, "the standard text; the package has none");
    }

    #[test]
    fn a_line_without_a_reason_a_path_or_a_repeat_is_refused() {
        assert!(
            parse("foo 1.0.0 MIT.txt")
                .unwrap_err()
                .contains("no reason")
        );
        assert!(parse("foo 1.0.0").is_err());
        assert!(parse("foo 1.0.0 ../x.txt why").is_err());
        assert!(parse("foo 1.0.0 texts/x.txt why").is_err());
        assert!(parse("foo 1.0.0 x.md why").is_err());
        assert!(
            parse("foo 1.0.0 MIT.txt a\nfoo 1.0.0 MIT.txt b")
                .unwrap_err()
                .contains("twice")
        );
    }
}
