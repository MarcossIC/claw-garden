//! Whether a session succeeded at its task, and where each tracer went. Pure:
//! the working directory is read through the function the caller passes.

use serde::Serialize;

use crate::stream::{is_tool, Transcript};
use crate::task::{contains_ci, Check, Task};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TracerFate {
    /// The agent wrote it: in the answer, in a tool's input or in its text.
    Used,
    /// A tool result or a hook brought it, and the agent did not write it.
    RetrievedButIgnored,
    /// Nothing in the session shows it.
    NeverReached,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckResult {
    pub check: String,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Score {
    /// Every check passed and the session ended without an error.
    pub success: bool,
    pub ended: bool,
    pub checks: Vec<CheckResult>,
    pub tracers: Vec<(String, TracerFate)>,
}

pub fn score(task: &Task, t: &Transcript, read_file: impl Fn(&str) -> Option<String>) -> Score {
    let checks: Vec<CheckResult> = task
        .checks
        .iter()
        .map(|check| {
            let passed = match check {
                Check::AnswerContains { text } => contains_ci(t.answer(), text),
                Check::AnswerLacks { text } => !contains_ci(t.answer(), text),
                Check::FileContains { path, text } => read_file(path).is_some_and(|body| body.contains(text.as_str())),
                Check::FileLacks { path, text } => !read_file(path).is_some_and(|body| body.contains(text.as_str())),
                Check::ToolCalled { name } => t.calls.iter().any(|c| is_tool(&c.name, name)),
            };
            CheckResult { check: check.describe(), passed }
        })
        .collect();
    let ended = t.result.as_ref().is_some_and(|r| !r.is_error);
    let tracers = task.tracers.iter().map(|tr| (tr.clone(), fate(t, tr))).collect();
    Score { success: ended && checks.iter().all(|c| c.passed), ended, checks, tracers }
}

fn fate(t: &Transcript, tracer: &str) -> TracerFate {
    let written = contains_ci(t.answer(), tracer)
        || t.texts.iter().any(|s| contains_ci(s, tracer))
        || t.calls.iter().any(|c| contains_ci(&c.input.to_string(), tracer));
    let brought = t.calls.iter().any(|c| c.result.as_deref().is_some_and(|r| contains_ci(r, tracer)))
        || t.hooks.iter().any(|h| contains_ci(&h.text, tracer));
    match (written, brought) {
        (true, _) => TracerFate::Used,
        (false, true) => TracerFate::RetrievedButIgnored,
        (false, false) => TracerFate::NeverReached,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::parse as parse_stream;
    use crate::task::parse as parse_task;

    fn task() -> Task {
        parse_task(
            r#"{"id": "t", "category": "K1", "fixture": "f", "prompt": "Which port?",
                "memories": [{"content": "Staging port 48213, proxy alias quokka"}],
                "tracers": ["48213", "quokka"],
                "checks": [
                    {"kind": "answer_contains", "text": "48213"},
                    {"kind": "answer_lacks", "text": "8080"},
                    {"kind": "file_contains", "path": "deploy.env", "text": "PORT=48213"},
                    {"kind": "file_lacks", "path": "gone.txt", "text": "x"},
                    {"kind": "tool_called", "name": "search_memory"}
                ]}"#,
        )
        .unwrap()
    }

    const FOUND: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"mcp__clawg__search_memory","input":{"query":"port"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"Staging port 48213, proxy alias quokka"}]}}
{"type":"result","subtype":"success","is_error":false,"result":"It is 48213."}
"#;

    fn files(path: &str) -> Option<String> {
        (path == "deploy.env").then(|| "PORT=48213\n".to_owned())
    }

    #[test]
    fn a_session_that_passes_every_check_succeeds() {
        let s = score(&task(), &parse_stream(FOUND), files);
        assert!(s.success && s.ended);
        assert!(s.checks.iter().all(|c| c.passed), "{:?}", s.checks);
        assert_eq!(
            s.tracers,
            vec![("48213".to_owned(), TracerFate::Used), ("quokka".to_owned(), TracerFate::RetrievedButIgnored)]
        );
    }

    #[test]
    fn one_failed_check_or_an_error_fails_the_task() {
        let wrong = FOUND.replace("It is 48213.", "It is 8080.");
        let s = score(&task(), &parse_stream(&wrong), files);
        assert!(!s.success);
        assert_eq!(s.checks.iter().filter(|c| !c.passed).count(), 2);
        assert_eq!(s.tracers[0].1, TracerFate::RetrievedButIgnored);

        let errored = FOUND.replace("\"is_error\":false,\"result\"", "\"is_error\":true,\"result\"");
        let s = score(&task(), &parse_stream(&errored), files);
        assert!(!s.success && !s.ended);

        let s = score(&task(), &parse_stream(FOUND), |_| None);
        assert!(!s.success);
        assert!(!s.checks[2].passed && s.checks[3].passed);
    }

    #[test]
    fn a_tracer_nothing_shows_never_reached() {
        let t = parse_stream(r#"{"type":"result","subtype":"success","is_error":false,"result":"I do not know."}"#);
        let s = score(&task(), &t, files);
        assert!(s.tracers.iter().all(|(_, f)| *f == TracerFate::NeverReached));
        assert!(!s.checks[4].passed);
    }

    #[test]
    fn a_tracer_a_hook_brought_counts_as_retrieved() {
        let t = parse_stream(
            r#"{"type":"system","subtype":"hook_response","output":"LAST SESSION: quokka"}
{"type":"result","subtype":"success","is_error":false,"result":"done"}"#,
        );
        assert_eq!(score(&task(), &t, files).tracers[1].1, TracerFate::RetrievedButIgnored);
    }
}
