//! Which language a memory is written in, English or Spanish, so that while a
//! store holds both, clawg can tell the agent how many memories a query in
//! the preferred language is likely to miss.
//!
//! The detector counts function words and the marks only Spanish has. Code,
//! paths and identifiers are left out first: in this kind of store they are
//! English whatever the prose around them is. Two languages need no model or
//! dependency; with too little evidence the answer is undecided. The detector
//! is pure; [`count_saves_in`] reads the column it fills.

use crate::memory::search::{memory_project_predicate, reader_bind, reader_predicate};
use crate::memory::sharing::Reader;
use crate::memory::{MemoryError, MemoryStore};
use crate::preferences::Lang;

/// Live saves in language `?1` of the scope a search of project `?2` (NULL
/// for every project) reads, within reach `?3` as [`reader_bind`] binds it. A
/// long save counts once, under the lowest save that holds the row; a row of
/// no save counts as its own, by a negative key no save id takes.
const COUNT_SAVES_SQL: &str = concat!(
    "SELECT COUNT(DISTINCT COALESCE(",
    "(SELECT MIN(p.save_id) FROM memory_piece p WHERE p.memory_id = m.id), -m.id)) ",
    "FROM memory m ",
    "WHERE m.lang = ?1 AND m.deleted_at IS NULL AND m.status IN ('active', 'disputed') AND ",
    memory_project_predicate!("m.", "?2"),
    " AND ",
    reader_predicate!("m.agent_id", "?3"),
);

/// How many live saves in `lang` a search of `project_id` by `reader` could
/// return: the project's, the ones without a project and every
/// `user_preference`, or all of them without a project. Rows not classified
/// yet, or undecided, count for no language.
pub async fn count_saves_in(
    store: &MemoryStore,
    lang: Lang,
    project_id: Option<&str>,
    reader: &Reader,
) -> Result<usize, MemoryError> {
    let reach = reader_bind(Some(reader));
    let project_id = project_id.map(str::to_owned);
    store
        .conn
        .call(move |c| {
            c.prepare_cached(COUNT_SAVES_SQL)?
                .query_row(rusqlite::params![lang.as_str(), project_id, reach], |r| r.get::<_, i64>(0))
                .map(|n| n as usize)
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
            other => MemoryError::Backend(format!("language count: {other}")),
        })
}

/// Fewest points of evidence, words and marks of both languages together,
/// before a text is given a language.
const MIN_EVIDENCE: usize = 3;

/// Words that, alone, say a text is English. Words that are also Spanish
/// (`a`, `no`, `me`, `he`, `has`) are left out.
const ENGLISH: &[&str] = &[
    "the", "of", "and", "to", "in", "is", "are", "was", "were", "be", "been", "being", "it", "its", "that",
    "this", "these", "those", "with", "for", "on", "at", "by", "from", "as", "an", "or", "but", "not",
    "which", "what", "when", "where", "who", "how", "have", "had", "does", "do", "did", "their", "they",
    "them", "there", "than", "then", "so", "if", "because", "into", "only", "also", "would", "should",
    "could", "can", "will", "after", "before", "about", "every", "each", "our", "we", "you", "your",
    "she", "his", "her",
];

/// Words that, alone, say a text is Spanish. `era`, also English, is left out.
const SPANISH: &[&str] = &[
    "el", "la", "los", "las", "de", "del", "que", "y", "en", "un", "una", "unos", "unas", "es", "son",
    "fue", "ser", "está", "están", "por", "para", "con", "sin", "se", "su", "sus", "lo", "al", "como",
    "pero", "más", "este", "esta", "esto", "estos", "estas", "ese", "esa", "eso", "cuando", "porque",
    "también", "hay", "muy", "ya", "entre", "sobre", "desde", "hasta", "donde", "cada", "sólo", "solo",
    "otro", "otra", "sino", "aunque", "nos", "le", "les", "mismo", "misma", "qué", "cómo",
];

/// The language of `text`'s prose, or `None` when there is too little of it
/// or neither language clearly wins.
pub fn detect(text: &str) -> Option<Lang> {
    let prose = without_code(text);
    let (mut english, mut spanish) = (0usize, 0usize);
    spanish += prose.chars().filter(|c| matches!(c, '¿' | '¡')).count();
    for token in prose.split_whitespace().filter(|t| !is_identifier(t)) {
        let word = token.trim_matches(|c: char| !c.is_alphabetic()).to_lowercase();
        if word.is_empty() {
            continue;
        }
        if ENGLISH.contains(&word.as_str()) {
            english += 1;
        } else if SPANISH.contains(&word.as_str()) || word.chars().any(is_spanish_letter) {
            spanish += 1;
        }
    }
    if english + spanish < MIN_EVIDENCE {
        return None;
    }
    if spanish >= 2 * english {
        Some(Lang::Es)
    } else if english >= 2 * spanish {
        Some(Lang::En)
    } else {
        None
    }
}

/// How `memory.lang` spells a detected language: `und` for undecided, so a
/// row that was looked at reads apart from one not yet classified (NULL).
pub fn column_value(lang: Option<Lang>) -> &'static str {
    lang.map_or("und", Lang::as_str)
}

fn is_spanish_letter(c: char) -> bool {
    matches!(c, 'ñ' | 'á' | 'é' | 'í' | 'ó' | 'ú' | 'ü')
}

/// A path, a URL, a name with digits, `_`, `::`, an inner dot or an inner
/// capital: what code names things with, not what prose says.
fn is_identifier(token: &str) -> bool {
    let word = token.trim_matches(|c: char| !c.is_alphanumeric());
    word.contains(['/', '\\', '_'])
        || word.contains("::")
        || word.chars().any(|c| c.is_ascii_digit())
        || word.contains('.')
        || word.chars().skip(1).any(char::is_uppercase)
}

/// `text` without its fenced blocks and backtick spans. An unclosed fence or
/// span is taken to run to the end of the text.
fn without_code(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, block) in text.split("```").enumerate() {
        if i % 2 == 1 {
            out.push(' ');
            continue;
        }
        for (j, span) in block.split('`').enumerate() {
            out.push_str(if j % 2 == 0 { span } else { " " });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::sharing::Reach;

    #[test]
    fn spanish_prose_is_spanish() {
        let text = "El daemon guarda cada memoria en una sola transacción, y la búsqueda las encuentra por su significado.";
        assert_eq!(detect(text), Some(Lang::Es));
        assert_eq!(detect("¿Por qué la búsqueda no encuentra lo que se guardó ayer?"), Some(Lang::Es));
    }

    #[test]
    fn english_prose_is_english() {
        let text = "The daemon stores every memory in one transaction, and the search finds them by their meaning.";
        assert_eq!(detect(text), Some(Lang::En));
        assert_eq!(detect("Why does the search not find what was saved yesterday?"), Some(Lang::En));
    }

    #[test]
    fn code_paths_and_identifiers_are_undecided() {
        for text in [
            "```rust\nfn main() { let the = 1; let de = 2; }\n```",
            "`MemoryStore::open`",
            "crates/clawg-core/src/memory/db.rs",
            "content_hash topic_key memory_type",
            "https://example.com/the/path/of/the/page",
            "",
        ] {
            assert_eq!(detect(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_short_text_is_undecided() {
        for text in ["the end", "por fin", "Fixed lexical bonus ignoring paths", "Se rompió"] {
            assert_eq!(detect(text), None, "{text:?}");
        }
    }

    #[test]
    fn identifiers_in_spanish_prose_do_not_make_it_english() {
        let text = "Arreglé el cálculo de `content_hash` en MemoryStore::open, que está en \
                    crates/clawg-core/src/memory/db.rs, porque ignoraba las rutas del proyecto.";
        assert_eq!(detect(text), Some(Lang::Es));
        let text = "La migración agrega la columna lang y el índice parcial; el relleno corre en el \
                    mantenimiento sin tocar el contenido.";
        assert_eq!(detect(text), Some(Lang::Es));
    }

    #[test]
    fn a_mixed_text_follows_its_prose() {
        let text = "The user asked for this in their own words, «quiero todo en español», and the agent \
                    saved it as a preference for every project they work on.";
        assert_eq!(detect(text), Some(Lang::En));
        let text = "El usuario pidió que the search finds them, y que se guarde para todos los proyectos \
                    en los que trabaja con el agente.";
        assert_eq!(detect(text), Some(Lang::Es));
    }

    struct Seed {
        id: i64,
        agent: Option<&'static str>,
        project: Option<&'static str>,
        lang: Option<&'static str>,
        status: &'static str,
        deleted: bool,
        memory_type: &'static str,
    }

    const fn row(id: i64, agent: Option<&'static str>, project: Option<&'static str>, lang: Option<&'static str>) -> Seed {
        Seed { id, agent, project, lang, status: "active", deleted: false, memory_type: "semantic_fact" }
    }

    async fn seeded(rows: Vec<Seed>, pieces: &'static [(i64, i64, i64)]) -> MemoryStore {
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .conn
            .call(move |c| {
                for s in &rows {
                    c.execute(
                        "INSERT INTO memory (id, scope, agent_id, project_id, content, memory_type, source, created_at, \
                         created_by_type, created_by_ref, last_modified_at, last_accessed_at, importance, tier, status, \
                         deleted_at, lang) \
                         VALUES (?1, CASE WHEN ?2 IS NOT NULL THEN 'agent' WHEN ?3 IS NULL THEN 'global' ELSE 'project' END, \
                         ?2, ?3, 'row ' || ?1, ?4, 'explicit', ?1, 'agent', 'x', ?1, ?1, 5.0, 3, ?5, ?6, ?7)",
                        rusqlite::params![s.id, s.agent, s.project, s.memory_type, s.status, s.deleted.then_some(9), s.lang],
                    )?;
                }
                for (save, piece, memory) in pieces {
                    c.execute(
                        "INSERT INTO memory_piece (save_id, piece, memory_id) VALUES (?1, ?2, ?3)",
                        rusqlite::params![save, piece, memory],
                    )?;
                }
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .unwrap();
        store
    }

    #[tokio::test]
    async fn counting_the_other_language_follows_the_search_scope() {
        let rows = vec![
            row(1, Some("r"), Some("pa"), Some("en")),
            row(2, Some("r"), Some("pa"), Some("en")),
            row(3, Some("r"), Some("pa"), Some("en")),
            row(4, Some("r"), Some("pa"), Some("en")),
            Seed { deleted: true, ..row(5, Some("r"), Some("pa"), Some("en")) },
            Seed { status: "superseded", ..row(6, Some("r"), Some("pa"), Some("en")) },
            row(7, Some("hidden"), Some("pa"), Some("en")),
            row(8, Some("a"), Some("pb"), Some("en")),
            Seed { memory_type: "user_preference", ..row(9, Some("a"), Some("pb"), Some("en")) },
            row(10, Some("a"), Some("pa"), Some("es")),
            row(11, None, Some("pa"), Some("und")),
            row(12, None, Some("pa"), None),
            row(13, None, None, Some("en")),
        ];
        let store = seeded(rows, &[(2, 1, 2), (2, 2, 3), (2, 3, 4)]).await;
        let reader = Reader::new("r", Reach::AllBut(vec!["hidden".into()]));

        let count = |lang, project| count_saves_in(&store, lang, project, &reader);
        assert_eq!(count(Lang::En, Some("pa")).await.unwrap(), 4, "rows 1 and 13, the save of 2-4, the preference 9");
        assert_eq!(count(Lang::En, None).await.unwrap(), 5, "row 8 of another project too");
        assert_eq!(count(Lang::Es, Some("pa")).await.unwrap(), 1);

        let own = Reader::new("r", Reach::Own);
        assert_eq!(count_saves_in(&store, Lang::En, Some("pa"), &own).await.unwrap(), 3, "rows 1 and 13, the save of 2-4");
    }

    #[tokio::test]
    async fn counting_by_language_uses_its_index() {
        let store = seeded(vec![row(1, Some("r"), Some("pa"), Some("en"))], &[]).await;
        let reader = crate::memory::search::reader_bind(Some(&Reader::new("r", Reach::AllBut(Vec::new()))));
        let plan: Vec<String> = store
            .conn
            .call(move |c| {
                let mut stmt = c.prepare(&format!("EXPLAIN QUERY PLAN {COUNT_SAVES_SQL}"))?;
                let rows = stmt.query_map(rusqlite::params!["en", "pa", reader], |r| r.get::<_, String>(3))?;
                rows.collect::<rusqlite::Result<_>>()
            })
            .await
            .unwrap();
        assert!(plan.iter().any(|d| d.contains("idx_memory_lang")), "{plan:?}");
        assert!(!plan.iter().any(|d| d.starts_with("SCAN m")), "{plan:?}");
    }

    #[test]
    fn the_column_spells_undecided_as_und() {
        assert_eq!(column_value(Some(Lang::Es)), "es");
        assert_eq!(column_value(Some(Lang::En)), "en");
        assert_eq!(column_value(None), "und");
    }
}
