//! Whether a session succeeded at its task, and where each tracer went. Pure:
//! the working directory is read through the function the caller passes.

use serde::{Deserialize, Serialize};

use crate::stream::{is_tool, tool_name, ToolCall, Transcript};
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

/// One session scored on its own: its own checks and where the tracers went.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionScore {
    /// The session's checks, those written in it; the tracers are the task's.
    pub score: Score,
    /// Every tracer of the task and each born in an earlier session.
    pub followed: Vec<Journey>,
}

/// Where a tracer went in one session and what put it in front of the agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Journey {
    pub tracer: String,
    pub fate: TracerFate,
    /// The name of each tool whose result showed it, `hook` when a hook did, and
    /// `context` when it reached the session without passing through the stream.
    pub brought_by: Vec<String>,
}

/// Session `k` (from 0) of the task, played as `t`; `delivered` is what reached
/// its context outside the stream.
pub fn score_session(task: &Task, k: usize, t: &Transcript, delivered: &[String]) -> SessionScore {
    let own: &[Check] = task.sessions.get(k).map_or(&[], |s| &s.checks);
    let calls: Vec<&ToolCall> = t.calls.iter().collect();
    let checks = own.iter().map(|check| run_check(check, t.answer(), &calls, |_| None)).collect::<Vec<_>>();
    let ended = t.result.as_ref().is_some_and(|r| !r.is_error);
    let journey = |tracer: &String| journey(t, &task.id, tracer, delivered);
    let born_before = task.sessions.iter().take(k).flat_map(|s| &s.tracers);
    let followed: Vec<Journey> = task.tracers.iter().chain(born_before).map(journey).collect();
    let tracers = followed.iter().take(task.tracers.len()).map(|j| (j.tracer.clone(), j.fate)).collect();
    SessionScore { score: Score { success: ended && checks.iter().all(|c| c.passed), ended, checks, tracers }, followed }
}

/// The score of a run: every check of its sessions, each labelled with its
/// session when the task has several, then those of the task, which read the
/// answer of the last session, the calls of all of them and the files as the
/// run left them. A run that played fewer sessions than the task has did not end.
pub fn score(task: &Task, played: &[&Transcript], sessions: &[SessionScore], read_file: impl Fn(&str) -> Option<String>) -> Score {
    let calls: Vec<&ToolCall> = played.iter().flat_map(|t| &t.calls).collect();
    let answer = played.last().map_or("", |t| t.answer());
    let several = task.prompts().len() > 1;
    let mut checks: Vec<CheckResult> = Vec::new();
    for (i, session) in sessions.iter().enumerate() {
        checks.extend(session.score.checks.iter().map(|c| {
            let check = if several { format!("session {}: {}", i + 1, c.check) } else { c.check.clone() };
            CheckResult { check, passed: c.passed }
        }));
    }
    checks.extend(task.checks.iter().map(|check| run_check(check, answer, &calls, &read_file)));
    let ended = sessions.len() == task.prompts().len() && sessions.iter().all(|s| s.score.ended);
    let tracers = sessions.last().map(|s| s.score.tracers.clone()).unwrap_or_default();
    Score { success: ended && checks.iter().all(|c| c.passed), ended, checks, tracers }
}

fn run_check(check: &Check, answer: &str, calls: &[&ToolCall], read_file: impl Fn(&str) -> Option<String>) -> CheckResult {
    let passed = match check {
        Check::AnswerContains { text } => contains_ci(answer, text),
        Check::AnswerLacks { text } => !contains_ci(answer, text),
        Check::FileContains { path, text } => read_file(path).is_some_and(|body| file_has(&body, text)),
        Check::FileLacks { path, text } => !read_file(path).is_some_and(|body| file_has(&body, text)),
        Check::ToolCalled { name } => calls.iter().any(|c| is_tool(&c.name, name)),
        Check::ToolFailedCount { name, max, text } => failed_calls(calls, name, text.as_deref()) <= *max as usize,
    };
    CheckResult { check: check.describe(), passed }
}

/// The calls to the tool `name`, a subagent's included, that failed: those
/// marked as an error or, given `text`, those whose result holds it, marked or
/// not, since a command that is missing behind a `|` comes back unmarked.
fn failed_calls(calls: &[&ToolCall], name: &str, text: Option<&str>) -> usize {
    calls
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
pub fn shows(text: &str, task_id: &str, tracer: &str) -> bool {
    contains_ci(&text.to_lowercase().replace(&task_id.to_lowercase(), ""), tracer)
}

fn journey(t: &Transcript, task_id: &str, tracer: &str, delivered: &[String]) -> Journey {
    let shows = |text: &str| shows(text, task_id, tracer);
    let written = shows(t.answer()) || t.texts.iter().any(|s| shows(s)) || t.calls.iter().any(|c| shows(&c.input.to_string()));
    let mut brought_by: Vec<String> = Vec::new();
    for call in t.calls.iter().filter(|c| c.result.as_deref().is_some_and(shows)) {
        if !brought_by.iter().any(|name| name == tool_name(&call.name)) {
            brought_by.push(tool_name(&call.name).to_owned());
        }
    }
    if t.hooks.iter().any(|h| shows(&h.text)) {
        brought_by.push("hook".to_owned());
    }
    if delivered.iter().any(|text| shows(text)) {
        brought_by.push("context".to_owned());
    }
    let fate = match (written, brought_by.is_empty()) {
        (true, _) => TracerFate::Used,
        (false, false) => TracerFate::RetrievedButIgnored,
        (false, true) => TracerFate::NeverReached,
    };
    Journey { tracer: tracer.to_owned(), fate, brought_by }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::parse as parse_stream;
    use crate::task::parse as parse_task;

    /// The score of a run of one session.
    fn one(task: &Task, t: &Transcript, read_file: impl Fn(&str) -> Option<String>) -> Score {
        let session = score_session(task, 0, t, &[]);
        score(task, &[t], &[session], read_file)
    }

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
        let s = one(&task(), &parse_stream(FOUND), files);
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
        let s = one(&task(), &parse_stream(&wrong), files);
        assert!(!s.success);
        assert_eq!(s.checks.iter().filter(|c| !c.passed).count(), 2);
        assert_eq!(s.tracers[0].1, TracerFate::RetrievedButIgnored);

        let errored = FOUND.replace("\"is_error\":false,\"result\"", "\"is_error\":true,\"result\"");
        let s = one(&task(), &parse_stream(&errored), files);
        assert!(!s.success && !s.ended);

        let s = one(&task(), &parse_stream(FOUND), |_| None);
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
        let s = one(&t, &parse_stream(FOUND), crlf);
        assert!(s.checks[0].passed, "a CRLF file passes an LF check");
        assert!(s.checks[1].passed, "the absent text is still absent");
        assert!(s.checks[2].passed);

        let t = parse_task(
            r#"{"id": "t", "category": "K1", "fixture": "f", "prompt": "p",
                "checks": [{"kind": "file_lacks", "path": ".gitignore", "text": "dist/\ndist2/"}]}"#,
        )
        .unwrap();
        assert!(!one(&t, &parse_stream(FOUND), crlf).checks[0].passed, "file_lacks sees the text through CRLF");
    }

    #[test]
    fn a_tracer_nothing_shows_never_reached() {
        let t = parse_stream(r#"{"type":"result","subtype":"success","is_error":false,"result":"I do not know."}"#);
        let s = one(&task(), &t, files);
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
        assert_eq!(one(&t, &parse_stream(stream), |_| None).tracers[0].1, TracerFate::NeverReached);

        let used = stream.replace(r#""content":"def f(): pass\n""#, r#""content":"import secrets\n""#);
        assert_eq!(one(&t, &parse_stream(&used), |_| None).tracers[0].1, TracerFate::Used, "the word itself still counts");
    }

    #[test]
    fn a_tracer_in_what_reached_the_context_counts_as_retrieved() {
        let t = parse_stream(r#"{"type":"result","subtype":"success","is_error":false,"result":"It is 48213."}"#);
        let s = one(&task(), &t, files);
        assert_eq!(s.tracers, vec![("48213".to_owned(), TracerFate::Used), ("quokka".to_owned(), TracerFate::NeverReached)]);
        let delivered = score_session(&task(), 0, &t, &["# Memory\n\nStaging port 48213, proxy alias QUOKKA\n".to_owned()]);
        assert_eq!(
            delivered.score.tracers,
            vec![("48213".to_owned(), TracerFate::Used), ("quokka".to_owned(), TracerFate::RetrievedButIgnored)],
            "a tracer the agent wrote stays used"
        );

        let alone = score_session(&task(), 0, &t, &["nothing of it".to_owned()]);
        assert_eq!(alone.score.tracers[1].1, TracerFate::NeverReached);
    }

    #[test]
    fn a_tracer_a_hook_brought_counts_as_retrieved() {
        let t = parse_stream(
            r#"{"type":"system","subtype":"hook_response","output":"LAST SESSION: quokka"}
{"type":"result","subtype":"success","is_error":false,"result":"done"}"#,
        );
        assert_eq!(one(&task(), &t, files).tracers[1].1, TracerFate::RetrievedButIgnored);
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
            one(&task, &t, |_| None).checks[0].clone()
        };
        assert!(check(1, None).passed, "without a text only the call marked as an error failed");
        assert!(!check(0, None).passed);
        assert!(!check(1, Some("eza: command not found")).passed, "two calls hold the text, whatever their mark and their case, a subagent's included");
        assert!(check(2, Some("eza: command not found")).passed);
        assert!(check(0, Some("no such text")).passed);
        assert_eq!(check(2, Some("eza: command not found")).check, "Bash failed at most 2 times with \"eza: command not found\"");
        assert_eq!(check(1, None).check, "Bash failed at most 1 times");
    }

    /// A session that ends with `answer`, after the `calls` lines.
    fn session_of(calls: &str, answer: &str) -> Transcript {
        parse_stream(&format!("{calls}{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"{answer}\"}}\n"))
    }

    const WRITE: &str = "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"id\":\"w\",\"name\":\"Write\",\"input\":{\"file_path\":\"a.txt\",\"content\":\"ok\"}}]}}\n";

    fn files_of(name: &'static str) -> impl Fn(&str) -> Option<String> {
        move |path| (path == name).then(|| "ok\n".to_owned())
    }

    #[test]
    fn a_check_of_a_session_reads_that_session_and_one_of_the_task_the_last() {
        let t = parse_task(
            r#"{"id": "t", "category": "R", "fixture": "f",
                "sessions": [
                    {"prompt": "Say alpha.", "checks": [{"kind": "answer_contains", "text": "alpha"}]},
                    {"prompt": "Say omega.", "checks": [{"kind": "answer_lacks", "text": "alpha"}]}],
                "checks": [{"kind": "answer_contains", "text": "omega"}, {"kind": "tool_called", "name": "Write"},
                           {"kind": "file_contains", "path": "a.txt", "text": "ok"}]}"#,
        )
        .unwrap();
        let first = session_of(WRITE, "alpha");
        let second = session_of("", "omega");
        let scored = [score_session(&t, 0, &first, &[]), score_session(&t, 1, &second, &[])];
        assert_eq!(scored[0].score.checks, [CheckResult { check: "answer contains \"alpha\"".into(), passed: true }]);
        assert_eq!(scored[1].score.checks, [CheckResult { check: "answer lacks \"alpha\"".into(), passed: true }]);
        assert!(scored[0].score.success && scored[1].score.success);

        let run = score(&t, &[&first, &second], &scored, files_of("a.txt"));
        let described: Vec<(&str, bool)> = run.checks.iter().map(|c| (c.check.as_str(), c.passed)).collect();
        assert_eq!(
            described,
            [
                ("session 1: answer contains \"alpha\"", true),
                ("session 2: answer lacks \"alpha\"", true),
                ("answer contains \"omega\"", true),
                ("Write was called", true),
                ("a.txt contains \"ok\"", true),
            ],
            "a check of the task reads the last answer, the calls of every session and the files as they were left"
        );
        assert!(run.success && run.ended);

        let stale = session_of("", "alpha again");
        let scored = [score_session(&t, 0, &first, &[]), score_session(&t, 1, &stale, &[])];
        let run = score(&t, &[&first, &stale], &scored, files_of("a.txt"));
        assert!(!scored[1].score.success, "the check of session 2 failed on its own answer");
        assert_eq!((run.checks[1].passed, run.checks[2].passed, run.success), (false, false, false));
        assert_eq!(run.tracers, scored[1].score.tracers, "the tracers of the run are those of its last session");

        let only_first = score(&t, &[&first], &[score_session(&t, 0, &first, &[])], files_of("a.txt"));
        assert!(!only_first.ended && !only_first.success, "a run that played fewer sessions than the task has did not end");
    }

    fn relay() -> Task {
        parse_task(
            r#"{"id": "t", "category": "R", "fixture": "f",
                "memories": [{"content": "Proxy alias quokka"}], "tracers": ["quokka"],
                "sessions": [{"prompt": "Decide on 4.5, 41.", "tracers": ["4.5, 41"]}, {"prompt": "Apply it."}],
                "checks": [{"kind": "answer_contains", "text": "x"}]}"#,
        )
        .unwrap()
    }

    #[test]
    fn a_tracer_born_in_a_session_is_followed_in_the_later_ones() {
        let t = relay();
        let first = score_session(&t, 0, &session_of("", "done"), &[]);
        let names = |s: &SessionScore| s.followed.iter().map(|j| j.tracer.clone()).collect::<Vec<_>>();
        assert_eq!(names(&first), ["quokka"], "a session does not follow the facts born in it");

        let later = session_of("", "applied");
        let second = score_session(&t, 1, &later, &[]);
        assert_eq!(names(&second), ["quokka", "4.5, 41"]);
        assert_eq!(second.followed[1].fate, TracerFate::NeverReached);
        assert_eq!(second.score.tracers, vec![("quokka".to_owned(), TracerFate::NeverReached)], "the score keeps the tracers of the task");

        let remembered = score_session(&t, 1, &later, &["Decide on 4.5, 41.".to_owned()]);
        assert_eq!(remembered.followed[1].fate, TracerFate::RetrievedButIgnored);
        let said = session_of("", "applied 4.5, 41");
        assert_eq!(score_session(&t, 1, &said, &[]).followed[1].fate, TracerFate::Used);
    }

    #[test]
    fn a_tracer_names_what_brought_it() {
        let stream = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"s1","name":"mcp__clawg__search_memory","input":{"query":"port"}},{"type":"tool_use","id":"r1","name":"Read","input":{"file_path":"notes.md"}},{"type":"tool_use","id":"r2","name":"Read","input":{"file_path":"other.md"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"s1","content":"Staging port 48213"},{"type":"tool_result","tool_use_id":"r1","content":"port 48213 here"},{"type":"tool_result","tool_use_id":"r2","content":"nothing"}]}}
{"type":"system","subtype":"hook_response","output":"LAST SESSION: quokka"}
{"type":"result","subtype":"success","is_error":false,"result":"done"}
"#;
        let t = parse_stream(stream);
        let followed = score_session(&task(), 0, &t, &[]).followed;
        let brought = |tracer: &str, followed: &[Journey]| followed.iter().find(|j| j.tracer == tracer).unwrap().brought_by.clone();
        assert_eq!(brought("48213", &followed), ["search_memory", "Read"], "each tool whose result showed it, once, by its own name");
        assert_eq!(brought("quokka", &followed), ["hook"]);
        assert_eq!(followed[0].fate, TracerFate::RetrievedButIgnored);

        let with_context = score_session(&task(), 0, &t, &["# Memory\n\nproxy alias quokka\n".to_owned()]).followed;
        assert_eq!(brought("quokka", &with_context), ["hook", "context"]);
        let silent = score_session(&task(), 0, &session_of("", "done"), &["# Memory\n\nproxy alias quokka\n".to_owned()]).followed;
        assert_eq!(brought("quokka", &silent), ["context"], "what reached the context without the stream");
        assert!(brought("48213", &silent).is_empty());
    }
}
