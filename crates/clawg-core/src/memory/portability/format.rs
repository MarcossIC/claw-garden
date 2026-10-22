//! The `clawg-memories` export format: JSON Lines, one object per line, each
//! with its `kind`, the header first.
//!
//! Versioned on its own, apart from the schema: a store migrates often, and a
//! file written by one clawg has to stay readable by the next. Ids in the file
//! are the source store's and only mean something inside the file.

use serde::{Deserialize, Serialize};

use crate::memory::store_insert::MemoryType;
use crate::memory::topic::validate_topic_key;

/// The `format` every header names.
pub const FORMAT: &str = "clawg-memories";
/// The only version this binary reads and writes.
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Header {
    pub format: String,
    pub version: u32,
    pub exported_at: i64,
    /// The clawg version that wrote the file.
    pub clawg: String,
    /// The model the source store's vectors came from; the import embeds again.
    #[serde(default)]
    pub embedding_model: Option<String>,
    #[serde(default)]
    pub filter: Filter,
}

/// What an export was limited to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filter {
    /// The project id the rows were limited to.
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub include_deleted: bool,
}

/// A registered project some exported row belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectLine {
    pub id: String,
    pub root_path: String,
}

/// One row of `memory`, every column but `content_hash`, which the import
/// computes again over the text it stores.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryLine {
    pub id: i64,
    pub scope: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    pub content: String,
    pub memory_type: String,
    pub source: String,
    pub created_at: i64,
    pub created_by_type: String,
    #[serde(default)]
    pub created_by_ref: String,
    #[serde(default)]
    pub source_session_id: Option<String>,
    pub last_modified_at: i64,
    #[serde(default)]
    pub last_modified_by: Option<String>,
    #[serde(default)]
    pub update_count: i64,
    pub last_accessed_at: i64,
    #[serde(default)]
    pub access_count: i64,
    pub importance: f64,
    pub tier: i64,
    pub status: String,
    #[serde(default)]
    pub superseded_by: Option<i64>,
    #[serde(default)]
    pub supersede_cause: Option<String>,
    #[serde(default)]
    pub dispute_count: i64,
    #[serde(default)]
    pub disputed_against: Vec<i64>,
    #[serde(default)]
    pub last_disputed_at: Option<i64>,
    #[serde(default)]
    pub deleted_at: Option<i64>,
    #[serde(default)]
    pub topic_key: Option<String>,
    #[serde(default)]
    pub revived_from: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivationLine {
    pub memory: i64,
    pub source: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PieceLine {
    pub save: i64,
    pub piece: i64,
    pub memory: i64,
    #[serde(default)]
    pub context: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConflictLine {
    pub new_id: i64,
    pub old_id: i64,
    #[serde(default)]
    pub distance: Option<f64>,
    pub state: String,
    #[serde(default)]
    pub verdict: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    pub created_at: i64,
    #[serde(default)]
    pub judged_at: Option<i64>,
    /// The save without a key was warned that it may revise `old_id`.
    #[serde(default)]
    pub topic_key_warned: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Line {
    Header(Header),
    Project(ProjectLine),
    Memory(MemoryLine),
    Derivation(DerivationLine),
    Piece(PieceLine),
    Conflict(ConflictLine),
}

/// A line that could not be read, by its number from 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invalid {
    pub line: usize,
    pub reason: String,
}

/// A file read, each line with its number from 1.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parsed {
    pub projects: Vec<(usize, ProjectLine)>,
    pub memories: Vec<(usize, MemoryLine)>,
    pub derivations: Vec<(usize, DerivationLine)>,
    pub pieces: Vec<(usize, PieceLine)>,
    pub conflicts: Vec<(usize, ConflictLine)>,
    pub invalid: Vec<Invalid>,
}

/// Why a file is not read at all.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormatError {
    #[error("the file is empty")]
    Empty,
    #[error("the first line is not a {FORMAT} header: {0}")]
    NoHeader(String),
    #[error("the file is in format {0:?}, not {FORMAT}")]
    OtherFormat(String),
    #[error("the file is {FORMAT} version {0}; this clawg reads version {VERSION}")]
    UnknownVersion(u32),
}

/// Reads `text`: the header must be the first line that is not blank, and be
/// of this format and version, or nothing is read. Every other line that
/// cannot be read is listed in `invalid` and the rest is kept.
pub fn parse(text: &str) -> Result<(Header, Parsed), FormatError> {
    let mut lines = text.lines().enumerate().map(|(i, l)| (i + 1, l)).filter(|(_, l)| !l.trim().is_empty());
    let (_, first) = lines.next().ok_or(FormatError::Empty)?;
    let header = read_header(first)?;
    let mut parsed = Parsed::default();
    for (n, raw) in lines {
        match read_line(raw) {
            Ok(Line::Header(_)) => parsed.invalid.push(Invalid { line: n, reason: "a second header".into() }),
            Ok(Line::Project(p)) => parsed.projects.push((n, p)),
            Ok(Line::Memory(m)) => parsed.memories.push((n, m)),
            Ok(Line::Derivation(d)) => parsed.derivations.push((n, d)),
            Ok(Line::Piece(p)) => parsed.pieces.push((n, p)),
            Ok(Line::Conflict(c)) => parsed.conflicts.push((n, c)),
            Err(reason) => parsed.invalid.push(Invalid { line: n, reason }),
        }
    }
    Ok((header, parsed))
}

fn read_header(raw: &str) -> Result<Header, FormatError> {
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|e| FormatError::NoHeader(e.to_string()))?;
    if value.get("kind").and_then(|k| k.as_str()) != Some("header") {
        return Err(FormatError::NoHeader("its kind is not \"header\"".into()));
    }
    match value.get("format").and_then(|f| f.as_str()) {
        Some(FORMAT) => {}
        Some(other) => return Err(FormatError::OtherFormat(other.to_string())),
        None => return Err(FormatError::NoHeader("it names no format".into())),
    }
    let header: Header = serde_json::from_value(value).map_err(|e| FormatError::NoHeader(e.to_string()))?;
    if header.version != VERSION {
        return Err(FormatError::UnknownVersion(header.version));
    }
    Ok(header)
}

fn read_line(raw: &str) -> Result<Line, String> {
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|e| format!("not JSON: {e}"))?;
    let kind = value.get("kind").and_then(|k| k.as_str()).ok_or("no kind")?.to_string();
    if !["header", "project", "memory", "derivation", "piece", "conflict"].contains(&kind.as_str()) {
        return Err(format!("unknown kind {kind:?}"));
    }
    let line: Line = serde_json::from_value(value).map_err(|e| format!("{kind}: {e}"))?;
    match &line {
        Line::Memory(m) => check_memory(m).map_err(|e| format!("memory {}: {e}", m.id))?,
        Line::Conflict(c) => check_conflict(c).map_err(|e| format!("conflict {}-{}: {e}", c.new_id, c.old_id))?,
        _ => {}
    }
    Ok(line)
}

/// What the store's own checks would refuse, and what the daemon refuses
/// from a save, said before anything is written.
fn check_memory(m: &MemoryLine) -> Result<(), String> {
    if m.content.trim().is_empty() {
        return Err("empty content".into());
    }
    if MemoryType::from_db_str(&m.memory_type).is_none() {
        return Err(format!("unknown memory_type {:?}", m.memory_type));
    }
    if !["active", "disputed", "superseded"].contains(&m.status.as_str()) {
        return Err(format!("unknown status {:?}", m.status));
    }
    let scoped = match m.scope.as_str() {
        "agent" => m.agent_id.is_some(),
        "project" => m.project_id.is_some() && m.agent_id.is_none(),
        "global" => m.agent_id.is_none() && m.project_id.is_none(),
        other => return Err(format!("unknown scope {other:?}")),
    };
    if !scoped {
        return Err(format!("scope {:?} does not match its agent and project", m.scope));
    }
    if let Some(key) = &m.topic_key {
        validate_topic_key(key).map_err(|e| e.to_string())?;
    }
    if let Some(cause) = &m.supersede_cause {
        if !["revision", "verdict"].contains(&cause.as_str()) {
            return Err(format!("unknown supersede_cause {cause:?}"));
        }
    }
    if !(m.importance.is_finite() && (0.0..=10.0).contains(&m.importance)) {
        return Err(format!("importance {} is outside 0 to 10", m.importance));
    }
    Ok(())
}

fn check_conflict(c: &ConflictLine) -> Result<(), String> {
    if !["pending", "escalated", "judged"].contains(&c.state.as_str()) {
        return Err(format!("unknown state {:?}", c.state));
    }
    if let Some(verdict) = &c.verdict {
        if !["supersedes", "compatible", "scoped"].contains(&verdict.as_str()) {
            return Err(format!("unknown verdict {verdict:?}"));
        }
    }
    if c.new_id == c.old_id {
        return Err("a pair of one row".into());
    }
    Ok(())
}

/// One line of the file, with its line break.
pub fn render(line: &Line) -> String {
    let mut text = serde_json::to_string(line).expect("a line serialises infallibly");
    text.push('\n');
    text
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A file as `export` writes it, one line of each kind.
    pub(crate) const EXAMPLE: &str = concat!(
        r#"{"kind":"header","format":"clawg-memories","version":1,"exported_at":1760000000000,"clawg":"0.36.0","embedding_model":"multilingual-e5-small","filter":{"project":null,"agent":null,"include_deleted":true}}"#,
        "\n",
        r#"{"kind":"project","id":"repo-1a2b","root_path":"/work/repo"}"#,
        "\n",
        r#"{"kind":"memory","id":7,"scope":"agent","agent_id":"claude-code","project_id":"repo-1a2b","content":"Storage engine: SQLite with WAL","memory_type":"architectural_decision","source":"explicit","created_at":1750000000000,"created_by_type":"user","created_by_ref":"claude-code","source_session_id":"s-1","last_modified_at":1750000000000,"last_modified_by":null,"update_count":0,"last_accessed_at":1755000000000,"access_count":4,"importance":8.0,"tier":1,"status":"superseded","superseded_by":9,"supersede_cause":"revision","dispute_count":0,"disputed_against":[],"last_disputed_at":null,"deleted_at":null,"topic_key":"storage/engine","revived_from":null}"#,
        "\n",
        r#"{"kind":"derivation","memory":9,"source":7,"created_at":1751000000000}"#,
        "\n",
        r#"{"kind":"piece","save":3,"piece":2,"memory":9,"context":"Storage engine"}"#,
        "\n",
        r#"{"kind":"conflict","new_id":9,"old_id":7,"distance":0.31,"state":"judged","verdict":"compatible","reason":"different scopes","created_at":1751000000000,"judged_at":1752000000000,"topic_key_warned":false}"#,
        "\n",
    );

    #[test]
    fn the_pinned_example_reads_and_writes_back_unchanged() {
        let (header, parsed) = parse(EXAMPLE).unwrap();
        assert!(parsed.invalid.is_empty(), "{:?}", parsed.invalid);
        assert_eq!(parsed.memories[0].0, 3, "line numbers count from 1");
        let mut written = render(&Line::Header(header));
        written += &render(&Line::Project(parsed.projects[0].1.clone()));
        written += &render(&Line::Memory(parsed.memories[0].1.clone()));
        written += &render(&Line::Derivation(parsed.derivations[0].1.clone()));
        written += &render(&Line::Piece(parsed.pieces[0].1.clone()));
        written += &render(&Line::Conflict(parsed.conflicts[0].1.clone()));
        assert_eq!(written, EXAMPLE);
    }

    #[test]
    fn a_header_of_another_format_or_version_is_refused() {
        let other = EXAMPLE.replacen("\"version\":1", "\"version\":2", 1);
        assert_eq!(parse(&other).unwrap_err(), FormatError::UnknownVersion(2));
        let foreign = EXAMPLE.replacen("clawg-memories", "engram", 1);
        assert_eq!(parse(&foreign).unwrap_err(), FormatError::OtherFormat("engram".into()));
        let headless = EXAMPLE.lines().skip(1).collect::<Vec<_>>().join("\n");
        assert!(matches!(parse(&headless).unwrap_err(), FormatError::NoHeader(_)));
        assert_eq!(parse("\n  \n").unwrap_err(), FormatError::Empty);
    }

    #[test]
    fn a_conflict_line_without_the_mark_reads_unmarked() {
        let older = EXAMPLE.replacen(",\"topic_key_warned\":false", "", 1);
        assert_ne!(older, EXAMPLE);
        let (_, parsed) = parse(&older).unwrap();
        assert!(parsed.invalid.is_empty(), "{:?}", parsed.invalid);
        assert!(!parsed.conflicts[0].1.topic_key_warned);

        let marked = EXAMPLE.replacen("\"topic_key_warned\":false", "\"topic_key_warned\":true", 1);
        let (_, parsed) = parse(&marked).unwrap();
        assert!(parsed.conflicts[0].1.topic_key_warned);
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let extra = EXAMPLE.replacen("\"tier\":1,", "\"tier\":1,\"weight\":3,", 1);
        let (_, parsed) = parse(&extra).unwrap();
        assert!(parsed.invalid.is_empty());
        assert_eq!(parsed.memories.len(), 1);
    }

    #[test]
    fn invalid_lines_are_reported_and_the_rest_imports() {
        let memory = EXAMPLE.lines().nth(2).unwrap();
        let bad = [
            ("not json", "not JSON"),
            (r#"{"kind":"weather"}"#, "unknown kind"),
            (&memory.replace("architectural_decision", "observation"), "unknown memory_type"),
            (&memory.replace("\"superseded\"", "\"archived\""), "unknown status"),
            (&memory.replace("\"scope\":\"agent\"", "\"scope\":\"global\""), "does not match"),
            (&memory.replace("storage/engine", "Storage Engine"), "topic"),
            (&memory.replace("\"revision\"", "\"whim\""), "supersede_cause"),
            (&memory.replace("\"importance\":8.0", "\"importance\":11.0"), "importance"),
            (&memory.replace("Storage engine: SQLite with WAL", "  "), "empty content"),
            (r#"{"kind":"conflict","new_id":1,"old_id":2,"state":"open","created_at":0}"#, "unknown state"),
            (r#"{"kind":"memory","id":1}"#, "memory:"),
        ];
        let mut text = EXAMPLE.to_string();
        for (line, _) in &bad {
            text.push_str(line);
            text.push('\n');
        }
        let (_, parsed) = parse(&text).unwrap();
        assert_eq!(parsed.memories.len(), 1, "the good row stays");
        assert_eq!(parsed.invalid.len(), bad.len(), "{:#?}", parsed.invalid);
        for (i, (invalid, (_, why))) in parsed.invalid.iter().zip(bad.iter()).enumerate() {
            assert_eq!(invalid.line, 7 + i);
            let reason = invalid.reason.to_lowercase();
            assert!(reason.contains(&why.to_lowercase()), "line {}: {reason}", invalid.line);
        }
    }
}
