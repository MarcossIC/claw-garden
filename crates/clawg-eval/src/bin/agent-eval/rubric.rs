//! The deterministic rubric of a save: each criterion is a rule of the memory
//! protocol (`crates/clawg-cli/src/mcp/protocol.md`) that can be checked
//! without a judge. What needs one, whether a save says why or whether it
//! repeats what the code already says, is left out.

use clawg_core::memory::language;
use clawg_core::preferences::Lang;
use serde::Serialize;
use serde_json::Value;

use crate::stream::{is_tool, ToolCall, Transcript};
use crate::task::MEMORY_TYPES;

/// The most a save holds.
const MAX_SAVE_BYTES: usize = 64 * 1024;

/// The longest first line that still reads as a subject.
const SUBJECT_MAX_CHARS: usize = 100;

const SUMMARY_SECTIONS_EN: [&str; 6] = ["Goal", "Instructions", "Discoveries", "Accomplished", "Next steps", "Relevant files"];
const SUMMARY_SECTIONS_ES: [&str; 6] =
    ["Objetivo", "Instrucciones", "Descubrimientos", "Logrado", "Próximos pasos", "Archivos relevantes"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    Pass,
    Fail,
    /// The rule does not apply to this save, or cannot be told.
    NotApplicable,
}

/// The criteria, in the order they are reported.
pub const CRITERIA: [&str; 12] = [
    "stored",
    "known_type",
    "importance_in_range",
    "fits_limit",
    "subject_line",
    "body_after_subject",
    "names_an_identifier",
    "profile_language",
    "key_for_revisable",
    "key_shape",
    "conclusion_not_user_stated",
    "summary_sections",
];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SaveMarks {
    pub content: String,
    pub marks: Vec<(&'static str, Mark)>,
}

impl SaveMarks {
    pub fn count(&self, mark: Mark) -> usize {
        self.marks.iter().filter(|(_, m)| *m == mark).count()
    }
}

/// Every `save_memory` call of the session, judged.
pub fn judge_session(t: &Transcript, lang: Option<Lang>) -> Vec<SaveMarks> {
    t.calls.iter().filter(|c| is_tool(&c.name, "save_memory")).map(|c| judge(c, lang)).collect()
}

pub fn judge(call: &ToolCall, lang: Option<Lang>) -> SaveMarks {
    let input = &call.input;
    let content = input.get("content").and_then(Value::as_str).unwrap_or("");
    let memory_type = input.get("memory_type").and_then(Value::as_str);
    let topic_key = input.get("topic_key").and_then(Value::as_str);
    let lang = lang.unwrap_or(Lang::En);
    let mut lines = content.lines().map(str::trim).filter(|l| !l.is_empty());
    let subject = lines.next().unwrap_or("");
    let has_body = lines.next().is_some();
    let pass = |ok: bool| if ok { Mark::Pass } else { Mark::Fail };

    let marks = vec![
        ("stored", pass(call.result.is_some() && !call.is_error)),
        ("known_type", memory_type.map_or(Mark::Pass, |t| pass(MEMORY_TYPES.contains(&t)))),
        (
            "importance_in_range",
            match input.get("importance") {
                None | Some(Value::Null) => Mark::Pass,
                Some(v) => pass(v.as_f64().is_some_and(|i| (0.0..=10.0).contains(&i))),
            },
        ),
        ("fits_limit", pass(!content.trim().is_empty() && content.len() <= MAX_SAVE_BYTES)),
        ("subject_line", pass(subject.chars().count() <= SUBJECT_MAX_CHARS && subject.split_whitespace().count() >= 2)),
        ("body_after_subject", pass(has_body)),
        ("names_an_identifier", pass(content.split_whitespace().any(is_identifier))),
        (
            "profile_language",
            match language::detect(content) {
                Some(found) => pass(found == lang),
                None => Mark::NotApplicable,
            },
        ),
        (
            "key_for_revisable",
            match memory_type {
                Some("architectural_decision" | "user_preference") => pass(topic_key.is_some_and(|k| !k.trim().is_empty())),
                _ => Mark::NotApplicable,
            },
        ),
        ("key_shape", topic_key.map_or(Mark::NotApplicable, |k| pass(is_key_shaped(k)))),
        (
            "conclusion_not_user_stated",
            match input.get("derived_from").and_then(Value::as_array) {
                Some(ids) if !ids.is_empty() => pass(input.get("user_stated").and_then(Value::as_bool) != Some(true)),
                _ => Mark::NotApplicable,
            },
        ),
        (
            "summary_sections",
            if memory_type == Some("session_summary") {
                let sections = if lang == Lang::Es { SUMMARY_SECTIONS_ES } else { SUMMARY_SECTIONS_EN };
                let lower = content.to_lowercase();
                pass(sections.iter().all(|s| lower.contains(&s.to_lowercase())))
            } else {
                Mark::NotApplicable
            },
        ),
    ];
    debug_assert!(marks.iter().map(|(c, _)| *c).eq(CRITERIA));
    SaveMarks { content: content.to_owned(), marks }
}

/// A word that names something in code: a path, a qualified name, a snake or
/// camel case identifier, a file name or anything in backticks.
fn is_identifier(word: &str) -> bool {
    if word.starts_with('`') {
        return true;
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
    let inner_dot = w.split('.').count() > 1 && w.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_alphanumeric() || c == '_'));
    let camel = w.chars().zip(w.chars().skip(1)).any(|(a, b)| a.is_lowercase() && b.is_uppercase());
    w.contains('/') || w.contains("::") || (w.contains('_') && w.chars().any(char::is_alphabetic)) || inner_dot || camel
}

/// A short name with no spaces, such as `storage/engine`.
fn is_key_shaped(key: &str) -> bool {
    !key.is_empty() && key.len() <= 64 && !key.chars().any(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(input: Value) -> ToolCall {
        ToolCall {
            id: "t".into(),
            name: "mcp__clawg__save_memory".into(),
            input,
            result: Some("saved #12".into()),
            ..Default::default()
        }
    }

    fn mark(marks: &SaveMarks, criterion: &str) -> Mark {
        marks.marks.iter().find(|(c, _)| *c == criterion).unwrap().1
    }

    const GOOD: &str = "Fixed staging port clash with the proxy\nThe staging deploy binds port 48213 because the proxy owns 8080; the value is set in deploy/staging.env and read by `load_settings`.";

    #[test]
    fn a_save_that_follows_the_protocol_passes_every_criterion_that_applies() {
        let marks = judge(&call(json!({ "content": GOOD, "memory_type": "semantic_fact" })), None);
        assert_eq!(marks.count(Mark::Fail), 0, "{:?}", marks.marks);
        assert_eq!(mark(&marks, "key_for_revisable"), Mark::NotApplicable);
        assert_eq!(mark(&marks, "summary_sections"), Mark::NotApplicable);
        assert_eq!(marks.marks.len(), CRITERIA.len());
    }

    #[test]
    fn a_one_line_save_without_an_identifier_fails_those_rules() {
        let marks = judge(&call(json!({ "content": "the port was changed because it clashed" })), None);
        assert_eq!(mark(&marks, "body_after_subject"), Mark::Fail);
        assert_eq!(mark(&marks, "names_an_identifier"), Mark::Fail);
        assert_eq!(mark(&marks, "subject_line"), Mark::Pass);
    }

    #[test]
    fn a_save_in_the_other_language_fails_the_profile_language() {
        let spanish = "Se corrigió el puerto de staging\nEl despliegue usa el puerto 48213 porque el proxy ocupa el 8080, en deploy/staging.env.";
        assert_eq!(mark(&judge(&call(json!({ "content": spanish })), None), "profile_language"), Mark::Fail);
        assert_eq!(mark(&judge(&call(json!({ "content": spanish })), Some(Lang::Es)), "profile_language"), Mark::Pass);
        assert_eq!(mark(&judge(&call(json!({ "content": "x_y" })), None), "profile_language"), Mark::NotApplicable);
    }

    #[test]
    fn a_decision_needs_a_key_and_a_key_needs_its_shape() {
        let no_key = judge(&call(json!({ "content": GOOD, "memory_type": "architectural_decision" })), None);
        assert_eq!(mark(&no_key, "key_for_revisable"), Mark::Fail);
        let keyed = judge(&call(json!({ "content": GOOD, "memory_type": "user_preference", "topic_key": "deploy/port" })), None);
        assert_eq!((mark(&keyed, "key_for_revisable"), mark(&keyed, "key_shape")), (Mark::Pass, Mark::Pass));
        let spaced = judge(&call(json!({ "content": GOOD, "topic_key": "deploy port" })), None);
        assert_eq!(mark(&spaced, "key_shape"), Mark::Fail);
    }

    #[test]
    fn type_importance_size_and_storage_are_checked() {
        let mut bad = call(json!({ "content": "x".repeat(MAX_SAVE_BYTES + 1), "memory_type": "fact", "importance": 12 }));
        bad.is_error = true;
        let marks = judge(&bad, None);
        for criterion in ["stored", "known_type", "importance_in_range", "fits_limit", "subject_line"] {
            assert_eq!(mark(&marks, criterion), Mark::Fail, "{criterion}");
        }
        let mut unanswered = call(json!({ "content": GOOD }));
        unanswered.result = None;
        assert_eq!(mark(&judge(&unanswered, None), "stored"), Mark::Fail);
    }

    #[test]
    fn a_conclusion_is_never_user_stated() {
        let both = judge(&call(json!({ "content": GOOD, "derived_from": [3, 4], "user_stated": true })), None);
        assert_eq!(mark(&both, "conclusion_not_user_stated"), Mark::Fail);
        let fine = judge(&call(json!({ "content": GOOD, "derived_from": [3] })), None);
        assert_eq!(mark(&fine, "conclusion_not_user_stated"), Mark::Pass);
    }

    #[test]
    fn a_summary_has_every_section_in_the_profile_language() {
        let summary = "Session on the staging port\nGoal: fix it.\nInstructions: none.\nDiscoveries: proxy owns 8080.\nAccomplished: moved to 48213.\nNext steps: deploy.\nRelevant files: deploy/staging.env";
        let marks = judge(&call(json!({ "content": summary, "memory_type": "session_summary" })), None);
        assert_eq!(mark(&marks, "summary_sections"), Mark::Pass);
        let short = judge(&call(json!({ "content": "Session\nGoal: fix it.", "memory_type": "session_summary" })), None);
        assert_eq!(mark(&short, "summary_sections"), Mark::Fail);
    }

    #[test]
    fn identifiers_are_told_from_prose() {
        for word in ["deploy/staging.env", "memory::search", "load_settings", "`x`", "main.rs,", "loadSettings"] {
            assert!(is_identifier(word), "{word}");
        }
        for word in ["port", "end.", "8080", "Port", "__"] {
            assert!(!is_identifier(word), "{word}");
        }
    }

    #[test]
    fn only_saves_are_judged() {
        let t = crate::stream::parse(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"a","name":"mcp__clawg__search_memory","input":{"query":"q"}},{"type":"tool_use","id":"b","name":"mcp__clawg__save_memory","input":{"content":"x"}}]}}"#,
        );
        let saves = judge_session(&t, None);
        assert_eq!(saves.len(), 1);
        assert_eq!(saves[0].content, "x");
    }
}
