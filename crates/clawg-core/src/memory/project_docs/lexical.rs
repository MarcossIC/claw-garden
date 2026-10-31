//! The term half of a docs search. A docs question is often a long sentence,
//! and OR-ing its words, as memory search does, proposes every passage that
//! shares one common word with it; requiring its rarest few first proposes
//! the passages that hold them together.

use std::cmp::Reverse;

use crate::memory::search::{fts_match_expression, fts_tokens, is_identifier, MAX_LEXICAL_TOKENS};

/// How many of a question's likely rarest tokens the first try requires.
const ALL_TOKENS: usize = 3;

/// The FTS5 MATCH expressions of a docs search, tried in order: the second
/// only when the first finds no passage.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct DocsQuery {
    /// The [`ALL_TOKENS`] likely rarest tokens, AND-joined.
    pub all: Option<String>,
    /// The expression memory search uses, unless it is the same as `all`.
    pub any: Option<String>,
}

/// The expressions of a docs search for `query`, each `None` when there is
/// nothing to try. Rarity is guessed from the token alone, since its real
/// document frequency would cost a lookup per token: identifiers first, then
/// longer words before shorter ones, ties by position.
pub(super) fn docs_match_expression(query: &str) -> DocsQuery {
    let mut tokens: Vec<(usize, &str)> = fts_tokens(query).enumerate().collect();
    tokens.sort_by_key(|&(position, token)| (!is_identifier(token), Reverse(token.chars().count()), position));
    let all = (!tokens.is_empty()).then(|| {
        tokens
            .iter()
            .take(ALL_TOKENS)
            .map(|(_, t)| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(" AND ")
    });
    let any = fts_match_expression(query, MAX_LEXICAL_TOKENS).filter(|any| all.as_ref() != Some(any));
    DocsQuery { all, any }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_try_ands_the_three_likely_rarest_tokens() {
        let cases: &[(&str, &str)] = &[
            ("where is CHUNK_MAX_TOKENS used and what is db_id", r#""CHUNK_MAX_TOKENS" AND "db_id" AND "where""#),
            ("gamma beta alpha delta", r#""gamma" AND "alpha" AND "delta""#),
            ("el puerto de e5", r#""e5" AND "puerto" AND "el""#),
            ("uno dos", r#""uno" AND "dos""#),
        ];
        for (query, all) in cases {
            assert_eq!(docs_match_expression(query).all.as_deref(), Some(*all), "{query}");
        }
    }

    #[test]
    fn the_fallback_is_the_memory_expression() {
        for query in ["where is CHUNK_MAX_TOKENS used and what is db_id", "uno dos"] {
            assert_eq!(docs_match_expression(query).any, fts_match_expression(query, MAX_LEXICAL_TOKENS), "{query}");
        }
    }

    #[test]
    fn one_token_is_tried_once_and_none_is_not_tried() {
        assert_eq!(docs_match_expression("tombstone"), DocsQuery { all: Some(r#""tombstone""#.to_string()), any: None });
        assert_eq!(docs_match_expression("¿?"), DocsQuery { all: None, any: None });
    }
}
