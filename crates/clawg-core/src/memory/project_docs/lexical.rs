//! The term half of a docs search. Memories keep their expression
//! (`fts_match_expression`); a docs question is often a long sentence, and
//! which of its tokens reach the BM25 channel decides what that channel finds.

use std::cmp::Reverse;

use crate::memory::search::{fts_tokens, is_identifier, MAX_LEXICAL_TOKENS};

/// The FTS5 MATCH expression of a docs search, or `None` when `query` has no
/// token. Its tokens are quoted and OR-joined as `fts_match_expression` joins
/// them, but the cap of [`MAX_LEXICAL_TOKENS`] keeps the likely rarest instead
/// of the first: identifiers, then longer words before shorter ones, ties by
/// position. Rarity is guessed from the token alone, since its real document
/// frequency would cost a lookup per token.
pub(super) fn docs_match_expression(query: &str) -> Option<String> {
    let mut tokens: Vec<(usize, &str)> = fts_tokens(query).enumerate().collect();
    tokens.sort_by_key(|&(position, token)| (!is_identifier(token), Reverse(token.chars().count()), position));

    if tokens.is_empty() {
        return None;
    }

    Some(
        tokens
            .iter()
            .take(MAX_LEXICAL_TOKENS)
            .map(|(_, t)| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(query: &str) -> Vec<String> {
        docs_match_expression(query)
            .unwrap()
            .split(" OR ")
            .map(|t| t.trim_matches('"').to_string())
            .collect()
    }

    #[test]
    fn identifiers_come_first_then_longer_words_then_position() {
        let cases: &[(&str, &[&str])] = &[
            (
                "where is CHUNK_MAX_TOKENS used and what is db_id",
                &["CHUNK_MAX_TOKENS", "db_id", "where", "used", "what", "and", "is", "is"],
            ),
            ("gamma beta alpha", &["gamma", "alpha", "beta"]),
            ("el puerto de e5", &["e5", "puerto", "el", "de"]),
        ];
        for (query, expected) in cases {
            assert_eq!(terms(query), *expected, "{query}");
        }
    }

    #[test]
    fn the_cap_keeps_the_rarest_words() {
        let query = format!("{} tombstone", ["de"; 20].join(" "));

        let terms = terms(&query);

        assert_eq!(terms.len(), MAX_LEXICAL_TOKENS);
        assert_eq!(terms[0], "tombstone");
    }

    #[test]
    fn a_query_without_tokens_has_no_expression() {
        assert_eq!(docs_match_expression("¿?"), None);
        assert_eq!(docs_match_expression(""), None);
    }
}
