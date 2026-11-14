//! Whether a session succeeded at its task, and where each tracer went. Pure:
//! the working directory is read through the function the caller passes.

use serde::{Deserialize, Serialize};

use crate::stream::{is_tool, Transcript};
use crate::task::{contains_ci, Check, Task};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TracerFate {
    /// The agent wrote it: in the answer, in a tool's input or in its text.
    Used,
    /// A tool result or a hook brought it, and the agent did not write it.
    RetrievedButIgnored,
    /// Nothing in the session shows it.
    NeverReached,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub check: String,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
                Check::FileContains { path, text } => read_file(path).is_some_and(|body| file_has(&body, text)),
                Check::FileLacks { path, text } => !read_file(path).is_some_and(|body| file_has(&body, text)),
                Check::ToolCalled { name } => t.calls.iter().any(|c| is_tool(&c.name, name)),
                Check::ToolFailedCount { name, max, text } => failed_calls(t, name, text.as_deref()) <= *max as usize,
            };
            CheckResult { check: check.describe(), passed }
        })
        .collect();
    let ended = t.result.as_ref().is_some_and(|r| !r.is_error);
    let tracers = task.tracers.iter().map(|tr| (tr.clone(), fate(t, &task.id, tr))).collect();
    Score { success: ended && checks.iter().all(|c| c.passed), ended, checks, tracers }
}

/// The calls to the tool `name`, a subagent's included, that failed: those
/// marked as an error or, given `text`, those whose result holds it, marked or
/// not, since a command that is missing behind a `|` comes back unmarked.
fn failed_calls(t: &Transcript, name: &str, text: Option<&str>) -> usize {
    t.calls
        .iter()
        .filter(|c| is_tool(&c.name, name))
        .filter(|c| match text {
            Some(text) => c.result.as_deref().is_some_and(|r| contains_ci(r, text)),
            None => c.is_error,
        })
        .count()
}

/// A checkout with `core.autocrlf` leaves CRLF in the fixtures and the agent's edits keep it,
/// while a check is written with LF.
fn file_has(body: &str, text: &str) -> bool {
    body.replace("\r\n", "\n").contains(&text.replace("\r\n", "\n"))
}

/// Whether `text` shows `tracer` once the task's id is taken out of it: a
/// run's directory may be named after its task, so every path the session
/// writes or reads holds the id, and an id can hold a tracer.
fn shows(text: &str, task_id: &str, tracer: &str) -> bool {
    contains_ci(&text.to_lowercase().replace(&task_id.to_lowercase(), ""), tracer)
}

/// Marks as retrieved each tracer that nothing in the stream showed and that
/// a text in `delivered` holds: what reached the session's context without
/// passing through the stream, such as the native memory loaded at startup.
pub fn add_delivered(score: &mut Score, task_id: &str, delivered: &[String]) {
    for (tracer, fate) in &mut score.tracers {
        if *fate == TracerFate::NeverReached && delivered.iter().any(|text| shows(text, task_id, tracer)) {
            *fate = TracerFate::RetrievedButIgnored;
        }
    }
}

fn fate(t: &Transcript, task_id: &str, tracer: &str) -> TracerFate {
    let shows = |text: &str| shows(text, task_id, tracer);
    let written = shows(t.answer())
        || t.texts.iter().any(|s| shows(s))
        || t.calls.iter().any(|c| shows(&c.input.to_string()));
    let brought = t.calls.iter().any(|c| c.result.as_deref().is_some_and(shows)) || t.hooks.iter().any(|h| shows(&h.text));
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
    fn file_checks_ignore_carriage_returns() {
        let t = parse_task(
            r#"{"id": "t", "category": "K1", "fixture": "f", "prompt": "p",
                "checks": [
                    {"kind": "file_contains", "path": ".gitignore", "text": "dist/\ndist2/\nlogs/"},
                    {"kind": "file_lacks", "path": ".gitignore", "text": "dist2/\ntmp/"},
                    {"kind": "file_lacks", "path": ".gitignore", "text": "dist/\nlogs/"}
                ]}"#,
        )
        .unwrap();
        let crlf = |_: &str| Some("dist/\r\ndist2/\r\nlogs/\r\n".to_owned());
        let s = score(&t, &parse_stream(FOUND), crlf);
        assert!(s.checks[0].passed, "a CRLF file passes an LF check");
        assert!(s.checks[1].passed, "the absent text is still absent");
        assert!(s.checks[2].passed);

        let t = parse_task(
            r#"{"id": "t", "category": "K1", "fixture": "f", "prompt": "p",
                "checks": [{"kind": "file_lacks", "path": ".gitignore", "text": "dist/\ndist2/"}]}"#,
        )
        .unwrap();
        assert!(!score(&t, &parse_stream(FOUND), crlf).checks[0].passed, "file_lacks sees the text through CRLF");
    }

    #[test]
    fn a_tracer_nothing_shows_never_reached() {
        let t = parse_stream(r#"{"type":"result","subtype":"success","is_error":false,"result":"I do not know."}"#);
        let s = score(&task(), &t, files);
        assert!(s.tracers.iter().all(|(_, f)| *f == TracerFate::NeverReached));
        assert!(!s.checks[4].passed);
    }

    #[test]
    fn a_tracer_in_the_task_id_is_not_seen_in_the_paths_of_the_run() {
        let t = parse_task(
            r#"{"id": "d14-import-secrets", "category": "K2", "fixture": "f", "prompt": "p",
                "memories": [{"content": "Always import the secrets module"}],
                "tracers": ["secrets"],
                "checks": [{"kind": "file_contains", "path": "a.py", "text": "import secrets"}]}"#,
        )
        .unwrap();
        let stream = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"w","name":"Write","input":{"file_path":"C:\\runs\\humo\\d14-import-secrets.a3.1\\work\\a.py","content":"def f(): pass\n"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"w","content":"File created successfully at: C:\\runs\\humo\\D14-Import-Secrets.a3.1\\work\\a.py"}]}}
{"type":"result","subtype":"success","is_error":false,"result":"Wrote C:/runs/humo/d14-import-secrets.a3.1/work/a.py"}"#;
        assert_eq!(score(&t, &parse_stream(stream), |_| None).tracers[0].1, TracerFate::NeverReached);

        let used = stream.replace(r#""content":"def f(): pass\n""#, r#""content":"import secrets\n""#);
        assert_eq!(score(&t, &parse_stream(&used), |_| None).tracers[0].1, TracerFate::Used, "the word itself still counts");
    }

    #[test]
    fn a_tracer_in_what_reached_the_context_counts_as_retrieved() {
        let t = parse_stream(r#"{"type":"result","subtype":"success","is_error":false,"result":"It is 48213."}"#);
        let mut s = score(&task(), &t, files);
        assert_eq!(s.tracers, vec![("48213".to_owned(), TracerFate::Used), ("quokka".to_owned(), TracerFate::NeverReached)]);
        add_delivered(&mut s, "t", &["# Memory\n\nStaging port 48213, proxy alias QUOKKA\n".to_owned()]);
        assert_eq!(
            s.tracers,
            vec![("48213".to_owned(), TracerFate::Used), ("quokka".to_owned(), TracerFate::RetrievedButIgnored)],
            "a tracer the agent wrote stays used"
        );

        let mut alone = score(&task(), &t, files);
        add_delivered(&mut alone, "t", &["nothing of it".to_owned()]);
        assert_eq!(alone.tracers[1].1, TracerFate::NeverReached);
    }

    #[test]
    fn a_tracer_a_hook_brought_counts_as_retrieved() {
        let t = parse_stream(
            r#"{"type":"system","subtype":"hook_response","output":"LAST SESSION: quokka"}
{"type":"result","subtype":"success","is_error":false,"result":"done"}"#,
        );
        assert_eq!(score(&task(), &t, files).tracers[1].1, TracerFate::RetrievedButIgnored);
    }

    #[test]
    fn the_failed_calls_of_a_tool_are_those_marked_or_those_that_hold_a_text() {
        let stream = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"b1","name":"Bash","input":{"command":"eza src | wc -l"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"b1","content":"bash: line 1: EZA: command not found\n0"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"b2","name":"Bash","input":{"command":"ls nope"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"b2","is_error":true,"content":"ls: cannot access 'nope'"}]}}
{"type":"assistant","parent_tool_use_id":"agent","message":{"content":[{"type":"tool_use","id":"b3","name":"Bash","input":{"command":"eza"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"b3","content":"eza: command not found"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"b4","name":"Bash","input":{"command":"ls src"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"b4","content":"a.py"}]}}
{"type":"result","subtype":"success","is_error":false,"result":"2 files"}
"#;
        let t = parse_stream(stream);
        let check = |max: u32, text: Option<&str>| {
            let task = parse_task(&format!(
                r#"{{"id": "t", "category": "E", "fixture": "f", "prompt": "p", "shell": ["ls", "eza"],
                    "checks": [{{"kind": "tool_failed_count", "name": "Bash", "max": {max}{}}}]}}"#,
                text.map_or(String::new(), |x| format!(r#", "text": "{x}""#))
            ))
            .unwrap();
            score(&task, &t, |_| None).checks[0].clone()
        };
        assert!(check(1, None).passed, "without a text only the call marked as an error failed");
        assert!(!check(0, None).passed);
        assert!(!check(1, Some("eza: command not found")).passed, "two calls hold the text, whatever their mark and their case, a subagent's included");
        assert!(check(2, Some("eza: command not found")).passed);
        assert!(check(0, Some("no such text")).passed);
        assert_eq!(check(2, Some("eza: command not found")).check, "Bash failed at most 2 times with \"eza: command not found\"");
        assert_eq!(check(1, None).check, "Bash failed at most 1 times");
    }
}
