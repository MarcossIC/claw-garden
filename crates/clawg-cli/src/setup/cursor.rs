//! `clawg setup cursor`: connect clawg to Cursor in one command that can be
//! repeated and undone.
//!
//! Two files of `~/.cursor` change: `mcp.json`, where the MCP server goes
//! with the absolute path of this binary and a pinned agent id, and
//! `hooks.json`, where the `sessionStart` hook goes. Cursor has no command
//! that registers a server, so both are written here, as `setup claude-code`
//! writes `settings.json`: a pure [`plan`] over what was read, then
//! [`apply`], and nothing that differs from the plan is replaced. There is no
//! rules file, since Cursor reads the server's instructions, and no hook
//! before a prompt, since Cursor's `beforeSubmitPrompt` cannot add to the
//! context.

use std::path::{Path, PathBuf};

use clawg_core::paths::Profile;
use clawg_core::preferences::{self, Lang};
use serde_json::{Map, Value};

use super::{
    current_exe, describe_command, done_or_planned, event_groups, file_lines, is_clawg_server, matches_server, now_ms,
    parse_object, read, runs_clawg_hook, sh_word, split_command, write_files, File, FileStep, Mode, Server, SetupError,
    Written,
};

/// The agent id the bridge and the hook are pinned to.
pub const AGENT: &str = "cursor";

/// The one event clawg installs, as Cursor names it, and as `clawg hook`
/// takes it.
const EVENT: &str = "sessionStart";
const EVENT_ARG: &str = "session-start";

/// How Cursor's files are expected to be laid out, as an error says it.
const LAYOUT: &str = "as Cursor reads it";

/// The shell a Cursor hook's command runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Posix,
    PowerShell,
}

impl Shell {
    /// The one Cursor runs a hook in on this system.
    pub const HOST: Shell = if cfg!(windows) { Shell::PowerShell } else { Shell::Posix };

    /// The words of a command line as this shell splits it, without
    /// PowerShell's call operator in front.
    pub fn words(self, command: &str) -> Vec<String> {
        match self {
            Shell::Posix => split_command(command),
            Shell::PowerShell => {
                let mut words = split_powershell(command);
                if words.first().is_some_and(|w| w == "&") {
                    words.remove(0);
                }
                words
            }
        }
    }
}

/// The words of a command line as PowerShell splits it: on whitespace, with
/// single quotes (`''` inside them is one quote), double quotes (`""` or a
/// backtick escape inside them) and backtick escapes.
fn split_powershell(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                while let Some(c) = chars.next() {
                    match c {
                        '\'' if chars.peek() == Some(&'\'') => {
                            chars.next();
                            word.push('\'');
                        }
                        '\'' => break,
                        c => word.push(c),
                    }
                }
            }
            '"' => {
                in_word = true;
                while let Some(c) = chars.next() {
                    match c {
                        '`' => word.extend(chars.next()),
                        '"' if chars.peek() == Some(&'"') => {
                            chars.next();
                            word.push('"');
                        }
                        '"' => break,
                        c => word.push(c),
                    }
                }
            }
            '`' => {
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

/// `word` in PowerShell's double quotes, where nothing expands: `` ` ``, `$`
/// and `"` escaped by a backtick.
fn powershell_word(word: &str) -> String {
    let mut quoted = String::from('"');
    for c in word.chars() {
        if matches!(c, '`' | '$' | '"') {
            quoted.push('`');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

/// What `clawg hook` takes for Cursor's `sessionStart` and `profile`.
fn hook_args(profile: Profile) -> [&'static str; 8] {
    ["hook", EVENT_ARG, "--agent", AGENT, "--client", "cursor", "--profile", profile.as_str()]
}

/// The command line of the hook, for the shell Cursor runs it in: PowerShell
/// takes the call operator and the path always quoted, a POSIX shell the path
/// quoted only when it needs it.
pub fn hook_line(exe: &str, profile: Profile, shell: Shell) -> String {
    let args = hook_args(profile).join(" ");
    match shell {
        Shell::Posix => format!("{} {args}", sh_word(exe)),
        Shell::PowerShell => format!("& {} {args}", powershell_word(exe)),
    }
}

/// Whether an entry of `sessionStart` is clawg's for `profile`.
fn is_clawg_hook(hook: &Value, profile: Profile, shell: Shell) -> bool {
    let Some(command) = hook.get("command").and_then(Value::as_str) else { return false };
    runs_clawg_hook(&shell.words(command), EVENT_ARG, profile)
}

/// Everything setup decides from, read before anything is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// The binary the server and the hook will run.
    pub exe: PathBuf,
    pub profile: Profile,
    /// `~/.cursor/mcp.json`, where the server goes.
    pub servers: File,
    /// `~/.cursor/hooks.json`, where the hook goes.
    pub hooks: File,
    /// The profile's preferred language, which the report speaks.
    pub language: Option<Lang>,
}

/// What happens to the server or to the hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Add,
    Unchanged,
    Remove,
    Absent,
    /// Something of clawg's, or not, that differs from the plan, shown as found.
    Conflict(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub mode: Mode,
    /// The profile's server name, under `mcpServers`.
    pub server: &'static str,
    pub server_step: Step,
    pub servers: FileStep,
    pub hook_step: Step,
    pub hooks: FileStep,
}

/// Decides both steps from what was read, the hook's line for `shell`.
/// Fails, so that nothing is written, when a file cannot be understood.
pub fn plan(state: &State, mode: Mode, shell: Shell) -> Result<Plan, SetupError> {
    if !state.exe.is_absolute() {
        return Err(SetupError::RelativeExe(state.exe.clone()));
    }
    let exe = state.exe.to_str().ok_or_else(|| SetupError::NonUnicodeExe(state.exe.clone()))?;
    let mut servers = parse_object(&state.servers, LAYOUT)?;
    let mut hooks = parse_object(&state.hooks, LAYOUT)?;
    let creates_hooks = state.hooks.text.as_deref().is_none_or(|t| t.trim().is_empty());

    let server = Server {
        name: state.profile.mcp_server_name(),
        command: exe.to_string(),
        args: ["mcp", "--profile", state.profile.as_str(), "--agent", AGENT].map(String::from).to_vec(),
    };
    let server_step = server_step(&mut servers, &server, mode, &state.servers.path)?;
    let planned = serde_json::json!({ "command": hook_line(exe, state.profile, shell) });
    let hook_step = hook_step(&mut hooks, &planned, creates_hooks, mode, state.profile, shell, &state.hooks.path)?;

    let write = |step: &Step, map: Option<Map<String, Value>>| match step {
        Step::Add | Step::Remove => {
            FileStep::Write(serde_json::to_string_pretty(&Value::Object(map.unwrap_or_default())).expect("a JSON value serializes") + "\n")
        }
        _ => FileStep::Unchanged,
    };
    Ok(Plan {
        mode,
        server: server.name,
        servers: write(&server_step, servers),
        server_step,
        hooks: write(&hook_step, hooks),
        hook_step,
    })
}

/// The profile's entry under `mcpServers`: added, left, taken away or found
/// to be something else. `servers` is the file's object, changed in place.
fn server_step(servers: &mut Option<Map<String, Value>>, planned: &Server, mode: Mode, path: &Path) -> Result<Step, SetupError> {
    let unexpected = || SetupError::Unexpected { path: path.to_path_buf(), layout: LAYOUT, what: "`mcpServers` is not an object" };
    let entries = match servers.as_mut().and_then(|s| s.get_mut("mcpServers")) {
        Some(entries) => Some(entries.as_object_mut().ok_or_else(unexpected)?),
        None => None,
    };
    let found = entries.as_ref().and_then(|e| e.get(planned.name));
    Ok(match (mode, found) {
        (Mode::Install, None) => {
            let entry = serde_json::json!({ "type": "stdio", "command": planned.command, "args": planned.args });
            let servers = servers.get_or_insert_with(Map::new);
            let entries = servers.entry("mcpServers").or_insert_with(|| Value::Object(Map::new()));
            entries.as_object_mut().expect("checked above").insert(planned.name.into(), entry);
            Step::Add
        }
        (Mode::Install, Some(entry)) if matches_server(entry, planned) => Step::Unchanged,
        (Mode::Remove, None) => Step::Absent,
        (Mode::Remove, Some(entry)) if is_clawg_server(entry) => {
            let servers = servers.as_mut().expect("found in it");
            let entries = servers.get_mut("mcpServers").and_then(Value::as_object_mut).expect("found in it");
            entries.remove(planned.name);
            if entries.is_empty() {
                servers.remove("mcpServers");
            }
            Step::Remove
        }
        (_, Some(entry)) => Step::Conflict(describe_command(entry)),
    })
}

/// clawg's entry under `hooks.sessionStart`: added, left, taken away or
/// found to be another of clawg's. `hooks` is the file's object, changed in
/// place; one that is created starts with `"version": 1`.
fn hook_step(
    hooks: &mut Option<Map<String, Value>>,
    planned: &Value,
    creates: bool,
    mode: Mode,
    profile: Profile,
    shell: Shell,
    path: &Path,
) -> Result<Step, SetupError> {
    let ours = |hook: &&Value| is_clawg_hook(hook, profile, shell);
    let listed = match hooks.as_mut() {
        Some(map) => event_groups(map, EVENT, false, path, LAYOUT)?,
        None => None,
    };
    match mode {
        Mode::Install => {
            let (mut installed, mut other) = (false, None);
            for hook in listed.iter().flat_map(|l| l.iter()).filter(ours) {
                if hook.get("command") == planned.get("command") {
                    installed = true;
                } else if other.is_none() {
                    other = Some(hook.get("command").and_then(Value::as_str).unwrap_or_default().to_string());
                }
            }
            if let Some(other) = other {
                return Ok(Step::Conflict(other));
            }
            if installed {
                return Ok(Step::Unchanged);
            }
            let map = hooks.get_or_insert_with(Map::new);
            if creates && !map.contains_key("version") {
                map.insert("version".into(), Value::from(1));
            }
            event_groups(map, EVENT, true, path, LAYOUT)?.expect("created").push(planned.clone());
            Ok(Step::Add)
        }
        Mode::Remove => {
            let Some(listed) = listed else { return Ok(Step::Absent) };
            let before = listed.len();
            listed.retain(|hook| !ours(&hook));
            if listed.len() == before {
                return Ok(Step::Absent);
            }
            let map = hooks.as_mut().expect("listed from it");
            let events = map.get_mut("hooks").and_then(Value::as_object_mut).expect("listed from it");
            if events.get(EVENT).and_then(Value::as_array).is_some_and(Vec::is_empty) {
                events.remove(EVENT);
            }
            if events.is_empty() {
                map.remove("hooks");
            }
            Ok(Step::Remove)
        }
    }
}

/// Where Cursor keeps its user configuration: `~/.cursor/mcp.json` and
/// `~/.cursor/hooks.json`.
pub fn locations(home: Option<&Path>) -> Result<(PathBuf, PathBuf), SetupError> {
    let dir = home.ok_or(SetupError::NoHome)?.join(".cursor");
    Ok((dir.join("mcp.json"), dir.join("hooks.json")))
}

/// Whether `text`, a `hooks.json`, has `profile`'s hook under `sessionStart`.
fn installed_in(text: &str, profile: Profile, shell: Shell) -> bool {
    let Ok(Value::Object(file)) = serde_json::from_str::<Value>(text) else { return false };
    file.get("hooks")
        .and_then(|hooks| hooks.get(EVENT))
        .and_then(Value::as_array)
        .is_some_and(|listed| listed.iter().any(|hook| is_clawg_hook(hook, profile, shell)))
}

/// Whether the `hooks.json` under `home` has the hook `setup cursor` writes
/// for `profile`; a missing or unreadable file says no.
pub fn hook_installed(home: Option<&Path>, profile: Profile) -> bool {
    let Ok((_, hooks)) = locations(home) else { return false };
    std::fs::read_to_string(hooks).is_ok_and(|text| installed_in(&text, profile, Shell::HOST))
}

/// Reads what [`plan`] decides from, the preferred language from `data_dir`.
pub fn read_state(exe: PathBuf, profile: Profile, home: Option<&Path>, data_dir: &Path) -> Result<State, SetupError> {
    let (servers, hooks) = locations(home)?;
    Ok(State { exe, profile, servers: read(servers)?, hooks: read(hooks)?, language: preferences::language_in(data_dir) })
}

/// Carries the plan out: each file that changes, copied first to
/// `<data>/backups/setup` and written through a temporary file.
pub fn apply(plan: &Plan, state: &State, base_dir: &Path, now_ms: i64) -> Written {
    write_files(&[(&plan.servers, &state.servers), (&plan.hooks, &state.hooks)], base_dir, now_ms)
}

/// The report, one line per file, and the exit code: 1 when a step did not
/// end as planned (a conflict, a failure). It speaks the profile's preferred
/// language; the paths and names it gives do not change.
pub fn report(plan: &Plan, state: &State, written: Option<&Written>) -> (String, u8) {
    let lang = state.language.unwrap_or(Lang::En);
    let dry = written.is_none();
    let will = |en: (&str, &str), es: (&str, &str)| done_or_planned(lang, dry, en, es);
    let mut incomplete = false;
    let mut lines = Vec::new();

    let name = plan.server;
    let servers = state.servers.path.display();
    let server = match &plan.server_step {
        Step::Add => will(("added", "add it"), ("agregado", "se agregaría")),
        Step::Remove => will(("removed", "remove it"), ("quitado", "se quitaría")),
        Step::Unchanged => match lang {
            Lang::En => "already there".to_string(),
            Lang::Es => "ya está".to_string(),
        },
        Step::Absent => match lang {
            Lang::En => "not there".to_string(),
            Lang::Es => "no está".to_string(),
        },
        Step::Conflict(found) => {
            incomplete = true;
            match (plan.mode, lang) {
                (Mode::Install, Lang::En) => format!(
                    "registered as `{found}`; left as it is (`clawg setup cursor --remove` takes away a clawg \
                     registration, and any other is edited in the file)"
                ),
                (Mode::Install, Lang::Es) => format!(
                    "registrado como `{found}`; queda como está (`clawg setup cursor --remove` quita un registro de \
                     clawg, y cualquier otro se edita en el archivo)"
                ),
                (Mode::Remove, Lang::En) => format!("registered as `{found}`; left as it is (it is not clawg's)"),
                (Mode::Remove, Lang::Es) => format!("registrado como `{found}`; queda como está (no es de clawg)"),
            }
        }
    };
    lines.push(match lang {
        Lang::En => format!("mcp server {name} in {servers}: {server}"),
        Lang::Es => format!("servidor mcp {name} en {servers}: {server}"),
    });

    let hooks = state.hooks.path.display();
    let hook = match &plan.hook_step {
        Step::Add => will(("added", "add it"), ("agregado", "se agregaría")),
        Step::Remove => will(("removed", "remove it"), ("quitado", "se quitaría")),
        Step::Unchanged => match lang {
            Lang::En => "already there".to_string(),
            Lang::Es => "ya está".to_string(),
        },
        Step::Absent => match lang {
            Lang::En => "none of clawg's".to_string(),
            Lang::Es => "ninguno de clawg".to_string(),
        },
        Step::Conflict(found) => {
            incomplete = true;
            match lang {
                Lang::En => format!(
                    "another clawg hook is there, `{found}`; left as it is (`clawg setup cursor --remove` takes it away)"
                ),
                Lang::Es => format!("hay otro hook de clawg, `{found}`; queda como está (`clawg setup cursor --remove` lo quita)"),
            }
        }
    };
    lines.push(match lang {
        Lang::En => format!("{EVENT} hook in {hooks}: {hook}"),
        Lang::Es => format!("hook {EVENT} en {hooks}: {hook}"),
    });

    if let Some(written) = written {
        incomplete |= file_lines(lang, &written.backups, &written.pruned, &written.failures, &mut lines);
    }
    if plan.mode == Mode::Install {
        lines.push(
            match lang {
                Lang::En => {
                    "Cursor: turn off \"Include Third-Party Plugins, Skills, and Other Configs\" (Settings → Agents → \
                     Third-Party Imports). While it is on, Cursor also loads Claude Code's hooks and plugins, and \
                     clawg cannot read that setting."
                }
                Lang::Es => {
                    "Cursor: apaga \"Include Third-Party Plugins, Skills, and Other Configs\" (Settings → Agents → \
                     Third-Party Imports). Mientras esté encendido, Cursor también carga los hooks y plugins de Claude \
                     Code, y clawg no puede leer ese ajuste."
                }
            }
            .to_string(),
        );
    }
    let changed = plan.servers != FileStep::Unchanged || plan.hooks != FileStep::Unchanged;
    if changed && !dry {
        lines.push(
            match lang {
                Lang::En => "Restart Cursor for the change to take effect.",
                Lang::Es => "Reinicia Cursor para que el cambio tenga efecto.",
            }
            .to_string(),
        );
    }
    (lines.join("\n"), if incomplete { 1 } else { 0 })
}

/// `clawg setup cursor` from start to end, given where things are.
pub fn run_in(state: State, mode: Mode, dry_run: bool, shell: Shell, base_dir: &Path, now_ms: i64) -> Result<(String, u8), SetupError> {
    let plan = plan(&state, mode, shell)?;
    let written = (!dry_run).then(|| apply(&plan, &state, base_dir, now_ms));
    Ok(report(&plan, &state, written.as_ref()))
}

/// `clawg setup cursor` for `profile`, reading the process's environment:
/// the running binary and the home directory.
pub fn run(profile: Profile, mode: Mode, dry_run: bool) -> Result<(String, u8), SetupError> {
    let base_dir = clawg_core::paths::data_dir(profile).map_err(|_| SetupError::NoHome)?;
    let home = clawg_core::paths::home_dir();
    let state = read_state(current_exe()?, profile, home.as_deref(), &base_dir)?;
    run_in(state, mode, dry_run, Shell::HOST, &base_dir, now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::plan::stamp;

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

    fn state(servers: Option<&str>, hooks: Option<&str>) -> State {
        State { exe: exe(), profile: Profile::Dev, servers: file("mcp.json", servers), hooks: file("hooks.json", hooks), language: None }
    }

    /// The files of `dir`, as `read_state` reads them.
    fn state_in(dir: &Path) -> State {
        State { servers: read(dir.join("mcp.json")).unwrap(), hooks: read(dir.join("hooks.json")).unwrap(), ..state(None, None) }
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

    fn json(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    /// `value` as clawg writes a file: two spaces and a final newline.
    fn pretty(value: &Value) -> String {
        serde_json::to_string_pretty(value).unwrap() + "\n"
    }

    fn planned_server() -> Value {
        serde_json::json!({ "type": "stdio", "command": exe().to_str().unwrap(), "args": ["mcp", "--profile", "dev", "--agent", "cursor"] })
    }

    fn planned_hook(shell: Shell) -> Value {
        serde_json::json!({ "command": hook_line(exe().to_str().unwrap(), Profile::Dev, shell) })
    }

    const ARGS: &str = "hook session-start --agent cursor --client cursor --profile dev";

    #[test]
    fn an_empty_state_plans_the_server_and_the_hook() {
        let plan = plan(&state(None, None), Mode::Install, Shell::Posix).unwrap();
        assert_eq!((&plan.server_step, &plan.hook_step), (&Step::Add, &Step::Add));
        assert_eq!(json(written(&plan.servers)), serde_json::json!({ "mcpServers": { "clawg-dev": planned_server() } }));
        let hooks = json(written(&plan.hooks));
        assert_eq!(hooks, serde_json::json!({ "version": 1, "hooks": { "sessionStart": [planned_hook(Shell::Posix)] } }));
        let keys: Vec<&String> = hooks.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["version", "hooks"]);
    }

    #[test]
    fn a_relative_binary_path_is_refused() {
        let s = State { exe: PathBuf::from("target/debug/clawg"), ..state(None, None) };
        assert!(matches!(plan(&s, Mode::Install, Shell::Posix), Err(SetupError::RelativeExe(_))));
    }

    #[test]
    fn an_invalid_file_writes_nothing() {
        let valid = r#"{"mcpServers":{}}"#;
        for (servers, hooks, named) in [
            (Some(valid), Some(r#"{"hooks": {"#), "hooks.json"),
            (Some(r#"{"mcpServers": "#), None, "mcp.json"),
            (Some("[]"), None, "mcp.json"),
            (Some(r#"{"mcpServers":[]}"#), None, "mcp.json"),
            (Some(valid), Some(r#"{"version":1,"hooks":[]}"#), "hooks.json"),
            (Some(valid), Some(r#"{"version":1,"hooks":{"sessionStart":{}}}"#), "hooks.json"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            for (name, text) in [("mcp.json", servers), ("hooks.json", hooks)] {
                if let Some(text) = text {
                    std::fs::write(dir.path().join(name), text).unwrap();
                }
            }
            let data = dir.path().join("data");
            for mode in [Mode::Install, Mode::Remove] {
                let err = run_in(state_in(dir.path()), mode, false, Shell::Posix, &data, 0).unwrap_err();
                assert!(err.to_string().contains(named), "{servers:?} {hooks:?}: {err}");
            }
            assert_eq!(std::fs::read_to_string(dir.path().join("mcp.json")).ok().as_deref(), servers, "mcp.json is as it was");
            assert_eq!(std::fs::read_to_string(dir.path().join("hooks.json")).ok().as_deref(), hooks, "hooks.json is as it was");
            assert!(!data.exists(), "no copy was made");
        }
    }

    #[test]
    fn a_different_server_is_a_conflict() {
        let servers = r#"{"mcpServers":{"clawg-dev":{"type":"stdio","command":"/old/clawg","args":["mcp","--profile","dev"]}}}"#;
        let s = state(Some(servers), None);
        let plan = plan(&s, Mode::Install, Shell::Posix).unwrap();
        assert_eq!(plan.server_step, Step::Conflict("/old/clawg mcp --profile dev".into()));
        assert_eq!(plan.servers, FileStep::Unchanged);
        let (text, code) = report(&plan, &s, None);
        assert_eq!(code, 1, "{text}");
        assert!(text.contains("registered as `/old/clawg mcp --profile dev`; left as it is"), "{text}");
    }

    #[test]
    fn other_servers_stay_and_remove_gives_the_file_back() {
        let mine = serde_json::json!({
            "mcpServers": {
                "github": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-github"], "env": { "GITHUB_TOKEN": "x" } },
                "docs": { "url": "http://localhost:3000/mcp" }
            },
            "note": "mine"
        });
        for original in [pretty(&mine), mine.to_string()] {
            let dir = tempfile::tempdir().unwrap();
            let data = dir.path().join("data");
            std::fs::write(dir.path().join("mcp.json"), &original).unwrap();

            let (text, code) = run_in(state_in(dir.path()), Mode::Install, false, Shell::Posix, &data, 1).unwrap();
            assert_eq!(code, 0, "{text}");
            let installed = json(&std::fs::read_to_string(dir.path().join("mcp.json")).unwrap());
            let keys: Vec<&String> = installed.as_object().unwrap().keys().collect();
            assert_eq!(keys, ["mcpServers", "note"]);
            let servers: Vec<&String> = installed["mcpServers"].as_object().unwrap().keys().collect();
            assert_eq!(servers, ["github", "docs", "clawg-dev"]);
            for name in ["github", "docs"] {
                assert_eq!(installed["mcpServers"][name], mine["mcpServers"][name], "{name}");
            }

            let (text, code) = run_in(state_in(dir.path()), Mode::Remove, false, Shell::Posix, &data, 2).unwrap();
            assert_eq!(code, 0, "{text}");
            let back = std::fs::read_to_string(dir.path().join("mcp.json")).unwrap();
            assert_eq!(json(&back), mine);
            if original.ends_with('\n') {
                assert_eq!(back, original, "byte for byte, laid out as clawg lays it out");
            }
            assert_eq!(json(&std::fs::read_to_string(dir.path().join("hooks.json")).unwrap()), serde_json::json!({ "version": 1 }));
        }
    }

    #[test]
    fn a_second_run_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::write(dir.path().join("mcp.json"), r#"{"mcpServers":{"docs":{"url":"http://localhost:3000/mcp"}}}"#).unwrap();
        let (text, code) = run_in(state_in(dir.path()), Mode::Install, false, Shell::Posix, &data, 1).unwrap();
        assert_eq!(code, 0, "{text}");
        let read = |name: &str| std::fs::read(dir.path().join(name)).unwrap();
        let (servers, hooks) = (read("mcp.json"), read("hooks.json"));
        let copies = || std::fs::read_dir(data.join("backups").join("setup")).unwrap().count();
        assert_eq!(copies(), 1, "the mcp.json that was there");

        let (text, code) = run_in(state_in(dir.path()), Mode::Install, false, Shell::Posix, &data, 2).unwrap();
        assert_eq!(code, 0, "{text}");
        assert_eq!(text.matches("already there").count(), 2, "{text}");
        assert!(!text.contains("Restart Cursor"), "{text}");
        assert_eq!((read("mcp.json"), read("hooks.json")), (servers, hooks));
        assert_eq!(copies(), 1, "nothing was copied again");
    }

    #[test]
    fn dry_run_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (text, code) = run_in(state_in(dir.path()), Mode::Install, true, Shell::Posix, dir.path(), 0).unwrap();
        assert_eq!(code, 0);
        assert_eq!(text.matches("would add it").count(), 2, "{text}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn remove_after_setup_gives_back_the_files() {
        let servers = pretty(&serde_json::json!({ "mcpServers": { "docs": { "url": "http://localhost:3000/mcp" } } }));
        let hooks = pretty(&serde_json::json!({ "version": 1, "hooks": { "sessionStart": [{ "command": "./start.sh" }] } }));
        for (servers, hooks) in [(None, None), (Some(servers.as_str()), Some(hooks.as_str())), (Some("{}"), Some(r#"{"version":1}"#))] {
            let installed = plan(&state(servers, hooks), Mode::Install, Shell::Posix).unwrap();
            let s = state(after(servers, &installed.servers).as_deref(), after(hooks, &installed.hooks).as_deref());
            let removed = plan(&s, Mode::Remove, Shell::Posix).unwrap();
            assert_eq!((&removed.server_step, &removed.hook_step), (&Step::Remove, &Step::Remove));
            let servers_back = after(s.servers.text.as_deref(), &removed.servers).unwrap();
            let hooks_back = after(s.hooks.text.as_deref(), &removed.hooks).unwrap();
            match servers {
                Some(text) if text.ends_with('\n') => assert_eq!(servers_back, text),
                _ => assert_eq!(servers_back, "{}\n", "{servers:?}"),
            }
            match hooks {
                Some(text) if text.ends_with('\n') => assert_eq!(hooks_back, text),
                _ => assert_eq!(json(&hooks_back), serde_json::json!({ "version": 1 }), "{hooks:?}"),
            }
        }
    }

    #[test]
    fn remove_leaves_a_foreign_server() {
        let servers = r#"{"mcpServers":{"clawg-dev":{"command":"npx","args":["other-server"]}}}"#;
        let plan = plan(&state(Some(servers), None), Mode::Remove, Shell::Posix).unwrap();
        assert_eq!(plan.server_step, Step::Conflict("npx other-server".into()));
        assert_eq!(plan.servers, FileStep::Unchanged);
        assert_eq!((&plan.hook_step, &plan.hooks), (&Step::Absent, &FileStep::Unchanged));
    }

    #[test]
    fn the_hook_line_is_written_for_the_shell_that_runs_it() {
        assert_eq!(hook_line("/opt/clawg/clawg", Profile::Dev, Shell::Posix), format!("/opt/clawg/clawg {ARGS}"));
        assert_eq!(hook_line("/home/Ana María/bin/clawg", Profile::Dev, Shell::Posix), format!("\"/home/Ana María/bin/clawg\" {ARGS}"));
        assert_eq!(
            hook_line(r"C:\Users\Ana María\AppData\Local\Programs\clawg\clawg.exe", Profile::Dev, Shell::PowerShell),
            format!(r#"& "C:\Users\Ana María\AppData\Local\Programs\clawg\clawg.exe" {ARGS}"#)
        );
        assert_eq!(
            hook_line(r"C:\tools\clawg.exe", Profile::Prod, Shell::PowerShell),
            r#"& "C:\tools\clawg.exe" hook session-start --agent cursor --client cursor --profile prod"#
        );
        assert_eq!(hook_line(r"C:\a$b`c\clawg.exe", Profile::Dev, Shell::PowerShell), format!(r#"& "C:\a`$b``c\clawg.exe" {ARGS}"#));
    }

    #[test]
    fn a_hook_line_reads_back_as_its_words() {
        let args: Vec<String> = ARGS.split(' ').map(String::from).collect();
        for (exe, shell) in [
            ("/opt/clawg/clawg", Shell::Posix),
            ("/home/Ana María/$HOME`x\"/clawg", Shell::Posix),
            (r"C:\Users\Ana María\clawg.exe", Shell::PowerShell),
            (r"C:\a$b`c\clawg.exe", Shell::PowerShell),
        ] {
            let words = shell.words(&hook_line(exe, Profile::Dev, shell));
            assert_eq!(words[0], exe, "{shell:?}");
            assert_eq!(words[1..], args[..], "{shell:?}");
        }
        assert_eq!(Shell::PowerShell.words(r#"& 'a''b' "c""d" e`f"#), ["a'b", "c\"d", "ef"]);
        assert_eq!(Shell::Posix.words("& x"), ["&", "x"], "the call operator is PowerShell's");
    }

    #[test]
    fn a_different_clawg_hook_is_a_conflict() {
        let claude = "clawg hook session-start --agent claude-code --profile dev";
        let hooks = serde_json::json!({ "version": 1, "hooks": { "sessionStart": [{ "command": claude }] } }).to_string();
        let s = state(Some(&serde_json::json!({ "mcpServers": { "clawg-dev": planned_server() } }).to_string()), Some(&hooks));
        let plan_ = plan(&s, Mode::Install, Shell::Posix).unwrap();
        assert_eq!(plan_.hook_step, Step::Conflict(claude.into()));
        assert_eq!(plan_.hooks, FileStep::Unchanged);
        let (text, code) = report(&plan_, &s, None);
        assert_eq!(code, 1, "{text}");
        assert!(text.contains(&format!("another clawg hook is there, `{claude}`; left as it is")), "{text}");

        let moved = r#"& "C:\old\clawg.exe" hook session-start --agent cursor --client cursor --profile=dev"#;
        let hooks = serde_json::json!({ "hooks": { "sessionStart": [{ "command": moved }] } }).to_string();
        let plan_ = plan(&state(None, Some(&hooks)), Mode::Install, Shell::PowerShell).unwrap();
        assert_eq!(plan_.hook_step, Step::Conflict(moved.into()));
        let removing = plan(&state(None, Some(&hooks)), Mode::Remove, Shell::PowerShell).unwrap();
        assert_eq!(removing.hook_step, Step::Remove);
        assert_eq!(json(written(&removing.hooks)), serde_json::json!({}));
    }

    #[test]
    fn foreign_hooks_and_keys_keep_their_order() {
        let hooks = r#"{"version":1,"hooks":{"afterFileEdit":[{"command":"./format.sh"}],"sessionStart":[{"command":"./start.sh","timeout":10}]},"note":true}"#;
        let plan = plan(&state(None, Some(hooks)), Mode::Install, Shell::Posix).unwrap();
        let value = json(written(&plan.hooks));
        let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["version", "hooks", "note"]);
        let events: Vec<&String> = value["hooks"].as_object().unwrap().keys().collect();
        assert_eq!(events, ["afterFileEdit", "sessionStart"]);
        assert_eq!(value["hooks"]["afterFileEdit"], json(hooks)["hooks"]["afterFileEdit"]);
        assert_eq!(value["hooks"]["sessionStart"], serde_json::json!([{ "command": "./start.sh", "timeout": 10 }, planned_hook(Shell::Posix)]));
    }

    #[test]
    fn a_hook_of_the_other_profile_is_not_ours() {
        let prod = "/opt/clawg/clawg hook session-start --agent cursor --client cursor --profile prod";
        let hooks = serde_json::json!({ "version": 1, "hooks": { "sessionStart": [{ "command": prod }] } }).to_string();
        let installing = plan(&state(None, Some(&hooks)), Mode::Install, Shell::Posix).unwrap();
        assert_eq!(installing.hook_step, Step::Add);
        assert_eq!(json(written(&installing.hooks))["hooks"]["sessionStart"][0]["command"], prod);
        let removing = plan(&state(None, Some(&hooks)), Mode::Remove, Shell::Posix).unwrap();
        assert_eq!((&removing.hook_step, &removing.hooks), (&Step::Absent, &FileStep::Unchanged), "the prod hook stays");
    }

    #[test]
    fn the_hook_of_this_profile_is_found_in_hooks_json() {
        let ours = hook_line("/opt/clawg/clawg", Profile::Dev, Shell::Posix);
        let hooks = |command: &str| serde_json::json!({ "version": 1, "hooks": { "sessionStart": [{ "command": "other" }, { "command": command }] } }).to_string();

        assert!(installed_in(&hooks(&ours), Profile::Dev, Shell::Posix));
        assert!(!installed_in(&hooks(&ours), Profile::Prod, Shell::Posix), "the other profile's hook is not ours");
        assert!(!installed_in(&hooks("echo hi"), Profile::Dev, Shell::Posix));
        for text in ["", "not json", "[]", r#"{"hooks":{"sessionStart":"x"}}"#, r#"{"version":1}"#] {
            assert!(!installed_in(text, Profile::Dev, Shell::Posix), "{text:?}");
        }

        let home = tempfile::tempdir().unwrap();
        assert!(!hook_installed(Some(home.path()), Profile::Dev), "no file");
        assert!(!hook_installed(None, Profile::Dev), "no home");
        let (_, hooks_path) = locations(Some(home.path())).unwrap();
        std::fs::create_dir_all(hooks_path.parent().unwrap()).unwrap();
        std::fs::write(&hooks_path, hooks(&hook_line("/opt/clawg/clawg", Profile::Dev, Shell::HOST))).unwrap();
        assert!(hook_installed(Some(home.path()), Profile::Dev));
    }

    #[test]
    fn nothing_is_installed_before_a_prompt() {
        let plan = plan(&state(None, None), Mode::Install, Shell::HOST).unwrap();
        let hooks = json(written(&plan.hooks));
        let events: Vec<&String> = hooks["hooks"].as_object().unwrap().keys().collect();
        assert_eq!(events, ["sessionStart"], "beforeSubmitPrompt cannot add to the context");
    }

    #[test]
    fn apply_copies_the_previous_file_first() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::write(dir.path().join("mcp.json"), r#"{"mcpServers":{}}"#).unwrap();
        let (text, code) = run_in(state_in(dir.path()), Mode::Install, false, Shell::Posix, &data, 1_700_000_000_000).unwrap();
        assert_eq!(code, 0, "{text}");
        let copy = data.join("backups").join("setup").join(format!("mcp.json-{}", stamp(1_700_000_000_000)));
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), r#"{"mcpServers":{}}"#);
        assert!(text.contains(&format!("previous file copied to {}", copy.display())), "{text}");
        assert_eq!(json(&std::fs::read_to_string(dir.path().join("mcp.json")).unwrap())["mcpServers"]["clawg-dev"], planned_server());
        let mut names = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned());
        assert!(!names.any(|n| n.ends_with(".clawg-tmp")), "no temporary file is left");
    }

    #[test]
    fn setup_warns_to_turn_off_third_party_configs() {
        let warning = "Cursor: turn off \"Include Third-Party Plugins, Skills, and Other Configs\" (Settings → Agents → Third-Party Imports)";
        let s = state(None, None);
        let installing = plan(&s, Mode::Install, Shell::Posix).unwrap();
        for written in [None, Some(&Written::default())] {
            let (text, code) = report(&installing, &s, written);
            assert_eq!(code, 0, "{text}");
            assert!(text.contains(warning), "{text}");
            assert!(text.contains("Claude Code's hooks and plugins"), "{text}");
        }
        let (text, _) = report(&installing, &s, Some(&Written::default()));
        assert!(text.ends_with("Restart Cursor for the change to take effect."), "{text}");
        let removing = plan(&s, Mode::Remove, Shell::Posix).unwrap();
        assert!(!report(&removing, &s, None).0.contains("Third-Party"));
    }

    #[test]
    fn the_report_speaks_spanish() {
        let s = State { language: Some(Lang::Es), ..state(None, None) };
        let plan_ = plan(&s, Mode::Install, Shell::Posix).unwrap();
        let (dry, code) = report(&plan_, &s, None);
        assert_eq!(code, 0);
        for needed in [
            "servidor mcp clawg-dev en mcp.json: se agregaría",
            "hook sessionStart en hooks.json: se agregaría",
            "Cursor: apaga \"Include Third-Party Plugins, Skills, and Other Configs\" (Settings → Agents → Third-Party Imports)",
        ] {
            assert!(dry.contains(needed), "missing {needed:?} in:\n{dry}");
        }
        let applied = Written { backups: vec![PathBuf::from("/b/mcp.json-1")], pruned: vec![], failures: vec![] };
        let (text, code) = report(&plan_, &s, Some(&applied));
        assert_eq!(code, 0);
        for needed in [": agregado", "archivo anterior copiado a /b/mcp.json-1", "Reinicia Cursor para que el cambio tenga efecto."] {
            assert!(text.contains(needed), "missing {needed:?} in:\n{text}");
        }

        let servers = r#"{"mcpServers":{"clawg-dev":{"command":"/old/clawg","args":["mcp"]}}}"#;
        let conflict = State { language: Some(Lang::Es), ..state(Some(servers), None) };
        let (text, code) = report(&plan(&conflict, Mode::Install, Shell::Posix).unwrap(), &conflict, None);
        assert_eq!(code, 1);
        assert!(text.contains("registrado como `/old/clawg mcp`; queda como está"), "{text}");
    }
}
