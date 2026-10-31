//! The term half of a docs search. Memories keep [`fts_match_expression`];
//! docs leave out the words nearly every passage shares, so the BM25 channel
//! ranks by the words that tell one passage from another.

use crate::memory::search::{fts_match_expression, fts_tokens, is_identifier, MAX_LEXICAL_TOKENS};

/// Function words of Spanish and English, in lower case. A word shorter than
/// [`MIN_WORD_CHARS`] goes by its length and is not listed. FTS5 folds
/// diacritics, so a word common with and without its accent is listed both
/// ways.
const DOCS_STOPWORDS: &[&str] = &[
    // English
    "about", "above", "after", "again", "against", "all", "also", "and", "another", "any", "are", "aren",
    "because", "been", "before", "being", "below", "between", "both", "but", "can", "cannot", "could",
    "couldn", "did", "didn", "does", "doesn", "doing", "don", "down", "during", "each", "either", "else",
    "even", "every", "few", "for", "from", "further", "had", "hadn", "has", "hasn", "have", "haven",
    "having", "her", "here", "hers", "herself", "him", "himself", "his", "how", "however", "into", "isn",
    "its", "itself", "just", "may", "might", "more", "most", "much", "must", "myself", "neither", "nor",
    "not", "now", "off", "once", "only", "onto", "other", "our", "ours", "ourselves", "out", "over", "own",
    "same", "shall", "she", "should", "shouldn", "some", "such", "than", "that", "the", "their", "theirs",
    "them", "themselves", "then", "there", "these", "they", "this", "those", "though", "through", "too",
    "under", "until", "upon", "very", "was", "wasn", "were", "weren", "what", "when", "where", "whether",
    "which", "while", "who", "whom", "whose", "why", "will", "with", "within", "without", "won", "would",
    "wouldn", "yet", "you", "your", "yours", "yourself",
    // Spanish
    "algo", "alguna", "algunas", "alguno", "algunos", "ante", "antes", "aquel", "aquella", "aquellas",
    "aquello", "aquellos", "aqui", "aquí", "asi", "así", "aun", "aún", "bajo", "cada", "como", "cómo",
    "con", "contra", "cual", "cuál", "cuales", "cuáles", "cuando", "cuándo", "cuanta", "cuánta", "cuantas",
    "cuántas", "cuanto", "cuánto", "cuantos", "cuántos", "del", "desde", "donde", "dónde", "durante",
    "ella", "ellas", "ello", "ellos", "entre", "era", "eran", "eres", "esa", "esas", "ese", "eso", "esos",
    "esta", "está", "estaba", "estaban", "estamos", "están", "estar", "estas", "estás", "este", "esto",
    "estos", "estoy", "fue", "fueron", "había", "habían", "han", "hacia", "hasta", "hay", "hemos",
    "hubo", "las", "les", "los", "mas", "más", "mis", "misma", "mismas", "mismo", "mismos", "mucha",
    "muchas", "mucho", "muchos", "muy", "nada", "ningun", "ningún", "ninguna", "ninguno", "nos", "nosotras",
    "nosotros", "nuestra", "nuestras", "nuestro", "nuestros", "otra", "otras", "otro", "otros", "para",
    "pero", "poca", "pocas", "poco", "pocos", "por", "porque", "pues", "que", "qué", "quien", "quién",
    "quienes", "quiénes", "sea", "sean", "segun", "según", "ser", "siendo", "sido", "sin", "sino", "sobre",
    "sois", "solo", "sólo", "somos", "son", "soy", "sus", "suya", "suyas", "suyo", "suyos", "tal",
    "tambien", "también", "tampoco", "tan", "tanto", "toda", "todas", "todavia", "todavía", "todo",
    "todos", "tras", "tus", "tuya", "tuyo", "una", "unas", "uno", "unos", "usted", "ustedes", "vos",
    "vosotras", "vosotros",
];

/// Fewest characters a word of a docs query needs to be searched by term,
/// unless it is written as an identifier (`e5`, `db_id`).
const MIN_WORD_CHARS: usize = 3;

/// The FTS5 MATCH expression of a docs search: the tokens of `query` that are
/// identifiers, or neither stopwords nor shorter than [`MIN_WORD_CHARS`], at
/// most [`MAX_LEXICAL_TOKENS`] of them, quoted and OR-joined as
/// [`fts_match_expression`] joins them. A query of nothing but such words
/// falls back to that expression rather than to no term search at all.
pub(super) fn docs_match_expression(query: &str) -> Option<String> {
    let tokens: Vec<&str> = fts_tokens(query)
        .filter(|t| is_identifier(t) || (t.chars().count() >= MIN_WORD_CHARS && !is_stopword(t)))
        .take(MAX_LEXICAL_TOKENS)
        .collect();

    if tokens.is_empty() {
        return fts_match_expression(query, MAX_LEXICAL_TOKENS);
    }

    Some(tokens.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(" OR "))
}

fn is_stopword(token: &str) -> bool {
    DOCS_STOPWORDS.contains(&token.to_lowercase().as_str())
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
    fn docs_match_expression_drops_stopwords_and_short_words() {
        let cases: &[(&str, &[&str], &[&str])] = &[
            (
                "¿qué hace el daemon con los saves pendientes cuando se apaga de golpe?",
                &["pendientes", "daemon", "saves"],
                &["qué", "el", "con", "los", "de", "se"],
            ),
            ("where is CHUNK_MAX_TOKENS used and what is db_id", &["CHUNK_MAX_TOKENS", "db_id"], &["is", "and", "what", "where"]),
            ("Which Port does THE daemon use", &["Port", "daemon", "use"], &["Which", "does", "THE"]),
            ("qué es e5 y el id del tope", &["e5", "tope"], &["qué", "es", "y", "el", "id", "del"]),
        ];
        for (query, kept, dropped) in cases {
            let terms = terms(query);
            for t in *kept {
                assert!(terms.iter().any(|x| x == t), "{query}: {t} missing from {terms:?}");
            }
            for t in *dropped {
                assert!(!terms.iter().any(|x| x == t), "{query}: {t} kept in {terms:?}");
            }
        }
    }

    #[test]
    fn a_query_of_only_stopwords_falls_back_to_the_memory_expression() {
        for query in ["what is the", "de la", "ok"] {
            assert_eq!(docs_match_expression(query), fts_match_expression(query, MAX_LEXICAL_TOKENS), "{query}");
        }
        assert_eq!(docs_match_expression("¿?"), None);
    }

    #[test]
    fn the_cap_counts_the_words_kept() {
        let query = (0..40).map(|i| format!("de palabra{i}")).collect::<Vec<_>>().join(" ");

        let terms = terms(&query);

        assert_eq!(terms.len(), MAX_LEXICAL_TOKENS);
        assert_eq!(terms.first().map(String::as_str), Some("palabra0"));
        assert_eq!(terms.last().map(String::as_str), Some("palabra15"));
    }
}
