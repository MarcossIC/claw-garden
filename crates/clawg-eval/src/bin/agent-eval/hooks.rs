//! The hooks the harness adds to a session, which `agent-eval` itself runs:
//! one prints the rules an arm delivers at startup, the other logs each
//! instruction file the client loads. Pure: the commands do the I/O.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// An instruction file the client reported loading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loaded {
    pub file_path: String,
    /// Why it loaded, as the client names it: `session_start` or
    /// `path_glob_match`, among others; empty when the entry gives none.
    #[serde(default)]
    pub load_reason: String,
}

/// The files a log of `hook-log` names, in order: each line whose input has
/// a `file_path`; any other line is skipped.
pub fn read_log(text: &str) -> Vec<Loaded> {
    text.lines().filter_map(|line| serde_json::from_str::<Loaded>(line).ok()).collect()
}

/// What `agent-eval hook-context` prints: `text` as the context a
/// `SessionStart` hook adds, the channel clawg's own startup hook uses.
pub fn hook_output(text: &str) -> String {
    json!({ "hookSpecificOutput": { "hookEventName": "SessionStart", "additionalContext": text } }).to_string()
}

/// The line `agent-eval hook-log` appends for the input a hook got: the JSON
/// on one line, or an input that is not JSON kept as text.
pub fn log_line(input: &str) -> String {
    let value = serde_json::from_str::<Value>(input).unwrap_or_else(|_| json!({ "unparsed": input }));
    format!("{value}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hook_output_is_the_additional_context_of_session_start() {
        let out: Value = serde_json::from_str(&hook_output("Rules\n- [3 days ago] Use \"quotes\"\n")).unwrap();
        assert_eq!(
            out,
            json!({ "hookSpecificOutput": { "hookEventName": "SessionStart", "additionalContext": "Rules\n- [3 days ago] Use \"quotes\"\n" } })
        );
    }

    #[test]
    fn a_log_line_keeps_the_input_on_one_line() {
        let input = "{\n  \"hook_event_name\": \"InstructionsLoaded\",\n  \"file_path\": \"C:\\\\w\\\\.claude\\\\rules\\\\clawg\\\\rules-1.md\"\n}\n";
        let line = log_line(input);
        assert!(line.ends_with('\n') && line.matches('\n').count() == 1, "{line:?}");
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), serde_json::from_str::<Value>(input).unwrap());

        let line = log_line("not json\nat all");
        assert_eq!(line.matches('\n').count(), 1, "{line:?}");
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), json!({ "unparsed": "not json\nat all" }));
    }

    #[test]
    fn the_log_reads_each_loaded_file_and_skips_what_is_not_an_entry() {
        let log = [
            log_line(r#"{"hook_event_name": "InstructionsLoaded", "file_path": "/w/.claude/rules/clawg/rules-1.md", "load_reason": "path_glob_match", "globs": ["**/*.md"]}"#),
            log_line("not json"),
            "{broken\n".to_owned(),
            log_line(r#"{"file_path": "/c/CLAUDE.md"}"#),
        ]
        .concat();
        assert_eq!(
            read_log(&log),
            [
                Loaded { file_path: "/w/.claude/rules/clawg/rules-1.md".into(), load_reason: "path_glob_match".into() },
                Loaded { file_path: "/c/CLAUDE.md".into(), load_reason: String::new() },
            ]
        );
        assert!(read_log("").is_empty());
    }
}
