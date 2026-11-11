//! The hooks the harness adds to a session, which `agent-eval` itself runs:
//! one prints the rules an arm delivers at startup, the other logs each
//! instruction file the client loads. Pure: the commands do the I/O.

use serde_json::{json, Value};

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
}
