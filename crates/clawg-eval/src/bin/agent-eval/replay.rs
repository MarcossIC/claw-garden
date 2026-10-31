//! The client a dry run starts instead of a real one: it prints a recorded
//! `stream-json` and makes the file changes its `Write` and `Edit` calls
//! record, so the scoring sees the working directory a session would leave.
//! Deciding the changes is pure; [`run`] does the I/O.

use std::io::Write as _;
use std::path::Path;

use serde_json::Value;

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

/// Makes the changes `stream` records in `cwd`, then prints `stream`.
pub fn run(stream: &Path, cwd: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(stream).map_err(|e| format!("cannot read {}: {e}", stream.display()))?;
    for edit in file_edits(&parse(&text))? {
        let path = cwd.join(edit.path());
        let current = std::fs::read_to_string(&path).ok();
        let after = apply(current.as_deref(), &edit)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        std::fs::write(&path, after).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    std::io::stdout().write_all(text.as_bytes()).map_err(|e| format!("cannot print the stream: {e}"))
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
        run(&recorded, dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("a.py")).unwrap(), "x = 2\n");
        assert_eq!(std::fs::read_to_string(dir.path().join("sub/b.py")).unwrap(), "y");
    }
}
