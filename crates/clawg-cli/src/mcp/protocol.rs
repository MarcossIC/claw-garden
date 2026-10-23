//! What the agent is told about memory: `protocol.md`, compiled in, and the
//! two sentences on language that a preference rewrites.
//!
//! The opening is what the host is told this server is for, at `initialize`
//! or `server/discover`; the whole text is what `clawg setup` writes into the
//! user's `CLAUDE.md`. Without a preference both are the file as it is.

use std::borrow::Cow;

use clawg_core::preferences::Lang;

/// The protocol as written, for a user without a language preference.
pub const PROTOCOL: &str = include_str!("protocol.md");

/// The opening's rule on language, as `protocol.md` words it. Matched across
/// the file's line breaks ([`find_spaced`]); a test keeps it occurring once.
const OPENING_RULE: &str = "Write memories and search queries in English, whatever language the conversation is in: \
                            a memory is found best by a query in its own language.";

/// The same rule in "How to write a save".
const SAVE_RULE: &str = "Write it in English, and search in English too. A memory saved in one language and \
                         searched in another is often missed.";

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

/// The protocol for a user whose preferred language is `language`: the file
/// itself without one, and with one the file where its two rules ask to save,
/// search and answer in that language. The new sentence of the opening keeps
/// it within the 2048 characters older clients read.
pub fn text(language: Option<Lang>) -> Cow<'static, str> {
    let Some(lang) = language else { return Cow::Borrowed(PROTOCOL) };
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
    fn a_language_rewrites_both_rules() {
        let es = text(Some(Lang::Es));
        assert!(!es.contains("whatever language the conversation is in"), "{es}");
        assert!(es.contains("Save memories and search in Spanish: a memory is found best by a query in its own language."));
        assert!(es.contains("Reply to the user in Spanish unless they write in another language."));
        assert!(es.contains("Write it in Spanish, and search in Spanish too."));
        assert!(es.contains("when a search says memories are still in English, search again with the query in English."));
        assert!(!opening(&es).contains("in English"), "{}", opening(&es));

        let en = text(Some(Lang::En));
        assert!(en.contains("Save memories and search in English:") && en.contains("Reply to the user in English unless"));
        assert!(en.contains("still in Spanish, search again with the query in Spanish."));
        assert!(en.len() < PROTOCOL.len() + 200, "only the two rules change");
    }
}
