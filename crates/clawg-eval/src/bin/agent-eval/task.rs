//! A task: the prompt an agent session gets, the fixture it works in, the
//! memories each arm is seeded with, the tracers that tell where a fact came
//! from and the checks that decide, without a judge, whether it succeeded.
//! Reading and validating are pure; [`load`] is the only part that reads disk.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The categories a task may have. Each one is checked without a judge.
pub const CATEGORIES: [&str; 12] = ["K1", "K2", "K3", "K4", "K5", "S", "C", "N", "Q", "D", "R", "E"];

/// The memory types `save_memory` takes, as the protocol lists them.
pub const MEMORY_TYPES: [&str; 7] = [
    "semantic_fact",
    "user_preference",
    "architectural_decision",
    "session_summary",
    "code_block",
    "chat_message",
    "tool_error",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: String,
    pub category: String,
    /// A task written to exercise the harness, not to measure anything.
    #[serde(default)]
    pub example: bool,
    /// A directory under `fixtures/`, copied as the session's working directory.
    pub fixture: String,
    /// The prompt of a task of one session; a task of several has [`Task::sessions`] instead.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    /// The sessions of a task of several, played in order on one working directory.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<SessionSpec>,
    /// What the arms with memory know before the session; the same facts for
    /// clawg's store and for the client's native memory.
    #[serde(default)]
    pub memories: Vec<SeedMemory>,
    /// Shell commands every arm may run, by name; with none, no arm has a shell.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shell: Vec<String>,
    /// Commands of `shell` that must not exist where the session runs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_commands: Vec<String>,
    /// Strings found only in the seeded memories: where one shows up in the
    /// session tells whether the fact reached the agent and whether it was used.
    #[serde(default)]
    pub tracers: Vec<String>,
    /// What decides success once every session ran.
    #[serde(default)]
    pub checks: Vec<Check>,
}

impl Task {
    /// The prompt of every session, in order: one for a task with a `prompt`.
    pub fn prompts(&self) -> Vec<&str> {
        if self.sessions.is_empty() { vec![self.prompt.as_str()] } else { self.sessions.iter().map(|s| s.prompt.as_str()).collect() }
    }

    /// The tracers of the task and those born in each session.
    pub fn every_tracer(&self) -> impl Iterator<Item = &String> {
        self.tracers.iter().chain(self.sessions.iter().flat_map(|s| &s.tracers))
    }
}

/// One session of a task of several.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSpec {
    pub prompt: String,
    /// Facts that are born in this prompt, and are told nowhere else.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracers: Vec<String>,
    /// Checks on this session alone, of those that read its stream.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedMemory {
    pub content: String,
    #[serde(default = "default_type")]
    pub memory_type: String,
    #[serde(default = "default_importance")]
    pub importance: f64,
    /// How long before the session the memory was saved.
    #[serde(default)]
    pub age_days: u32,
    /// Accesses the row carries; a fresh save starts at 1.
    #[serde(default = "default_accesses")]
    pub access_count: i64,
    #[serde(default)]
    pub topic_key: Option<String>,
    /// The person said it, so the row is theirs and not an agent's.
    #[serde(default)]
    pub user_stated: bool,
    /// The memory holds as a rule, which the arms that deliver rules hand
    /// the agent without a search.
    #[serde(default)]
    pub rule: Option<Rule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Globs relative to the working directory: the rule holds when the agent
    /// works on a file they match, and in every session when there are none.
    #[serde(default)]
    pub paths: Vec<String>,
}

/// The rules of other topics, in the data directory, that every arm with
/// memory is seeded with besides a task's memories: what a real store holds.
/// A run seeds the noise it names, which is this one when it names none.
pub const DEFAULT_NOISE: &str = "noise";

fn default_type() -> String {
    "semantic_fact".to_owned()
}

fn default_importance() -> f64 {
    5.0
}

fn default_accesses() -> i64 {
    1
}

/// One deterministic condition of success.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Check {
    /// The final answer holds `text`, ignoring case.
    AnswerContains { text: String },
    /// The final answer does not hold `text`, ignoring case.
    AnswerLacks { text: String },
    /// The file at `path`, relative to the working directory, holds `text`.
    FileContains { path: String, text: String },
    /// The file at `path` is missing or does not hold `text`.
    FileLacks { path: String, text: String },
    /// A tool named `name`, or an MCP tool ending in `__name`, was called.
    ToolCalled { name: String },
    /// At most `max` calls to the tool `name` failed: those that came back
    /// marked as an error or, with `text`, those whose result holds `text`.
    ToolFailedCount {
        name: String,
        max: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
}

impl Check {
    /// Whether the check reads the stream of a session and not the files it left.
    pub fn reads_only_the_stream(&self) -> bool {
        matches!(self, Check::AnswerContains { .. } | Check::AnswerLacks { .. } | Check::ToolCalled { .. } | Check::ToolFailedCount { .. })
    }

    pub fn describe(&self) -> String {
        match self {
            Check::AnswerContains { text } => format!("answer contains {text:?}"),
            Check::AnswerLacks { text } => format!("answer lacks {text:?}"),
            Check::FileContains { path, text } => format!("{path} contains {text:?}"),
            Check::FileLacks { path, text } => format!("{path} lacks {text:?}"),
            Check::ToolCalled { name } => format!("{name} was called"),
            Check::ToolFailedCount { name, max, text: None } => format!("{name} failed at most {max} times"),
            Check::ToolFailedCount { name, max, text: Some(text) } => format!("{name} failed at most {max} times with {text:?}"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: std::io::Error },
    #[error("{path} is not a task: {source}")]
    Json { path: PathBuf, source: serde_json::Error },
    #[error("{path} is not a list of seeded memories: {source}")]
    NoiseJson { path: PathBuf, source: serde_json::Error },
    #[error("{path}: {problems}")]
    Invalid { path: PathBuf, problems: String },
    #[error("{0:?} is not a noise: its name is noise, or noise- and letters, digits and dashes")]
    NoiseName(String),
    #[error("{}: no such noise file", .0.display())]
    NoNoise(PathBuf),
    #[error("two tasks have the id {0}")]
    DuplicateId(String),
    #[error("no task has the id {0}")]
    Unknown(String),
}

pub fn parse(text: &str) -> Result<Task, serde_json::Error> {
    serde_json::from_str(text)
}

/// What is wrong with a task, empty when nothing is.
pub fn problems(task: &Task) -> Vec<String> {
    let mut out = Vec::new();
    if task.id.is_empty() || !task.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        out.push(format!("the id {:?} is not lowercase letters, digits and dashes", task.id));
    }
    if !CATEGORIES.contains(&task.category.as_str()) {
        out.push(format!("unknown category {}", task.category));
    }
    if !is_relative_inside(&task.fixture) || task.fixture.contains(['/', '\\']) {
        out.push(format!("the fixture {:?} is not a directory name", task.fixture));
    }
    match (task.prompt.is_empty(), task.sessions.is_empty()) {
        (false, false) => out.push("a task has a prompt or sessions, not both".to_owned()),
        (true, true) => out.push("a task needs a prompt or sessions".to_owned()),
        (false, true) if task.prompt.trim().is_empty() => out.push("the prompt is empty".to_owned()),
        _ => {}
    }
    for (i, session) in task.sessions.iter().enumerate() {
        let n = i + 1;
        if session.prompt.trim().is_empty() {
            out.push(format!("the prompt of session {n} is empty"));
        }
        for check in session.checks.iter().filter(|c| !c.reads_only_the_stream()) {
            out.push(format!("session {n} has the check {}, which reads files and not the stream", check.describe()));
        }
        for tracer in &session.tracers {
            if tracer.trim().is_empty() {
                out.push(format!("a tracer of session {n} is empty"));
                continue;
            }
            if !contains_ci(&session.prompt, tracer) {
                out.push(format!("the tracer {tracer:?} of session {n} is not in its prompt"));
            }
            if let Some(later) = task.sessions.iter().enumerate().skip(n).find(|(_, s)| contains_ci(&s.prompt, tracer)) {
                out.push(format!("the tracer {tracer:?} of session {n} is in the prompt of session {}", later.0 + 1));
            }
            if task.memories.iter().any(|m| contains_ci(&m.content, tracer)) {
                out.push(format!("the tracer {tracer:?} of session {n} is in a seeded memory"));
            }
        }
    }
    if task.checks.is_empty() && task.sessions.iter().all(|s| s.checks.is_empty()) {
        out.push("a task needs at least one check".to_owned());
    }
    for check in &task.checks {
        match check {
            Check::FileContains { path, .. } | Check::FileLacks { path, .. } if !is_relative_inside(path) => {
                out.push(format!("the check path {path:?} leaves the working directory"));
            }
            _ => {}
        }
    }
    for (i, memory) in task.memories.iter().enumerate() {
        out.extend(memory_problems(&format!("memory {}", i + 1), memory));
    }
    for command in &task.shell {
        if command.is_empty() || !command.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            out.push(format!("the shell command {command:?} is not letters, digits, dashes and underscores"));
        }
    }
    for command in task.missing_commands.iter().filter(|c| !task.shell.contains(c)) {
        out.push(format!("the missing command {command:?} is not in the shell"));
    }
    for tracer in &task.tracers {
        if tracer.trim().is_empty() {
            out.push("a tracer is empty".to_owned());
        } else if !task.memories.iter().any(|m| contains_ci(&m.content, tracer)) {
            out.push(format!("the tracer {tracer:?} is in no seeded memory"));
        } else if task.prompts().iter().any(|p| contains_ci(p, tracer)) {
            out.push(format!("the tracer {tracer:?} is in the prompt"));
        }
    }
    out
}

/// What is wrong with the seeded memory `name` names.
fn memory_problems(name: &str, memory: &SeedMemory) -> Vec<String> {
    let mut out = Vec::new();
    if memory.content.trim().is_empty() {
        out.push(format!("{name} is empty"));
    }
    if !MEMORY_TYPES.contains(&memory.memory_type.as_str()) {
        out.push(format!("{name} has the unknown type {}", memory.memory_type));
    }
    if !(0.0..=10.0).contains(&memory.importance) {
        out.push(format!("{name} has importance {} outside 0 to 10", memory.importance));
    }
    if memory.access_count < 1 {
        out.push(format!("{name} has fewer than one access"));
    }
    for path in memory.rule.iter().flat_map(|r| &r.paths) {
        if path.is_empty() {
            out.push(format!("{name} has an empty rule path"));
        } else if !is_relative_inside(path) {
            out.push(format!("{name} has the rule path {path:?}, which leaves the working directory"));
        }
    }
    out
}

/// Whether `name` names a noise: [`DEFAULT_NOISE`], or that and a dash and a name
/// in lowercase letters, digits and dashes, so it cannot reach another file.
pub fn check_noise_name(name: &str) -> Result<(), TaskError> {
    let named = name.strip_prefix("noise-").is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
    if name == DEFAULT_NOISE || named { Ok(()) } else { Err(TaskError::NoiseName(name.to_owned())) }
}

/// The names of the noises `data` holds, by file: every `noise.json` and `noise-NAME.json`.
pub fn noise_names(data: &Path) -> Result<Vec<String>, TaskError> {
    let entries = std::fs::read_dir(data).map_err(|source| TaskError::Read { path: data.to_owned(), source })?;
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|file| file.strip_suffix(".json").map(str::to_owned))
        .filter(|name| check_noise_name(name).is_ok())
        .collect();
    names.sort();
    Ok(names)
}

/// The noise `name` of `data`, each memory a rule that holds no tracer of
/// `tasks`. The default noise is none when the directory has no file for it;
/// any other that is not there is refused.
pub fn load_noise(data: &Path, name: &str, tasks: &[Task]) -> Result<Vec<SeedMemory>, TaskError> {
    check_noise_name(name)?;
    let path = data.join(format!("{name}.json"));
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && name == DEFAULT_NOISE => return Ok(Vec::new()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(TaskError::NoNoise(path)),
        Err(source) => return Err(TaskError::Read { path, source }),
    };
    let noise: Vec<SeedMemory> = serde_json::from_str(&text).map_err(|source| TaskError::NoiseJson { path: path.clone(), source })?;
    let mut found = Vec::new();
    for (i, memory) in noise.iter().enumerate() {
        let name = format!("noise memory {}", i + 1);
        found.extend(memory_problems(&name, memory));
        if memory.rule.is_none() {
            found.push(format!("{name} is not a rule"));
        }
        for task in tasks {
            for tracer in task.every_tracer().filter(|tr| contains_ci(&memory.content, tr)) {
                found.push(format!("{name} holds the tracer {tracer:?} of the task {}", task.id));
            }
        }
    }
    if found.is_empty() { Ok(noise) } else { Err(TaskError::Invalid { path, problems: found.join("; ") }) }
}

/// The tracers of `task` that a fixture file already holds: the agent could
/// read them without any memory, so the task measures nothing.
pub fn leaked_tracers(task: &Task, files: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    for tracer in task.every_tracer() {
        for (path, text) in files {
            if contains_ci(text, tracer) {
                out.push(format!("the tracer {tracer:?} is in the fixture file {path}"));
            }
        }
    }
    out
}

pub fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// A relative path that stays below the directory it is joined to.
pub fn is_relative_inside(path: &str) -> bool {
    let p = Path::new(path);
    !path.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// Every task in `data/tasks/*.json`, by file name, each valid and with its fixture.
pub fn load(data: &Path) -> Result<Vec<Task>, TaskError> {
    let dir = data.join("tasks");
    let entries = std::fs::read_dir(&dir).map_err(|source| TaskError::Read { path: dir.clone(), source })?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    let mut tasks: Vec<Task> = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(&path).map_err(|source| TaskError::Read { path: path.clone(), source })?;
        let task = parse(&text).map_err(|source| TaskError::Json { path: path.clone(), source })?;
        let mut found = problems(&task);
        if !fixture_dir(data, &task).is_dir() {
            found.push(format!("the fixture {} is not in {}", task.fixture, data.join("fixtures").display()));
        }
        if !found.is_empty() {
            return Err(TaskError::Invalid { path, problems: found.join("; ") });
        }
        if tasks.iter().any(|t| t.id == task.id) {
            return Err(TaskError::DuplicateId(task.id));
        }
        tasks.push(task);
    }
    Ok(tasks)
}

pub fn find(tasks: Vec<Task>, id: &str) -> Result<Task, TaskError> {
    tasks.into_iter().find(|t| t.id == id).ok_or_else(|| TaskError::Unknown(id.to_owned()))
}

pub fn fixture_dir(data: &Path, task: &Task) -> PathBuf {
    data.join("fixtures").join(&task.fixture)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TASK: &str = r#"{
        "id": "staging-port",
        "category": "K1",
        "fixture": "tally",
        "prompt": "Which port does staging use?",
        "memories": [{"content": "Staging listens on port 48213", "age_days": 40}],
        "tracers": ["48213"],
        "checks": [
            {"kind": "answer_contains", "text": "48213"},
            {"kind": "file_lacks", "path": "src/app.py", "text": "8080"},
            {"kind": "tool_called", "name": "search_memory"}
        ]
    }"#;

    #[test]
    fn a_task_reads_with_its_defaults() {
        let task = parse(TASK).unwrap();
        assert_eq!(task.id, "staging-port");
        assert!(!task.example);
        let memory = &task.memories[0];
        assert_eq!((memory.memory_type.as_str(), memory.importance, memory.access_count), ("semantic_fact", 5.0, 1));
        assert_eq!(memory.age_days, 40);
        assert!(!memory.user_stated, "an agent saved it");
        assert_eq!(memory.rule, None);
        assert_eq!(task.checks[0], Check::AnswerContains { text: "48213".into() });
        assert_eq!(task.checks[1], Check::FileLacks { path: "src/app.py".into(), text: "8080".into() });
        assert!(problems(&task).is_empty(), "{:?}", problems(&task));
    }

    #[test]
    fn an_unknown_field_or_check_is_refused() {
        assert!(parse(&TASK.replace("\"prompt\"", "\"promt\"")).is_err());
        assert!(parse(&TASK.replace("answer_contains", "answer_is")).is_err());
        assert!(parse(&TASK.replace("\"text\": \"48213\"}", "\"text\": \"48213\", \"path\": \"x\"}")).is_err());
    }

    #[test]
    fn a_task_names_what_is_wrong_with_it() {
        let mut task = parse(TASK).unwrap();
        task.id = "Staging Port".into();
        task.category = "Z".into();
        task.fixture = "../up".into();
        task.checks.push(Check::FileContains { path: "../outside".into(), text: "x".into() });
        task.memories.push(SeedMemory {
            content: " ".into(),
            memory_type: "fact".into(),
            importance: 11.0,
            age_days: 0,
            access_count: 0,
            topic_key: None,
            user_stated: false,
            rule: None,
        });
        task.tracers.push("not seeded".into());
        let found = problems(&task).join("\n");
        for expected in [
            "the id \"Staging Port\"",
            "unknown category Z",
            "the fixture \"../up\"",
            "\"../outside\" leaves",
            "memory 2 is empty",
            "unknown type fact",
            "importance 11",
            "fewer than one access",
            "\"not seeded\" is in no seeded memory",
        ] {
            assert!(found.contains(expected), "{expected} missing from:\n{found}");
        }
    }

    const SESSIONS: &str = r#"{
        "id": "relay",
        "category": "R",
        "fixture": "tally",
        "sessions": [
            {"prompt": "Inventory the timeouts and decide that the default is 4.5, 41.", "tracers": ["4.5, 41"],
             "checks": [{"kind": "tool_called", "name": "Write"}]},
            {"prompt": "The review passed: apply it."}
        ],
        "checks": [{"kind": "file_contains", "path": "a.py", "text": "x"}]
    }"#;

    #[test]
    fn a_task_has_one_prompt_or_a_list_of_sessions() {
        let one = parse(TASK).unwrap();
        assert_eq!(one.prompts(), ["Which port does staging use?"]);
        let many = parse(SESSIONS).unwrap();
        assert_eq!(many.prompts(), ["Inventory the timeouts and decide that the default is 4.5, 41.", "The review passed: apply it."]);
        assert!(problems(&many).is_empty(), "{:?}", problems(&many));

        let mut both = many.clone();
        both.prompt = "one".into();
        assert!(problems(&both).iter().any(|p| p.contains("a prompt or sessions, not both")), "{:?}", problems(&both));
        let mut neither = many.clone();
        neither.sessions.clear();
        assert!(problems(&neither).iter().any(|p| p.contains("needs a prompt or sessions")), "{:?}", problems(&neither));
        let mut empty = many.clone();
        empty.sessions[1].prompt = " ".into();
        assert!(problems(&empty).iter().any(|p| p.contains("the prompt of session 2 is empty")));

        let mut checks_in_a_session_only = many.clone();
        checks_in_a_session_only.checks.clear();
        assert!(problems(&checks_in_a_session_only).is_empty(), "a check of a session is a check");
        checks_in_a_session_only.sessions[0].checks.clear();
        assert!(problems(&checks_in_a_session_only).iter().any(|p| p.contains("at least one check")));
    }

    #[test]
    fn a_tracer_of_a_session_is_born_in_its_prompt_and_told_nowhere_else() {
        let many = parse(SESSIONS).unwrap();
        let found = |t: &Task| problems(t).join("
");

        let mut repeated = many.clone();
        repeated.sessions[1].prompt = "Use 4.5, 41 as agreed.".into();
        assert!(found(&repeated).contains("the tracer \"4.5, 41\" of session 1 is in the prompt of session 2"), "{}", found(&repeated));

        let mut elsewhere = many.clone();
        elsewhere.sessions[0].tracers = vec!["99.9".into()];
        assert!(found(&elsewhere).contains("the tracer \"99.9\" of session 1 is not in its prompt"), "{}", found(&elsewhere));

        let mut seeded = many.clone();
        seeded.memories.push(SeedMemory { content: "The default is 4.5, 41".into(), ..memory_of_age(10) });
        assert!(found(&seeded).contains("of session 1 is in a seeded memory"), "{}", found(&seeded));

        let mut of_the_task = many.clone();
        of_the_task.tracers = vec!["timeouts".into()];
        of_the_task.memories.push(SeedMemory { content: "About timeouts".into(), ..memory_of_age(10) });
        assert!(found(&of_the_task).contains("\"timeouts\" is in the prompt"), "a tracer of the task is in no prompt: {}", found(&of_the_task));

        let files = vec![("a.py".to_owned(), "TIMEOUT = (4.5,41)".to_owned()), ("b.py".to_owned(), "nothing".to_owned())];
        assert!(leaked_tracers(&many, &files).is_empty(), "the formatting differs");
        let files = vec![("a.py".to_owned(), "# 4.5, 41".to_owned())];
        assert_eq!(leaked_tracers(&many, &files), vec!["the tracer \"4.5, 41\" is in the fixture file a.py".to_owned()]);
    }

    fn memory_of_age(age_days: u32) -> SeedMemory {
        SeedMemory {
            content: String::new(),
            memory_type: default_type(),
            importance: default_importance(),
            age_days,
            access_count: 1,
            topic_key: None,
            user_stated: false,
            rule: None,
        }
    }

    #[test]
    fn a_check_of_a_session_reads_only_the_stream() {
        let mut many = parse(SESSIONS).unwrap();
        many.sessions[1].checks = vec![
            Check::AnswerContains { text: "done".into() },
            Check::AnswerLacks { text: "no".into() },
            Check::ToolCalled { name: "Edit".into() },
            Check::FileContains { path: "a.py".into(), text: "x".into() },
            Check::FileLacks { path: "a.py".into(), text: "y".into() },
        ];
        let found = problems(&many);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].contains("session 2 has the check a.py contains \"x\", which reads files and not the stream"), "{found:?}");
    }

    #[test]
    fn a_task_written_before_sessions_still_reads() {
        let task = parse(TASK).unwrap();
        assert!(task.sessions.is_empty());
        let back = serde_json::to_value(&task).unwrap();
        assert!(back.get("sessions").is_none(), "{back}");
        assert_eq!(back["prompt"], "Which port does staging use?");
        let many = serde_json::to_value(parse(SESSIONS).unwrap()).unwrap();
        assert!(many.get("prompt").is_none(), "{many}");
        assert_eq!(parse(&many.to_string()).unwrap(), parse(SESSIONS).unwrap());
        assert!(parse(&SESSIONS.replace("\"prompt\": \"The review", "\"prompts\": \"The review")).is_err());
    }

    #[test]
    fn the_noise_holds_no_tracer_born_in_a_session() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("noise.json"), r#"[{"content": "Defaults\nUse 4.5, 41 for every call.", "rule": {}}]"#).unwrap();
        let found = load_noise(dir.path(), DEFAULT_NOISE, &[parse(SESSIONS).unwrap()]).unwrap_err().to_string();
        assert!(found.contains("noise memory 1 holds the tracer \"4.5, 41\" of the task relay"), "{found}");
    }

    #[test]
    fn a_task_names_the_shell_commands_it_allows_and_those_that_must_be_missing() {
        let with = |fields: &str| TASK.replace("\"checks\"", &format!("{fields}, \"checks\""));
        let task = parse(&with(r#""shell": ["eza", "ls", "wc-l_2"], "missing_commands": ["eza"]"#)).unwrap();
        assert_eq!((task.shell.as_slice(), task.missing_commands.as_slice()), (["eza".to_owned(), "ls".to_owned(), "wc-l_2".to_owned()].as_slice(), ["eza".to_owned()].as_slice()));
        assert!(problems(&task).is_empty(), "{:?}", problems(&task));
        let plain = serde_json::to_value(parse(TASK).unwrap()).unwrap();
        assert!(plain.get("shell").is_none() && plain.get("missing_commands").is_none(), "{plain}");

        let found = problems(&parse(&with(r#""shell": ["ls -a", "", "a/b", "rm;ls"], "missing_commands": ["eza"]"#)).unwrap()).join("
");
        for expected in [
            "the shell command \"ls -a\" is not letters, digits, dashes and underscores",
            "the shell command \"\" is not letters",
            "the shell command \"a/b\" is not letters",
            "\"rm;ls\" is not letters",
            "the missing command \"eza\" is not in the shell",
        ] {
            assert!(found.contains(expected), "{expected} missing from:
{found}");
        }
    }

    #[test]
    fn e_is_a_category() {
        assert!(CATEGORIES.contains(&"E"));
        let task = parse(&TASK.replace("\"K1\"", "\"E\"")).unwrap();
        assert!(problems(&task).is_empty());
    }

    fn with_memory_fields(fields: &str) -> String {
        TASK.replace(r#""age_days": 40}"#, &format!(r#""age_days": 40, {fields}}}"#))
    }

    #[test]
    fn a_rule_reads_with_or_without_paths() {
        let task = parse(&with_memory_fields(r#""user_stated": true, "rule": {"paths": ["deploy/*.env", "**/*.md"]}"#)).unwrap();
        let memory = &task.memories[0];
        assert!(memory.user_stated);
        assert_eq!(memory.rule, Some(Rule { paths: vec!["deploy/*.env".into(), "**/*.md".into()] }));
        assert!(problems(&task).is_empty(), "{:?}", problems(&task));

        let always = parse(&with_memory_fields(r#""rule": {}"#)).unwrap();
        assert_eq!(always.memories[0].rule, Some(Rule { paths: Vec::new() }), "a rule without paths is one of every session");
        assert!(parse(&with_memory_fields(r#""rule": {"globs": ["*.md"]}"#)).is_err());
    }

    #[test]
    fn a_rule_path_must_stay_inside() {
        let mut task = parse(TASK).unwrap();
        task.memories[0].rule = Some(Rule { paths: vec!["../*.md".into(), String::new(), "src/**/*.py".into()] });
        let found = problems(&task).join("\n");
        assert!(found.contains("memory 1 has the rule path \"../*.md\", which leaves the working directory"), "{found}");
        assert!(found.contains("memory 1 has an empty rule path"), "{found}");
        assert!(!found.contains("src/**/*.py"), "{found}");
    }

    #[test]
    fn the_noise_is_rules_that_hold_no_tracer() {
        let dir = tempfile::tempdir().unwrap();
        let tasks = vec![parse(TASK).unwrap()];
        let write = |text: &str| std::fs::write(dir.path().join("noise.json"), text).unwrap();

        write(r#"[{"content": "Commit messages\nConventional commits.", "rule": {}},
                  {"content": "Go errors\nWrap them.", "age_days": 30, "rule": {"paths": ["**/*.go"]}}]"#);
        let noise = load_noise(dir.path(), DEFAULT_NOISE, &tasks).unwrap();
        assert_eq!(noise.len(), 2);
        assert_eq!(noise[1].rule, Some(Rule { paths: vec!["**/*.go".into()] }));

        write(r#"[{"content": "Not a rule"}, {"content": "The proxy listens on 48213", "rule": {}},
                  {"content": " ", "rule": {"paths": ["../x"]}}]"#);
        let found = load_noise(dir.path(), DEFAULT_NOISE, &tasks).unwrap_err().to_string();
        for expected in [
            "noise.json",
            "noise memory 1 is not a rule",
            "noise memory 2 holds the tracer \"48213\" of the task staging-port",
            "noise memory 3 is empty",
            "noise memory 3 has the rule path \"../x\"",
        ] {
            assert!(found.contains(expected), "{expected} missing from: {found}");
        }

        write("{}");
        assert!(load_noise(dir.path(), DEFAULT_NOISE, &tasks).unwrap_err().to_string().contains("is not a list of seeded memories"));
    }

    #[test]
    fn a_data_directory_without_noise_has_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_noise(dir.path(), DEFAULT_NOISE, &[]).unwrap(), Vec::<SeedMemory>::new());
    }

    #[test]
    fn a_run_is_seeded_with_the_noise_it_names() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, text: &str| std::fs::write(dir.path().join(name), text).unwrap();
        write("noise.json", r#"[{"content": "Small\nOne rule.", "rule": {}}]"#);
        write("noise-big.json", r#"[{"content": "Big one\nA rule.", "rule": {}}, {"content": "Big two\nAnother rule.", "rule": {"paths": ["**/*.go"]}}]"#);
        write("other.json", r#"[{"content": "Not noise\nA rule.", "rule": {}}]"#);

        assert_eq!(load_noise(dir.path(), "noise", &[]).unwrap().len(), 1, "noise is the default");
        assert_eq!(load_noise(dir.path(), "noise-big", &[]).unwrap().len(), 2);
        for refused in ["other", "tasks", "noise_big", "noise-", "noise-../x", "noise/x", "", "Noise-big", "noise-big.json"] {
            let said = load_noise(dir.path(), refused, &[]).unwrap_err().to_string();
            assert!(said.contains("is not a noise") && said.contains(&format!("{refused:?}")), "{refused}: {said}");
        }
        let missing = load_noise(dir.path(), "noise-none", &[]).unwrap_err().to_string();
        assert!(missing.contains("noise-none.json") && missing.contains("no such"), "a named noise that is not there is refused: {missing}");
        assert_eq!(noise_names(dir.path()).unwrap(), ["noise", "noise-big"], "every noise file, and no other");

        write("noise-big.json", r#"[{"content": "Big one\nThe proxy listens on 48213", "rule": {}}]"#);
        let found = load_noise(dir.path(), "noise-big", &[parse(TASK).unwrap()]).unwrap_err().to_string();
        assert!(found.contains("noise-big.json") && found.contains("holds the tracer \"48213\""), "each noise is checked as the default one is: {found}");
    }

    #[test]
    fn a_tracer_in_the_prompt_or_the_fixture_gives_the_fact_away() {
        let mut task = parse(TASK).unwrap();
        task.prompt = "Is staging still on 48213?".into();
        assert!(problems(&task).iter().any(|p| p.contains("is in the prompt")));
        let files = vec![("README.md".to_owned(), "nothing".to_owned()), ("deploy.env".to_owned(), "PORT=48213".to_owned())];
        assert_eq!(leaked_tracers(&task, &files), vec!["the tracer \"48213\" is in the fixture file deploy.env".to_owned()]);
    }

    #[test]
    fn a_path_must_stay_inside() {
        assert!(is_relative_inside("src/app.py"));
        assert!(is_relative_inside("./a"));
        assert!(!is_relative_inside("../a"));
        assert!(!is_relative_inside("a/../../b"));
        assert!(!is_relative_inside("/etc/passwd"));
        assert!(!is_relative_inside(""));
    }

    #[test]
    fn load_reads_every_task_and_refuses_a_missing_fixture_or_a_repeated_id() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("tasks")).unwrap();
        std::fs::create_dir_all(dir.path().join("fixtures/tally")).unwrap();
        std::fs::write(dir.path().join("tasks/a.json"), TASK).unwrap();
        std::fs::write(dir.path().join("tasks/notes.txt"), "not a task").unwrap();
        let tasks = load(dir.path()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(find(tasks, "nope").is_err());

        std::fs::write(dir.path().join("tasks/b.json"), TASK).unwrap();
        assert!(matches!(load(dir.path()), Err(TaskError::DuplicateId(id)) if id == "staging-port"));

        std::fs::remove_file(dir.path().join("tasks/b.json")).unwrap();
        std::fs::remove_dir(dir.path().join("fixtures/tally")).unwrap();
        assert!(load(dir.path()).unwrap_err().to_string().contains("the fixture tally is not in"));
    }
}
