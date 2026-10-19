//! `clawg setup claude-code`: connect clawg to Claude Code in one command
//! that can be repeated and undone.
//!
//! Three things are installed: the MCP server, registered through Claude
//! Code's own CLI with the absolute path of this binary and a pinned agent id;
//! the lifecycle hooks in the user `settings.json`; and the memory protocol,
//! between markers, in the user `CLAUDE.md`. `.claude.json` is read to see
//! what is already registered but never written here: Claude Code rewrites it
//! while it runs, and it holds far more than servers.
//!
//! Everything is read first, a pure [`plan`] decides, and [`apply`] only
//! carries the plan out. Nothing that differs from the plan is replaced: a
//! server or a clawg hook that does not match is reported as a conflict, and
//! what is not clawg's is never touched. The one exception is the hook as
//! earlier versions of setup wrote it, a command line for a shell, which is
//! rewritten in place when it is exactly what setup wrote.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use clawg_core::paths::{backups_dir, Profile};
use clawg_core::private_dir::ensure_private_dir;
use serde_json::{Map, Value};

use crate::update::plan::stamp;

/// The agent id the bridge and the hooks are pinned to: the name Claude Code
/// reports of itself, so a user who never pinned it keeps their memories.
pub const AGENT: &str = "claude-code";

pub const BEGIN_MARKER: &str = "<!-- BEGIN CLAWG MEMORY PROTOCOL -->";
pub const END_MARKER: &str = "<!-- END CLAWG MEMORY PROTOCOL -->";

/// What the agent is told about memory, the same text the MCP server's
/// instructions open with.
const PROTOCOL: &str = include_str!("mcp/protocol.md");

const SESSION_START_MATCHER: &str = "startup|resume|clear|compact";

/// Seconds a hook may take before the client cancels it, above the three
/// seconds `clawg hook` gives itself.
const HOOK_TIMEOUT_SECS: u64 = 5;

/// The client lifecycle events `clawg hook` answers, as the client names
/// them and as `clawg hook` takes them.
const HOOK_EVENTS: [(&str, &str); 2] = [("SessionStart", "session-start"), ("UserPromptSubmit", "user-prompt-submit")];

/// Copies of each file kept in `backups/setup`, the newest.
const KEPT_COPIES: usize = 5;

/// Whether setup installs or takes away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Install,
    Remove,
}

/// One configuration file: where it is and its text, `None` when missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub path: PathBuf,
    pub text: Option<String>,
}

/// Everything setup decides from, read before anything is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// The binary the server and the hooks will run.
    pub exe: PathBuf,
    pub profile: Profile,
    /// `.claude.json`, where Claude Code keeps the user's servers.
    pub servers: File,
    /// The user `settings.json`, where the hooks go.
    pub settings: File,
    /// The user `CLAUDE.md`, where the protocol goes.
    pub instructions: File,
}

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("the path of this clawg binary, {0}, is not absolute; run clawg by its full path")]
    RelativeExe(PathBuf),
    #[error("the path of this clawg binary, {0}, is not valid Unicode")]
    NonUnicodeExe(PathBuf),
    #[error("cannot locate the running clawg binary: {0}")]
    CurrentExe(std::io::Error),
    #[error("cannot find the home directory")]
    NoHome,
    #[error("cannot read {path}: {source}; nothing was changed")]
    Read { path: PathBuf, source: std::io::Error },
    #[error("{path} is not valid JSON ({source}); nothing was changed")]
    InvalidJson { path: PathBuf, source: serde_json::Error },
    #[error("{path} is not laid out as Claude Code writes it ({what}); nothing was changed")]
    Unexpected { path: PathBuf, what: &'static str },
    #[error("{path} has one clawg protocol marker without the other; fix or remove it by hand, nothing was changed")]
    LoneMarker { path: PathBuf },
}

/// The registration of the profile's MCP server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub name: &'static str,
    pub command: String,
    pub args: Vec<String>,
}

impl Server {
    fn planned(exe: &str, profile: Profile) -> Server {
        Server {
            name: profile.mcp_server_name(),
            command: exe.to_string(),
            args: ["mcp", "--profile", profile.as_str(), "--agent", AGENT].map(String::from).to_vec(),
        }
    }

    /// The arguments `claude` takes to register it for the user.
    pub fn add_args(&self) -> Vec<String> {
        let mut args: Vec<String> = ["mcp", "add", "--scope", "user", self.name, "--"].map(String::from).to_vec();
        args.push(self.command.clone());
        args.extend(self.args.iter().cloned());
        args
    }

    pub fn remove_args(&self) -> Vec<String> {
        ["mcp", "remove", "--scope", "user", self.name].map(String::from).to_vec()
    }

    /// The registration as a command line a user can paste.
    pub fn add_line(&self) -> String {
        let mut line = format!("claude mcp add --scope user {} -- {}", self.name, display_word(&self.command));
        for arg in &self.args {
            line.push(' ');
            line.push_str(arg);
        }
        line
    }
}

/// What happens to the MCP server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerStep {
    Add,
    Unchanged,
    Remove,
    Absent,
    /// Registered under the profile's name with something else, shown as found.
    Conflict(String),
}

/// What happens to one file: its new text, its removal, or nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStep {
    Write(String),
    Delete,
    Unchanged,
}

/// What happens to the hooks, event by event.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HooksStep {
    pub added: Vec<&'static str>,
    pub unchanged: Vec<&'static str>,
    pub removed: Vec<&'static str>,
    /// Events whose hook, as earlier versions of setup wrote it, is rewritten.
    pub migrated: Vec<&'static str>,
    /// Events holding a clawg hook of this profile that differs from the plan,
    /// with its command.
    pub conflicts: Vec<(&'static str, String)>,
}

/// What happens to the protocol block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockStep {
    Added,
    Rewritten,
    Unchanged,
    Removed,
    Absent,
    /// Left for the other profile's server, still registered.
    Kept,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub mode: Mode,
    pub server: Server,
    pub server_step: ServerStep,
    pub hooks: HooksStep,
    pub settings: FileStep,
    pub block: BlockStep,
    pub instructions: FileStep,
}

/// Decides every step from what was read. Fails, so that nothing is written,
/// when a file cannot be understood.
pub fn plan(state: &State, mode: Mode) -> Result<Plan, SetupError> {
    if !state.exe.is_absolute() {
        return Err(SetupError::RelativeExe(state.exe.clone()));
    }
    let exe = state.exe.to_str().ok_or_else(|| SetupError::NonUnicodeExe(state.exe.clone()))?;
    let server = Server::planned(exe, state.profile);
    let servers = parse_object(&state.servers)?;
    let registered = |name: &str| servers.as_ref().and_then(|s| s.get("mcpServers")).and_then(|m| m.get(name));

    let found = registered(server.name);
    let server_step = match (mode, found) {
        (Mode::Install, None) => ServerStep::Add,
        (Mode::Install, Some(entry)) if matches_server(entry, &server) => ServerStep::Unchanged,
        (Mode::Remove, None) => ServerStep::Absent,
        (Mode::Remove, Some(entry)) if is_clawg_server(entry) => ServerStep::Remove,
        (_, Some(entry)) => ServerStep::Conflict(describe_command(entry)),
    };

    let mut settings = parse_object(&state.settings)?;
    if mode == Mode::Install && settings.is_none() {
        settings = Some(Map::new());
    }
    let hooks = match (mode, settings.as_mut()) {
        (_, None) => HooksStep::default(),
        (Mode::Install, Some(s)) => install_hooks(s, exe, state.profile, &state.settings.path)?,
        (Mode::Remove, Some(s)) => remove_hooks(s, state.profile, &state.settings.path)?,
    };
    let settings_step = if hooks.added.is_empty() && hooks.removed.is_empty() && hooks.migrated.is_empty() {
        FileStep::Unchanged
    } else {
        let value = Value::Object(settings.unwrap_or_default());
        FileStep::Write(serde_json::to_string_pretty(&value).expect("a JSON value serializes") + "\n")
    };

    let other = match state.profile {
        Profile::Dev => Profile::Prod,
        Profile::Prod => Profile::Dev,
    };
    let other_registered = registered(other.mcp_server_name()).is_some_and(is_clawg_server);
    let (block, instructions) = match mode {
        Mode::Install => install_block(&state.instructions)?,
        Mode::Remove if other_registered => {
            find_block(&state.instructions)?;
            (BlockStep::Kept, FileStep::Unchanged)
        }
        Mode::Remove => remove_block(&state.instructions)?,
    };

    Ok(Plan { mode, server, server_step, hooks, settings: settings_step, block, instructions })
}

/// The file's JSON object, `None` when the file is missing.
fn parse_object(file: &File) -> Result<Option<Map<String, Value>>, SetupError> {
    let Some(text) = &file.text else { return Ok(None) };
    if text.trim().is_empty() {
        return Ok(Some(Map::new()));
    }
    match serde_json::from_str(text) {
        Ok(Value::Object(map)) => Ok(Some(map)),
        Ok(_) => Err(SetupError::Unexpected { path: file.path.clone(), what: "not an object" }),
        Err(source) => Err(SetupError::InvalidJson { path: file.path.clone(), source }),
    }
}

fn matches_server(entry: &Value, planned: &Server) -> bool {
    let stdio = entry.get("type").is_none_or(|t| t == "stdio");
    let args = entry.get("args").and_then(Value::as_array).map(|a| a.iter().map(Value::as_str).collect::<Vec<_>>());
    let planned_args: Vec<Option<&str>> = planned.args.iter().map(|a| Some(a.as_str())).collect();
    stdio && entry.get("command").and_then(Value::as_str) == Some(planned.command.as_str()) && args == Some(planned_args)
}

/// A server entry that runs some `clawg` binary's bridge, which is all
/// `--remove` takes away.
fn is_clawg_server(entry: &Value) -> bool {
    let command = entry.get("command").and_then(Value::as_str).unwrap_or_default();
    let first = entry.get("args").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_str);
    is_clawg_program(command) && first == Some("mcp")
}

/// A server entry or a hook as a command line: its `command`, then its `args`.
fn describe_command(entry: &Value) -> String {
    let command = entry.get("command").and_then(Value::as_str);
    let args: Vec<&str> =
        entry.get("args").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    match command {
        Some(command) => std::iter::once(command).chain(args).collect::<Vec<_>>().join(" "),
        None => entry.to_string(),
    }
}

/// Whether a program, by its file name, is a `clawg` binary.
fn is_clawg_program(program: &str) -> bool {
    let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let lower = name.to_ascii_lowercase();
    lower == "clawg" || lower == "clawg.exe"
}

/// What `clawg hook` takes for `event_arg` (`session-start`, …) and `profile`.
fn hook_args(event_arg: &str, profile: Profile) -> Vec<String> {
    ["hook", event_arg, "--agent", AGENT, "--profile", profile.as_str()].map(String::from).to_vec()
}

/// The hook the plan installs: the binary run directly with its arguments,
/// without a shell between the client and clawg, so no shell can misread the
/// path or print ahead of the hook's JSON.
fn planned_hook(exe: &str, event_arg: &str, profile: Profile) -> Value {
    serde_json::json!({ "type": "command", "command": exe, "args": hook_args(event_arg, profile), "timeout": HOOK_TIMEOUT_SECS })
}

/// The command line earlier versions of setup wrote for a hook, `words`
/// being the program and its arguments: the program in double quotes only
/// when it had anything outside `[A-Za-z0-9/._-:]`.
fn legacy_command(words: &[String]) -> String {
    let program = &words[0];
    let mut line = if !program.is_empty() && program.chars().all(|c| c.is_ascii_alphanumeric() || "/._-:".contains(c)) {
        program.clone()
    } else {
        let mut quoted = String::from('"');
        for c in program.chars() {
            if matches!(c, '"' | '$' | '`' | '\\') {
                quoted.push('\\');
            }
            quoted.push(c);
        }
        quoted.push('"');
        quoted
    };
    for word in &words[1..] {
        line.push(' ');
        line.push_str(word);
    }
    line
}

/// A word of a command line shown to the user, quoted when it has a space.
fn display_word(word: &str) -> String {
    if word.contains(char::is_whitespace) {
        format!("\"{word}\"")
    } else {
        word.to_string()
    }
}

/// The words of a hook's command, split as a simple shell would: on
/// whitespace, with single and double quotes and backslash escapes.
fn split_command(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                word.extend(chars.by_ref().take_while(|&c| c != '\''));
            }
            '"' => {
                in_word = true;
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => word.extend(chars.next()),
                        c => word.push(c),
                    }
                }
            }
            '\\' => {
                in_word = true;
                word.extend(chars.next());
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

/// A hook's program and arguments: `command` and `args` when it has `args`,
/// which the client runs without a shell, else `command` split as a shell
/// would. `None` when an argument is not text.
fn hook_words(hook: &Value) -> Option<Vec<String>> {
    let command = hook.get("command").and_then(Value::as_str)?;
    match hook.get("args") {
        None => Some(split_command(command)),
        Some(args) => std::iter::once(Some(command.to_string()))
            .chain(args.as_array()?.iter().map(|a| a.as_str().map(String::from)))
            .collect(),
    }
}

/// Whether a hook is clawg's for `event_arg` (`session-start`, …) and
/// `profile`, in either form. One without `--profile` belongs to no profile.
fn is_clawg_hook(hook: &Value, event_arg: &str, profile: Profile) -> bool {
    let Some(words) = hook_words(hook) else { return false };
    let profile_given = words.windows(2).any(|w| w[0] == "--profile" && w[1] == profile.as_str())
        || words.iter().any(|w| w.strip_prefix("--profile=") == Some(profile.as_str()));
    words.len() >= 3 && is_clawg_program(&words[0]) && words[1] == "hook" && words[2] == event_arg && profile_given
}

/// Whether a hook is, character for character, the command line earlier
/// versions of setup wrote for `event_arg` and `profile`, whatever clawg
/// binary it names. Anything else was edited by hand and is not rewritten.
fn is_legacy_hook(hook: &Value, event_arg: &str, profile: Profile) -> bool {
    let (Some(command), None) = (hook.get("command").and_then(Value::as_str), hook.get("args")) else { return false };
    let words = split_command(command);
    words.len() == 7 && is_clawg_program(&words[0]) && words[1..] == hook_args(event_arg, profile)[..] && legacy_command(&words) == command
}

/// Whether a hook runs what `planned` runs.
fn runs_as(hook: &Value, planned: &Value) -> bool {
    hook.get("command") == planned.get("command") && hook.get("args") == planned.get("args")
}

/// The hooks of an event's groups, in order.
fn hooks_of(groups: &[Value]) -> impl Iterator<Item = &Value> {
    groups.iter().filter_map(|g| g.get("hooks").and_then(Value::as_array)).flatten()
}

/// The `hooks` object of the settings, and the array of one event in it.
fn event_groups<'a>(
    settings: &'a mut Map<String, Value>,
    event: &str,
    create: bool,
    path: &Path,
) -> Result<Option<&'a mut Vec<Value>>, SetupError> {
    if !settings.contains_key("hooks") {
        if !create {
            return Ok(None);
        }
        settings.insert("hooks".into(), Value::Object(Map::new()));
    }
    let hooks = settings
        .get_mut("hooks")
        .and_then(Value::as_object_mut)
        .ok_or(SetupError::Unexpected { path: path.to_path_buf(), what: "`hooks` is not an object" })?;
    if !hooks.contains_key(event) {
        if !create {
            return Ok(None);
        }
        hooks.insert(event.into(), Value::Array(Vec::new()));
    }
    hooks
        .get_mut(event)
        .and_then(Value::as_array_mut)
        .map(Some)
        .ok_or(SetupError::Unexpected { path: path.to_path_buf(), what: "an event under `hooks` is not a list" })
}

fn install_hooks(settings: &mut Map<String, Value>, exe: &str, profile: Profile, path: &Path) -> Result<HooksStep, SetupError> {
    let mut step = HooksStep::default();
    for (event, arg) in HOOK_EVENTS {
        let planned = planned_hook(exe, arg, profile);
        // Checked before anything is created, so an unchanged file stays byte
        // for byte as it was.
        let (mut installed, mut legacy, mut other) = (false, false, None);
        if let Some(groups) = event_groups(settings, event, false, path)? {
            for hook in hooks_of(groups).filter(|h| is_clawg_hook(h, arg, profile)) {
                if runs_as(hook, &planned) {
                    installed = true;
                } else if is_legacy_hook(hook, arg, profile) {
                    legacy = true;
                } else if other.is_none() {
                    other = Some(describe_command(hook));
                }
            }
        }
        if let Some(other) = other {
            step.conflicts.push((event, other));
        } else if legacy {
            let groups = event_groups(settings, event, false, path)?.expect("found above");
            migrate_hooks(groups, &planned, installed, arg, profile);
            step.migrated.push(event);
        } else if installed {
            step.unchanged.push(event);
        } else {
            let groups = event_groups(settings, event, true, path)?.expect("created");
            let mut group = Map::new();
            if event == "SessionStart" {
                group.insert("matcher".into(), SESSION_START_MATCHER.into());
            }
            group.insert("hooks".into(), Value::Array(vec![planned]));
            groups.push(Value::Object(group));
            step.added.push(event);
        }
    }
    Ok(step)
}

/// Puts `planned` where the first hook as setup wrote it was, so its group
/// keeps its place and its matcher, and drops the others: one more would run
/// clawg twice. A group left without hooks by that goes too.
fn migrate_hooks(groups: &mut Vec<Value>, planned: &Value, installed: bool, event_arg: &str, profile: Profile) {
    let mut placed = installed;
    groups.retain_mut(|group| {
        let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) else { return true };
        let before = hooks.len();
        hooks.retain_mut(|hook| {
            if !is_legacy_hook(hook, event_arg, profile) {
                return true;
            }
            if placed {
                return false;
            }
            *hook = planned.clone();
            placed = true;
            true
        });
        !(hooks.len() != before && hooks.is_empty())
    });
}

fn remove_hooks(settings: &mut Map<String, Value>, profile: Profile, path: &Path) -> Result<HooksStep, SetupError> {
    let mut step = HooksStep::default();
    for (event, arg) in HOOK_EVENTS {
        let Some(groups) = event_groups(settings, event, false, path)? else { continue };
        let mut removed = false;
        groups.retain_mut(|group| {
            let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) else { return true };
            let before = hooks.len();
            hooks.retain(|h| !is_clawg_hook(h, arg, profile));
            let emptied = hooks.len() != before;
            removed |= emptied;
            !(emptied && hooks.is_empty())
        });
        if !removed {
            continue;
        }
        step.removed.push(event);
        let hooks = settings.get_mut("hooks").and_then(Value::as_object_mut).expect("checked above");
        if hooks.get(event).and_then(Value::as_array).is_some_and(Vec::is_empty) {
            hooks.remove(event);
        }
        if hooks.is_empty() {
            settings.remove("hooks");
        }
    }
    Ok(step)
}

/// The line ending a file already uses.
fn line_ending(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// The protocol between its markers, without a final line ending.
fn block(eol: &str) -> String {
    let protocol = PROTOCOL.replace("\r\n", "\n");
    let body = protocol.trim_end().replace('\n', eol);
    format!("{BEGIN_MARKER}{eol}{body}{eol}{END_MARKER}")
}

/// Where the block is in the file: from its first marker to the end of its
/// last. A marker without its pair is an error, since there is no knowing
/// where clawg's text ends.
fn find_block(file: &File) -> Result<Option<(usize, usize)>, SetupError> {
    let Some(text) = &file.text else { return Ok(None) };
    let lone = || SetupError::LoneMarker { path: file.path.clone() };
    match (text.find(BEGIN_MARKER), text.find(END_MARKER)) {
        (None, None) => Ok(None),
        (Some(begin), Some(end)) if begin < end => {
            let end = end + END_MARKER.len();
            if text[end..].contains(BEGIN_MARKER) || text[end..].contains(END_MARKER) {
                return Err(lone());
            }
            Ok(Some((begin, end)))
        }
        _ => Err(lone()),
    }
}

fn install_block(file: &File) -> Result<(BlockStep, FileStep), SetupError> {
    let range = find_block(file)?;
    let Some(text) = &file.text else {
        return Ok((BlockStep::Added, FileStep::Write(block("\n") + "\n")));
    };
    let eol = line_ending(text);
    let wanted = block(eol);
    if let Some((begin, end)) = range {
        if text[begin..end] == wanted {
            return Ok((BlockStep::Unchanged, FileStep::Unchanged));
        }
        return Ok((BlockStep::Rewritten, FileStep::Write(format!("{}{wanted}{}", &text[..begin], &text[end..]))));
    }
    let mut out = text.clone();
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push_str(eol);
        }
        out.push_str(eol);
    }
    out.push_str(&wanted);
    out.push_str(eol);
    Ok((BlockStep::Added, FileStep::Write(out)))
}

fn remove_block(file: &File) -> Result<(BlockStep, FileStep), SetupError> {
    let (Some((begin, end)), Some(text)) = (find_block(file)?, &file.text) else {
        return Ok((BlockStep::Absent, FileStep::Unchanged));
    };
    let eol = line_ending(text);
    let before = &text[..begin];
    let after = text[end..].strip_prefix(eol).unwrap_or(&text[end..]);
    // The blank line setup put before the block goes with it.
    let before = before.strip_suffix(eol).filter(|b| b.ends_with('\n') || b.is_empty()).unwrap_or(before);
    let out = format!("{before}{after}");
    let step = if out.is_empty() { FileStep::Delete } else { FileStep::Write(out) };
    Ok((BlockStep::Removed, step))
}

/// Where Claude Code keeps its configuration: `$CLAUDE_CONFIG_DIR` when set,
/// with `.claude.json` inside it; otherwise `~/.claude`, with `.claude.json`
/// beside it in the home directory.
pub fn locations(home: Option<&Path>, config_dir: Option<OsString>) -> Result<(PathBuf, PathBuf, PathBuf), SetupError> {
    let (dir, servers) = match config_dir.filter(|d| !d.is_empty()) {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            let servers = dir.join(".claude.json");
            (dir, servers)
        }
        None => {
            let home = home.ok_or(SetupError::NoHome)?;
            (home.join(".claude"), home.join(".claude.json"))
        }
    };
    Ok((servers, dir.join("settings.json"), dir.join("CLAUDE.md")))
}

fn read(path: PathBuf) -> Result<File, SetupError> {
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(File { path, text: Some(text) }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(File { path, text: None }),
        Err(source) => Err(SetupError::Read { path, source }),
    }
}

/// Reads what [`plan`] decides from.
pub fn read_state(exe: PathBuf, profile: Profile, home: Option<&Path>, config_dir: Option<OsString>) -> Result<State, SetupError> {
    let (servers, settings, instructions) = locations(home, config_dir)?;
    Ok(State { exe, profile, servers: read(servers)?, settings: read(settings)?, instructions: read(instructions)? })
}

/// The first `claude` the PATH resolves to.
pub fn find_claude(path_entries: &[PathBuf], exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["claude.exe", "claude.cmd"] } else { &["claude"] };
    path_entries
        .iter()
        .filter(|dir| !dir.as_os_str().is_empty())
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| exists(candidate))
}

/// How the server step went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerOutcome {
    /// Nothing to run: unchanged, absent, a conflict or a dry run.
    NotRun,
    Ran,
    /// `claude` is not on the PATH.
    Manual,
    Failed(String),
}

/// What carrying the plan out did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub server: ServerOutcome,
    /// Copies of files as they were before being changed.
    pub backups: Vec<PathBuf>,
    /// Older copies removed to keep [`KEPT_COPIES`] of each file.
    pub pruned: Vec<PathBuf>,
    /// A file that could not be written, with why.
    pub failures: Vec<String>,
}

/// Carries the plan out: runs `claude` for the server, then writes the
/// files, each through a temporary file and a rename after copying the old
/// one to `<data>/backups/setup`, where only the newest [`KEPT_COPIES`] of
/// each file stay. A failed step does not stop the others; each can be run
/// again.
pub fn apply(plan: &Plan, state: &State, claude: Option<&Path>, base_dir: &Path, now_ms: i64) -> Applied {
    let args = match plan.server_step {
        ServerStep::Add => Some(plan.server.add_args()),
        ServerStep::Remove => Some(plan.server.remove_args()),
        _ => None,
    };
    let server = match (args, claude) {
        (None, _) => ServerOutcome::NotRun,
        (Some(_), None) => ServerOutcome::Manual,
        (Some(args), Some(claude)) => run_claude(claude, &args),
    };
    let mut applied = Applied { server, backups: Vec::new(), pruned: Vec::new(), failures: Vec::new() };
    for (step, file) in [(&plan.settings, &state.settings), (&plan.instructions, &state.instructions)] {
        if *step == FileStep::Unchanged {
            continue;
        }
        match write_file(step, file, base_dir, now_ms) {
            Ok(Some(backup)) => {
                prune_copies(&copies_dir(base_dir), &copy_name(&file.path), &mut applied);
                applied.backups.push(backup);
            }
            Ok(None) => {}
            Err(e) => applied.failures.push(format!("cannot write {}: {e}", file.path.display())),
        }
    }
    applied
}

fn copies_dir(base_dir: &Path) -> PathBuf {
    backups_dir(base_dir).join("setup")
}

/// The name the copies of `path` start with.
fn copy_name(path: &Path) -> String {
    path.file_name().map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned())
}

/// Of the names in the copies' directory, the copies of `name` beyond the
/// newest `keep`, oldest first. The stamp sorts as text, and a name that is
/// not `<name>-` and a stamp is not a copy setup made.
fn stale_copies(names: &[String], name: &str, keep: usize) -> Vec<String> {
    let is_copy = |n: &&String| {
        let stamp = n.strip_prefix(name).and_then(|rest| rest.strip_prefix('-'));
        stamp.is_some_and(|s| s.len() == 17 && s.bytes().all(|b| b.is_ascii_digit()))
    };
    let mut copies: Vec<&String> = names.iter().filter(is_copy).collect();
    copies.sort();
    let stale = copies.len().saturating_sub(keep);
    copies[..stale].iter().map(|n| n.to_string()).collect()
}

fn prune_copies(dir: &Path, name: &str, applied: &mut Applied) {
    let names: Vec<String> = match std::fs::read_dir(dir) {
        Ok(entries) => entries.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect(),
        Err(e) => {
            applied.failures.push(format!("cannot list the copies in {}: {e}", dir.display()));
            return;
        }
    };
    for stale in stale_copies(&names, name, KEPT_COPIES) {
        let path = dir.join(stale);
        match std::fs::remove_file(&path) {
            Ok(()) => applied.pruned.push(path),
            Err(e) => applied.failures.push(format!("cannot remove the older copy {}: {e}", path.display())),
        }
    }
}

fn run_claude(claude: &Path, args: &[String]) -> ServerOutcome {
    match Command::new(claude).args(args).output() {
        Ok(out) if out.status.success() => ServerOutcome::Ran,
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            let said = if stderr.trim().is_empty() { stdout } else { stderr };
            ServerOutcome::Failed(format!("{} ({})", said.trim(), out.status))
        }
        Err(e) => ServerOutcome::Failed(e.to_string()),
    }
}

fn write_file(step: &FileStep, file: &File, base_dir: &Path, now_ms: i64) -> std::io::Result<Option<PathBuf>> {
    let target = link_target(&file.path)?;
    let backup = match &file.text {
        Some(text) => {
            // Setup can be the first thing run over a data directory an
            // older clawg left open, before any daemon tightens it.
            ensure_private_dir(base_dir)?;
            let dir = copies_dir(base_dir);
            ensure_private_dir(&dir)?;
            let copy = dir.join(format!("{}-{}", copy_name(&file.path), stamp(now_ms)));
            create_fresh(&copy, Some(0o600))?.write_all(text.as_bytes())?;
            Some(copy)
        }
        None => None,
    };
    match (step, target) {
        (FileStep::Write(text), target) => replace(target.as_deref().unwrap_or(&file.path), text)?,
        // Taking away the link, or what it points to, would break whatever
        // manages it, such as a dotfiles repository.
        (FileStep::Delete, Some(target)) => replace(&target, "")?,
        (FileStep::Delete, None) => std::fs::remove_file(&file.path)?,
        (FileStep::Unchanged, _) => {}
    }
    Ok(backup)
}

/// The file `path` points to when it is a symbolic link, so a write lands
/// there and the link stays a link. A broken link is an error.
fn link_target(path: &Path) -> std::io::Result<Option<PathBuf>> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::canonicalize(path).map(Some),
        _ => Ok(None),
    }
}

/// Puts `text` in `path` through a temporary file beside it and a rename, so
/// a reader sees the old file or the new one and never half of it. The
/// temporary file has the mode of the file it replaces from the moment it
/// exists, and reaches the disk before the rename.
fn replace(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".clawg-tmp");
    let tmp = PathBuf::from(tmp);
    let mut out = create_fresh(&tmp, mode_of(path))?;
    out.write_all(text.as_bytes())?;
    out.sync_all()?;
    drop(out);
    std::fs::rename(&tmp, path)
}

#[cfg(unix)]
fn mode_of(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).ok().map(|m| m.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn mode_of(_path: &Path) -> Option<u32> {
    None
}

/// A new, empty file at `path`, replacing any there, with `mode` when given
/// on Unix: set at creation and again after it, since the umask may have
/// taken bits from the first.
#[cfg(unix)]
fn create_fresh(path: &Path, mode: Option<u32>) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    if let Some(mode) = mode {
        options.mode(mode);
    }
    let file = options.open(path)?;
    if let Some(mode) = mode {
        file.set_permissions(std::fs::Permissions::from_mode(mode))?;
    }
    Ok(file)
}

#[cfg(not(unix))]
fn create_fresh(path: &Path, _mode: Option<u32>) -> std::io::Result<std::fs::File> {
    std::fs::File::create(path)
}

/// The report, one line per step, and the exit code: 1 when a step did not
/// end as planned (a conflict, the manual command, a failure).
pub fn report(plan: &Plan, state: &State, applied: Option<&Applied>) -> (String, u8) {
    let dry = applied.is_none();
    let will = |done: &str, planned: &str| if dry { format!("would {planned}") } else { done.to_string() };
    let mut lines = Vec::new();
    let mut incomplete = false;
    let name = plan.server.name;

    let server_line = match (&plan.server_step, applied.map(|a| &a.server)) {
        (ServerStep::Add | ServerStep::Remove, Some(ServerOutcome::Manual)) => {
            incomplete = true;
            if plan.server_step == ServerStep::Add {
                format!("mcp server {name}: claude is not on the PATH; register it with:\n  {}", plan.server.add_line())
            } else {
                format!("mcp server {name}: claude is not on the PATH; remove it with:\n  claude mcp remove --scope user {name}")
            }
        }
        (_, Some(ServerOutcome::Failed(why))) => {
            incomplete = true;
            format!("mcp server {name}: claude failed: {why}")
        }
        (ServerStep::Add, _) => format!("mcp server {name}: {} ({})", will("registered", "register it"), plan.server.add_line()),
        (ServerStep::Remove, _) => format!("mcp server {name}: {}", will("removed", "remove it")),
        (ServerStep::Unchanged, _) => format!("mcp server {name}: already registered"),
        (ServerStep::Absent, _) => format!("mcp server {name}: not registered"),
        (ServerStep::Conflict(found), _) => {
            incomplete = true;
            let fix = match plan.mode {
                Mode::Install => format!(
                    "`clawg setup claude-code --remove` takes away a clawg registration, `claude mcp remove --scope user {name}` any other"
                ),
                Mode::Remove => "it is not clawg's".to_string(),
            };
            format!("mcp server {name}: registered as `{found}`; left as it is ({fix})")
        }
    };
    lines.push(server_line);

    let hooks = &plan.hooks;
    let mut parts = Vec::new();
    if !hooks.added.is_empty() {
        parts.push(format!("{} {}", will("added", "add"), hooks.added.join(", ")));
    }
    if !hooks.removed.is_empty() {
        parts.push(format!("{} {}", will("removed", "remove"), hooks.removed.join(", ")));
    }
    if !hooks.migrated.is_empty() {
        parts.push(format!("{} {} to run without a shell", will("migrated", "migrate"), hooks.migrated.join(", ")));
    }
    if !hooks.unchanged.is_empty() {
        parts.push(format!("{} already there", hooks.unchanged.join(", ")));
    }
    for (event, command) in &hooks.conflicts {
        incomplete = true;
        parts.push(format!("{event} has another clawg hook, `{command}`; left as it is (`clawg setup claude-code --remove` takes it away)"));
    }
    if parts.is_empty() {
        parts.push("none of clawg's".to_string());
    }
    lines.push(format!("hooks in {}: {}", state.settings.path.display(), parts.join("; ")));

    let block = match plan.block {
        BlockStep::Added => will("added", "add it"),
        BlockStep::Rewritten => will("updated", "update it"),
        BlockStep::Removed => will("removed", "remove it"),
        BlockStep::Unchanged => "up to date".to_string(),
        BlockStep::Absent => "not there".to_string(),
        BlockStep::Kept => "kept, the other profile's server still uses it".to_string(),
    };
    lines.push(format!("memory protocol in {}: {block}", state.instructions.path.display()));

    if let Some(applied) = applied {
        for backup in &applied.backups {
            lines.push(format!("previous file copied to {}", backup.display()));
        }
        for copy in &applied.pruned {
            lines.push(format!("older copy removed: {}", copy.display()));
        }
        for failure in &applied.failures {
            incomplete = true;
            lines.push(failure.clone());
        }
    }
    let changed = matches!(plan.server_step, ServerStep::Add | ServerStep::Remove)
        || plan.settings != FileStep::Unchanged
        || plan.instructions != FileStep::Unchanged;
    if changed && !dry {
        lines.push("Open a new Claude Code session for the change to take effect.".to_string());
    }
    (lines.join("\n"), if incomplete { 1 } else { 0 })
}

/// `clawg setup claude-code` from start to end, given where things are.
pub fn run_in(
    state: State,
    mode: Mode,
    dry_run: bool,
    claude: Option<&Path>,
    base_dir: &Path,
    now_ms: i64,
) -> Result<(String, u8), SetupError> {
    let plan = plan(&state, mode)?;
    let applied = (!dry_run).then(|| apply(&plan, &state, claude, base_dir, now_ms));
    Ok(report(&plan, &state, applied.as_ref()))
}

/// `clawg setup claude-code` for `profile`, reading the process's
/// environment: the running binary, the home directory, `CLAUDE_CONFIG_DIR`
/// and the PATH.
pub fn run(profile: Profile, mode: Mode, dry_run: bool) -> Result<(String, u8), SetupError> {
    let base_dir = clawg_core::paths::data_dir(profile).map_err(|_| SetupError::NoHome)?;
    let home = clawg_core::paths::home_dir();
    let state = read_state(current_exe()?, profile, home.as_deref(), std::env::var_os("CLAUDE_CONFIG_DIR"))?;
    let entries: Vec<PathBuf> = std::env::var_os("PATH").map(|raw| std::env::split_paths(&raw).collect()).unwrap_or_default();
    let claude = find_claude(&entries, Path::is_file);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    run_in(state, mode, dry_run, claude.as_deref(), &base_dir, now_ms)
}

/// The running binary's path as a client should launch it: without the
/// `\\?\` prefix Windows may report.
pub fn current_exe() -> Result<PathBuf, SetupError> {
    let exe = std::env::current_exe().map_err(SetupError::CurrentExe)?;
    let text = exe.to_string_lossy();
    Ok(match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => exe,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exe() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\tools\clawg.exe")
        } else {
            PathBuf::from("/opt/clawg/clawg")
        }
    }

    fn file(name: &str, text: Option<&str>) -> File {
        File { path: PathBuf::from(name), text: text.map(String::from) }
    }

    fn state(servers: Option<&str>, settings: Option<&str>, instructions: Option<&str>) -> State {
        State {
            exe: exe(),
            profile: Profile::Dev,
            servers: file(".claude.json", servers),
            settings: file("settings.json", settings),
            instructions: file("CLAUDE.md", instructions),
        }
    }

    fn written(step: &FileStep) -> &str {
        match step {
            FileStep::Write(text) => text,
            other => panic!("expected a write, got {other:?}"),
        }
    }

    /// The state a file is in once `step` is carried out.
    fn after(before: Option<&str>, step: &FileStep) -> Option<String> {
        match step {
            FileStep::Write(text) => Some(text.clone()),
            FileStep::Delete => None,
            FileStep::Unchanged => before.map(String::from),
        }
    }

    fn planned_server_json() -> String {
        serde_json::json!({ "mcpServers": { "clawg-dev": {
            "type": "stdio", "command": exe().to_str().unwrap(),
            "args": ["mcp", "--profile", "dev", "--agent", "claude-code"], "env": {}
        }}})
        .to_string()
    }

    /// The hook the plan installs for `event_arg` in the dev profile.
    fn hook(event_arg: &str) -> Value {
        serde_json::json!({
            "type": "command", "command": exe().to_str().unwrap(),
            "args": ["hook", event_arg, "--agent", "claude-code", "--profile", "dev"], "timeout": 5
        })
    }

    /// A settings file whose events hold these hooks, one group each.
    fn settings_with(hooks: &[(&str, Value)]) -> String {
        let mut events = Map::new();
        for (event, hook) in hooks {
            let groups = events.entry(event.to_string()).or_insert_with(|| Value::Array(Vec::new()));
            groups.as_array_mut().unwrap().push(serde_json::json!({ "hooks": [hook] }));
        }
        serde_json::json!({ "hooks": events }).to_string()
    }

    fn shell_hook(command: &str) -> Value {
        serde_json::json!({ "type": "command", "command": command, "timeout": 5 })
    }

    #[test]
    fn an_empty_state_plans_the_server_the_hooks_and_the_block() {
        let plan = plan(&state(None, None, None), Mode::Install).unwrap();
        assert_eq!(plan.server_step, ServerStep::Add);
        assert_eq!(
            plan.server.add_args(),
            ["mcp", "add", "--scope", "user", "clawg-dev", "--", exe().to_str().unwrap(), "mcp", "--profile", "dev", "--agent", "claude-code"]
        );
        assert_eq!(plan.hooks.added, ["SessionStart", "UserPromptSubmit"]);
        let settings: Value = serde_json::from_str(written(&plan.settings)).unwrap();
        assert_eq!(
            settings,
            serde_json::json!({ "hooks": {
                "SessionStart": [{ "matcher": "startup|resume|clear|compact", "hooks": [hook("session-start")] }],
                "UserPromptSubmit": [{ "hooks": [hook("user-prompt-submit")] }]
            }})
        );
        assert_eq!(plan.block, BlockStep::Added);
        let md = written(&plan.instructions);
        assert!(md.starts_with(&format!("{BEGIN_MARKER}\n# Claw Garden memory protocol\n")), "{md}");
        assert!(md.ends_with(&format!("{END_MARKER}\n")), "{md}");
    }

    #[test]
    fn a_relative_binary_path_is_refused() {
        let mut s = state(None, None, None);
        s.exe = PathBuf::from("target/debug/clawg");
        assert!(matches!(plan(&s, Mode::Install), Err(SetupError::RelativeExe(_))));
    }

    #[test]
    fn a_matching_server_is_left_alone() {
        let plan = plan(&state(Some(&planned_server_json()), None, None), Mode::Install).unwrap();
        assert_eq!(plan.server_step, ServerStep::Unchanged);
    }

    #[test]
    fn a_different_server_is_a_conflict() {
        let servers = r#"{"mcpServers":{"clawg-dev":{"type":"stdio","command":"/old/clawg","args":["mcp","--profile","dev"]}}}"#;
        let s = state(Some(servers), None, None);
        let plan = plan(&s, Mode::Install).unwrap();
        assert_eq!(plan.server_step, ServerStep::Conflict("/old/clawg mcp --profile dev".into()));
        let (text, code) = report(&plan, &s, None);
        assert_eq!(code, 1);
        assert!(text.contains("registered as `/old/clawg mcp --profile dev`; left as it is"), "{text}");
    }

    #[test]
    fn foreign_hooks_and_keys_keep_their_order() {
        let settings = r#"{
  "theme": "dark",
  "hooks": {
    "UserPromptSubmit": [ { "hooks": [ { "type": "command", "command": "~/bin/remind.sh" } ] } ],
    "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "guard" } ] } ]
  },
  "model": "x"
}"#;
        let plan = plan(&state(None, Some(settings), None), Mode::Install).unwrap();
        let value: Value = serde_json::from_str(written(&plan.settings)).unwrap();
        let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["theme", "hooks", "model"]);
        let events: Vec<&String> = value["hooks"].as_object().unwrap().keys().collect();
        assert_eq!(events, ["UserPromptSubmit", "PreToolUse", "SessionStart"]);
        let prompt = value["hooks"]["UserPromptSubmit"].as_array().unwrap();
        assert_eq!(prompt[0]["hooks"][0]["command"], "~/bin/remind.sh");
        assert_eq!(prompt[1]["hooks"][0], hook("user-prompt-submit"));
        assert_eq!(value["hooks"]["PreToolUse"], serde_json::json!([{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "guard" }] }]));
    }

    #[test]
    fn a_different_clawg_hook_is_a_conflict() {
        let settings = r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"\"/old dir/clawg\" hook session-start --agent other --profile dev"}]}]}}"#;
        let plan = plan(&state(None, Some(settings), None), Mode::Install).unwrap();
        assert_eq!(plan.hooks.conflicts, [("SessionStart", "\"/old dir/clawg\" hook session-start --agent other --profile dev".to_string())]);
        assert_eq!(plan.hooks.added, ["UserPromptSubmit"]);
        let value: Value = serde_json::from_str(written(&plan.settings)).unwrap();
        assert_eq!(value["hooks"]["SessionStart"].as_array().unwrap().len(), 1, "the conflicting hook is not replaced");

        let moved = serde_json::json!({ "type": "command", "command": "/old/clawg",
            "args": ["hook", "session-start", "--agent", "claude-code", "--profile", "dev"] });
        let moved = super::plan(&state(None, Some(&settings_with(&[("SessionStart", moved)])), None), Mode::Install).unwrap();
        let shown = "/old/clawg hook session-start --agent claude-code --profile dev".to_string();
        assert_eq!(moved.hooks.conflicts, [("SessionStart", shown)]);
    }

    #[test]
    fn the_hook_runs_the_binary_without_a_shell() {
        let exe = if cfg!(windows) {
            r"C:\Users\Ana María\AppData\Local\Programs\clawg\clawg.exe"
        } else {
            "/home/Ana María/bin/clawg"
        };
        let s = State { exe: PathBuf::from(exe), ..state(None, None, None) };
        let plan = plan(&s, Mode::Install).unwrap();
        let value: Value = serde_json::from_str(written(&plan.settings)).unwrap();
        for (event, arg) in [("SessionStart", "session-start"), ("UserPromptSubmit", "user-prompt-submit")] {
            let hook = &value["hooks"][event][0]["hooks"][0];
            assert_eq!(hook["command"], exe, "the path as it is, unquoted");
            assert_eq!(hook["args"], serde_json::json!(["hook", arg, "--agent", "claude-code", "--profile", "dev"]));
        }
    }

    #[test]
    fn an_old_shell_hook_of_clawg_is_migrated_in_place() {
        let settings = serde_json::json!({ "theme": "dark", "hooks": {
            "SessionStart": [{ "matcher": "startup|resume|clear|compact",
                "hooks": [shell_hook("/old/clawg hook session-start --agent claude-code --profile dev")] }],
            "UserPromptSubmit": [
                { "hooks": [shell_hook(r#""/home/Ana María/clawg" hook user-prompt-submit --agent claude-code --profile dev"#)] },
                { "hooks": [{ "type": "command", "command": "guard" }] }
            ]
        }})
        .to_string();
        let first = plan(&state(None, Some(&settings), None), Mode::Install).unwrap();
        assert_eq!(first.hooks.migrated, ["SessionStart", "UserPromptSubmit"]);
        assert!(first.hooks.added.is_empty() && first.hooks.conflicts.is_empty(), "{:?}", first.hooks);
        let value: Value = serde_json::from_str(written(&first.settings)).unwrap();
        assert_eq!(
            value,
            serde_json::json!({ "theme": "dark", "hooks": {
                "SessionStart": [{ "matcher": "startup|resume|clear|compact", "hooks": [hook("session-start")] }],
                "UserPromptSubmit": [{ "hooks": [hook("user-prompt-submit")] }, { "hooks": [{ "type": "command", "command": "guard" }] }]
            }})
        );
        let s = state(None, Some(&settings), None);
        let (text, code) = report(&first, &s, None);
        assert_eq!(code, 0, "{text}");
        assert!(text.contains("would migrate SessionStart, UserPromptSubmit to run without a shell"), "{text}");

        let second = plan(&state(None, after(Some(&settings), &first.settings).as_deref(), None), Mode::Install).unwrap();
        assert_eq!(second.hooks.unchanged, ["SessionStart", "UserPromptSubmit"]);
        assert_eq!(second.settings, FileStep::Unchanged);

        let both = settings_with(&[
            ("SessionStart", hook("session-start")),
            ("SessionStart", shell_hook("/opt/clawg/clawg hook session-start --agent claude-code --profile dev")),
        ]);
        let plan = plan(&state(None, Some(&both), None), Mode::Install).unwrap();
        assert_eq!(plan.hooks.migrated, ["SessionStart"]);
        let value: Value = serde_json::from_str(written(&plan.settings)).unwrap();
        assert_eq!(value["hooks"]["SessionStart"], serde_json::json!([{ "hooks": [hook("session-start")] }]), "it would run twice");
    }

    #[test]
    fn an_edited_shell_hook_is_still_a_conflict() {
        for edited in [
            "/old/clawg hook session-start --agent claude-code --profile dev --verbose",
            "/old/clawg  hook session-start --agent claude-code --profile dev",
            "'/old/clawg' hook session-start --agent claude-code --profile dev",
            "/old/clawg hook session-start --agent claude-code --profile=dev",
        ] {
            let settings = settings_with(&[("SessionStart", shell_hook(edited))]);
            let plan = plan(&state(None, Some(&settings), None), Mode::Install).unwrap();
            assert_eq!(plan.hooks.conflicts, [("SessionStart", edited.to_string())], "{edited}");
            assert!(plan.hooks.migrated.is_empty(), "{edited}");
            let value: Value = serde_json::from_str(written(&plan.settings)).unwrap();
            assert_eq!(value["hooks"]["SessionStart"], serde_json::json!([{ "hooks": [shell_hook(edited)] }]), "{edited}");
        }
    }

    #[test]
    fn remove_takes_both_forms() {
        let foreign = serde_json::json!({ "type": "command", "command": "echo hi" });
        let exec = serde_json::json!({ "type": "command", "command": "/elsewhere/clawg",
            "args": ["hook", "session-start", "--agent", "a", "--profile", "dev"] });
        let settings = settings_with(&[
            ("SessionStart", exec),
            ("SessionStart", foreign.clone()),
            ("UserPromptSubmit", shell_hook("clawg hook user-prompt-submit --agent a --profile dev")),
        ]);
        let plan = plan(&state(None, Some(&settings), None), Mode::Remove).unwrap();
        assert_eq!(plan.hooks.removed, ["SessionStart", "UserPromptSubmit"]);
        let value: Value = serde_json::from_str(written(&plan.settings)).unwrap();
        assert_eq!(value, serde_json::json!({ "hooks": { "SessionStart": [{ "hooks": [foreign] }] } }));
    }

    #[test]
    fn a_hook_of_the_other_profile_is_not_ours() {
        let prod = "clawg hook session-start --agent claude-code --profile prod";
        let settings = format!(r#"{{"hooks":{{"SessionStart":[{{"hooks":[{{"type":"command","command":"{prod}"}}]}}]}}}}"#);
        let plan_ = plan(&state(None, Some(&settings), None), Mode::Install).unwrap();
        assert_eq!(plan_.hooks.added, ["SessionStart", "UserPromptSubmit"]);
        let removing = plan(&state(None, Some(&settings), None), Mode::Remove).unwrap();
        assert_eq!(removing.settings, FileStep::Unchanged, "the prod hook stays");
    }

    #[test]
    fn the_block_is_appended_and_rewritten_in_place() {
        let mine = "# My rules\n\nUse tabs.\n";
        let plan_ = plan(&state(None, None, Some(mine)), Mode::Install).unwrap();
        let added = written(&plan_.instructions).to_string();
        assert_eq!(added, format!("{mine}\n{}\n", block("\n")));

        let stale = format!("# My rules\n\n{BEGIN_MARKER}\nold protocol\n{END_MARKER}\n\nAfter.\n");
        let plan_ = plan(&state(None, None, Some(&stale)), Mode::Install).unwrap();
        assert_eq!(plan_.block, BlockStep::Rewritten);
        assert_eq!(written(&plan_.instructions), format!("# My rules\n\n{}\n\nAfter.\n", block("\n")));

        let crlf = "line one\r\nline two\r\n";
        let plan_ = plan(&state(None, None, Some(crlf)), Mode::Install).unwrap();
        let text = written(&plan_.instructions);
        assert!(text.starts_with("line one\r\nline two\r\n\r\n<!--"), "{text:?}");
        assert!(!text.replace("\r\n", "").contains('\n'), "every line ends in CRLF");
    }

    #[test]
    fn a_lone_marker_writes_nothing() {
        for text in [format!("x\n{BEGIN_MARKER}\ny\n"), format!("{END_MARKER}\n{BEGIN_MARKER}\n"), format!("{BEGIN_MARKER}\n{END_MARKER}\n{END_MARKER}\n")] {
            for mode in [Mode::Install, Mode::Remove] {
                let err = plan(&state(None, None, Some(&text)), mode).unwrap_err();
                assert!(matches!(err, SetupError::LoneMarker { .. }), "{text:?}: {err}");
            }
        }
    }

    #[test]
    fn an_invalid_settings_file_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let s = State {
            settings: File { path: dir.path().join("settings.json"), text: Some(r#"{"hooks": {"#.into()) },
            instructions: File { path: dir.path().join("CLAUDE.md"), text: None },
            ..state(None, None, None)
        };
        let err = run_in(s, Mode::Install, false, None, dir.path(), 0).unwrap_err();
        assert!(err.to_string().contains("is not valid JSON"), "{err}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "nothing was written");

        let not_a_list = r#"{"hooks":{"SessionStart":{}}}"#;
        assert!(matches!(plan(&state(None, Some(not_a_list), None), Mode::Install), Err(SetupError::Unexpected { .. })));
    }

    #[test]
    fn a_second_run_changes_nothing() {
        let settings = r#"{"theme":"dark"}"#;
        let mine = "Use tabs.\n";
        let first = plan(&state(None, Some(settings), Some(mine)), Mode::Install).unwrap();
        let s = state(
            Some(&planned_server_json()),
            after(Some(settings), &first.settings).as_deref(),
            after(Some(mine), &first.instructions).as_deref(),
        );
        let second = plan(&s, Mode::Install).unwrap();
        assert_eq!(second.server_step, ServerStep::Unchanged);
        assert_eq!(second.hooks.unchanged, ["SessionStart", "UserPromptSubmit"]);
        assert_eq!((&second.settings, &second.instructions), (&FileStep::Unchanged, &FileStep::Unchanged));
        let (text, code) = report(&second, &s, Some(&Applied { server: ServerOutcome::NotRun, backups: vec![], pruned: vec![], failures: vec![] }));
        assert_eq!(code, 0);
        assert!(!text.contains("Open a new"), "{text}");
    }

    #[test]
    fn dry_run_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let s = State {
            settings: File { path: dir.path().join("settings.json"), text: None },
            instructions: File { path: dir.path().join("CLAUDE.md"), text: None },
            ..state(None, None, None)
        };
        let (text, code) = run_in(s, Mode::Install, true, Some(Path::new("/nonexistent/claude")), dir.path(), 0).unwrap();
        assert_eq!(code, 0);
        assert!(text.contains("would register it"), "{text}");
        assert!(text.contains("would add SessionStart, UserPromptSubmit"), "{text}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn remove_after_setup_gives_back_the_files() {
        let foreign = r#"{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "echo hi"
          }
        ]
      }
    ]
  }
}
"#;
        for (settings, mine) in [(None, None), (Some(foreign), Some("Use tabs.\n")), (Some("{}"), Some(""))] {
            let installed = plan(&state(None, settings, mine), Mode::Install).unwrap();
            let s = state(
                Some(&planned_server_json()),
                after(settings, &installed.settings).as_deref(),
                after(mine, &installed.instructions).as_deref(),
            );
            let removed = plan(&s, Mode::Remove).unwrap();
            assert_eq!(removed.server_step, ServerStep::Remove);
            assert_eq!(removed.hooks.removed, ["SessionStart", "UserPromptSubmit"]);
            let settings_back = after(s.settings.text.as_deref(), &removed.settings);
            let expected_settings = match settings {
                None | Some("{}") => Some("{}\n".to_string()),
                Some(text) => Some(text.to_string()),
            };
            assert_eq!(settings_back, expected_settings, "{settings:?}");
            let md_back = after(s.instructions.text.as_deref(), &removed.instructions);
            assert_eq!(md_back.as_deref(), mine.filter(|m| !m.is_empty()), "{mine:?}");
        }
    }

    #[test]
    fn remove_keeps_the_block_while_the_other_profile_is_registered() {
        let both = r#"{"mcpServers":{"clawg":{"command":"/opt/clawg/clawg","args":["mcp","--profile","prod"]},"clawg-dev":{"command":"/x/clawg","args":["mcp"]}}}"#;
        let md = format!("{}\n", block("\n"));
        let plan = plan(&state(Some(both), None, Some(&md)), Mode::Remove).unwrap();
        assert_eq!(plan.server_step, ServerStep::Remove);
        assert_eq!((plan.block, &plan.instructions), (BlockStep::Kept, &FileStep::Unchanged));
    }

    #[test]
    fn remove_leaves_a_foreign_server() {
        let servers = r#"{"mcpServers":{"clawg-dev":{"command":"npx","args":["other-server"]}}}"#;
        let plan = plan(&state(Some(servers), None, None), Mode::Remove).unwrap();
        assert_eq!(plan.server_step, ServerStep::Conflict("npx other-server".into()));
        assert_eq!(plan.block, BlockStep::Absent);
    }

    #[test]
    fn without_claude_the_command_is_printed() {
        let dir = tempfile::tempdir().unwrap();
        let s = State {
            settings: File { path: dir.path().join("claude").join("settings.json"), text: None },
            instructions: File { path: dir.path().join("claude").join("CLAUDE.md"), text: None },
            ..state(None, None, None)
        };
        let (text, code) = run_in(s, Mode::Install, false, None, dir.path(), 0).unwrap();
        assert_eq!(code, 1);
        let line = format!("claude mcp add --scope user clawg-dev -- {} mcp --profile dev --agent claude-code", exe().display());
        assert!(text.contains(&format!("claude is not on the PATH; register it with:\n  {line}")), "{text}");
        assert!(dir.path().join("claude").join("settings.json").is_file(), "the hooks are installed anyway");
        assert!(dir.path().join("claude").join("CLAUDE.md").is_file());
    }

    #[test]
    fn apply_copies_the_previous_file_first() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        std::fs::write(&settings, r#"{"theme":"dark"}"#).unwrap();
        let s = State {
            settings: File { path: settings.clone(), text: Some(r#"{"theme":"dark"}"#.into()) },
            instructions: File { path: dir.path().join("CLAUDE.md"), text: None },
            servers: File { path: dir.path().join(".claude.json"), text: Some(planned_server_json()) },
            ..state(None, None, None)
        };
        let data = dir.path().join("data");
        let (text, code) = run_in(s, Mode::Install, false, None, &data, 1_700_000_000_000).unwrap();
        assert_eq!(code, 0, "{text}");
        let copy = data.join("backups").join("setup").join(format!("settings.json-{}", stamp(1_700_000_000_000)));
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), r#"{"theme":"dark"}"#);
        assert!(text.contains(&format!("previous file copied to {}", copy.display())), "{text}");
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(value["hooks"]["SessionStart"][0]["hooks"][0], hook("session-start"));
        assert!(temporaries(dir.path()).is_empty());
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[cfg(unix)]
    fn chmod(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    /// The names in `dir` a write left behind.
    fn temporaries(dir: &Path) -> Vec<std::ffi::OsString> {
        let names = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name());
        names.filter(|n| n.to_string_lossy().ends_with(".clawg-tmp")).collect()
    }

    /// The files as read from `config`, as `read_state` reads them.
    #[cfg(unix)]
    fn state_in(config: &Path, servers: Option<&str>) -> State {
        let read = |name: &str| super::read(config.join(name)).unwrap();
        State {
            servers: File { path: config.join(".claude.json"), text: servers.map(String::from) },
            settings: read("settings.json"),
            instructions: read("CLAUDE.md"),
            ..state(None, None, None)
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_file_is_written_through_its_link() {
        let dir = tempfile::tempdir().unwrap();
        let (config, dotfiles, data) = (dir.path().join("config"), dir.path().join("dotfiles"), dir.path().join("data"));
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(&dotfiles).unwrap();
        std::fs::write(dotfiles.join("settings.json"), r#"{"theme":"dark"}"#).unwrap();
        chmod(&dotfiles.join("settings.json"), 0o600);
        std::fs::write(dotfiles.join("CLAUDE.md"), "").unwrap();
        std::os::unix::fs::symlink(dotfiles.join("settings.json"), config.join("settings.json")).unwrap();
        std::os::unix::fs::symlink(dotfiles.join("CLAUDE.md"), config.join("CLAUDE.md")).unwrap();

        let (text, code) = run_in(state_in(&config, Some(&planned_server_json())), Mode::Install, false, None, &data, 0).unwrap();
        assert_eq!(code, 0, "{text}");
        for name in ["settings.json", "CLAUDE.md"] {
            let link = config.join(name);
            assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "{name} is still a link");
            assert_eq!(std::fs::read_link(&link).unwrap(), dotfiles.join(name));
        }
        let value: Value = serde_json::from_str(&std::fs::read_to_string(dotfiles.join("settings.json")).unwrap()).unwrap();
        assert_eq!(value["hooks"]["SessionStart"][0]["hooks"][0], hook("session-start"));
        assert_eq!(mode(&dotfiles.join("settings.json")), 0o600);
        assert!(std::fs::read_to_string(dotfiles.join("CLAUDE.md")).unwrap().starts_with(BEGIN_MARKER));
        assert!(temporaries(&config).is_empty() && temporaries(&dotfiles).is_empty());

        let (text, code) = run_in(state_in(&config, None), Mode::Remove, false, None, &data, 1).unwrap();
        assert_eq!(code, 0, "{text}");
        assert!(std::fs::symlink_metadata(config.join("CLAUDE.md")).unwrap().file_type().is_symlink(), "removing the block keeps the link");
        assert_eq!(std::fs::read_to_string(dotfiles.join("CLAUDE.md")).unwrap(), "", "and leaves what it points to empty");

        let dangling = dir.path().join("elsewhere");
        std::fs::create_dir_all(&dangling).unwrap();
        std::fs::remove_file(config.join("CLAUDE.md")).unwrap();
        std::os::unix::fs::symlink(dangling.join("missing").join("CLAUDE.md"), config.join("CLAUDE.md")).unwrap();
        let (text, code) = run_in(state_in(&config, Some(&planned_server_json())), Mode::Install, false, None, &data, 2).unwrap();
        assert_eq!(code, 1, "{text}");
        assert!(text.contains(&format!("cannot write {}", config.join("CLAUDE.md").display())), "{text}");
        assert!(std::fs::symlink_metadata(config.join("CLAUDE.md")).unwrap().file_type().is_symlink(), "a broken link is left as it is");
    }

    #[cfg(unix)]
    #[test]
    fn a_file_keeps_its_mode_and_copies_are_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let (config, data) = (dir.path().join("config"), dir.path().join("data"));
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("settings.json"), r#"{"theme":"dark"}"#).unwrap();
        chmod(&config.join("settings.json"), 0o640);
        std::fs::write(config.join("CLAUDE.md"), "Use tabs.\n").unwrap();
        chmod(&config.join("CLAUDE.md"), 0o600);

        let (text, code) = run_in(state_in(&config, Some(&planned_server_json())), Mode::Install, false, None, &data, 0).unwrap();
        assert_eq!(code, 0, "{text}");
        assert_eq!(mode(&config.join("settings.json")), 0o640);
        assert_eq!(mode(&config.join("CLAUDE.md")), 0o600);
        let copies: Vec<PathBuf> = std::fs::read_dir(data.join("backups").join("setup")).unwrap().map(|e| e.unwrap().path()).collect();
        assert_eq!(copies.len(), 2, "{copies:?}");
        for copy in copies {
            assert_eq!(mode(&copy), 0o600, "{}", copy.display());
        }
    }

    #[cfg(unix)]
    #[test]
    fn setup_copies_go_in_a_private_directory() {
        let dir = tempfile::tempdir().unwrap();
        let (config, data) = (dir.path().join("config"), dir.path().join("data"));
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("settings.json"), r#"{"theme":"dark"}"#).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        chmod(&data, 0o755);

        let (text, code) = run_in(state_in(&config, Some(&planned_server_json())), Mode::Install, false, None, &data, 0).unwrap();

        assert_eq!(code, 0, "{text}");
        assert_eq!(mode(&data), 0o700);
        assert_eq!(mode(&data.join("backups")), 0o700);
        assert_eq!(mode(&data.join("backups").join("setup")), 0o700);
    }

    #[test]
    fn stale_copies_are_the_oldest_beyond_the_limit() {
        let copy = |name: &str, ms: i64| format!("{name}-{}", stamp(ms));
        let mut names: Vec<String> = (1..=7).rev().map(|ms| copy("settings.json", ms)).collect();
        names.extend([copy("CLAUDE.md", 0), "settings.json-old".into(), "settings.json.bak".into(), copy("settings.json", 8) + "x"]);
        assert_eq!(stale_copies(&names, "settings.json", 5), [copy("settings.json", 1), copy("settings.json", 2)]);
        assert!(stale_copies(&names, "CLAUDE.md", 5).is_empty());
    }

    #[test]
    fn only_the_newest_copies_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let copies = data.join("backups").join("setup");
        std::fs::create_dir_all(&copies).unwrap();
        for ms in 1..=6 {
            std::fs::write(copies.join(format!("settings.json-{}", stamp(ms))), "old").unwrap();
        }
        std::fs::write(copies.join(format!("CLAUDE.md-{}", stamp(1))), "old").unwrap();
        std::fs::write(copies.join("notes.txt"), "mine").unwrap();
        let s = State {
            settings: File { path: dir.path().join("settings.json"), text: Some("{}".into()) },
            instructions: File { path: dir.path().join("CLAUDE.md"), text: None },
            servers: File { path: dir.path().join(".claude.json"), text: Some(planned_server_json()) },
            ..state(None, None, None)
        };
        let (text, code) = run_in(s, Mode::Install, false, None, &data, 1_700_000_000_000).unwrap();
        assert_eq!(code, 0, "{text}");
        let mut left: Vec<String> = std::fs::read_dir(&copies).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        let mut expected: Vec<String> = (3..=6).map(|ms| format!("settings.json-{}", stamp(ms))).collect();
        expected.extend([format!("settings.json-{}", stamp(1_700_000_000_000)), format!("CLAUDE.md-{}", stamp(1)), "notes.txt".into()]);
        expected.sort();
        assert_eq!(left, expected);
        for ms in [1, 2] {
            let gone = copies.join(format!("settings.json-{}", stamp(ms)));
            assert!(text.contains(&format!("older copy removed: {}", gone.display())), "{text}");
        }
    }

    #[test]
    fn locations_follow_claude_config_dir() {
        let home = Path::new("/home/u");
        assert_eq!(
            locations(Some(home), None).unwrap(),
            (home.join(".claude.json"), home.join(".claude").join("settings.json"), home.join(".claude").join("CLAUDE.md"))
        );
        let cfg = PathBuf::from("/cfg");
        assert_eq!(
            locations(None, Some(cfg.clone().into_os_string())).unwrap(),
            (cfg.join(".claude.json"), cfg.join("settings.json"), cfg.join("CLAUDE.md"))
        );
        assert!(matches!(locations(None, Some(OsString::new())), Err(SetupError::NoHome)));
    }

    #[test]
    fn find_claude_takes_the_first_on_the_path() {
        let name = if cfg!(windows) { "claude.exe" } else { "claude" };
        let entries = [PathBuf::new(), PathBuf::from("/a"), PathBuf::from("/b")];
        let found = find_claude(&entries, |p| p.starts_with("/b") || p == Path::new("/c").join(name));
        assert_eq!(found, Some(PathBuf::from("/b").join(name)));
        assert_eq!(find_claude(&entries, |_| false), None);
    }

    #[test]
    fn a_hook_command_splits_like_a_shell() {
        assert_eq!(split_command(r#""/my dir/clawg" hook  session-start --profile 'dev'"#), ["/my dir/clawg", "hook", "session-start", "--profile", "dev"]);
        assert_eq!(split_command(r"a\ b c"), ["a b", "c"]);
        assert_eq!(split_command(r#""" x"#), ["", "x"]);
        let ours = |hook: Value| is_clawg_hook(&hook, "session-start", Profile::Dev);
        assert!(ours(shell_hook(r#""C:/Program Files/clawg/clawg.exe" hook session-start --agent a --profile=dev"#)));
        assert!(!ours(shell_hook("clawg hook session-start --agent a")), "no profile");
        assert!(!ours(shell_hook("clawgx hook session-start --profile dev")));
        assert!(!ours(shell_hook("clawg hook user-prompt-submit --profile dev")));
        assert!(ours(serde_json::json!({ "command": r"C:\x\clawg.exe", "args": ["hook", "session-start", "--profile=dev"] })));
        assert!(!ours(serde_json::json!({ "command": "clawg hook session-start --profile dev", "args": [] })), "args make command a program");
        assert!(!ours(serde_json::json!({ "command": "/x/clawg", "args": ["hook", "session-start", "--profile", 1] })));
    }

    #[test]
    fn the_old_command_is_quoted_as_setup_quoted_it() {
        let args = |program: &str| [program, "hook", "session-start"].map(String::from);
        assert_eq!(legacy_command(&args("/opt/clawg/clawg")), "/opt/clawg/clawg hook session-start");
        assert_eq!(legacy_command(&args("C:/Users/me/clawg.exe")), "C:/Users/me/clawg.exe hook session-start");
        assert_eq!(legacy_command(&args("/home/a b/clawg")), "\"/home/a b/clawg\" hook session-start");
        assert_eq!(legacy_command(&args("/x/$HOME`/clawg")), "\"/x/\\$HOME\\`/clawg\" hook session-start");
        for word in ["/opt/clawg/clawg", "/home/a b/clawg", "/x/$HOME`/clawg"] {
            assert_eq!(split_command(&legacy_command(&args(word)))[0], word, "a quoted word reads back as itself");
        }
    }
}
