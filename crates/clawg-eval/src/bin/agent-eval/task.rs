//! A task: the prompt an agent session gets, the fixture it works in, the
//! memories each arm is seeded with, the tracers that tell where a fact came
//! from and the checks that decide, without a judge, whether it succeeded.
//! Reading and validating are pure; [`load`] is the only part that reads disk.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The categories a task may have. Each one is checked without a judge.
pub const CATEGORIES: [&str; 11] = ["K1", "K2", "K3", "K4", "K5", "S", "C", "N", "Q", "D", "R"];

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
    pub prompt: String,
    /// What the arms with memory know before the session; the same facts for
    /// clawg's store and for the client's native memory.
    #[serde(default)]
    pub memories: Vec<SeedMemory>,
    /// Strings found only in the seeded memories: where one shows up in the
    /// session tells whether the fact reached the agent and whether it was used.
    #[serde(default)]
    pub tracers: Vec<String>,
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
}

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
}

impl Check {
    pub fn describe(&self) -> String {
        match self {
            Check::AnswerContains { text } => format!("answer contains {text:?}"),
            Check::AnswerLacks { text } => format!("answer lacks {text:?}"),
            Check::FileContains { path, text } => format!("{path} contains {text:?}"),
            Check::FileLacks { path, text } => format!("{path} lacks {text:?}"),
            Check::ToolCalled { name } => format!("{name} was called"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: std::io::Error },
    #[error("{path} is not a task: {source}")]
    Json { path: PathBuf, source: serde_json::Error },
    #[error("{path}: {problems}")]
    Invalid { path: PathBuf, problems: String },
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
    if task.prompt.trim().is_empty() {
        out.push("the prompt is empty".to_owned());
    }
    if task.checks.is_empty() {
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
        if memory.content.trim().is_empty() {
            out.push(format!("memory {} is empty", i + 1));
        }
        if !MEMORY_TYPES.contains(&memory.memory_type.as_str()) {
            out.push(format!("memory {} has the unknown type {}", i + 1, memory.memory_type));
        }
        if !(0.0..=10.0).contains(&memory.importance) {
            out.push(format!("memory {} has importance {} outside 0 to 10", i + 1, memory.importance));
        }
        if memory.access_count < 1 {
            out.push(format!("memory {} has fewer than one access", i + 1));
        }
    }
    for tracer in &task.tracers {
        if tracer.trim().is_empty() {
            out.push("a tracer is empty".to_owned());
        } else if !task.memories.iter().any(|m| contains_ci(&m.content, tracer)) {
            out.push(format!("the tracer {tracer:?} is in no seeded memory"));
        } else if contains_ci(&task.prompt, tracer) {
            out.push(format!("the tracer {tracer:?} is in the prompt"));
        }
    }
    out
}

/// The tracers of `task` that a fixture file already holds: the agent could
/// read them without any memory, so the task measures nothing.
pub fn leaked_tracers(task: &Task, files: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    for tracer in &task.tracers {
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
