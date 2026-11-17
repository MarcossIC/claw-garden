//! The client a dry run starts instead of a real one: it prints a recorded
//! `stream-json` and makes the file changes its `Write` and `Edit` calls
//! record, so the scoring sees the working directory a session would leave.
//! Deciding the changes is pure; [`run`] does the I/O.

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::stream::{parse, Transcript};
use crate::task::is_relative_inside;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileEdit {
    Write { path: String, content: String },
    Edit { path: String, old: String, new: String, replace_all: bool },
}

impl FileEdit {
    pub fn path(&self) -> &str {
        match self {
            FileEdit::Write { path, .. } | FileEdit::Edit { path, .. } => path,
        }
    }
}

/// The file changes the session's calls record, in order. A path that is
/// not relative to the working directory, or leaves it, is refused.
pub fn file_edits(t: &Transcript) -> Result<Vec<FileEdit>, String> {
    let mut out = Vec::new();
    for call in &t.calls {
        let field = |name: &str| call.input.get(name).and_then(Value::as_str).map(str::to_owned);
        let edit = match call.name.as_str() {
            "Write" => FileEdit::Write { path: field("file_path").unwrap_or_default(), content: field("content").unwrap_or_default() },
            "Edit" => FileEdit::Edit {
                path: field("file_path").unwrap_or_default(),
                old: field("old_string").unwrap_or_default(),
                new: field("new_string").unwrap_or_default(),
                replace_all: call.input.get("replace_all").and_then(Value::as_bool).unwrap_or(false),
            },
            _ => continue,
        };
        if !is_relative_inside(edit.path()) {
            return Err(format!("the replayed {} names {:?}, not a path inside the working directory", call.name, edit.path()));
        }
        out.push(edit);
    }
    Ok(out)
}

/// The text a file holds after `edit`, from `current`, `None` when missing.
pub fn apply(current: Option<&str>, edit: &FileEdit) -> Result<String, String> {
    match edit {
        FileEdit::Write { content, .. } => Ok(content.clone()),
        FileEdit::Edit { path, old, new, replace_all } => {
            let text = current.ok_or_else(|| format!("the replayed Edit names {path}, which does not exist"))?;
            match (text.matches(old.as_str()).count(), replace_all) {
                (0, _) => Err(format!("the replayed Edit of {path} does not find its old text")),
                (1, _) | (_, true) => Ok(text.replace(old.as_str(), new)),
                (_, false) => Err(format!("the replayed Edit of {path} finds its old text more than once")),
            }
        }
    }
}

/// Refuses `path` unless it, or its nearest existing ancestor when it does
/// not exist, resolves inside `root`, a canonical path: the check of
/// [`file_edits`] reads the path's text, and a link in the working directory
/// can still lead out of it.
fn resolves_inside(root: &Path, path: &Path) -> Result<(), String> {
    let existing = path.ancestors().find(|p| p.symlink_metadata().is_ok()).unwrap_or(path);
    let resolved = std::fs::canonicalize(existing).map_err(|e| format!("cannot resolve {}: {e}", existing.display()))?;
    if resolved.starts_with(root) {
        Ok(())
    } else {
        Err(format!("the replayed change of {} resolves outside the working directory", path.display()))
    }
}

/// How long the server may take to answer a request.
const ANSWER_BUDGET: Duration = Duration::from_secs(60);

/// How long a server that was told to end may take before it is killed.
const END_BUDGET: Duration = Duration::from_secs(5);

/// How an MCP server is started: its name in the config, which the client
/// puts in front of its tool names, and its command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}

/// The only server of an `mcp.json`.
pub fn launch(config: &str) -> Result<Launch, String> {
    let config: Value = serde_json::from_str(config).map_err(|e| format!("the mcp config is not JSON: {e}"))?;
    let (name, server) = config["mcpServers"].as_object().and_then(|s| s.iter().next()).ok_or("the mcp config names no server")?;
    let args = server["args"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
    let command = server["command"].as_str().ok_or_else(|| format!("the mcp config gives the server {name} no command"))?.to_owned();
    Ok(Launch { name: name.clone(), command, args })
}

/// A call the stream makes to a tool of the run's server.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerCall {
    pub id: String,
    /// The tool's own name, without the client's `mcp__SERVER__`.
    pub tool: String,
    pub input: Value,
}

/// What the server answered a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub text: String,
    pub is_error: bool,
}

/// The calls of the transcript to tools of the MCP server `server`, in order.
pub fn server_calls(t: &Transcript, server: &str) -> Vec<ServerCall> {
    let prefix = format!("mcp__{server}__");
    t.calls
        .iter()
        .filter_map(|c| c.name.strip_prefix(&prefix).map(|tool| ServerCall { id: c.id.clone(), tool: tool.to_owned(), input: c.input.clone() }))
        .collect()
}

/// `stream` with the result of each call `answers` names replaced by its
/// answer; every other line is as it was.
pub fn with_results(stream: &str, answers: &BTreeMap<String, Answer>) -> String {
    stream
        .lines()
        .map(|line| {
            let Ok(mut event) = serde_json::from_str::<Value>(line) else { return line.to_owned() };
            let mut changed = false;
            if let Some(blocks) = event.pointer_mut("/message/content").and_then(Value::as_array_mut) {
                for block in blocks.iter_mut().filter(|b| b["type"] == "tool_result") {
                    let Some(answer) = block["tool_use_id"].as_str().and_then(|id| answers.get(id)) else { continue };
                    block["content"] = Value::String(answer.text.clone());
                    block["is_error"] = Value::Bool(answer.is_error);
                    changed = true;
                }
            }
            if changed { event.to_string() } else { line.to_owned() }
        })
        .map(|line| line + "\n")
        .collect()
}

/// Starts `launch` in `cwd`, speaks JSON-RPC to it over stdio and sends each
/// of `calls`. A call it cannot answer fails the whole, naming the tool.
fn ask(launch: &Launch, calls: &[ServerCall], cwd: &Path) -> Result<BTreeMap<String, Answer>, String> {
    let mut child = Command::new(&launch.command)
        .args(&launch.args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("cannot start the server {}: {e}", launch.command))?;
    let mut stdin = child.stdin.take().ok_or("the server has no stdin")?;
    let stdout = child.stdout.take().ok_or("the server has no stdout")?;
    let (lines, replies) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if lines.send(line).is_err() {
                return;
            }
        }
    });
    let answers = converse(&mut stdin, &replies, calls);
    drop(stdin);
    let deadline = Instant::now() + END_BUDGET;
    while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    if matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill();
        let _ = child.wait();
    }
    answers
}

fn converse(stdin: &mut impl std::io::Write, replies: &Receiver<String>, calls: &[ServerCall]) -> Result<BTreeMap<String, Answer>, String> {
    let mut send = |message: Value| writeln!(stdin, "{message}").and_then(|()| stdin.flush()).map_err(|e| format!("cannot write to the server: {e}"));
    let reply = |id: u64, what: &str| -> Result<Value, String> {
        let deadline = Instant::now() + ANSWER_BUDGET;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = replies.recv_timeout(left).map_err(|_| format!("the server did not answer {what}"))?;
            if let Ok(frame) = serde_json::from_str::<Value>(&line) {
                if frame["id"] == json!(id) {
                    return Ok(frame);
                }
            }
        }
    };
    send(json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "agent-eval-replay", "version": "0" } } }))?;
    reply(0, "initialize")?;
    send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))?;
    let mut answers = BTreeMap::new();
    for (i, call) in calls.iter().enumerate() {
        let id = i as u64 + 1;
        send(json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": call.tool, "arguments": call.input } }))?;
        let frame = reply(id, &call.tool)?;
        if let Some(error) = frame.get("error") {
            return Err(format!("the server of the run could not answer {}: {}", call.tool, error["message"].as_str().unwrap_or("an error")));
        }
        let text: Vec<&str> = frame["result"]["content"].as_array().into_iter().flatten().filter_map(|b| b["text"].as_str()).collect();
        answers.insert(call.id.clone(), Answer { text: text.join("\n"), is_error: frame["result"]["isError"].as_bool().unwrap_or(false) });
    }
    Ok(answers)
}

/// Makes the changes `stream` records in `cwd`, then prints `stream`; with the
/// `mcp_config` of the run, each call to its server is made to that server and
/// the stream is printed with what it answered.
pub fn run(stream: &Path, cwd: &Path, mcp_config: Option<&Path>) -> Result<(), String> {
    let text = std::fs::read_to_string(stream).map_err(|e| format!("cannot read {}: {e}", stream.display()))?;
    let root = std::fs::canonicalize(cwd).map_err(|e| format!("cannot resolve {}: {e}", cwd.display()))?;
    for edit in file_edits(&parse(&text))? {
        let path = cwd.join(edit.path());
        resolves_inside(&root, &path)?;
        let current = std::fs::read_to_string(&path).ok();
        let after = apply(current.as_deref(), &edit)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        resolves_inside(&root, &path)?;
        std::fs::write(&path, after).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    let printed = match mcp_config {
        Some(config) => {
            let config = std::fs::read_to_string(config).map_err(|e| format!("cannot read {}: {e}", config.display()))?;
            let launch = launch(&config)?;
            let calls = server_calls(&parse(&text), &launch.name);
            if calls.is_empty() { text } else { with_results(&text, &ask(&launch, &calls, cwd)?) }
        }
        None => text,
    };
    std::io::stdout().write_all(printed.as_bytes()).map_err(|e| format!("cannot print the stream: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(calls: &str) -> Transcript {
        let calls = calls.replace('\n', " ");
        parse(&format!(r#"{{"type":"assistant","message":{{"content":[{calls}]}}}}"#))
    }

    #[test]
    fn writes_and_edits_are_read_in_order_and_other_calls_skipped() {
        let t = stream(
            r#"{"type":"tool_use","id":"1","name":"Write","input":{"file_path":"a.py","content":"x = 1\n"}},
               {"type":"tool_use","id":"2","name":"Read","input":{"file_path":"a.py"}},
               {"type":"tool_use","id":"3","name":"Edit","input":{"file_path":"a.py","old_string":"1","new_string":"2"}}"#,
        );
        assert_eq!(
            file_edits(&t).unwrap(),
            vec![
                FileEdit::Write { path: "a.py".into(), content: "x = 1\n".into() },
                FileEdit::Edit { path: "a.py".into(), old: "1".into(), new: "2".into(), replace_all: false },
            ]
        );
    }

    const SERVER_STREAM: &str = r#"{"type":"system","subtype":"init","session_id":"s"}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"mcp__clawg-dev__save_memory","input":{"content":"Port 48213"}},{"type":"tool_use","id":"toolu_2","name":"Read","input":{"file_path":"a.py"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"saved"},{"type":"tool_result","tool_use_id":"toolu_2","content":"x = 1"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_3","name":"mcp__clawg-dev__search_memory","input":{"query":"port"}}]},"parent_tool_use_id":"agent"}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_3","content":[{"type":"text","text":"nothing found"}]}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_4","name":"mcp__other__search_memory","input":{}}]}}
{"type":"result","subtype":"success","is_error":false,"result":"done"}
"#;

    #[test]
    fn a_replayed_call_to_the_server_of_the_run_carries_what_the_server_answered() {
        let calls = server_calls(&parse(SERVER_STREAM), "clawg-dev");
        assert_eq!(calls.iter().map(|c| (c.id.as_str(), c.tool.as_str())).collect::<Vec<_>>(), [("toolu_1", "save_memory"), ("toolu_3", "search_memory")]);
        assert_eq!(calls[0].input["content"], "Port 48213");

        let answers = BTreeMap::from([
            ("toolu_1".to_owned(), Answer { text: "saved as memory 7".into(), is_error: false }),
            ("toolu_3".to_owned(), Answer { text: "#1 Port 48213".into(), is_error: true }),
        ]);
        let printed = with_results(SERVER_STREAM, &answers);
        let t = parse(&printed);
        let result = |id: &str| t.calls.iter().find(|c| c.id == id).map(|c| (c.result.clone().unwrap(), c.is_error));
        assert_eq!(result("toolu_1"), Some(("saved as memory 7".to_owned(), false)));
        assert_eq!(result("toolu_3"), Some(("#1 Port 48213".to_owned(), true)), "what the server answered replaces what was recorded, a text block or not");
        assert_eq!(result("toolu_2"), Some(("x = 1".to_owned(), false)));
        assert_eq!(printed.lines().count(), SERVER_STREAM.lines().count());
        assert_eq!(printed.lines().next(), SERVER_STREAM.lines().next(), "a line that holds no result is printed as it was");
        assert!(printed.ends_with('\n'));
    }

    #[test]
    fn a_call_to_any_other_tool_stays_as_recorded() {
        assert_eq!(with_results(SERVER_STREAM, &BTreeMap::new()), SERVER_STREAM);
        let t = parse(SERVER_STREAM);
        assert!(server_calls(&t, "clawg").is_empty(), "a server of another name");
        let only_other = BTreeMap::from([("toolu_2".to_owned(), Answer { text: "changed".into(), is_error: false })]);
        let printed = parse(&with_results(SERVER_STREAM, &only_other));
        assert_eq!(printed.calls[1].result.as_deref(), Some("changed"), "an answer replaces the result of its call, whichever call it is");
        assert_eq!(printed.calls[0].result.as_deref(), Some("saved"));
    }

    #[test]
    fn the_server_of_a_run_is_launched_as_its_mcp_config_names_it() {
        let config = r#"{"mcpServers": {"clawg-dev": {"command": "/bin/clawg", "args": ["mcp", "--profile", "dev"]}}}"#;
        assert_eq!(
            launch(config).unwrap(),
            Launch { name: "clawg-dev".into(), command: "/bin/clawg".into(), args: vec!["mcp".into(), "--profile".into(), "dev".into()] }
        );
        assert!(launch(r#"{"mcpServers": {}}"#).unwrap_err().contains("no server"));
        assert!(launch("{").unwrap_err().contains("mcp config"));
    }

    #[test]
    fn a_server_that_cannot_answer_fails_the_client_naming_the_tool() {
        let launch = Launch { name: "clawg-dev".into(), command: "agent-eval-no-such-server".into(), args: Vec::new() };
        let calls = [ServerCall { id: "toolu_1".into(), tool: "save_memory".into(), input: serde_json::json!({}) }];
        let dir = tempfile::tempdir().unwrap();
        let error = ask(&launch, &calls, dir.path()).unwrap_err();
        assert!(error.contains("agent-eval-no-such-server"), "{error}");
    }

    #[test]
    fn a_path_outside_the_working_directory_is_refused() {
        let t = stream(r#"{"type":"tool_use","id":"1","name":"Write","input":{"file_path":"/etc/hosts","content":""}}"#);
        assert!(file_edits(&t).unwrap_err().contains("/etc/hosts"));
        let t = stream(r#"{"type":"tool_use","id":"1","name":"Edit","input":{"file_path":"../x","old_string":"a","new_string":"b"}}"#);
        assert!(file_edits(&t).is_err());
    }

    #[test]
    fn an_edit_replaces_its_text_once_or_everywhere_when_asked() {
        let edit = |old: &str, replace_all| FileEdit::Edit { path: "a".into(), old: old.into(), new: "Z".into(), replace_all };
        assert_eq!(apply(Some("a b a"), &edit("b", false)).unwrap(), "a Z a");
        assert_eq!(apply(Some("a b a"), &edit("a", true)).unwrap(), "Z b Z");
        assert!(apply(Some("a b a"), &edit("a", false)).unwrap_err().contains("more than once"));
        assert!(apply(Some("a b a"), &edit("q", false)).unwrap_err().contains("does not find"));
        assert!(apply(None, &edit("a", false)).unwrap_err().contains("does not exist"));
        assert_eq!(apply(None, &FileEdit::Write { path: "a".into(), content: "new".into() }).unwrap(), "new");
    }

    #[test]
    fn run_changes_the_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.py"), "x = 1\n").unwrap();
        let recorded = dir.path().join("s.jsonl");
        std::fs::write(
            &recorded,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"1","name":"Edit","input":{"file_path":"a.py","old_string":"1","new_string":"2"}},{"type":"tool_use","id":"2","name":"Write","input":{"file_path":"sub/b.py","content":"y"}}]}}"#,
        )
        .unwrap();
        run(&recorded, dir.path(), None).unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("a.py")).unwrap(), "x = 2\n");
        assert_eq!(std::fs::read_to_string(dir.path().join("sub/b.py")).unwrap(), "y");
    }

    #[test]
    fn a_change_must_resolve_inside_the_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("cwd");
        std::fs::create_dir(&cwd).unwrap();
        let root = std::fs::canonicalize(&cwd).unwrap();
        assert!(resolves_inside(&root, &cwd.join("new").join("a.py")).is_ok());
        assert!(resolves_inside(&root, &dir.path().join("b.py")).unwrap_err().contains("outside the working directory"));
    }

    #[cfg(unix)]
    #[test]
    fn run_refuses_a_change_a_link_leads_out_of_the_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        let (cwd, outside) = (dir.path().join("cwd"), dir.path().join("outside"));
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("f.py"), "x = 1\n").unwrap();
        std::os::unix::fs::symlink(&outside, cwd.join("dir")).unwrap();
        std::os::unix::fs::symlink(outside.join("f.py"), cwd.join("f.py")).unwrap();
        let recorded = dir.path().join("s.jsonl");
        for path in ["dir/a.py", "f.py"] {
            let call = format!(r#"{{"type":"tool_use","id":"1","name":"Write","input":{{"file_path":"{path}","content":"y"}}}}"#);
            std::fs::write(&recorded, format!(r#"{{"type":"assistant","message":{{"content":[{call}]}}}}"#)).unwrap();
            assert!(run(&recorded, &cwd, None).unwrap_err().contains("outside the working directory"), "{path}");
        }
        assert!(!outside.join("a.py").exists());
        assert_eq!(std::fs::read_to_string(outside.join("f.py")).unwrap(), "x = 1\n");
    }
}
