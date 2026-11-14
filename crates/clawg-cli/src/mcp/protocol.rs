//! What the agent is told about memory: `protocol.md` and `protocol.es.md`,
//! compiled in, and the two sentences on language that a preference for
//! English rewrites.
//!
//! The opening is what the host is told this server is for, at `initialize`
//! or `server/discover`; the whole text is what `clawg setup` writes into the
//! user's `CLAUDE.md`. Without a preference both are the English file as it
//! is.

use std::borrow::Cow;

use clawg_core::preferences::Lang;

/// The protocol as written, for a user without a language preference.
pub const PROTOCOL: &str = include_str!("protocol.md");

/// The protocol written in Spanish, for a user who prefers it: written for
/// Spanish rather than translated sentence by sentence, so its opening fits
/// the same 2048 characters.
pub const PROTOCOL_ES: &str = include_str!("protocol.es.md");

/// The opening's rule on language, as `protocol.md` words it. Matched across
/// the file's line breaks ([`find_spaced`]); a test keeps it occurring once.
const OPENING_RULE: &str = "Write memories and search queries in English, whatever language the conversation is in: \
                            a memory is found best by a query in its own language.";

/// The same rule in "How to write a save".
const SAVE_RULE: &str = "Write it in English, and search in English too. A memory saved in one language and \
                         searched in another is often missed.";

/// How much of the protocol `clawg setup` writes into the user's `CLAUDE.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Full,
    Short,
    Off,
}

impl Variant {
    pub const ALL: [Variant; 3] = [Variant::Full, Variant::Short, Variant::Off];

    pub fn as_str(self) -> &'static str {
        match self {
            Variant::Full => "full",
            Variant::Short => "short",
            Variant::Off => "off",
        }
    }

    pub fn parse(name: &str) -> Option<Variant> {
        Variant::ALL.into_iter().find(|v| v.as_str() == name)
    }
}

/// The paragraphs between the title and the first section, each on one line:
/// the file is wrapped for reading, and a host shows instructions as prose.
pub fn opening(protocol: &str) -> String {
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in protocol.lines().skip(1).take_while(|line| !line.starts_with("## ")) {
        let line = line.trim();
        if line.is_empty() {
            if !current.is_empty() {
                paragraphs.push(current.join(" "));
                current.clear();
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        paragraphs.push(current.join(" "));
    }
    paragraphs.join("\n\n")
}

/// The protocol for a user whose preferred language is `language`: the
/// English file without one, the Spanish file for Spanish, and for English
/// the file where its two rules ask to save, search and answer in English.
/// The new sentence of the opening keeps it within the 2048 characters older
/// clients read.
pub fn text(language: Option<Lang>) -> Cow<'static, str> {
    let lang = match language {
        None => return Cow::Borrowed(PROTOCOL),
        Some(Lang::Es) => return Cow::Borrowed(PROTOCOL_ES),
        Some(lang @ Lang::En) => lang,
    };
    let (name, other) = (lang.name(), lang.other().name());
    let opening_rule = format!(
        "Save memories and search in {name}: a memory is found best by a query in its own language. \
         Reply to the user in {name} unless they write in another language."
    );
    let save_rule = format!(
        "Write it in {name}, and search in {name} too. A memory saved in one language and searched in another \
         is often missed; when a search says memories are still in {other}, search again with the query in {other}."
    );
    let text = replace_spaced(PROTOCOL, OPENING_RULE, &opening_rule);
    Cow::Owned(replace_spaced(&text, SAVE_RULE, &save_rule))
}

/// What the short block says of the rest, in place of the rules the server's
/// instructions and the tools' descriptions already carry.
const SHORT_LEAD: &str = "The rules for clawg's tools come with its server's instructions; this block keeps only what they leave out.";

const SHORT_LEAD_ES: &str = "Las reglas de las herramientas de clawg vienen con las instrucciones de su servidor; este bloque guarda sólo lo que les falta.";

/// The sections of the protocol that neither the opening nor the tools'
/// descriptions say: the shape of a `session_summary`, and what to do after a
/// compaction.
const SHORT_SECTIONS: [&str; 2] = ["Session summary", "After a compaction"];

const SHORT_SECTIONS_ES: [&str; 2] = ["Resumen de sesión", "Después de una compactación"];

/// The short protocol for `language`: the title of the protocol, a line on
/// where its rules are, and the two sections of [`SHORT_SECTIONS`] as the
/// protocol has them, with `\n` line endings. The file in English for any
/// language but Spanish.
pub fn short(language: Option<Lang>) -> String {
    let (protocol, lead, sections) = match language {
        Some(Lang::Es) => (PROTOCOL_ES, SHORT_LEAD_ES, SHORT_SECTIONS_ES),
        _ => (PROTOCOL, SHORT_LEAD, SHORT_SECTIONS),
    };
    let protocol = protocol.replace("\r\n", "\n");
    let mut parts = vec![protocol.lines().next().unwrap_or_default().to_string(), lead.to_string()];
    parts.extend(sections.iter().filter_map(|heading| section(&protocol, heading)));
    parts.join("\n\n")
}

/// The `## {heading}` section of `protocol` (`\n` line endings), heading
/// included, up to the next section.
fn section(protocol: &str, heading: &str) -> Option<String> {
    let start = protocol.find(&format!("\n## {heading}\n"))? + 1;
    let rest = &protocol[start..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    Some(rest[..end].trim_end().to_string())
}

/// `text` with every match of `anchor` ([`find_spaced`]) replaced.
fn replace_spaced(text: &str, anchor: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(text.len() + replacement.len());
    let mut last = 0;
    for (start, end) in find_spaced(text, anchor) {
        out.push_str(&text[last..start]);
        out.push_str(replacement);
        last = end;
    }
    out.push_str(&text[last..]);
    out
}

/// Where `anchor` occurs in `text` as byte ranges, each single space of the
/// anchor matching any run of whitespace, line breaks and indentation
/// included, so a sentence is found however the file wraps it.
fn find_spaced(text: &str, anchor: &str) -> Vec<(usize, usize)> {
    let words: Vec<&str> = anchor.split(' ').collect();
    if words.iter().any(|w| w.is_empty()) {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(offset) = text[from..].find(words[0]) {
        let start = from + offset;
        let mut at = start + words[0].len();
        let matched = words[1..].iter().all(|word| {
            let rest = &text[at..];
            let gap = rest.len() - rest.trim_start().len();
            if gap == 0 || !rest[gap..].starts_with(word) {
                return false;
            }
            at += gap + word.len();
            true
        });
        if matched {
            found.push((start, at));
            from = at;
        } else {
            from = start + words[0].len();
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_joins_the_lines_of_each_paragraph() {
        assert_eq!(opening("# T\n\nuno\ndos\n\ntres\n## S\ncuatro"), "uno dos\n\ntres");
        assert_eq!(opening("# T\r\n\r\nuno\r\n  dos\r\n\r\n\r\ntres\r\n## S\r\n"), "uno dos\n\ntres");
        assert_eq!(opening("# T\nsolo"), "solo");
    }

    #[test]
    fn a_spaced_match_spans_line_breaks_and_indentation() {
        let text = "one two\n  three four. Five";
        assert_eq!(find_spaced(text, "two three four."), vec![(4, 21)]);
        assert_eq!(find_spaced(text, "two three"), vec![(4, 15)]);
        assert!(find_spaced(text, "twothree").is_empty());
        assert!(find_spaced(text, "one  two").is_empty(), "an anchor spells one space between words");
        assert_eq!(find_spaced("a b a\nb", "a b").len(), 2);
    }

    #[test]
    fn the_language_rules_occur_once_in_the_protocol() {
        assert_eq!(find_spaced(PROTOCOL, OPENING_RULE).len(), 1, "{OPENING_RULE}");
        assert_eq!(find_spaced(PROTOCOL, SAVE_RULE).len(), 1, "{SAVE_RULE}");
        assert!(opening(PROTOCOL).contains(OPENING_RULE), "the first rule is part of the opening");
    }

    #[test]
    fn without_a_language_the_protocol_is_the_file() {
        assert_eq!(text(None), PROTOCOL);
    }

    #[test]
    fn spanish_is_the_spanish_protocol() {
        let es = text(Some(Lang::Es));
        assert_eq!(es, PROTOCOL_ES);
        let opening = opening(&es);
        assert!(opening.contains("Guarda las memorias y busca en español"), "{opening}");
        assert!(opening.contains("Responde a la persona en español salvo que escriba en otro idioma."), "{opening}");
        assert!(es.contains("Escríbela en español, y busca en español también."), "{es}");
        assert!(es.contains("todavía quedan memorias en inglés, busca de nuevo con la consulta en inglés."), "{es}");
    }

    /// Spans with a space and no `<` are examples of a saved text, written in
    /// each language; every other span is a name or a marker.
    fn names_and_markers(text: &str) -> std::collections::BTreeSet<&str> {
        text.split('`').skip(1).step_by(2).filter(|s| !s.contains(' ') || s.contains('<')).collect()
    }

    fn sections(text: &str) -> usize {
        text.lines().filter(|l| l.starts_with("## ")).count()
    }

    #[test]
    fn the_spanish_protocol_has_the_sections_and_spans_of_the_english_one() {
        assert_eq!(sections(PROTOCOL_ES), sections(PROTOCOL));
        assert_eq!(names_and_markers(PROTOCOL_ES), names_and_markers(PROTOCOL));
        for marker in ["ACTIVE CONFLICTS", "LAST SESSION", "supersedes", "compatible", "scoped", "conflicts", "inferred"] {
            assert_eq!(PROTOCOL_ES.contains(marker), PROTOCOL.contains(marker), "{marker}");
        }
    }

    /// The text of the `## {heading}` section of `text`, up to the next one,
    /// with `\n` line breaks: a checkout on Windows can turn them into `\r\n`.
    fn section(text: &str, heading: &str) -> String {
        let text = text.replace("\r\n", "\n");
        let start = text.find(&format!("\n## {heading}\n")).unwrap_or_else(|| panic!("no section {heading}"));
        let body = &text[start + heading.len() + 5..];
        body.find("\n## ").map_or(body, |end| &body[..end]).to_string()
    }

    #[test]
    fn the_protocol_says_which_preferences_open_every_session() {
        for (text, heading) in [(PROTOCOL, "Type and importance"), (PROTOCOL_ES, "Tipo e importancia")] {
            let types = section(text, heading);
            for name in ["`user_stated`", "`all_projects`", "inferred"] {
                assert!(types.contains(name), "{heading} names {name}:\n{types}");
            }
        }
    }

    #[test]
    fn the_protocol_says_where_a_call_without_a_project_goes() {
        for (text, heading) in [(PROTOCOL, "Projects"), (PROTOCOL_ES, "Proyectos")] {
            let projects = section(text, heading);
            for name in [
                "`project_path`",
                "`project`",
                "`all_projects`",
                "`project=`",
                "`recall_context`",
                "`search_project`",
                "`synthesize`",
                "`answer_suggestion`",
            ] {
                assert!(projects.contains(name), "{heading} names {name}:\n{projects}");
            }
        }
        for text in [PROTOCOL, PROTOCOL_ES] {
            let opening = opening(text);
            for name in ["`project_path`", "`project`", "`all_projects`"] {
                assert!(opening.contains(name), "the opening names {name} for a save's project:\n{opening}");
            }
        }
    }

    #[test]
    fn the_short_protocol_keeps_the_session_summary_in_both_languages() {
        let en = short(None);
        assert_eq!(short(Some(Lang::En)), en, "the English file, whichever way it is asked for");
        assert!(en.starts_with(
            "# Claw Garden memory protocol\n\nThe rules for clawg's tools come with its server's instructions; \
             this block keeps only what they leave out.\n\n## Session summary\n"
        ));
        for kept in ["- Goal:", "- Instructions:", "- Discoveries:", "- Accomplished:", "- Next steps:", "- Relevant files:", "- Validation:"] {
            assert!(en.contains(kept), "{kept} in:\n{en}");
        }
        assert!(en.contains("\n## After a compaction\n\nSave the summary of what was compacted"), "{en}");
        assert!(!en.contains("## How to write a save") && !en.contains("## Conclusions"), "{en}");

        let es = short(Some(Lang::Es));
        assert!(es.starts_with(
            "# Protocolo de memoria de Claw Garden\n\nLas reglas de las herramientas de clawg vienen con las \
             instrucciones de su servidor; este bloque guarda sólo lo que les falta.\n\n## Resumen de sesión\n"
        ));
        for kept in ["- Objetivo:", "- Instrucciones:", "- Descubrimientos:", "- Logrado:", "- Próximos pasos:", "- Archivos relevantes:", "- Validación:"] {
            assert!(es.contains(kept), "{kept} in:\n{es}");
        }
        assert!(es.contains("\n## Después de una compactación\n\nGuarda el resumen de lo compactado"), "{es}");
        assert!(!es.contains("## Cómo escribir un save") && !es.contains("## Conclusiones"), "{es}");

        for text in [&en, &es] {
            assert!(text.len() < 2000, "{} bytes", text.len());
            assert!(!text.contains('\r'));
        }
    }

    #[test]
    fn the_short_protocol_has_no_opening() {
        for language in [None, Some(Lang::En), Some(Lang::Es)] {
            let short = short(language);
            for paragraph in opening(&text(language)).split("\n\n") {
                assert!(!short.contains(paragraph), "{language:?} repeats: {paragraph}");
            }
            assert!(!short.contains("`search_memory`"), "{language:?}:\n{short}");
        }
    }

    #[test]
    fn a_variant_is_named_by_its_option() {
        for variant in Variant::ALL {
            assert_eq!(Variant::parse(variant.as_str()), Some(variant));
        }
        assert_eq!(Variant::parse("tiny"), None);
        assert_eq!(Variant::parse("Full"), None);
    }

    #[test]
    fn a_language_rewrites_both_rules_of_the_english_protocol() {
        let en = text(Some(Lang::En));
        assert!(en.contains("Save memories and search in English:") && en.contains("Reply to the user in English unless"));
        assert!(en.contains("still in Spanish, search again with the query in Spanish."));
        assert!(en.len() < PROTOCOL.len() + 200, "only the two rules change");
    }
}
