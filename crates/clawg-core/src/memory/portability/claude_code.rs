//! The adapter for Claude Code's auto memory: under the client's projects
//! directory, a `memory/` per project with an index, `MEMORY.md`, and one
//! Markdown file per memory, read into the notes of [`super::foreign`].
//!
//! The client does not version this format and writes the type of a memory
//! either as `type` or nested as `metadata.type`, so both are read, and what
//! the adapter does not know is counted, never a reason to refuse a file.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::foreign::{parse_utc, ForeignError, ForeignExport, ForeignNote, NoteKind, When};

pub const SOURCE: &str = "claude-code";
/// The index of a memory directory, one line per memory.
pub const INDEX: &str = "MEMORY.md";
/// The directory of a project that holds its memory.
pub const MEMORY_DIR: &str = "memory";

/// The frontmatter keys a note is made of; every other is counted.
const KNOWN_KEYS: [&str; 6] = ["name", "description", "type", "metadata.type", "modified", "metadata.modified"];

/// Claude Code's memory as read from disk, one project per memory directory:
/// what an import from it carries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryDocument {
    pub projects: Vec<MemoryProject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryProject {
    /// The name of the project's directory under the client's projects.
    pub name: String,
    /// Every entry of its memory directory, by name.
    pub entries: Vec<MemoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub name: String,
    /// The text of a topic file or of the index; `None` for any other entry,
    /// which is never read, and for a file that is not UTF-8.
    #[serde(default)]
    pub text: Option<String>,
    /// When the file was last written, in milliseconds since the epoch.
    #[serde(default)]
    pub modified_ms: Option<i64>,
}

/// Names a topic file in its source, stable across imports.
pub fn origin(project: &str, file: &str) -> String {
    format!("{SOURCE}:{project}/{file}")
}

/// A Markdown file of a memory directory that is not its index.
pub fn is_topic_file(name: &str) -> bool {
    name.ends_with(".md") && name != INDEX
}

/// Why a directory gives no memory to import.
#[derive(Debug, thiserror::Error)]
pub enum CollectError {
    #[error("{} is not a directory", .path.display())]
    NotADirectory { path: PathBuf },
    #[error(
        "{} holds no Claude Code memory: name a project's memory directory, the project directory that holds it, \
         or the projects directory",
        .path.display()
    )]
    NoMemory { path: PathBuf },
    #[error("{} cannot be read: {source}", .path.display())]
    Unreadable { path: PathBuf, source: std::io::Error },
    #[error("the memory is {bytes} bytes; at most {limit} are imported")]
    TooLarge { bytes: usize, limit: usize },
}

/// Reads the memory under `path`: a project's memory directory (it holds the
/// index or is named `memory`), the project directory that holds one, or the
/// projects directory, whose every project with one is read. At most
/// `max_bytes` of text are read.
pub fn collect(path: &Path, max_bytes: usize) -> Result<MemoryDocument, CollectError> {
    if !path.is_dir() {
        return Err(CollectError::NotADirectory { path: path.to_owned() });
    }
    let name_of = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let dirs: Vec<(String, PathBuf)> = if path.join(INDEX).is_file() || name_of(path) == MEMORY_DIR {
        vec![(path.parent().map(name_of).unwrap_or_default(), path.to_owned())]
    } else if path.join(MEMORY_DIR).is_dir() {
        vec![(name_of(path), path.join(MEMORY_DIR))]
    } else {
        sorted_entries(path)?
            .into_iter()
            .map(|p| (name_of(&p), p.join(MEMORY_DIR)))
            .filter(|(_, memory)| memory.is_dir())
            .collect()
    };
    if dirs.is_empty() {
        return Err(CollectError::NoMemory { path: path.to_owned() });
    }
    let mut bytes = 0;
    let mut projects = Vec::with_capacity(dirs.len());
    for (name, dir) in dirs {
        let mut entries = Vec::new();
        for entry in sorted_entries(&dir)? {
            let file = name_of(&entry);
            let meta = std::fs::metadata(&entry).ok();
            let readable = meta.as_ref().is_some_and(|m| m.is_file()) && (file == INDEX || is_topic_file(&file));
            let text = if readable {
                let raw = std::fs::read(&entry).map_err(|source| CollectError::Unreadable { path: entry.clone(), source })?;
                bytes += raw.len();
                if bytes > max_bytes {
                    return Err(CollectError::TooLarge { bytes, limit: max_bytes });
                }
                String::from_utf8(raw).ok()
            } else {
                None
            };
            let modified_ms = meta
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .and_then(|d| i64::try_from(d.as_millis()).ok());
            entries.push(MemoryEntry { name: file, text, modified_ms });
        }
        projects.push(MemoryProject { name, entries });
    }
    Ok(MemoryDocument { projects })
}

fn sorted_entries(dir: &Path) -> Result<Vec<PathBuf>, CollectError> {
    let unreadable = |source| CollectError::Unreadable { path: dir.to_owned(), source };
    let mut paths = std::fs::read_dir(dir)
        .map_err(unreadable)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(unreadable)?;
    paths.sort();
    Ok(paths)
}

/// Reads a [`MemoryDocument`] written as JSON.
pub fn read(text: &str) -> Result<ForeignExport, ForeignError> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| ForeignError::NotJson { format: SOURCE, cause: e.to_string() })?;
    let document = serde_json::from_value(value).map_err(|e| ForeignError::Malformed {
        format: SOURCE,
        version: String::new(),
        cause: e.to_string(),
    })?;
    Ok(from_document(document))
}

/// Every topic file of `document` as a note; the index and every other entry
/// only counted where they hold something no note carries.
pub fn from_document(document: MemoryDocument) -> ForeignExport {
    let mut notes = Vec::new();
    let mut unplaced = BTreeMap::new();
    let mut count = |key: String| *unplaced.entry(key).or_insert(0) += 1;
    for project in &document.projects {
        let topics: BTreeSet<&str> = project
            .entries
            .iter()
            .filter(|e| is_topic_file(&e.name) && e.text.is_some())
            .map(|e| e.name.as_str())
            .collect();
        for entry in &project.entries {
            match &entry.text {
                Some(text) if entry.name == INDEX => {
                    for _ in index_lines_without_a_topic(text, &topics) {
                        count("index_line".into());
                    }
                }
                Some(text) if is_topic_file(&entry.name) => {
                    let (note, keys) = note_of(&project.name, entry, text);
                    keys.into_iter().for_each(&mut count);
                    notes.push(note);
                }
                _ => count("entry".into()),
            }
        }
    }
    ForeignExport {
        source: SOURCE,
        version: String::new(),
        notes,
        relations: Vec::new(),
        unplaced,
        rewritten: 0,
        discarded_sessions: 0,
    }
}

/// The note of a topic file, and the keys of its frontmatter no note carries.
fn note_of(project: &str, entry: &MemoryEntry, text: &str) -> (ForeignNote, BTreeSet<String>) {
    let front = Frontmatter::of(text);
    let value = |key: &str| front.values.get(key).map(|v| v.trim()).filter(|v| !v.is_empty());
    let title = value("name").map_or_else(|| entry.name.trim_end_matches(".md").to_owned(), str::to_owned);
    let body = front.body.trim();
    let content = match value("description") {
        Some(description) if !body.is_empty() => format!("{description}\n\n{body}"),
        Some(description) => description.to_owned(),
        None => body.to_owned(),
    };
    let written = value("modified")
        .or_else(|| value("metadata.modified"))
        .and_then(parse_utc)
        .or(entry.modified_ms)
        .map_or(When::Unreadable, When::At);
    let note = ForeignNote {
        origin: origin(project, &entry.name),
        kind: NoteKind::Observation,
        source_type: value("type").or_else(|| value("metadata.type")).map(str::to_owned),
        title,
        content,
        project: Some(project.to_owned()).filter(|p| !p.is_empty()),
        personal: false,
        topic_key: None,
        session: None,
        written,
        seen: 0,
        last_seen: None,
        deleted: None,
        pinned: false,
    };
    (note, front.unplaced)
}

/// The lines of an index that are not blank, not a heading, and link to no
/// topic file of `topics`.
fn index_lines_without_a_topic<'a>(text: &'a str, topics: &'a BTreeSet<&str>) -> impl Iterator<Item = &'a str> {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).filter(move |line| {
        !line.split("](").skip(1).any(|rest| {
            let target = rest.split(')').next().unwrap_or_default();
            let target = target.split('#').next().unwrap_or_default().trim_matches(['<', '>']);
            topics.contains(target.rsplit('/').next().unwrap_or_default())
        })
    })
}

/// What a file's frontmatter says: its known keys by path, the others
/// counted, and the body under it.
struct Frontmatter<'a> {
    values: BTreeMap<String, String>,
    unplaced: BTreeSet<String>,
    body: &'a str,
}

impl<'a> Frontmatter<'a> {
    /// The frontmatter between a first line `---` and the next `---`; without
    /// both, the whole text is body.
    fn of(text: &'a str) -> Self {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut front = Frontmatter { values: BTreeMap::new(), unplaced: BTreeSet::new(), body: text };
        let mut lines = text.split_inclusive('\n');
        if lines.next().map(str::trim_end) != Some("---") {
            return front;
        }
        let mut block = Vec::new();
        let mut offset = text.split_inclusive('\n').next().map_or(0, str::len);
        for line in lines {
            offset += line.len();
            if line.trim_end() == "---" {
                front.read(&block, &mut 0, 0, "", 0);
                front.body = &text[offset..];
                return front;
            }
            block.push(line.trim_end_matches(['\n', '\r']));
        }
        front.unplaced.insert("frontmatter.unclosed".into());
        front
    }

    /// Reads the entries of `lines` from `*i` that are indented at least
    /// `indent`, under `prefix`. A map is read one level deep; a list or a
    /// deeper map is counted by its key.
    fn read(&mut self, lines: &[&str], i: &mut usize, indent: usize, prefix: &str, depth: usize) {
        while let Some(line) = lines.get(*i) {
            let own = indent_of(line);
            if ignored(line) {
                *i += 1;
                continue;
            }
            if own < indent {
                return;
            }
            *i += 1;
            let Some((key, value)) = key_value(line.trim()) else {
                self.count(if prefix.is_empty() { "line" } else { prefix });
                skip_deeper(lines, i, own);
                continue;
            };
            let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
            if value.is_empty() {
                match next_line(lines, *i) {
                    Some(next) if indent_of(next) > own => {
                        if depth == 0 && key_value(next.trim()).is_some() {
                            self.read(lines, i, indent_of(next), &path, depth + 1);
                        } else {
                            self.count(&path);
                            skip_deeper(lines, i, own);
                        }
                    }
                    _ => self.set(path, String::new()),
                }
            } else if let Some(folded) = block_style(value) {
                let text = deeper_lines(lines, i, own);
                let start = text.iter().filter(|l| !l.trim().is_empty()).map(|l| indent_of(l)).min().unwrap_or(0);
                let text: Vec<&str> = text.iter().map(|l| l.get(start..).unwrap_or("")).collect();
                let joined = if folded { fold(&text) } else { text.join("\n") };
                self.set(path, joined.trim_end().to_owned());
            } else {
                let mut text = unquote(value);
                for more in deeper_lines(lines, i, own) {
                    if !more.trim().is_empty() {
                        text.push(' ');
                        text.push_str(more.trim());
                    }
                }
                self.set(path, text);
            }
        }
    }

    fn set(&mut self, path: String, value: String) {
        if KNOWN_KEYS.contains(&path.as_str()) {
            self.values.insert(path, value);
        } else {
            self.count(&path);
        }
    }

    fn count(&mut self, path: &str) {
        self.unplaced.insert(format!("frontmatter.{path}"));
    }
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

fn ignored(line: &str) -> bool {
    let text = line.trim();
    text.is_empty() || text.starts_with('#')
}

fn next_line<'l>(lines: &[&'l str], from: usize) -> Option<&'l str> {
    lines[from..].iter().copied().find(|l| !ignored(l))
}

/// The lines after `*i` more indented than `own`, blank ones among them.
fn deeper_lines<'l>(lines: &[&'l str], i: &mut usize, own: usize) -> Vec<&'l str> {
    let start = *i;
    while let Some(line) = lines.get(*i) {
        if !line.trim().is_empty() && indent_of(line) <= own {
            break;
        }
        *i += 1;
    }
    let mut taken = lines[start..*i].to_vec();
    while taken.last().is_some_and(|l| l.trim().is_empty()) {
        taken.pop();
    }
    taken
}

fn skip_deeper(lines: &[&str], i: &mut usize, own: usize) {
    deeper_lines(lines, i, own);
}

/// `key: value` or `key:`, the key unquoted; `None` for a list item or a
/// line that is no key.
fn key_value(text: &str) -> Option<(String, &str)> {
    if text == "-" || text.starts_with("- ") {
        return None;
    }
    let at = text.char_indices().find(|&(at, c)| c == ':' && text[at + 1..].chars().next().is_none_or(|n| n == ' '))?.0;
    let key = unquote(text[..at].trim());
    (!key.is_empty()).then(|| (key, text[at + 1..].trim()))
}

/// `Some(true)` for a folded block (`>`), `Some(false)` for a literal one (`|`).
fn block_style(value: &str) -> Option<bool> {
    let folded = match value.chars().next()? {
        '>' => true,
        '|' => false,
        _ => return None,
    };
    value[1..].chars().all(|c| matches!(c, '-' | '+' | '0'..='9')).then_some(folded)
}

/// Lines joined with spaces, a blank line kept as a line break.
fn fold(lines: &[&str]) -> String {
    let mut text = String::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            text.push('\n');
        } else {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push(' ');
            }
            text.push_str(line);
        }
    }
    text
}

fn unquote(value: &str) -> String {
    let quoted = |q: char| value.len() >= 2 && value.starts_with(q) && value.ends_with(q);
    if quoted('\'') {
        return value[1..value.len() - 1].replace("''", "'");
    }
    if !quoted('"') {
        return value.to_owned();
    }
    let mut text = String::new();
    let mut chars = value[1..value.len() - 1].chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            text.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => text.push('\n'),
            Some('t') => text.push('\t'),
            Some(other) => text.push(other),
            None => text.push('\\'),
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::super::foreign::{ForeignNote, NoteKind, When};
    use super::*;
    use crate::test_dir::test_dir;

    const NOON: i64 = 1_793_361_600_000; // 2026-10-30 12:00:00 UTC

    fn entry(name: &str, text: &str) -> MemoryEntry {
        MemoryEntry { name: name.into(), text: Some(text.into()), modified_ms: None }
    }

    fn document(project: &str, entries: Vec<MemoryEntry>) -> MemoryDocument {
        MemoryDocument { projects: vec![MemoryProject { name: project.into(), entries }] }
    }

    fn one(text: &str) -> ForeignNote {
        let export = from_document(document("-work-app", vec![entry("note.md", text)]));
        let [note] = export.notes.as_slice() else { panic!("one note: {export:?}") };
        note.clone()
    }

    fn unplaced(export: &ForeignExport) -> Vec<(&str, u64)> {
        export.unplaced.iter().map(|(k, v)| (k.as_str(), *v)).collect()
    }

    #[test]
    fn a_topic_file_is_a_note_named_by_its_project_and_file() {
        let note = one("---\nname: Deploys\ndescription: How the app ships\ntype: project\n---\n\nTag, then push.\n");
        assert_eq!(note.origin, "claude-code:-work-app/note.md");
        assert_eq!(note.origin, origin("-work-app", "note.md"));
        assert_eq!(note.kind, NoteKind::Observation);
        assert_eq!(note.source_type.as_deref(), Some("project"));
        assert_eq!((note.title.as_str(), note.content.as_str()), ("Deploys", "How the app ships\n\nTag, then push."));
        assert_eq!(note.project.as_deref(), Some("-work-app"));
        assert!(!note.personal && !note.pinned);
        assert_eq!((note.topic_key.clone(), note.session.clone(), note.seen), (None, None, 0));
        assert_eq!((note.last_seen, note.deleted), (None, None));

        let bare = one("---\ntype: user\n---\nOnly a body");
        assert_eq!((bare.title.as_str(), bare.content.as_str()), ("note", "Only a body"));
        let described = one("---\nname: n\ndescription: only a description\n---\n");
        assert_eq!(described.content, "only a description");
    }

    #[test]
    fn the_type_reads_top_level_or_nested_under_metadata() {
        assert_eq!(one("---\nname: a\ntype: feedback\n---\nx").source_type.as_deref(), Some("feedback"));
        assert_eq!(one("---\nname: a\nmetadata:\n  type: feedback\n---\nx").source_type.as_deref(), Some("feedback"));
        assert_eq!(
            one("---\nname: a\ntype: user\nmetadata:\n  type: feedback\n---\nx").source_type.as_deref(),
            Some("user"),
            "the top-level type wins"
        );
        assert_eq!(one("---\nname: a\n---\nx").source_type, None);
        let export = from_document(document("p", vec![entry("a.md", "---\nmetadata:\n  type: project\n---\nx")]));
        assert!(export.unplaced.is_empty(), "{:?}", export.unplaced);
    }

    #[test]
    fn quoted_and_block_values_read_as_text() {
        let note = one("---\nname: \"Port: 27380\"\ndescription: 'it''s the daemon'\ntype: \"user\"\n---\nx");
        assert_eq!((note.title.as_str(), note.source_type.as_deref()), ("Port: 27380", Some("user")));
        assert!(note.content.starts_with("it's the daemon\n\n"), "{}", note.content);
        let literal = one("---\nname: a\ndescription: |\n  first line\n  second line\n---\nbody");
        assert_eq!(literal.content, "first line\nsecond line\n\nbody");
        let folded = one("---\nname: a\ndescription: >-\n  one\n  sentence\n---\nbody");
        assert_eq!(folded.content, "one sentence\n\nbody");
        let plain = one("---\nname: a\ndescription: a long\n  description\n---\nbody");
        assert_eq!(plain.content, "a long description\n\nbody");
    }

    #[test]
    fn unknown_keys_and_an_unclosed_frontmatter_are_counted() {
        let export = from_document(document(
            "p",
            vec![
                entry("a.md", "---\nname: a\ntags:\n  - x\n  - y\nmetadata:\n  type: user\n  owner: me\n# a comment\nmood: calm\n---\nx"),
                entry("b.md", "---\nname: b\nmood: tired\n---\ny"),
                entry("c.md", "---\nname: c\ntype: user\nno end here"),
            ],
        ));
        assert_eq!(export.notes.len(), 3, "nothing is refused");
        assert_eq!(
            unplaced(&export),
            [("frontmatter.metadata.owner", 1), ("frontmatter.mood", 2), ("frontmatter.tags", 1), ("frontmatter.unclosed", 1)]
        );
        let c = export.notes.iter().find(|n| n.origin.ends_with("/c.md")).unwrap();
        assert_eq!((c.title.as_str(), c.source_type.clone()), ("c", None), "an unclosed frontmatter is body");
        assert_eq!(c.content, "---\nname: c\ntype: user\nno end here");
    }

    #[test]
    fn a_file_without_frontmatter_is_all_body() {
        let note = one("\u{feff}# Heading\n\nJust notes.\n");
        assert_eq!((note.title.as_str(), note.content.as_str()), ("note", "# Heading\n\nJust notes."));
        assert_eq!(note.source_type, None);
    }

    #[test]
    fn the_date_is_modified_or_else_the_files_time() {
        let at = |text: &str, modified_ms: Option<i64>| {
            let file = MemoryEntry { name: "n.md".into(), text: Some(text.into()), modified_ms };
            from_document(document("p", vec![file])).notes[0].written
        };
        assert_eq!(at("---\nmodified: 2026-10-30T12:00:00Z\n---\nx", Some(5)), When::At(NOON));
        assert_eq!(at("---\nmetadata:\n  modified: 2026-10-30T14:00:00+02:00\n---\nx", Some(5)), When::At(NOON));
        assert_eq!(at("---\nmodified: yesterday\n---\nx", Some(5)), When::At(5));
        assert_eq!(at("x", Some(7)), When::At(7));
        assert_eq!(at("x", None), When::Unreadable);
    }

    #[test]
    fn index_lines_without_a_topic_file_are_counted() {
        let index = "# Memory index\n\n- [Deploys](deploys.md) — how the app ships\n- [Gone](gone.md) — deleted\n\
                     - [Port](./port.md#top) and [Deploys](deploys.md)\nA stray line\n";
        let export = from_document(document(
            "p",
            vec![entry(INDEX, index), entry("deploys.md", "Tag, then push."), entry("port.md", "27380")],
        ));
        assert_eq!(export.notes.len(), 2, "the index is no note");
        assert_eq!(unplaced(&export), [("index_line", 2)]);
    }

    #[test]
    fn entries_that_are_not_topic_files_are_counted() {
        let export = from_document(document(
            "p",
            vec![
                entry("a.md", "kept"),
                MemoryEntry { name: "b.md".into(), text: None, modified_ms: None },
                MemoryEntry { name: "notes.txt".into(), text: None, modified_ms: None },
                MemoryEntry { name: "old".into(), text: None, modified_ms: None },
            ],
        ));
        assert_eq!(export.notes.len(), 1);
        assert_eq!(unplaced(&export), [("entry", 3)]);
        assert!(is_topic_file("a.md") && !is_topic_file(INDEX) && !is_topic_file("notes.txt"));
    }

    #[test]
    fn the_document_reads_from_text_and_has_no_format_version() {
        let doc = document("-work-app", vec![entry("a.md", "---\nname: A\n---\nbody")]);
        let export = read(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!((export.source, export.version.as_str()), ("claude-code", ""));
        assert_eq!(export, from_document(doc));
        assert!(matches!(read("not json"), Err(ForeignError::NotJson { .. })));
        assert!(matches!(read(r#"{"projects": 3}"#), Err(ForeignError::Malformed { .. })));
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn a_memory_dir_a_project_dir_and_the_projects_dir_read_the_same_note() {
        let dir = test_dir();
        let projects = dir.path().join("projects");
        let memory = projects.join("-work-app").join(MEMORY_DIR);
        write(&memory.join("deploys.md"), "---\nname: Deploys\ntype: project\n---\nTag, then push.");
        write(&memory.join(INDEX), "- [Deploys](deploys.md)");
        write(&projects.join("-work-empty").join("notes.txt"), "no memory dir");

        let notes = |path: &Path| from_document(collect(path, 1 << 20).unwrap()).notes;
        let from_projects = notes(&projects);
        assert_eq!(from_projects.len(), 1);
        assert_eq!(from_projects[0].origin, "claude-code:-work-app/deploys.md");
        assert!(matches!(from_projects[0].written, When::At(_)), "the file's time");
        assert_eq!(notes(&projects.join("-work-app")), from_projects);
        assert_eq!(notes(&memory), from_projects);
    }

    #[test]
    fn a_directory_without_memory_is_refused() {
        let dir = test_dir();
        let file = dir.path().join("file.md");
        write(&file, "x");
        assert!(matches!(collect(dir.path(), 1 << 20), Err(CollectError::NoMemory { .. })));
        assert!(matches!(collect(&file, 1 << 20), Err(CollectError::NotADirectory { .. })));
        assert!(matches!(collect(&dir.path().join("missing"), 1 << 20), Err(CollectError::NotADirectory { .. })));

        let memory = dir.path().join("p").join(MEMORY_DIR);
        write(&memory.join("a.md"), "0123456789");
        assert!(matches!(collect(&memory, 9), Err(CollectError::TooLarge { bytes: 10, limit: 9 })));
        assert!(collect(&memory, 10).is_ok());
    }

    #[test]
    fn a_file_that_is_not_utf8_is_listed_unread() {
        let dir = test_dir();
        let memory = dir.path().join("p").join(MEMORY_DIR);
        std::fs::create_dir_all(memory.join("sub")).unwrap();
        std::fs::write(memory.join("bad.md"), [0xff, 0xfe, 0x00]).unwrap();
        write(&memory.join("other.json"), "{}");
        let doc = collect(&memory, 1 << 20).unwrap();
        let entries: Vec<(&str, bool)> =
            doc.projects[0].entries.iter().map(|e| (e.name.as_str(), e.text.is_some())).collect();
        assert_eq!(entries, [("bad.md", false), ("other.json", false), ("sub", false)]);
        assert_eq!(doc.projects[0].name, "p");
    }
}
