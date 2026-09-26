//! Turning what the user typed into an FTS5 query that cannot be anything but words.

/// Builds the FTS5 `MATCH` expression for a search box's text, or `None` when there is nothing
/// to search for.
///
/// The user's text is never FTS5 syntax. It is split on whitespace and control characters, and
/// each piece becomes a quoted string (inner quotes doubled), so `AND`, `NOT`, `NEAR`, `-`, `*`,
/// `^`, `:`, brackets and stray quotes are plain text: the tokenizer then treats punctuation as a
/// separator. A piece with punctuation inside (`e-mail`) stays one phrase. Each piece matches
/// as a word prefix (`"ship"*` finds "shipping"), so results follow typing. Pieces are joined
/// with `OR` and bm25 ranks the segments matching more of them, and more often, first.
///
/// Pieces without a letter or digit are dropped: they tokenize to nothing.
pub(crate) fn match_expression(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|piece| piece.chars().any(char::is_alphanumeric))
        .map(|piece| format!("\"{}\"*", piece.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" OR "))
}

#[cfg(test)]
mod tests {
    use super::match_expression;

    #[test]
    fn pieces_become_quoted_prefix_strings() {
        assert_eq!(match_expression("ship"), Some("\"ship\"*".into()));
        assert_eq!(
            match_expression("  budget\tAND  (plan) "),
            Some("\"budget\"* OR \"AND\"* OR \"(plan)\"*".into())
        );
        assert_eq!(
            match_expression("say \"hi\""),
            Some("\"say\"* OR \"\"\"hi\"\"\"*".into())
        );
        assert_eq!(match_expression("a\0b"), Some("\"a\"* OR \"b\"*".into()));
    }

    #[test]
    fn nothing_searchable_is_none() {
        for input in ["", "   ", "\t\n", "\"", "* - ( )", "\0", "^:"] {
            assert_eq!(match_expression(input), None, "{input:?}");
        }
    }
}
