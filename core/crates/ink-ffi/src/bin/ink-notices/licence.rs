//! Which of a crate's licence files ship, read from the files themselves.
//!
//! A crate names its licence as an SPDX expression (`MIT OR Apache-2.0`), and its package carries
//! licence files under names that vary (`LICENSE`, `LICENSE-MIT`, `COPYING`, `license`). The files
//! are matched to licences by their text, never by their names: [`classify`] finds each licence
//! whose full text a file contains. [`select`] then picks, for each choice the expression offers,
//! the first licence in [`PREFERENCE`] whose text the crate carries, and keeps every file a reader
//! needs: the chosen licences' texts, any NOTICE (Apache-2.0 §4(d)), copyright statements, and
//! licences of third-party code inside the crate. Only the texts of alternatives not taken are
//! left out.

use std::collections::BTreeSet;

/// The order in which an `OR` is decided: MIT first (short, and it carries the crate's own
/// copyright line), then the rest of the allowlist in `core/deny.toml`.
pub const PREFERENCE: [&str; 6] = [
    "MIT",
    "Apache-2.0",
    "BSD-3-Clause",
    "BSD-2-Clause",
    "ISC",
    "Zlib",
];

/// An SPDX licence expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    /// One licence.
    Id(String),
    /// A licence with an exception (`Apache-2.0 WITH LLVM-exception`).
    With(String, String),
    /// All of these.
    And(Vec<Expr>),
    /// Any one of these.
    Or(Vec<Expr>),
}

impl Expr {
    /// Every licence and exception id the expression names.
    fn ids(&self, out: &mut BTreeSet<String>) {
        match self {
            Expr::Id(id) => {
                out.insert(id.clone());
            }
            Expr::With(id, exception) => {
                out.insert(id.clone());
                out.insert(exception.clone());
            }
            Expr::And(parts) | Expr::Or(parts) => parts.iter().for_each(|p| p.ids(out)),
        }
    }
}

/// Parses an SPDX expression, with the old `/` for `OR` that some crates still publish. `WITH`
/// binds tighter than `AND`, and `AND` tighter than `OR`.
pub fn parse(expression: &str) -> Result<Expr, String> {
    let spaced = expression
        .replace('(', " ( ")
        .replace(')', " ) ")
        .replace('/', " OR ");
    let tokens: Vec<&str> = spaced.split_whitespace().collect();
    let mut at = 0;
    let expr = parse_or(&tokens, &mut at)?;
    if at != tokens.len() {
        return Err(format!(
            "unexpected `{}` in licence `{expression}`",
            tokens[at]
        ));
    }
    Ok(expr)
}

fn parse_or(tokens: &[&str], at: &mut usize) -> Result<Expr, String> {
    let mut parts = vec![parse_and(tokens, at)?];
    while tokens
        .get(*at)
        .is_some_and(|t| t.eq_ignore_ascii_case("OR"))
    {
        *at += 1;
        parts.push(parse_and(tokens, at)?);
    }
    Ok(if parts.len() == 1 {
        parts.remove(0)
    } else {
        Expr::Or(parts)
    })
}

fn parse_and(tokens: &[&str], at: &mut usize) -> Result<Expr, String> {
    let mut parts = vec![parse_with(tokens, at)?];
    while tokens
        .get(*at)
        .is_some_and(|t| t.eq_ignore_ascii_case("AND"))
    {
        *at += 1;
        parts.push(parse_with(tokens, at)?);
    }
    Ok(if parts.len() == 1 {
        parts.remove(0)
    } else {
        Expr::And(parts)
    })
}

fn parse_with(tokens: &[&str], at: &mut usize) -> Result<Expr, String> {
    let token = *tokens.get(*at).ok_or("the licence ends too early")?;
    *at += 1;
    if token == "(" {
        let inner = parse_or(tokens, at)?;
        if tokens.get(*at) != Some(&")") {
            return Err("a `(` is not closed".into());
        }
        *at += 1;
        return Ok(inner);
    }
    if is_operator(token) || token == ")" {
        return Err(format!("expected a licence, found `{token}`"));
    }
    if tokens
        .get(*at)
        .is_some_and(|t| t.eq_ignore_ascii_case("WITH"))
    {
        *at += 1;
        let exception = *tokens.get(*at).ok_or("`WITH` names no exception")?;
        if is_operator(exception) || exception == "(" || exception == ")" {
            return Err(format!(
                "expected an exception after `WITH`, found `{exception}`"
            ));
        }
        *at += 1;
        return Ok(Expr::With(token.to_string(), exception.to_string()));
    }
    Ok(Expr::Id(token.to_string()))
}

fn is_operator(token: &str) -> bool {
    ["AND", "OR", "WITH"]
        .iter()
        .any(|op| token.eq_ignore_ascii_case(op))
}

/// Lower case with every run of whitespace one space, and typographic quotes plain: the same
/// licence wrapped at another width, or typeset, reads the same.
fn normalise(text: &str) -> String {
    let plain: String = text
        .chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201c}' | '\u{201d}' => '"',
            c => c.to_ascii_lowercase(),
        })
        .collect();
    plain.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The licences whose full text `text` contains (a statement that only names a licence, such as
/// "Licensed under MIT or Apache-2.0", contains none). Each is recognised by phrases of its own
/// text, so MIT-0, which lacks MIT's notice clause, is not MIT, and BSD-2-Clause is BSD-3-Clause
/// without the endorsement clause.
pub fn classify(text: &str) -> BTreeSet<&'static str> {
    let t = normalise(text);
    let has = |phrase: &str| t.contains(phrase);
    let mut ids = BTreeSet::new();
    if has("permission is hereby granted, free of charge, to any person obtaining a copy")
        && has("the above copyright notice and this permission notice shall be included")
    {
        ids.insert("MIT");
    }
    if has("apache license")
        && has("version 2.0, january 2004")
        && has("terms and conditions for use, reproduction, and distribution")
    {
        ids.insert("Apache-2.0");
    }
    if has("llvm exceptions to the apache 2.0 license") {
        ids.insert("LLVM-exception");
    }
    if has(
        "redistribution and use in source and binary forms, with or without modification, are permitted",
    ) {
        if has("to endorse or promote products derived from this software") {
            ids.insert("BSD-3-Clause");
        } else {
            ids.insert("BSD-2-Clause");
        }
    }
    if has(
        "permission to use, copy, modify, and/or distribute this software for any purpose with or without fee is hereby granted",
    ) || has(
        "permission to use, copy, modify, and distribute this software for any purpose with or without fee is hereby granted",
    ) {
        ids.insert("ISC");
    }
    if has("this software is provided 'as-is', without any express or implied warranty")
        && has("altered source versions must be plainly marked as such")
    {
        ids.insert("Zlib");
    }
    if has("this is free and unencumbered software released into the public domain") {
        ids.insert("Unlicense");
    }
    if has("unicode license v3") {
        ids.insert("Unicode-3.0");
    }
    if has("blue oak model license")
        && has(
            "this license gives everyone as much permission to work with this software as possible",
        )
    {
        ids.insert("BlueOak-1.0.0");
    }
    if has("cc0 1.0 universal") && has("statement of purpose") {
        ids.insert("CC0-1.0");
    }
    ids
}

/// Whether a line of `text` states a copyright ("Copyright (c) 2024 Someone", "© Someone"), not
/// counting a template's placeholders ("Copyright [yyyy] [name of copyright owner]").
pub fn has_copyright_line(text: &str) -> bool {
    text.lines().any(|line| {
        let l = line.trim_start().to_ascii_lowercase();
        let states = l.starts_with("copyright")
            || l.starts_with("(c)")
            || line.trim_start().starts_with('©');
        let template = [
            "[yyyy]",
            "<year>",
            "<copyright holders>",
            "[name of copyright owner]",
        ]
        .iter()
        .any(|p| l.contains(p));
        states && !template
    })
}

/// A licence file of a crate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LicenceFile {
    /// Its name in the package, or the override's text name.
    pub name: String,
    /// Its text, line ends normalised.
    pub text: String,
    /// Set when the file is an override (the package carries no licence text): why, and where
    /// the text came from.
    pub supplied: Option<String>,
}

/// A file that ships, and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Included {
    /// The file.
    pub file: LicenceFile,
    /// The copyright line shown above an MIT text whose file names no holder: the crate's
    /// authors, from its manifest.
    pub holder_from_manifest: Option<String>,
}

/// What ships for a crate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    /// The licence the texts are, where the expression offered a choice (`MIT`, or
    /// `MIT AND BSD-3-Clause`).
    pub shown: String,
    /// The files, by name.
    pub files: Vec<Included>,
}

/// The authors as a copyright holder: names only, e-mail addresses dropped. Empty when the
/// manifest names no one.
pub fn holder(authors: &[String]) -> String {
    let names: Vec<String> = authors
        .iter()
        .map(|a| match a.find('<') {
            Some(at) => a[..at].trim().to_string(),
            None => a.trim().to_string(),
        })
        .filter(|a| !a.is_empty() && !(a.contains('@') && !a.contains(' ')))
        .collect();
    names.join(", ")
}

/// Whether `id` can ship from `files`: a file carries its text, and for MIT every such file names
/// its copyright holder or the manifest names the authors.
fn satisfiable(id: &str, files: &[LicenceFile], holder: &str) -> bool {
    let carrying: Vec<&LicenceFile> = files
        .iter()
        .filter(|f| classify(&f.text).contains(id))
        .collect();
    if carrying.is_empty() {
        return false;
    }
    id != "MIT" || !holder.is_empty() || carrying.iter().all(|f| has_copyright_line(&f.text))
}

/// A licence taken from an expression: how it reads, and the ids whose texts it ships.
struct Taken {
    display: String,
    ids: Vec<String>,
}

/// The licences taken from `expr`, in its order: every part of an `AND`, and of each `OR` the
/// first alternative in [`PREFERENCE`] order (then the rest in the expression's order) that can
/// ship.
fn choose(expr: &Expr, files: &[LicenceFile], holder: &str) -> Option<Vec<Taken>> {
    match expr {
        Expr::Id(id) => satisfiable(id, files, holder).then(|| {
            vec![Taken {
                display: id.clone(),
                ids: vec![id.clone()],
            }]
        }),
        Expr::With(id, exception) => (satisfiable(id, files, holder)
            && files
                .iter()
                .any(|f| classify(&f.text).contains(exception.as_str())))
        .then(|| {
            vec![Taken {
                display: format!("{id} WITH {exception}"),
                ids: vec![id.clone(), exception.clone()],
            }]
        }),
        Expr::And(parts) => {
            let mut all = Vec::new();
            for part in parts {
                all.extend(choose(part, files, holder)?);
            }
            Some(all)
        }
        Expr::Or(alternatives) => {
            let rank = |a: &Expr| match a {
                Expr::Id(id) => PREFERENCE
                    .iter()
                    .position(|p| p == id)
                    .unwrap_or(PREFERENCE.len()),
                _ => PREFERENCE.len(),
            };
            let mut ordered: Vec<&Expr> = alternatives.iter().collect();
            // Stable: equal ranks keep the expression's order.
            ordered.sort_by_key(|a| rank(a));
            ordered.into_iter().find_map(|a| choose(a, files, holder))
        }
    }
}

impl std::fmt::Display for Expr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Parentheses only where SPDX precedence needs them: an OR inside an AND.
        fn part(e: &Expr, inside_and: bool) -> String {
            match e {
                Expr::Or(_) if inside_and => format!("({e})"),
                _ => e.to_string(),
            }
        }
        match self {
            Expr::Id(id) => f.write_str(id),
            Expr::With(id, exception) => write!(f, "{id} WITH {exception}"),
            Expr::And(parts) => f.write_str(
                &parts
                    .iter()
                    .map(|p| part(p, true))
                    .collect::<Vec<_>>()
                    .join(" AND "),
            ),
            Expr::Or(parts) => f.write_str(
                &parts
                    .iter()
                    .map(|p| part(p, false))
                    .collect::<Vec<_>>()
                    .join(" OR "),
            ),
        }
    }
}

/// Picks what ships for a crate licensed as `expr`, from its licence `files` (sorted by name),
/// whose manifest names `authors`. An error says which licence text is missing.
pub fn select(expr: &Expr, files: &[LicenceFile], authors: &[String]) -> Result<Selection, String> {
    let holder = holder(authors);
    let Some(taken) = choose(expr, files, &holder) else {
        let found: BTreeSet<&str> = files.iter().flat_map(|f| classify(&f.text)).collect();
        let found = if found.is_empty() {
            "no licence text".to_string()
        } else {
            format!(
                "the text of {} only",
                found.into_iter().collect::<Vec<_>>().join(", ")
            )
        };
        let names = if files.is_empty() {
            "no licence files".to_string()
        } else {
            files
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let no_author = if holder.is_empty() {
            "; its manifest names no author"
        } else {
            ""
        };
        return Err(format!(
            "its licence is `{expr}`, but its package carries {found} ({names}){no_author}"
        ));
    };
    let mut named = BTreeSet::new();
    expr.ids(&mut named);
    let chosen: BTreeSet<&str> = taken
        .iter()
        .flat_map(|t| t.ids.iter().map(String::as_str))
        .collect();
    let mut included = Vec::new();
    for file in files {
        let ids = classify(&file.text);
        let is_notice = file.name.to_ascii_uppercase().starts_with("NOTICE");
        let takes_chosen = ids.iter().any(|id| chosen.contains(id));
        // A file holding only licences the expression offers that were not taken.
        let alternative_not_taken = !ids.is_empty()
            && ids
                .iter()
                .all(|id| named.contains(*id) && !chosen.contains(id));
        if alternative_not_taken && !is_notice && !takes_chosen {
            continue;
        }
        let holder_from_manifest = (ids.contains("MIT")
            && chosen.contains("MIT")
            && !has_copyright_line(&file.text)
            && !holder.is_empty())
        .then(|| format!("Copyright (c) {holder}"));
        included.push(Included {
            file: file.clone(),
            holder_from_manifest,
        });
    }
    let shown = taken
        .into_iter()
        .map(|t| t.display)
        .collect::<Vec<_>>()
        .join(" AND ");
    Ok(Selection {
        shown,
        files: included,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIT_BODY: &str = "Permission is hereby granted, free of charge, to any\nperson obtaining a copy of this software and associated\ndocumentation files (the \"Software\"), to deal in the Software.\n\nThe above copyright notice and this permission notice\nshall be included in all copies or substantial portions\nof the Software.\n";
    const APACHE: &str = "                                 Apache License\n                           Version 2.0, January 2004\n\n   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION\n";
    const BSD3: &str = "Copyright (c) 2011, Someone\n\nRedistribution and use in source and binary forms, with or without\nmodification, are permitted provided that ...\n* Neither the name of X nor the names of its contributors may be used\nto endorse or promote products derived from this software.\n";
    const BSD2: &str = "Copyright (c) 2011, Someone\n\nRedistribution and use in source and binary forms, with or without\nmodification, are permitted provided that ...\n";
    const UNLICENSE: &str =
        "This is free and unencumbered software released into the public domain.\n";

    fn file(name: &str, text: &str) -> LicenceFile {
        LicenceFile {
            name: name.into(),
            text: text.into(),
            supplied: None,
        }
    }

    fn mit(holder: &str) -> String {
        format!("Copyright (c) {holder}\n\n{MIT_BODY}")
    }

    fn names(s: &Selection) -> Vec<&str> {
        s.files.iter().map(|f| f.file.name.as_str()).collect()
    }

    #[test]
    fn expressions_parse_with_spdx_precedence_and_the_old_slash() {
        let id = |s: &str| Expr::Id(s.into());
        assert_eq!(parse("MIT").unwrap(), id("MIT"));
        assert_eq!(
            parse("MIT OR Apache-2.0").unwrap(),
            Expr::Or(vec![id("MIT"), id("Apache-2.0")])
        );
        assert_eq!(
            parse("MIT/Apache-2.0").unwrap(),
            Expr::Or(vec![id("MIT"), id("Apache-2.0")])
        );
        assert_eq!(
            parse("(MIT OR Apache-2.0) AND Unicode-3.0").unwrap(),
            Expr::And(vec![
                Expr::Or(vec![id("MIT"), id("Apache-2.0")]),
                id("Unicode-3.0")
            ])
        );
        assert_eq!(
            parse("Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT").unwrap(),
            Expr::Or(vec![
                Expr::With("Apache-2.0".into(), "LLVM-exception".into()),
                id("Apache-2.0"),
                id("MIT")
            ])
        );
        assert_eq!(
            parse("MIT OR Apache-2.0 AND BSD-3-Clause").unwrap(),
            Expr::Or(vec![
                id("MIT"),
                Expr::And(vec![id("Apache-2.0"), id("BSD-3-Clause")])
            ]),
            "AND binds tighter than OR"
        );
        assert_eq!(
            parse("(MIT OR Apache-2.0) AND Unicode-3.0")
                .unwrap()
                .to_string(),
            "(MIT OR Apache-2.0) AND Unicode-3.0"
        );
        for bad in [
            "",
            "MIT OR",
            "(MIT",
            "MIT)",
            "OR MIT",
            "Apache-2.0 WITH",
            "MIT Apache-2.0",
        ] {
            assert!(parse(bad).is_err(), "`{bad}` is refused");
        }
    }

    #[test]
    fn files_are_recognised_by_their_text_not_their_name() {
        assert_eq!(classify(&mit("A")), BTreeSet::from(["MIT"]));
        // Wrapped at another width, and with typographic quotes.
        assert_eq!(
            classify(&MIT_BODY.replace('\n', " ").replace('"', "\u{201c}")),
            BTreeSet::from(["MIT"])
        );
        assert_eq!(classify(APACHE), BTreeSet::from(["Apache-2.0"]));
        assert_eq!(classify(BSD3), BTreeSet::from(["BSD-3-Clause"]));
        assert_eq!(classify(BSD2), BTreeSet::from(["BSD-2-Clause"]));
        assert_eq!(classify(UNLICENSE), BTreeSet::from(["Unlicense"]));
        let zlib = "This software is provided 'as-is', without any express or implied warranty.\n2. Altered source versions must be plainly marked as such.";
        assert_eq!(classify(zlib), BTreeSet::from(["Zlib"]));
        let isc = "Permission to use, copy, modify, and/or distribute this software for any\npurpose with or without fee is hereby granted";
        assert_eq!(classify(isc), BTreeSet::from(["ISC"]));
        // A statement naming licences carries none of their texts.
        assert!(classify("Licensed under either of Apache License, Version 2.0 or MIT license at your option.").is_empty());
        // MIT-0 has no notice clause: not MIT.
        assert!(classify("Permission is hereby granted, free of charge, to any person obtaining a copy of this software, to deal in the Software without restriction.").is_empty());
        // A file holding two texts carries both.
        assert_eq!(
            classify(&format!("{}\n{BSD3}", mit("A"))),
            BTreeSet::from(["BSD-3-Clause", "MIT"])
        );
    }

    #[test]
    fn a_copyright_line_is_a_stated_holder_not_a_template() {
        assert!(has_copyright_line("Copyright (c) 2024 Someone\n"));
        assert!(has_copyright_line("  (c) Someone"));
        assert!(has_copyright_line("© 2014 Someone"));
        assert!(
            !has_copyright_line(MIT_BODY),
            "the MIT clause mentions a copyright notice, but states none"
        );
        assert!(!has_copyright_line(
            "Copyright [yyyy] [name of copyright owner]"
        ));
        assert!(!has_copyright_line(
            "Copyright (c) <year> <copyright holders>"
        ));
    }

    #[test]
    fn a_dual_licence_ships_the_mit_text_with_the_crates_copyright_line() {
        let files = [
            file("LICENSE-APACHE", APACHE),
            file("LICENSE-MIT", &mit("2015 Someone")),
        ];
        let s = select(&parse("MIT OR Apache-2.0").unwrap(), &files, &[]).unwrap();
        assert_eq!(s.shown, "MIT");
        assert_eq!(names(&s), ["LICENSE-MIT"]);
        assert_eq!(
            s.files[0].holder_from_manifest, None,
            "the file names its holder"
        );
        // The same with the licences written the other way round.
        let s = select(&parse("Apache-2.0 OR MIT").unwrap(), &files, &[]).unwrap();
        assert_eq!(s.shown, "MIT");
    }

    #[test]
    fn an_mit_file_naming_no_holder_gets_the_manifests_authors() {
        let files = [
            file("LICENSE-APACHE", APACHE),
            file("LICENSE-MIT", MIT_BODY),
        ];
        let authors = [
            "Ada Example <ada@example.com>".to_string(),
            "Bo Example".to_string(),
        ];
        let s = select(&parse("MIT OR Apache-2.0").unwrap(), &files, &authors).unwrap();
        assert_eq!(s.shown, "MIT");
        assert_eq!(
            s.files[0].holder_from_manifest.as_deref(),
            Some("Copyright (c) Ada Example, Bo Example"),
            "names only: no e-mail address"
        );
    }

    #[test]
    fn with_no_holder_anywhere_the_apache_text_ships_instead() {
        let files = [
            file("LICENSE-APACHE", APACHE),
            file("LICENSE-MIT", MIT_BODY),
        ];
        let s = select(
            &parse("Apache-2.0 OR MIT").unwrap(),
            &files,
            &["x@example.com".into()],
        )
        .unwrap();
        assert_eq!(s.shown, "Apache-2.0", "an address alone names no holder");
        assert_eq!(names(&s), ["LICENSE-APACHE"]);
    }

    #[test]
    fn statements_notices_and_third_party_licences_ship_alternatives_not_taken_do_not() {
        let files = [
            file(
                "COPYING",
                "This project is dual-licensed under the Unlicense and MIT licenses.",
            ),
            file("LICENSE-MIT", &mit("2015 Someone")),
            file("LICENSE-THIRD-PARTY", BSD3),
            file("NOTICE", "Some Project\nCopyright 2020 Some Org"),
            file("UNLICENSE", UNLICENSE),
        ];
        let s = select(&parse("Unlicense OR MIT").unwrap(), &files, &[]).unwrap();
        assert_eq!(s.shown, "MIT");
        assert_eq!(
            names(&s),
            ["COPYING", "LICENSE-MIT", "LICENSE-THIRD-PARTY", "NOTICE"]
        );
    }

    #[test]
    fn an_and_ships_every_part() {
        let files = [
            file("LICENSE-APACHE", APACHE),
            file("LICENSE-MIT", &mit("Mozilla Foundation")),
            file("LICENSE-WHATWG", BSD3),
        ];
        let s = select(
            &parse("(Apache-2.0 OR MIT) AND BSD-3-Clause").unwrap(),
            &files,
            &[],
        )
        .unwrap();
        assert_eq!(s.shown, "MIT AND BSD-3-Clause");
        assert_eq!(names(&s), ["LICENSE-MIT", "LICENSE-WHATWG"]);
    }

    #[test]
    fn an_exception_licence_not_taken_is_left_out() {
        let with_llvm = format!("{APACHE}\n---- LLVM Exceptions to the Apache 2.0 License ----\n");
        let files = [
            file("LICENSE-APACHE", APACHE),
            file("LICENSE-Apache-2.0_WITH_LLVM-exception", &with_llvm),
            file("LICENSE-MIT", MIT_BODY),
        ];
        let expr = parse("Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT").unwrap();
        let s = select(&expr, &files, &["Dan Example".into()]).unwrap();
        assert_eq!(s.shown, "MIT");
        assert_eq!(names(&s), ["LICENSE-MIT"]);
    }

    #[test]
    fn the_preference_decides_between_allowed_alternatives() {
        let files = [file("LICENSE-BSD", BSD2), file("LICENSE-APACHE", APACHE)];
        let s = select(
            &parse("BSD-2-Clause OR Apache-2.0 OR MIT").unwrap(),
            &files,
            &[],
        )
        .unwrap();
        assert_eq!(
            s.shown, "Apache-2.0",
            "MIT has no text here; Apache-2.0 comes before BSD-2-Clause"
        );
        assert_eq!(names(&s), ["LICENSE-APACHE"]);
        // A file holding MIT and BSD-3 (a port of BSD code) ships whole when MIT is taken.
        let both = format!("{}\n{BSD3}", mit("2025 Someone"));
        let s = select(
            &parse("MIT OR BSD-3-Clause").unwrap(),
            &[file("LICENSE", &both)],
            &[],
        )
        .unwrap();
        assert_eq!(s.shown, "MIT");
        assert_eq!(names(&s), ["LICENSE"]);
    }

    #[test]
    fn a_crate_without_the_text_of_any_licence_it_offers_is_refused_with_the_reason() {
        let e = select(&parse("MIT OR Apache-2.0").unwrap(), &[], &[]).unwrap_err();
        assert!(
            e.contains("no licence text")
                && e.contains("no licence files")
                && e.contains("names no author"),
            "{e}"
        );
        // A statement is not a licence text.
        let files = [file(
            "COPYING",
            "Copyright 2014 Someone. Distributed under MIT or Apache-2.0.",
        )];
        let e = select(
            &parse("BlueOak-1.0.0 OR MIT OR Apache-2.0").unwrap(),
            &files,
            &["Someone".into()],
        )
        .unwrap_err();
        assert!(e.contains("COPYING"), "{e}");
        // MIT text without any holder, and no Apache text to fall back on.
        let e = select(&parse("MIT").unwrap(), &[file("LICENSE", MIT_BODY)], &[]).unwrap_err();
        assert!(e.contains("names no author"), "{e}");
    }

    #[test]
    fn a_holder_is_names_without_addresses() {
        assert_eq!(
            holder(&["A B <a@b.c>".into(), " C ".into(), "d@e.f".into()]),
            "A B, C"
        );
        assert_eq!(holder(&[]), "");
    }
}
