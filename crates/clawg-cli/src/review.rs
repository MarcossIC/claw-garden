//! `clawg review`: the Cursor CLI reviews a change with this profile's
//! memory, and the run is recorded as a session of its own.
//!
//! The CLI runs with its home in `<data>/review/cursor`, whose `.cursor` this
//! module writes on every run: an `mcp.json` that pins the bridge to the
//! reviewer's agent and this profile, a `cli-config.json` that lets it read
//! and run only the commands it is given, and a `hooks.json` with none, so
//! the user's own Cursor setup neither runs nor changes. The prompt goes in
//! on stdin, and the CLI's `stream-json` lines come back on stdout: the
//! first names the session, which is registered before any tool is called,
//! and the last carries the report and what it spent.
//!
//! Everything that decides is a pure function over text and paths; [`run`]
//! is the only part that reads, writes or starts anything.

pub mod prefs;

use std::ffi::OsString;
use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use clawg_core::metrics::sessions::Usage;
use clawg_core::paths::Profile;
use clawg_core::preferences::{self, Lang};
use serde_json::{json, Value};

use crate::cli::ReviewOpts;
use crate::client::{DaemonClient, EndedRun, ProcessSpawner, SessionEvent};
use crate::error::CliError;

/// The agent id the reviewer reads memories as, unless told another.
pub const DEFAULT_AGENT: &str = "cursor-review";

/// The commands a reviewer may run unless told others: enough to read the
/// change and its history.
pub const DEFAULT_ALLOW: [&str; 3] = ["git diff", "git log", "git show"];

/// The bridge's tools the reviewer may call: those that read. It saves,
/// changes and settles nothing; what it learns comes back in its report.
pub const READ_TOOLS: [&str; 6] = ["search_memory", "recall_context", "read_memory", "list_memories", "search_project", "memory_status"];

/// The model kept for a run that named none: the CLI's own default.
pub const AUTO_MODEL: &str = "auto";

/// The client a review's session is recorded under.
const CLIENT: &str = "cursor";

/// Exit code for a review that cannot start: no message to give it.
const EXIT_USAGE: u8 = 2;

/// Exit code for a review that ran and failed.
const EXIT_FAILURE: u8 = 1;

/// The text of `instructions` without the frontmatter at its top, then a
/// blank line and `message`.
pub fn prompt(instructions: &str, message: &str) -> String {
    format!("{}\n\n{}", without_frontmatter(instructions).trim(), message.trim())
}

/// `text` from after a `---` block that opens its first line, or all of it.
fn without_frontmatter(text: &str) -> &str {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.split_inclusive('\n');
    if lines.next().map(str::trim_end) != Some("---") {
        return text;
    }
    let mut taken = text.split_inclusive('\n').next().map_or(0, str::len);
    for line in lines {
        taken += line.len();
        if line.trim_end() == "---" {
            return &text[taken..];
        }
    }
    text
}

/// What a report says of the change: its last line that is not blank, when
/// it is `CLEAN`, `WARNINGS` or `BLOCKED` in any case and with or without
/// backticks or asterisks around it; `none` otherwise.
pub fn verdict(report: &str) -> &'static str {
    let last = report.lines().map(str::trim).rfind(|line| !line.is_empty()).unwrap_or_default();
    match last.trim_matches(|c| c == '`' || c == '*').trim().to_ascii_uppercase().as_str() {
        "CLEAN" => "clean",
        "WARNINGS" => "warnings",
        "BLOCKED" => "blocked",
        _ => "none",
    }
}

/// The last line of a run: the report, whether the CLI says it failed, and
/// what it spent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub report: String,
    pub is_error: bool,
    pub usage: Usage,
}

/// One line of the CLI's `stream-json`, as far as a review reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// The run started, under this session.
    Init { session_id: String },
    /// The run ended.
    Result(Outcome),
    /// Anything else: a tool call, a message, a line that is not JSON.
    Other,
}

/// Reads one line of `stream-json`. A count the CLI leaves out reads as zero.
pub fn read_line(line: &str) -> Line {
    let Ok(event) = serde_json::from_str::<Value>(line) else { return Line::Other };
    match (event["type"].as_str(), event["subtype"].as_str()) {
        (Some("system"), Some("init")) => match event["session_id"].as_str() {
            Some(session_id) => Line::Init { session_id: session_id.to_string() },
            None => Line::Other,
        },
        (Some("result"), _) => {
            let usage = &event["usage"];
            let count = |key: &str| usage[key].as_u64().unwrap_or(0);
            Line::Result(Outcome {
                report: event["result"].as_str().unwrap_or_default().to_string(),
                is_error: event["is_error"].as_bool().unwrap_or(false),
                usage: Usage {
                    input: count("inputTokens"),
                    output: count("outputTokens"),
                    cache_read: count("cacheReadTokens"),
                    cache_write: count("cacheWriteTokens"),
                    reasoning: count("reasoningTokens"),
                },
            })
        }
        _ => Line::Other,
    }
}

/// The model a run is kept under: the one it was given, in lowercase, or
/// [`AUTO_MODEL`].
pub fn kept_model(model: Option<&str>) -> String {
    model.map(str::trim).filter(|m| !m.is_empty()).map_or_else(|| AUTO_MODEL.to_string(), str::to_ascii_lowercase)
}

/// What the files of the reviewer's `.cursor` are made from.
#[derive(Debug, Clone)]
pub struct CursorSetup<'a> {
    /// The profile's server name, as `mcp.json` lists it.
    pub server: &'a str,
    /// The clawg the bridge runs.
    pub exe: &'a Path,
    pub profile: Profile,
    pub agent: &'a str,
    /// The user's home, which the bridge gets back.
    pub home: &'a Path,
    pub allow: &'a [String],
    /// The variables a test build is pointed with, passed on to the bridge.
    pub test_env: &'a [(&'static str, OsString)],
}

/// The reviewer's `.cursor/mcp.json`: this profile's bridge alone, pinned
/// to the reviewer's agent, with the user's home given back to it, since
/// the CLI runs with another one.
pub fn mcp_file(setup: &CursorSetup<'_>) -> String {
    let home = setup.home.to_string_lossy();
    let mut env = serde_json::Map::new();
    env.insert("HOME".into(), json!(home));
    env.insert("USERPROFILE".into(), json!(home));
    for (var, value) in setup.test_env {
        env.insert((*var).into(), json!(value.to_string_lossy()));
    }
    let server = json!({
        "type": "stdio",
        "command": setup.exe.to_string_lossy(),
        "args": ["mcp", "--profile", setup.profile.as_str(), "--agent", setup.agent],
        "env": env,
    });
    pretty(&json!({ "mcpServers": { setup.server: server } }))
}

/// What the reviewer may never do, whatever it is allowed: write a file,
/// with its tools or through `--output`, which `git diff`, `git log` and
/// `git show` take to truncate and write the file it names.
pub const DENY: [&str; 2] = ["Write(**)", "Shell(* --output*)"];

/// The reviewer's `.cursor/cli-config.json`: the bridge's tools that read,
/// the commands it was allowed, and [`DENY`].
pub fn config_file(server: &str, allow: &[String]) -> String {
    let tools = READ_TOOLS.iter().map(|tool| format!("Mcp({server}:{tool})"));
    let commands = allow.iter().map(|command| format!("Shell({})", command.trim()));
    let allowed: Vec<String> = tools.chain(commands).collect();
    pretty(&json!({ "version": 1, "permissions": { "allow": allowed, "deny": DENY } }))
}

/// What the CLI's environment changes: the review's folder as its home, and
/// as the place of PowerShell's module cache, which the shell it runs
/// commands in would otherwise write under the reviewed folder, since that
/// shell does not get `LOCALAPPDATA`.
pub fn cursor_env(review_home: &Path) -> [(&'static str, PathBuf); 3] {
    [
        ("USERPROFILE", review_home.to_path_buf()),
        ("HOME", review_home.to_path_buf()),
        ("PSModuleAnalysisCachePath", review_home.join("powershell").join("ModuleAnalysisCache")),
    ]
}

/// The reviewer's `.cursor/hooks.json`: none, so no hook of clawg or of the
/// user runs in a review.
pub fn hooks_file() -> String {
    pretty(&json!({ "version": 1, "hooks": {} }))
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("a JSON value serializes") + "\n"
}

/// The CLI's arguments for a run in `workspace`: print mode with every line
/// as JSON, the folder trusted and its MCP server started without asking.
/// Never `--force`: what the permissions do not allow is refused.
pub fn cursor_args(workspace: &Path, model: Option<&str>, resume: Option<&str>) -> Vec<OsString> {
    let mut args: Vec<OsString> =
        ["-p", "--trust", "--approve-mcps", "--output-format", "stream-json", "--workspace"].map(OsString::from).to_vec();
    args.push(workspace.as_os_str().to_owned());
    if let Some(model) = model {
        args.extend(["--model".into(), model.into()]);
    }
    if let Some(resume) = resume {
        args.extend(["--resume".into(), resume.into()]);
    }
    args
}

/// Where the CLI is when `--cursor` does not say: where its installer puts
/// it on Windows, `agent` on the `PATH` elsewhere. `None` when it is not
/// there.
pub fn find_cursor(
    windows: bool,
    local_app_data: Option<&Path>,
    path: Option<&std::ffi::OsStr>,
    is_file: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if windows {
        return local_app_data.map(|dir| dir.join("cursor-agent").join("agent.cmd")).filter(|cli| is_file(cli));
    }
    std::env::split_paths(path?).map(|dir| dir.join("agent")).find(|cli| is_file(cli))
}

/// The sentence for a CLI that is not where it was looked for.
pub fn missing_cursor(lang: Lang) -> String {
    match lang {
        Lang::En => "the Cursor CLI was not found where its installer puts it; install it, or name it with --cursor".into(),
        Lang::Es => "no se encontró el CLI de Cursor donde lo deja su instalador; instalalo, o indicalo con --cursor".into(),
    }
}

/// The line after a report: what to quote of the session for the next round.
pub fn summary_line(lang: Lang, session_id: &str, model: &str, verdict: &str, seconds: u64, usage: &Usage) -> String {
    let verdict = verdict.to_ascii_uppercase();
    match lang {
        Lang::En => format!(
            "clawg review: session {session_id} · model {model} · verdict {verdict} · {seconds} s · tokens {} in, {} out, {} cache read · next round: --resume {session_id}",
            usage.input, usage.output, usage.cache_read
        ),
        Lang::Es => format!(
            "clawg review: sesión {session_id} · modelo {model} · veredicto {verdict} · {seconds} s · tokens {} de entrada, {} de salida, {} de caché · próxima ronda: --resume {session_id}",
            usage.input, usage.output, usage.cache_read
        ),
    }
}

/// What a run of the CLI gave: the session it named, its last line, its
/// exit status and what it wrote on stderr.
struct Ran {
    session_id: Option<String>,
    outcome: Option<Outcome>,
    succeeded: bool,
    stderr: String,
}

/// Runs a review: the message from stdin, the CLI in this folder, the
/// session recorded in the profile's daemon, started if it is not running.
pub async fn run(opts: ReviewOpts, profile: Profile, base_dir: &Path) -> Result<ExitCode, CliError> {
    let lang = preferences::language_in(base_dir).unwrap_or(Lang::En);
    let stdin = std::io::stdin();
    let mut message = String::new();
    if !stdin.is_terminal() {
        stdin.lock().read_to_string(&mut message)?;
    }
    if message.trim().is_empty() {
        eprintln!(
            "clawg: {}",
            match lang {
                Lang::En => "review takes its message on stdin: what to review and what already ran",
                Lang::Es => "review toma su mensaje por stdin: qué revisar y lo que ya se corrió",
            }
        );
        return Ok(ExitCode::from(EXIT_USAGE));
    }
    let instructions = std::fs::read_to_string(&opts.instructions)?;
    let cursor = match &opts.cursor {
        Some(cursor) => cursor.clone(),
        None => find_cursor(
            cfg!(windows),
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from).as_deref(),
            std::env::var_os("PATH").as_deref(),
            Path::is_file,
        )
        .ok_or_else(|| CliError::Failed(missing_cursor(lang)))?,
    };
    let home = clawg_core::paths::home_dir().ok_or(clawg_core::paths::PathsError::NoHome)?;
    let workspace = std::env::current_dir()?;

    let spawner = ProcessSpawner { base_dir: base_dir.to_path_buf() };
    let client = crate::client::ensure_running(profile, true, &spawner).await?;
    let project = crate::hook::project_of(&client, &workspace).await;

    let review_home = base_dir.join("review").join("cursor");
    let dot_cursor = review_home.join(".cursor");
    clawg_core::private_dir::ensure_private_dir(&review_home)?;
    clawg_core::private_dir::ensure_private_dir(&dot_cursor)?;
    let exe = crate::setup::current_exe()?;
    let server = profile.mcp_server_name();
    let test_env = clawg_core::paths::test_env();
    let setup =
        CursorSetup { server, exe: &exe, profile, agent: &opts.agent, home: &home, allow: &opts.allow, test_env: &test_env };
    std::fs::write(dot_cursor.join("mcp.json"), mcp_file(&setup))?;
    std::fs::write(dot_cursor.join("cli-config.json"), config_file(server, &opts.allow))?;
    std::fs::write(dot_cursor.join("hooks.json"), hooks_file())?;

    let started = std::time::Instant::now();
    let mut command = Command::new(&cursor);
    command
        .args(cursor_args(&workspace, opts.model.as_deref(), opts.resume.as_deref()))
        .envs(cursor_env(&review_home))
        .current_dir(&workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let source = if opts.resume.is_some() { "resume" } else { "startup" };
    let ran = drive(command, prompt(&instructions, &message), &client, &opts.agent, project.as_deref(), source).await?;
    let seconds = started.elapsed().as_secs();

    let Some(session_id) = ran.session_id else {
        eprint!("{}", ran.stderr);
        eprintln!(
            "clawg: {}",
            match lang {
                Lang::En => "the Cursor CLI ended before starting a session; nothing was recorded",
                Lang::Es => "el CLI de Cursor terminó antes de empezar una sesión; no se registró nada",
            }
        );
        return Ok(ExitCode::from(EXIT_FAILURE));
    };
    let outcome = ran.outcome.unwrap_or_else(|| Outcome { is_error: true, ..Outcome::default() });
    let completed = ran.succeeded && !outcome.is_error;
    let model = kept_model(opts.model.as_deref());
    let said = verdict(&outcome.report);
    let ended = EndedRun { usage: outcome.usage, model: &model, verdict: said };
    if let Err(e) = client.end_session_with(&session_id, if completed { "completed" } else { "error" }, &ended).await {
        log::warn!("the review's session could not be closed in the daemon: {e}");
    }
    println!("{}", outcome.report.trim_end());
    println!();
    println!("{}", summary_line(lang, &session_id, &model, said, seconds, &outcome.usage));
    if !completed {
        eprint!("{}", ran.stderr);
        return Ok(ExitCode::from(EXIT_FAILURE));
    }
    Ok(ExitCode::SUCCESS)
}

/// Starts `command`, gives it `prompt` on stdin and reads its lines,
/// registering the session the first one names before any other is read.
async fn drive(
    mut command: Command,
    prompt: String,
    client: &DaemonClient,
    agent: &str,
    project: Option<&str>,
    source: &str,
) -> Result<Ran, CliError> {
    let mut child = command.spawn()?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    // A CLI that ends without reading closes the pipe: what it says then is
    // in its exit status and its stderr, not in this write.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(prompt.as_bytes());
    });
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let (lines, mut read) = tokio::sync::mpsc::unbounded_channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if lines.send(read_line(&line)).is_err() {
                break;
            }
        }
    });

    let (mut session_id, mut outcome) = (None, None);
    while let Some(line) = read.recv().await {
        match line {
            Line::Init { session_id: id } if session_id.is_none() => {
                let event = SessionEvent { event: "start", source: Some(source), client: CLIENT };
                if let Err(e) = client.register_session(agent, project, &id, event).await {
                    log::warn!("the review's session could not be registered in the daemon: {e}");
                }
                session_id = Some(id);
            }
            Line::Result(result) => outcome = Some(result),
            Line::Init { .. } | Line::Other => {}
        }
    }
    let status = tokio::task::spawn_blocking(move || child.wait()).await.map_err(std::io::Error::other)??;
    let _ = writer.join();
    let _ = reader.join();
    let stderr = errors.join().unwrap_or_default();
    Ok(Ran { session_id, outcome, succeeded: status.success(), stderr })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_is_the_instructions_without_frontmatter_and_the_message() {
        let agent = "---\nname: change-reviewer\ntools: Read\n---\n\nYou review a change.\n";
        assert_eq!(prompt(agent, "Plan: p.md\nRange: a..b\n"), "You review a change.\n\nPlan: p.md\nRange: a..b");
        assert_eq!(prompt("No frontmatter here.", "go"), "No frontmatter here.\n\ngo");
        let windows = "---\r\nname: r\r\n---\r\nBody\r\n";
        assert_eq!(prompt(windows, "m"), "Body\n\nm");
        assert_eq!(prompt("---\nnever closed\n", "m"), "---\nnever closed\n\nm", "an unclosed block is text");
    }

    #[test]
    fn a_verdict_is_read_from_the_last_line() {
        assert_eq!(verdict("Findings…\n\nWARNINGS\n"), "warnings");
        assert_eq!(verdict("ok\nclean"), "clean");
        assert_eq!(verdict("x\n`BLOCKED`\n\n"), "blocked");
        assert_eq!(verdict("x\n**Blocked**"), "blocked");
    }

    #[test]
    fn a_report_without_a_verdict_line_reads_as_none() {
        assert_eq!(verdict("CLEAN\nbut then more text"), "none");
        assert_eq!(verdict(""), "none");
        assert_eq!(verdict("The verdict is CLEAN."), "none");
    }

    const INIT: &str = r#"{"type":"system","subtype":"init","apiKeySource":"login","cwd":"C:\\w","session_id":"86d7c6e0-622d-4737-a280-dc35daa236e1","model":"Gemini 3.8 Flash Low","permissionMode":"default"}"#;
    const RESULT: &str = r#"{"type":"result","subtype":"success","duration_ms":18692,"duration_api_ms":18692,"is_error":false,"result":"Report\nWARNINGS","session_id":"86d7c6e0-622d-4737-a280-dc35daa236e1","request_id":"r","usage":{"inputTokens":26476,"outputTokens":1064,"cacheReadTokens":29981,"cacheWriteTokens":0,"reasoningTokens":667}}"#;

    #[test]
    fn the_init_line_gives_the_session() {
        assert_eq!(read_line(INIT), Line::Init { session_id: "86d7c6e0-622d-4737-a280-dc35daa236e1".into() });
        assert_eq!(read_line(r#"{"type":"system","subtype":"init"}"#), Line::Other, "an init without a session");
    }

    #[test]
    fn the_result_line_gives_the_report_and_the_usage() {
        let usage = Usage { input: 26476, output: 1064, cache_read: 29981, cache_write: 0, reasoning: 667 };
        assert_eq!(read_line(RESULT), Line::Result(Outcome { report: "Report\nWARNINGS".into(), is_error: false, usage }));
        let failed = read_line(r#"{"type":"result","subtype":"error","is_error":true,"result":"boom"}"#);
        assert!(matches!(failed, Line::Result(Outcome { is_error: true, .. })), "{failed:?}");
    }

    #[test]
    fn a_missing_usage_reads_as_zero() {
        let Line::Result(bare) = read_line(r#"{"type":"result","is_error":false,"result":"r"}"#) else { panic!("a result") };
        assert_eq!(bare.usage, Usage::default());
        let Line::Result(part) = read_line(r#"{"type":"result","result":"r","usage":{"inputTokens":5}}"#) else { panic!("a result") };
        assert_eq!(part.usage, Usage { input: 5, ..Usage::default() });
    }

    #[test]
    fn other_lines_are_skipped() {
        for line in [
            "",
            "not json",
            r#"{"type":"user","message":{}}"#,
            r#"{"type":"tool_call","subtype":"started"}"#,
            r#"{"type":"assistant","message":{"role":"assistant"}}"#,
        ] {
            assert_eq!(read_line(line), Line::Other, "{line:?}");
        }
    }

    fn setup<'a>(exe: &'a Path, home: &'a Path, allow: &'a [String], test_env: &'a [(&'static str, OsString)]) -> CursorSetup<'a> {
        CursorSetup { server: "clawg-dev", exe, profile: Profile::Dev, agent: "cursor-review", home, allow, test_env }
    }

    #[test]
    fn the_cursor_mcp_file_pins_the_bridge() {
        let (exe, home) = (Path::new("/opt/clawg/clawg"), Path::new("/home/u"));
        let allow = DEFAULT_ALLOW.map(String::from);
        let file: Value = serde_json::from_str(&mcp_file(&setup(exe, home, &allow, &[]))).unwrap();
        let servers = file["mcpServers"].as_object().unwrap();
        assert_eq!(servers.keys().collect::<Vec<_>>(), ["clawg-dev"], "this profile's bridge alone");
        let server = &servers["clawg-dev"];
        assert_eq!(server["command"], "/opt/clawg/clawg");
        assert_eq!(server["args"], json!(["mcp", "--profile", "dev", "--agent", "cursor-review"]));
        assert_eq!(server["env"], json!({ "HOME": "/home/u", "USERPROFILE": "/home/u" }));

        let test_env = [("CLAWG_TEST_HOME", OsString::from("/tmp/t")), ("CLAWG_TEST_EMBEDDER", OsString::from("fake"))];
        let file: Value = serde_json::from_str(&mcp_file(&setup(exe, home, &allow, &test_env))).unwrap();
        assert_eq!(
            file["mcpServers"]["clawg-dev"]["env"],
            json!({ "HOME": "/home/u", "USERPROFILE": "/home/u", "CLAWG_TEST_HOME": "/tmp/t", "CLAWG_TEST_EMBEDDER": "fake" })
        );
    }

    #[test]
    fn the_cursor_config_allows_only_reading() {
        let file: Value = serde_json::from_str(&config_file("clawg", &["git diff".into(), " git log ".into()])).unwrap();
        assert_eq!(
            file["permissions"]["allow"],
            json!([
                "Mcp(clawg:search_memory)",
                "Mcp(clawg:recall_context)",
                "Mcp(clawg:read_memory)",
                "Mcp(clawg:list_memories)",
                "Mcp(clawg:search_project)",
                "Mcp(clawg:memory_status)",
                "Shell(git diff)",
                "Shell(git log)"
            ])
        );
        assert_eq!(file["permissions"]["deny"], json!(["Write(**)", "Shell(* --output*)"]));
        assert!(READ_TOOLS.iter().all(|tool| crate::daemon::sessions::TOOLS.contains(tool)), "each is a tool of the bridge");
        for writes in ["save_memory", "delete_memory", "restore_memory", "update_memory", "resolve_conflict", "answer_suggestion", "synthesize"] {
            assert!(!READ_TOOLS.contains(&writes), "{writes} is not allowed");
        }
    }

    #[test]
    fn the_cursor_runs_with_the_review_folder_as_its_home_and_powershell_cache() {
        let home = Path::new("/data/review/cursor");
        let env = cursor_env(home);
        let names: Vec<&str> = env.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["USERPROFILE", "HOME", "PSModuleAnalysisCachePath"]);
        assert!(env.iter().all(|(_, path)| path.starts_with(home)), "{env:?}");
    }

    #[test]
    fn the_cursor_hooks_file_has_no_hook() {
        let file: Value = serde_json::from_str(&hooks_file()).unwrap();
        assert_eq!(file, json!({ "version": 1, "hooks": {} }));
    }

    #[test]
    fn the_cursor_arguments_follow_the_options() {
        let workspace = Path::new("/work/repo");
        let words = |args: Vec<OsString>| args.into_iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>();
        let base = ["-p", "--trust", "--approve-mcps", "--output-format", "stream-json", "--workspace", "/work/repo"];
        assert_eq!(words(cursor_args(workspace, None, None)), base);
        let all = words(cursor_args(workspace, Some("gpt-5.3-codex"), Some("a01efb1a")));
        assert_eq!(all, [&base[..], &["--model", "gpt-5.3-codex", "--resume", "a01efb1a"]].concat());
        assert!(!all.iter().any(|a| a == "--force" || a == "--yolo"));
    }

    #[test]
    fn the_cursor_cli_is_found_where_its_installer_puts_it() {
        let windows = Path::new("C:/Users/u/AppData/Local/cursor-agent/agent.cmd");
        let found = find_cursor(true, Some(Path::new("C:/Users/u/AppData/Local")), None, |p| p == windows);
        assert_eq!(found.as_deref(), Some(windows));
        let path = std::env::join_paths(["/usr/bin", "/home/u/.local/bin"]).unwrap();
        let unix = Path::new("/home/u/.local/bin/agent");
        assert_eq!(find_cursor(false, None, Some(&path), |p| p == unix).as_deref(), Some(unix));
    }

    #[test]
    fn a_missing_cursor_cli_names_the_option() {
        assert_eq!(find_cursor(true, Some(Path::new("C:/L")), None, |_| false), None);
        assert_eq!(find_cursor(true, None, None, |_| true), None);
        assert_eq!(find_cursor(false, None, None, |_| true), None);
        for lang in [Lang::En, Lang::Es] {
            assert!(missing_cursor(lang).contains("--cursor"), "{lang:?}");
        }
    }

    #[test]
    fn the_model_kept_is_the_one_passed_or_auto() {
        assert_eq!(kept_model(Some("GPT-5.3-Codex")), "gpt-5.3-codex");
        assert_eq!(kept_model(None), AUTO_MODEL);
        assert_eq!(kept_model(Some("  ")), AUTO_MODEL);
    }
}
