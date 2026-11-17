//! `clawg setup claude-code --local` and `--path DIR`: Claude Code's hooks in
//! one folder's `.claude/settings.local.json`, which Claude Code reads for the
//! sessions it opens there, and nothing else. The MCP server and the memory
//! protocol stay the user's, as `setup claude-code` installs them.
//!
//! The hooks are planned as the user's are ([`super::plan_hooks`]) and written
//! with the same copies. A remove that leaves the file with nothing deletes
//! it, and its `.claude` folder when that is left empty.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clawg_core::paths::Profile;
use clawg_core::preferences::{self, Lang};

use super::{
    current_exe, display_word, done_or_planned, exe_text, file_lines, hooks_line, now_ms, plan_hooks, read, settings_text,
    write_files, File, FileStep, HooksStep, Mode, SetupError, Written,
};

/// The file, as `.gitignore` names it from the folder.
const LOCAL_FILE: &str = ".claude/settings.local.json";

/// Everything a folder's plan decides from, read before anything is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// The binary the hooks will run.
    pub exe: PathBuf,
    pub profile: Profile,
    /// The folder the hooks are for.
    pub folder: PathBuf,
    /// Its `.claude/settings.local.json`.
    pub settings: File,
    /// Whether git ignores that file, `None` outside a repository or without
    /// git; only read when installing.
    pub ignored: Option<bool>,
    /// The profile's preferred language, which the report speaks.
    pub language: Option<Lang>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub mode: Mode,
    pub hooks: HooksStep,
    pub settings: FileStep,
}

/// Decides the hooks' steps from what was read. Fails, so that nothing is
/// written, when the file cannot be understood.
pub fn plan(state: &State, mode: Mode) -> Result<Plan, SetupError> {
    let exe = exe_text(&state.exe)?;
    let (hooks, changed) = plan_hooks(&state.settings, exe, state.profile, mode)?;
    let settings = match changed {
        None => FileStep::Unchanged,
        Some(settings) if mode == Mode::Remove && settings.is_empty() => FileStep::Delete,
        Some(settings) => FileStep::Write(settings_text(settings)),
    };
    Ok(Plan { mode, hooks, settings })
}

/// The folder at `path`, resolved and without the `\\?\` prefix Windows
/// gives a resolved path. Refused when it is not a folder, or when it is the
/// home directory: its `.claude` is the user's own configuration, and it is
/// where a new terminal opens on Windows.
pub fn folder(path: &Path, home: Option<&Path>) -> Result<PathBuf, SetupError> {
    let resolved = std::fs::canonicalize(path).ok().filter(|p| p.is_dir());
    let folder = resolved.ok_or_else(|| SetupError::NotAFolder(path.to_path_buf()))?;
    if home.and_then(|home| std::fs::canonicalize(home).ok()).is_some_and(|home| home == folder) {
        return Err(SetupError::HomeFolder(without_verbatim(folder)));
    }
    Ok(without_verbatim(folder))
}

/// `path` as a user writes it: `\\?\C:\…` as `C:\…`, `\\?\UNC\server\…` as
/// `\\server\…`.
fn without_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{share}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path,
    }
}

/// Whether git ignores the folder's `.claude/settings.local.json`: `None`
/// when git cannot be run or the folder is not in a repository.
pub fn ignored_by_git(folder: &Path) -> Option<bool> {
    let status = git_in(folder)
        .args(["check-ignore", "-q", "--", LOCAL_FILE])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    match status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

/// `git -C folder` without the `GIT_*` of the caller's environment, which
/// could point it at another repository.
fn git_in(folder: &Path) -> Command {
    let mut command = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.as_encoded_bytes().get(..4).is_some_and(|p| p.eq_ignore_ascii_case(b"GIT_")) {
            command.env_remove(key);
        }
    }
    command.arg("-C").arg(folder);
    command
}

/// The name the copies of a folder's file start with: the folder's name
/// first, so pruning one folder's copies leaves those of a folder with
/// another name alone.
fn copy_name(folder: &Path) -> String {
    match folder.file_name() {
        Some(name) => format!("{}.settings.local.json", name.to_string_lossy()),
        None => "settings.local.json".into(),
    }
}

/// Reads what [`plan`] decides from: the folder's file, the preferred
/// language from `data_dir` and, when installing, whether git ignores the file.
pub fn read_state(exe: PathBuf, profile: Profile, folder: PathBuf, mode: Mode, data_dir: &Path) -> Result<State, SetupError> {
    let settings = read(folder.join(".claude").join("settings.local.json"))?;
    let ignored = if mode == Mode::Install { ignored_by_git(&folder) } else { None };
    Ok(State { exe, profile, folder, settings, ignored, language: preferences::language_in(data_dir) })
}

/// Carries the plan out: the file, copied first to `<data>/backups/setup`
/// and written through a temporary file, and the `.claude` folder a deleted
/// file leaves empty.
pub fn apply(plan: &Plan, state: &State, base_dir: &Path, now_ms: i64) -> Written {
    let written = write_files(&[(&plan.settings, &state.settings, copy_name(&state.folder))], base_dir, now_ms);
    if plan.settings == FileStep::Delete && written.failures.is_empty() {
        // `remove_dir` takes away only an empty folder, and one that keeps
        // something is meant to stay.
        let _ = std::fs::remove_dir(state.folder.join(".claude"));
    }
    written
}

/// The report, one line per step, and the exit code: 1 when a step did not
/// end as planned (a conflict, a failure). The warning that git does not
/// ignore the file is not one. It speaks the profile's preferred language.
pub fn report(plan: &Plan, state: &State, written: Option<&Written>) -> (String, u8) {
    let lang = state.language.unwrap_or(Lang::En);
    let dry = written.is_none();
    let path = state.settings.path.display();
    let folder = state.folder.display();
    let remove = format!("clawg setup claude-code --path {} --remove", display_word(&folder.to_string()));
    let (hooks, mut incomplete) = hooks_line(lang, dry, &plan.hooks, &state.settings.path, &remove);
    let mut lines = vec![hooks];

    if plan.settings == FileStep::Delete {
        let step = done_or_planned(lang, dry, ("deleted", "delete it"), ("borrado", "se borraría"));
        lines.push(match lang {
            Lang::En => format!("nothing else was in {path}: {step}"),
            Lang::Es => format!("no quedaba nada más en {path}: {step}"),
        });
    }
    if plan.mode == Mode::Install && state.ignored == Some(false) {
        lines.push(match lang {
            Lang::En => format!(
                "git does not ignore {path}, which names this binary's path: add `{LOCAL_FILE}` to .gitignore"
            ),
            Lang::Es => format!("git no ignora {path}, que lleva la ruta de este binario: agrega `{LOCAL_FILE}` a .gitignore"),
        });
    }
    if let Some(written) = written {
        incomplete |= file_lines(lang, &written.backups, &written.pruned, &written.failures, &mut lines);
    }
    if plan.settings != FileStep::Unchanged && !dry {
        lines.push(match lang {
            Lang::En => format!("Open a new Claude Code session in {folder} for the change to take effect."),
            Lang::Es => format!("Abre una sesión nueva de Claude Code en {folder} para que el cambio tenga efecto."),
        });
    }
    (lines.join("\n"), if incomplete { 1 } else { 0 })
}

/// `clawg setup claude-code --local` (or `--path`) from start to end, given
/// where things are.
pub fn run_in(state: State, mode: Mode, dry_run: bool, base_dir: &Path, now_ms: i64) -> Result<(String, u8), SetupError> {
    let plan = plan(&state, mode)?;
    let written = (!dry_run).then(|| apply(&plan, &state, base_dir, now_ms));
    Ok(report(&plan, &state, written.as_ref()))
}

/// `clawg setup claude-code` for the folder at `path`, reading the process's
/// environment: the running binary and the home directory.
pub fn run(profile: Profile, mode: Mode, dry_run: bool, path: &Path) -> Result<(String, u8), SetupError> {
    let base_dir = clawg_core::paths::data_dir(profile).map_err(|_| SetupError::NoHome)?;
    let folder = folder(path, clawg_core::paths::home_dir().as_deref())?;
    let state = read_state(current_exe()?, profile, folder, mode, &base_dir)?;
    run_in(state, mode, dry_run, &base_dir, now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::plan::stamp;
    use serde_json::Value;

    fn exe() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\tools\clawg.exe")
        } else {
            PathBuf::from("/opt/clawg/clawg")
        }
    }

    fn work() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\work\app")
        } else {
            PathBuf::from("/work/app")
        }
    }

    fn state_in(folder: &Path, settings: Option<&str>) -> State {
        State {
            exe: exe(),
            profile: Profile::Dev,
            folder: folder.to_path_buf(),
            settings: File { path: folder.join(".claude").join("settings.local.json"), text: settings.map(String::from) },
            ignored: None,
            language: None,
        }
    }

    fn state(settings: Option<&str>) -> State {
        state_in(&work(), settings)
    }

    /// The state `folder` is in on disk, as `run` reads it.
    fn read_in(folder: &Path) -> State {
        let path = folder.join(".claude").join("settings.local.json");
        state_in(folder, std::fs::read_to_string(path).ok().as_deref())
    }

    fn hook(event_arg: &str) -> Value {
        serde_json::json!({
            "type": "command", "command": exe().to_str().unwrap(),
            "args": ["hook", event_arg, "--agent", "claude-code", "--profile", "dev"], "timeout": 5
        })
    }

    fn installed() -> Value {
        serde_json::json!({ "hooks": {
            "SessionStart": [{ "matcher": "startup|resume|clear|compact", "hooks": [hook("session-start")] }],
            "UserPromptSubmit": [{ "hooks": [hook("user-prompt-submit")] }],
            "SessionEnd": [{ "hooks": [hook("session-end")] }],
            "PostToolUse": [{ "matcher": "Bash", "hooks": [hook("post-tool-use")] }],
            "PostToolUseFailure": [{ "matcher": "Bash", "hooks": [hook("post-tool-use-failure")] }]
        }})
    }

    fn written(step: &FileStep) -> Value {
        match step {
            FileStep::Write(text) => serde_json::from_str(text).unwrap(),
            other => panic!("expected a write, got {other:?}"),
        }
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    fn nothing_written() -> Written {
        Written { backups: vec![], pruned: vec![], failures: vec![] }
    }

    #[test]
    fn local_hooks_go_in_the_folder_settings_alone() {
        let plan = plan(&state(None), Mode::Install).unwrap();
        assert_eq!(plan.hooks.added, ["SessionStart", "UserPromptSubmit", "SessionEnd", "PostToolUse", "PostToolUseFailure"]);
        assert_eq!(written(&plan.settings), installed());

        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("app");
        std::fs::create_dir_all(&app).unwrap();
        let (text, code) = run_in(read_in(&app), Mode::Install, false, &dir.path().join("data"), 0).unwrap();
        assert_eq!(code, 0, "{text}");
        assert_eq!(names(dir.path()), ["app"], "nothing beside the folder, and no copy of a file that was not there");
        assert_eq!(names(&app.join(".claude")), ["settings.local.json"]);
        let value: Value = serde_json::from_str(&std::fs::read_to_string(app.join(".claude").join("settings.local.json")).unwrap()).unwrap();
        assert_eq!(value, installed());
        let shown = app.join(".claude").join("settings.local.json");
        assert!(text.contains(&format!("hooks in {}: added SessionStart, UserPromptSubmit, SessionEnd, PostToolUse, PostToolUseFailure", shown.display())), "{text}");
        assert!(text.contains(&format!("Open a new Claude Code session in {}", app.display())), "{text}");
        assert!(!text.contains("mcp server") && !text.contains("memory protocol"), "the server and the protocol are the user's: {text}");
    }

    #[test]
    fn a_second_local_run_changes_nothing() {
        let first = plan(&state(Some(r#"{"permissions":{"allow":["Bash(ls)"]}}"#)), Mode::Install).unwrap();
        let FileStep::Write(text) = &first.settings else { panic!("{:?}", first.settings) };
        let again = state(Some(text));
        let second = plan(&again, Mode::Install).unwrap();
        assert_eq!(second.hooks.unchanged, ["SessionStart", "UserPromptSubmit", "SessionEnd", "PostToolUse", "PostToolUseFailure"]);
        assert_eq!(second.settings, FileStep::Unchanged);
        let (text, code) = report(&second, &again, Some(&nothing_written()));
        assert_eq!(code, 0, "{text}");
        assert!(text.contains("SessionStart, UserPromptSubmit, SessionEnd, PostToolUse, PostToolUseFailure already there"), "{text}");
        assert!(!text.contains("Open a new"), "{text}");
    }

    #[test]
    fn a_local_run_over_an_install_without_the_session_end_hook_adds_only_it() {
        let mut earlier = installed();
        earlier["hooks"].as_object_mut().unwrap().remove("SessionEnd");
        let s = state(Some(&earlier.to_string()));
        let plan = plan(&s, Mode::Install).unwrap();
        assert_eq!(plan.hooks.added, ["SessionEnd"]);
        assert_eq!(plan.hooks.unchanged, ["SessionStart", "UserPromptSubmit", "PostToolUse", "PostToolUseFailure"]);
        assert_eq!(written(&plan.settings), installed());
        let (text, code) = report(&plan, &s, None);
        assert_eq!(code, 0, "{text}");
        assert!(text.contains("would add SessionEnd; SessionStart, UserPromptSubmit, PostToolUse, PostToolUseFailure already there"), "{text}");
    }

    #[test]
    fn a_different_clawg_hook_in_the_folder_is_a_conflict() {
        let moved = serde_json::json!({ "hooks": { "SessionStart": [{ "hooks": [{ "type": "command", "command": "/old/clawg",
            "args": ["hook", "session-start", "--agent", "claude-code", "--profile", "dev"] }] }] } })
        .to_string();
        let s = state(Some(&moved));
        let plan = plan(&s, Mode::Install).unwrap();
        assert_eq!(plan.hooks.conflicts.len(), 1, "{:?}", plan.hooks);
        assert_eq!(plan.hooks.added, ["UserPromptSubmit", "SessionEnd", "PostToolUse", "PostToolUseFailure"]);
        let value = written(&plan.settings);
        assert_eq!(value["hooks"]["SessionStart"].as_array().unwrap().len(), 1, "the conflicting hook is not replaced");
        let (text, code) = report(&plan, &s, None);
        assert_eq!(code, 1, "{text}");
        let remove = format!("`clawg setup claude-code --path {} --remove` takes it away", work().display());
        assert!(text.contains(&remove), "{text}");
    }

    #[test]
    fn remove_deletes_a_file_left_with_nothing() {
        let s = state(Some(&installed().to_string()));
        let plan = plan(&s, Mode::Remove).unwrap();
        assert_eq!(plan.hooks.removed, ["SessionStart", "UserPromptSubmit", "SessionEnd", "PostToolUse", "PostToolUseFailure"]);
        assert_eq!(plan.settings, FileStep::Delete);
        let (text, code) = report(&plan, &s, None);
        assert_eq!(code, 0, "{text}");
        assert!(text.contains("would remove SessionStart, UserPromptSubmit, SessionEnd, PostToolUse, PostToolUseFailure"), "{text}");
        assert!(text.contains(&format!("nothing else was in {}: would delete it", s.settings.path.display())), "{text}");
    }

    #[test]
    fn remove_keeps_what_is_not_clawg_s() {
        let mut mine = installed();
        mine["permissions"] = serde_json::json!({ "allow": ["Bash(ls)"] });
        mine["hooks"]["SessionStart"].as_array_mut().unwrap().push(serde_json::json!({ "hooks": [{ "type": "command", "command": "guard" }] }));
        let plan_ = plan(&state(Some(&mine.to_string())), Mode::Remove).unwrap();
        assert_eq!(
            written(&plan_.settings),
            serde_json::json!({ "hooks": { "SessionStart": [{ "hooks": [{ "type": "command", "command": "guard" }] }] },
                "permissions": { "allow": ["Bash(ls)"] } })
        );

        let foreign = r#"{"permissions":{"allow":["Bash(ls)"]}}"#;
        let untouched = plan(&state(Some(foreign)), Mode::Remove).unwrap();
        assert_eq!(untouched.settings, FileStep::Unchanged);
        assert_eq!(plan(&state(None), Mode::Remove).unwrap().settings, FileStep::Unchanged, "a missing file stays missing");
    }

    #[test]
    fn remove_takes_away_the_emptied_claude_folder() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let (bare, shared) = (dir.path().join("bare"), dir.path().join("shared"));
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::create_dir_all(shared.join(".claude")).unwrap();
        std::fs::write(shared.join(".claude").join("settings.json"), "{}").unwrap();
        for folder in [&bare, &shared] {
            let (text, code) = run_in(read_in(folder), Mode::Install, false, &data, 1).unwrap();
            assert_eq!(code, 0, "{text}");
            let (text, code) = run_in(read_in(folder), Mode::Remove, false, &data, 2).unwrap();
            assert_eq!(code, 0, "{text}");
            assert!(text.contains(": deleted"), "{text}");
        }
        assert!(!bare.join(".claude").exists(), "the folder is as it was");
        assert_eq!(names(&shared.join(".claude")), ["settings.json"], "a .claude with something else stays");
    }

    #[test]
    fn the_copy_is_named_by_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let copies = data.join("backups").join("setup");
        std::fs::create_dir_all(&copies).unwrap();
        for ms in 1..=5 {
            std::fs::write(copies.join(format!("api.settings.local.json-{}", stamp(ms))), "old").unwrap();
        }
        let app = dir.path().join("app");
        std::fs::create_dir_all(app.join(".claude")).unwrap();
        std::fs::write(app.join(".claude").join("settings.local.json"), r#"{"model":"x"}"#).unwrap();

        let (text, code) = run_in(read_in(&app), Mode::Install, false, &data, 1_700_000_000_000).unwrap();
        assert_eq!(code, 0, "{text}");
        let copy = copies.join(format!("app.settings.local.json-{}", stamp(1_700_000_000_000)));
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), r#"{"model":"x"}"#);
        assert!(text.contains(&format!("previous file copied to {}", copy.display())), "{text}");
        let api = names(&copies).into_iter().filter(|n| n.starts_with("api.")).count();
        assert_eq!(api, 5, "another folder's copies are not pruned");
        assert!(!text.contains("older copy removed"), "{text}");
    }

    #[test]
    fn the_report_says_when_git_does_not_ignore_the_file() {
        let warning = "git does not ignore";
        for (ignored, mode, warned) in [
            (Some(false), Mode::Install, true),
            (Some(true), Mode::Install, false),
            (None, Mode::Install, false),
            (Some(false), Mode::Remove, false),
        ] {
            let s = State { ignored, ..state(Some(&installed().to_string())) };
            let plan = plan(&s, mode).unwrap();
            let (text, code) = report(&plan, &s, None);
            assert_eq!(code, 0, "the warning is not a failure: {text}");
            assert_eq!(text.contains(warning), warned, "{ignored:?} {mode:?}: {text}");
            if warned {
                assert!(text.contains("add `.claude/settings.local.json` to .gitignore"), "{text}");
            }
        }
    }

    #[test]
    fn git_says_whether_it_ignores_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        assert_eq!(ignored_by_git(&repo), None, "not a repository");
        let git = |args: &[&str]| {
            let status = git_in(&repo).args(args).status().unwrap();
            assert!(status.success(), "git {args:?}");
        };
        git(&["init", "-q"]);
        // The user's global excludes may already list the file.
        let excludes = dir.path().join("excludes");
        std::fs::write(&excludes, "").unwrap();
        git(&["config", "core.excludesFile", excludes.to_str().unwrap()]);
        assert_eq!(ignored_by_git(&repo), Some(false));
        std::fs::write(repo.join(".gitignore"), ".claude/settings.local.json\n").unwrap();
        assert_eq!(ignored_by_git(&repo), Some(true));
    }

    #[test]
    fn the_home_folder_and_a_missing_one_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let project = home.join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(home.join("notes.txt"), "").unwrap();

        assert!(matches!(folder(&home, Some(&home)), Err(SetupError::HomeFolder(_))));
        assert!(matches!(folder(&project.join(".."), Some(&home)), Err(SetupError::HomeFolder(_))), "compared once resolved");
        assert!(matches!(folder(&home.join("missing"), Some(&home)), Err(SetupError::NotAFolder(_))));
        assert!(matches!(folder(&home.join("notes.txt"), Some(&home)), Err(SetupError::NotAFolder(_))));
        let found = folder(&project, Some(&home)).unwrap();
        assert!(found.is_absolute() && found.ends_with("project"), "{}", found.display());
        assert!(!found.to_string_lossy().starts_with(r"\\?\"), "{}", found.display());
        assert!(folder(&project, None).is_ok());
    }

    #[test]
    fn the_local_report_speaks_spanish() {
        let s = State { language: Some(Lang::Es), ignored: Some(false), ..state(None) };
        let plan_ = plan(&s, Mode::Install).unwrap();
        let (dry, code) = report(&plan_, &s, None);
        assert_eq!(code, 0);
        for needed in ["hooks en ", ": se agregarían SessionStart, UserPromptSubmit, SessionEnd", "git no ignora", "agrega `.claude/settings.local.json` a .gitignore"] {
            assert!(dry.contains(needed), "missing {needed:?} in:\n{dry}");
        }
        let (text, _) = report(&plan_, &s, Some(&nothing_written()));
        assert!(text.contains(&format!("Abre una sesión nueva de Claude Code en {}", work().display())), "{text}");

        let removing = State { language: Some(Lang::Es), ..state(Some(&installed().to_string())) };
        let (text, _) = report(&plan(&removing, Mode::Remove).unwrap(), &removing, Some(&nothing_written()));
        assert!(text.contains(": quitados SessionStart, UserPromptSubmit, SessionEnd"), "{text}");
        assert!(text.contains(&format!("no quedaba nada más en {}: borrado", removing.settings.path.display())), "{text}");
    }
}
