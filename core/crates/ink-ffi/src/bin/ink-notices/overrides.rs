//! Licence texts for crates whose published package carries none (`notices/overrides.txt`).
//!
//! Each line names one crate version, the text file under `notices/texts/` that stands in for the
//! licence file its package lacks, whether that text has been compared with the licence file in
//! the crate's own repository (`verified=no`, or the date it was), and why and where the text
//! comes from. A release tag waits until every line has a date (`mac/scripts/notices-verified.sh`). The generator uses a
//! line only for a crate whose own files cannot ship any licence it offers, and refuses a line that
//! no crate needs, so a version bump or a crate that starts shipping its files forces a new look.

use std::collections::BTreeMap;

/// One override.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Override {
    /// The text file under `notices/texts/`.
    pub text: String,
    /// The date (YYYY-MM-DD) the text was compared with the crate's upstream licence file, or
    /// `None` while it has not been.
    pub verified: Option<String>,
    /// Why, and where the text comes from; shown with the text.
    pub reason: String,
}

/// The table, by crate name and version.
pub type Overrides = BTreeMap<(String, String), Override>;

/// Reads the table: `<crate> <version> <text file> verified=<no|YYYY-MM-DD> <reason...>` per
/// line; blank lines and lines starting with `#` are comments.
pub fn parse(table: &str) -> Result<Overrides, String> {
    let mut out = Overrides::new();
    for (n, line) in table.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = n + 1;
        let mut words = line.split_whitespace();
        let (Some(name), Some(version), Some(text)) = (words.next(), words.next(), words.next())
        else {
            return Err(format!(
                "line {at}: expected `<crate> <version> <text file> verified=<no|YYYY-MM-DD> <reason>`"
            ));
        };
        let verified = match words.next().and_then(|w| w.strip_prefix("verified=")) {
            Some("no") => None,
            Some(date) if is_date(date) => Some(date.to_string()),
            _ => {
                return Err(format!(
                    "line {at}: {name} {version} needs verified=no or verified=YYYY-MM-DD after its text file"
                ));
            }
        };
        let reason = words.collect::<Vec<_>>().join(" ");
        if reason.is_empty() {
            return Err(format!("line {at}: {name} {version} gives no reason"));
        }
        if text.contains(['/', '\\']) || text.starts_with('.') || !text.ends_with(".txt") {
            return Err(format!(
                "line {at}: `{text}` is not a .txt file name under notices/texts/"
            ));
        }
        let key = (name.to_string(), version.to_string());
        if out.contains_key(&key) {
            return Err(format!("line {at}: {name} {version} is listed twice"));
        }
        out.insert(
            key,
            Override {
                text: text.to_string(),
                verified,
                reason,
            },
        );
    }
    Ok(out)
}

/// `YYYY-MM-DD`, digits only.
fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_reads_one_override_per_line_with_its_upstream_check() {
        let table = "# comment\n\nfoo 1.2.3 Apache-2.0.txt verified=no the standard text; the package has none\n\
                     bar 0.1.0 MIT.txt verified=2026-10-04 the MIT licence's text\n";
        let o = parse(table).unwrap();
        let got = &o[&("foo".to_string(), "1.2.3".to_string())];
        assert_eq!(got.text, "Apache-2.0.txt");
        assert_eq!(got.verified, None, "not yet compared with upstream");
        assert_eq!(got.reason, "the standard text; the package has none");
        assert_eq!(
            o[&("bar".to_string(), "0.1.0".to_string())]
                .verified
                .as_deref(),
            Some("2026-10-04")
        );
    }

    #[test]
    fn a_line_without_a_reason_a_marker_a_path_or_a_repeat_is_refused() {
        assert!(
            parse("foo 1.0.0 MIT.txt verified=no")
                .unwrap_err()
                .contains("no reason")
        );
        assert!(
            parse("foo 1.0.0 MIT.txt why")
                .unwrap_err()
                .contains("verified="),
            "the marker is required"
        );
        for bad in [
            "verified=soon",
            "verified=2026-1-4",
            "verified=26-10-04",
            "verified=",
            "verified=yes",
        ] {
            assert!(
                parse(&format!("foo 1.0.0 MIT.txt {bad} why"))
                    .unwrap_err()
                    .contains("verified="),
                "{bad}"
            );
        }
        assert!(parse("foo 1.0.0").is_err());
        assert!(parse("foo 1.0.0 ../x.txt verified=no why").is_err());
        assert!(parse("foo 1.0.0 texts/x.txt verified=no why").is_err());
        assert!(parse("foo 1.0.0 x.md verified=no why").is_err());
        assert!(
            parse("foo 1.0.0 MIT.txt verified=no a\nfoo 1.0.0 MIT.txt verified=no b")
                .unwrap_err()
                .contains("twice")
        );
    }
}
