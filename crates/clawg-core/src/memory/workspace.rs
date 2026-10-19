//! Owns the on-disk `workspace.md` file for a (agent_id, project_id) pair:
//! the AUTO section is regenerated from DB queries, the USER section is
//! preserved byte-for-byte.

use std::borrow::Cow;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::memory::{record::TableKind, MemoryError, MemoryStore};

pub(crate) const DEFAULT_PROJECT_SEGMENT: &str = "default";
pub const AUTO_START: &str = "<!-- AUTO-START -->";
pub const AUTO_END: &str = "<!-- AUTO-END -->";
pub const USER_START: &str = "<!-- USER-START -->";
pub const USER_END: &str = "<!-- USER-END -->";

/// Hard cap on the AUTO section's token count (approximation: 4 bytes/token).
pub(crate) const AUTO_TOKEN_CAP: usize = 1500;
pub(crate) const AUTO_BYTE_CAP: usize = AUTO_TOKEN_CAP * 4;

/// Stateless façade. Holds only the base directory under which all workspaces live.
/// Trivially `Clone + Send + Sync`.
#[derive(Debug, Clone)]
pub struct WorkspaceManager {
    workspaces_root: PathBuf,
}

// Compile-time Send + Sync + Clone assertions
const _: fn() = || {
    fn _ss<T: Send + Sync>() {}
    fn _clone<T: Clone>() {}
    _ss::<WorkspaceManager>();
    _clone::<WorkspaceManager>();
};

impl WorkspaceManager {
    /// Construct from the WORKSPACES root.
    pub fn new(workspaces_root: PathBuf) -> Self {
        Self { workspaces_root }
    }

    /// Resolves the on-disk path. Pure — no I/O.
    pub fn workspace_path(&self, agent_id: &str, project_id: Option<&str>) -> PathBuf {
        self.workspaces_root
            .join(workspace_segment(agent_id).as_ref())
            .join(project_segment(project_id.unwrap_or(DEFAULT_PROJECT_SEGMENT)).as_ref())
            .join("workspace.md")
    }

    /// Gives every agent's `workspace.md` under `legacy_id` to `project_id`
    /// when the new project has none yet, so a hand-written USER section
    /// follows its rows. The legacy file is copied, never moved or changed: if
    /// the user keeps editing it, both copies still exist. When both already
    /// exist nothing is merged, only reported, because either could be the one
    /// the user wants.
    pub async fn adopt_legacy(&self, legacy_id: &str, project_id: &str) -> Result<(), MemoryError> {
        let io_err = |path: &std::path::Path| {
            let path = path.to_path_buf();
            move |source| MemoryError::WorkspaceIo { path, source }
        };
        let mut agents = match tokio::fs::read_dir(&self.workspaces_root).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(io_err(&self.workspaces_root)(e)),
        };
        while let Some(agent) = agents.next_entry().await.map_err(io_err(&self.workspaces_root))? {
            // Already a segment on disk: going through `workspace_path` would
            // encode an encoded name a second time.
            let agent_dir = agent.path();
            let from = agent_dir.join(project_segment(legacy_id).as_ref()).join("workspace.md");
            let to = agent_dir.join(project_segment(project_id).as_ref()).join("workspace.md");
            let legacy = match tokio::fs::read(&from).await {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(io_err(&from)(e)),
            };
            match tokio::fs::read(&to).await {
                Ok(current) => {
                    if current != legacy {
                        log::warn!(
                            "workspace {} was kept beside {}; merge them by hand if both matter",
                            from.display(),
                            to.display()
                        );
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    if let Some(dir) = to.parent() {
                        tokio::fs::create_dir_all(dir).await.map_err(io_err(dir))?;
                    }
                    write_atomically(&to, &legacy).await.map_err(io_err(&to))?;
                }
                Err(e) => return Err(io_err(&to)(e)),
            }
        }
        Ok(())
    }

    /// Regenerates the AUTO section from DB queries. USER section is preserved
    /// byte-for-byte.
    ///
    /// Returns the ids this call actually rendered, so the retrieval channel
    /// can skip them rather than delivering the same rows a second time. They
    /// are reported by the renderer itself, after the byte cap has done its
    /// trimming — a second copy of the selection predicate elsewhere would be
    /// free to drift from what the file really says.
    pub async fn regenerate_auto_section(
        &self,
        agent_id: &str,
        project_id: Option<&str>,
        store: &MemoryStore,
    ) -> Result<EmittedIds, MemoryError> {
        let path = self.workspace_path(agent_id, project_id);

        // Read existing file (may be empty / missing on first run)
        let existing = match tokio::fs::read_to_string(&path).await {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(MemoryError::WorkspaceIo { path: path.clone(), source: e }),
        };

        // Fetch rows from DB
        let rows = fetch_auto_rows(store, agent_id, project_id).await?;

        // Render new AUTO section
        let (auto_section, emitted) = render_auto_section(&rows);

        // Assemble file content
        let assembled = assemble_file(&existing, &auto_section);

        // Create parent dirs if needed
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| MemoryError::WorkspaceIo { path: path.clone(), source: e })?;
        }

        write_atomically(&path, assembled.as_bytes())
            .await
            .map_err(|e| MemoryError::WorkspaceIo { path: path.clone(), source: e })?;

        Ok(emitted)
    }

    /// Reads the full file (AUTO + USER sections together).
    /// Returns `Ok(String::new())` when the file does not exist.
    pub(crate) async fn read(
        &self,
        agent_id: &str,
        project_id: Option<&str>,
    ) -> Result<String, MemoryError> {
        let path = self.workspace_path(agent_id, project_id);
        match tokio::fs::read_to_string(&path).await {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(MemoryError::WorkspaceIo { path, source: e }),
        }
    }
}

// ---------------------------------------------------------------------------
// Private data types
// ---------------------------------------------------------------------------

/// One `memory` row selected for the AUTO section.
#[derive(Clone)]
struct MemoryLine {
    id: i64,
    content: String,
}

/// One `procedural_skill` row selected for the AUTO section. `trigger` says
/// WHEN to apply the skill and `tool_sequence` says WHAT it does — a bare
/// `skill_name` tells the model neither, which is why all three travel
/// together.
#[derive(Clone)]
struct SkillLine {
    id: i64,
    skill_name: String,
    trigger: String,
    /// Raw column value: a JSON array of tool names, parsed at render time.
    tool_sequence_json: String,
}

#[derive(Clone)]
struct AutoRows {
    user_prefs: Vec<MemoryLine>,
    arch_decisions: Vec<MemoryLine>,
    top_skills: Vec<SkillLine>,
}

/// The rows the deterministic workspace channel actually rendered.
///
/// The two delivery channels — this one and the KNN pool — used to each carry
/// their own SQL predicate for what the other was allowed to return, and the
/// two predicates were not complements: rows matching neither reached the
/// model through nobody. Reporting what was rendered, instead of restating a
/// rule about what might have been, makes "never deliver the same row twice"
/// impossible to violate by drift.
#[derive(Debug, Clone, Default)]
pub struct EmittedIds {
    memory: HashSet<i64>,
    procedural: HashSet<i64>,
}

impl EmittedIds {
    pub(crate) fn contains(&self, kind: TableKind, id: i64) -> bool {
        match kind {
            TableKind::Memory => self.memory.contains(&id),
            TableKind::Procedural => self.procedural.contains(&id),
            // The AUTO section has no episodic query, so no trajectory can
            // ever have been emitted through it.
            TableKind::Episodic => false,
        }
    }

    pub(crate) fn memory_ids(&self) -> impl Iterator<Item = i64> + '_ {
        self.memory.iter().copied()
    }

    pub(crate) fn len(&self) -> usize {
        self.memory.len() + self.procedural.len()
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

async fn fetch_auto_rows(
    store: &MemoryStore,
    agent_id: &str,
    project_id: Option<&str>,
) -> Result<AutoRows, MemoryError> {
    let agent = agent_id.to_string();
    let proj = project_id.map(|s| s.to_string());

    store
        .conn
        .call(move |c| -> Result<AutoRows, rusqlite::Error> {
            // Query 1: user_preference rows with importance >= 8.
            //
            // Filtered by agent only, deliberately: a preference describes the
            // person the agent is working with — how they want to be spoken to,
            // what they never want done — and that does not stop being true
            // when they switch projects. The other two queries below scope by
            // project because a decision or a skill belongs to a codebase;
            // this one must not, or the same preference has to be re-taught
            // once per project. Narrowing it is a behavior change, not a fix.
            //
            // All three memory queries drop superseded rows but keep disputed
            // ones: a disputed row ranks like an active one everywhere else,
            // and the conflicts block reaches the prompt for every row this
            // section emits, so the dispute is shown next to it.
            let mut stmt = c.prepare(
                "SELECT id, content FROM memory
                 WHERE memory_type = 'user_preference'
                   AND importance >= 8
                   AND agent_id = ?1
                   AND tier = 1
                   AND deleted_at IS NULL
                   AND status <> 'superseded'
                 ORDER BY importance DESC",
            )?;
            let user_prefs: Vec<MemoryLine> = readable(
                stmt.query_map(rusqlite::params![agent], |row| {
                    Ok(MemoryLine { id: row.get(0)?, content: row.get(1)? })
                })?,
                "user_preference",
            );

            // Query 2: architectural_decision rows, scoped to project when provided,
            // otherwise all architectural decisions for the agent. importance >= 8.
            //
            // A row with no project of its own is visible from every project:
            // NULL there means "not attributed", not "belongs to nobody", and
            // the writers that record a decision outside a project context
            // leave it NULL. Matching only on equality would make those rows
            // reachable exclusively from the unscoped listing — invisible from
            // every real workspace, which is where they are actually needed.
            let arch_decisions: Vec<MemoryLine> = if let Some(ref pid) = proj {
                let mut stmt2 = c.prepare(
                    "SELECT id, content FROM memory
                     WHERE memory_type = 'architectural_decision'
                       AND importance >= 8
                       AND agent_id = ?1
                       AND (project_id = ?2 OR project_id IS NULL)
                       AND tier = 1
                       AND deleted_at IS NULL
                       AND status <> 'superseded'
                     ORDER BY importance DESC",
                )?;
                let rows: Vec<MemoryLine> = readable(
                    stmt2.query_map(rusqlite::params![agent, pid], |row| {
                        Ok(MemoryLine { id: row.get(0)?, content: row.get(1)? })
                    })?,
                    "architectural_decision",
                );
                rows
            } else {
                let mut stmt2 = c.prepare(
                    "SELECT id, content FROM memory
                     WHERE memory_type = 'architectural_decision'
                       AND importance >= 8
                       AND agent_id = ?1
                       AND tier = 1
                       AND deleted_at IS NULL
                       AND status <> 'superseded'
                     ORDER BY importance DESC",
                )?;
                let rows: Vec<MemoryLine> = readable(
                    stmt2.query_map(rusqlite::params![agent], |row| {
                        Ok(MemoryLine { id: row.get(0)?, content: row.get(1)? })
                    })?,
                    "architectural_decision",
                );
                rows
            };

            // Query 3: top-5 procedural skills by access_count, under the same
            // eligibility rule the KNN pool applies to this table — own project
            // or unattributed. There are only five slots, so a skill learned in
            // another codebase does not merely appear, it displaces one that
            // belongs here. No cross-project carve-out exists for skills: a
            // procedure is written against a specific stack and layout, and
            // replaying it elsewhere is how the wrong command gets run.
            let map_skill = |row: &rusqlite::Row<'_>| -> Result<SkillLine, rusqlite::Error> {
                Ok(SkillLine {
                    id: row.get(0)?,
                    skill_name: row.get(1)?,
                    trigger: row.get(2)?,
                    tool_sequence_json: row.get(3)?,
                })
            };
            let top_skills: Vec<SkillLine> = if let Some(ref pid) = proj {
                let mut stmt3 = c.prepare(
                    "SELECT id, skill_name, trigger, tool_sequence FROM procedural_skill
                     WHERE agent_id = ?1
                       AND (project_id = ?2 OR project_id IS NULL)
                       AND tier = 1
                       AND deleted_at IS NULL
                     ORDER BY access_count DESC
                     LIMIT 5",
                )?;
                let rows: Vec<SkillLine> =
                    readable(stmt3.query_map(rusqlite::params![agent, pid], |row| map_skill(row))?, "skill");
                rows
            } else {
                let mut stmt3 = c.prepare(
                    "SELECT id, skill_name, trigger, tool_sequence FROM procedural_skill
                     WHERE agent_id = ?1
                       AND tier = 1
                       AND deleted_at IS NULL
                     ORDER BY access_count DESC
                     LIMIT 5",
                )?;
                let rows: Vec<SkillLine> =
                    readable(stmt3.query_map(rusqlite::params![agent], |row| map_skill(row))?, "skill");
                rows
            };

            Ok(AutoRows {
                user_prefs,
                arch_decisions,
                top_skills,
            })
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
            tokio_rusqlite::Error::ConnectionClosed => {
                MemoryError::Backend("connection closed during fetch_auto_rows".into())
            }
            other => MemoryError::Backend(other.to_string()),
        })
}

/// The rows that read. One that does not is left out of its section rather
/// than failing the whole file, and logged so the gap has a trace.
fn readable<T>(rows: impl Iterator<Item = rusqlite::Result<T>>, section: &str) -> Vec<T> {
    rows.filter_map(|r| r.map_err(|e| log::warn!("workspace: a {section} row did not read: {e}")).ok())
        .collect()
}

/// Renders the AUTO section AND reports which rows ended up inside it.
///
/// The byte cap is applied here, in the same pass, rather than as a post-hoc
/// pass over finished text: a row the cap dropped was not delivered, and
/// reporting it as emitted would make the retrieval channel skip it too —
/// the row would then reach the model through no channel at all.
fn render_auto_section(rows: &AutoRows) -> (String, EmittedIds) {
    let mut kept = rows.clone();
    let mut content = render_rows(&kept);

    // Reverse priority: skills → decisions. Preferences are never trimmed —
    // they are the standing contract with the user, and dropping one silently
    // changes how the model behaves for a whole session.
    while content.len() > AUTO_BYTE_CAP && !kept.top_skills.is_empty() {
        kept.top_skills.pop();
        content = render_rows(&kept);
    }
    while content.len() > AUTO_BYTE_CAP && !kept.arch_decisions.is_empty() {
        kept.arch_decisions.pop();
        content = render_rows(&kept);
    }

    let emitted = EmittedIds {
        memory: kept
            .user_prefs
            .iter()
            .chain(kept.arch_decisions.iter())
            .map(|r| r.id)
            .collect(),
        procedural: kept.top_skills.iter().map(|s| s.id).collect(),
    };
    (content, emitted)
}

/// `["read","edit"]` → `"read → edit"`. Returns `None` for an empty or
/// unparseable column: a raw JSON blob in the prompt is noise the model has
/// to decode, and a half-rendered sequence is worse than no sequence.
fn format_tool_sequence(raw: &str) -> Option<String> {
    let tools: Vec<String> = serde_json::from_str(raw).ok()?;
    if tools.is_empty() {
        return None;
    }
    Some(tools.join(" → "))
}

/// A skill is only actionable with all three parts: the name identifies it,
/// the trigger says WHEN to reach for it, the sequence says what it does.
/// Either optional part is dropped when absent rather than rendered empty.
fn format_skill_line(skill: &SkillLine) -> String {
    let mut line = format!("- {}", skill.skill_name);
    let trigger = skill.trigger.trim();
    if !trigger.is_empty() {
        line.push_str(" — when: ");
        line.push_str(trigger);
    }
    if let Some(seq) = format_tool_sequence(&skill.tool_sequence_json) {
        line.push_str(" — tools: ");
        line.push_str(&seq);
    }
    line.push('\n');
    line
}

fn render_rows(rows: &AutoRows) -> String {
    let mut out = String::new();
    out.push_str(AUTO_START);
    out.push('\n');
    out.push_str("## User Preferences (always active)\n");
    for row in &rows.user_prefs {
        out.push_str(&format!("- {}\n", row.content));
    }
    out.push_str("## Architectural Decisions\n");
    for row in &rows.arch_decisions {
        out.push_str(&format!("- {}\n", row.content));
    }
    out.push_str("## Procedural Skills\n");
    for skill in &rows.top_skills {
        out.push_str(&format_skill_line(skill));
    }
    out.push_str(AUTO_END);
    out.push('\n');
    out
}

/// Writes through `<file>.tmp` in the same directory and renames it over
/// `path`, so a write that dies halfway leaves the previous file whole: a
/// plain write truncates the target first, USER section included. The
/// rename replaces an existing target on Windows too (`MoveFileExW` with
/// `MOVEFILE_REPLACE_EXISTING`).
pub(crate) async fn write_atomically(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let written = match tokio::fs::write(&tmp, bytes).await {
        Ok(()) => tokio::fs::rename(&tmp, path).await,
        Err(e) => Err(e),
    };
    if written.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    written
}

/// Where a `workspace.md` holds a text: in the AUTO section, which the next
/// regeneration rewrites from the store, or anywhere else, which is the
/// user's and never rewritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Holding {
    pub auto: bool,
    pub user: bool,
}

/// Where `file` holds `text`. Pure, so the split is under test without a
/// filesystem.
pub(crate) fn sections_holding(file: &str, text: &str) -> Holding {
    let auto = file.find(AUTO_START).and_then(|start| {
        let inner = start + AUTO_START.len();
        file[inner..].find(AUTO_END).map(|end| start..inner + end + AUTO_END.len())
    });
    match auto {
        Some(range) => Holding {
            auto: file[range.clone()].contains(text),
            user: file[..range.start].contains(text) || file[range.end..].contains(text),
        },
        None => Holding { auto: false, user: file.contains(text) },
    }
}

/// Every `<root>/<agent>/<project>/workspace.md` that holds `text`, by path,
/// in path order. Reads only; a file that cannot be read is skipped.
pub(crate) fn workspaces_holding(root: &Path, text: &str) -> Vec<(PathBuf, Holding)> {
    let dirs = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect())
            .unwrap_or_default()
    };
    let mut holding: Vec<(PathBuf, Holding)> = dirs(root)
        .iter()
        .flat_map(|agent| dirs(agent))
        .map(|project| project.join("workspace.md"))
        .filter_map(|path| {
            let bytes = std::fs::read(&path).ok()?;
            let found = sections_holding(&String::from_utf8_lossy(&bytes), text);
            (found.auto || found.user).then_some((path, found))
        })
        .collect();
    holding.sort_by(|a, b| a.0.cmp(&b.0));
    holding
}

/// Parse the existing file content to find AUTO and USER sections.
struct Sections {
    before_auto: String,
    user_content: String,
    after_user: String,
}

fn split_existing(content: &str) -> Sections {
    // Find AUTO section boundaries
    let auto_start_pos = content.find(AUTO_START);
    let auto_end_pos = content.find(AUTO_END);

    // Find USER section boundaries
    let user_start_pos = content.find(USER_START);
    let user_end_pos = content.find(USER_END);

    // Extract before-auto preamble
    let before_auto = match auto_start_pos {
        Some(pos) => content[..pos].to_string(),
        None => String::new(),
    };

    // Extract user content (between USER_START and USER_END)
    let user_content = match (user_start_pos, user_end_pos) {
        (Some(start), Some(end)) => {
            let inner_start = start + USER_START.len();
            if inner_start < end {
                content[inner_start..end].to_string()
            } else {
                String::new()
            }
        }
        _ => String::new(),
    };

    // Extract after-user content
    let after_user = match user_end_pos {
        Some(pos) => {
            let after = pos + USER_END.len();
            content[after..].to_string()
        }
        None => String::new(),
    };

    // Extract content between AUTO_END and USER_START (if any)
    let between = match (auto_end_pos, user_start_pos) {
        (Some(ae), Some(us)) => {
            let after_auto_end = ae + AUTO_END.len();
            if after_auto_end < us {
                content[after_auto_end..us].to_string()
            } else {
                String::new()
            }
        }
        _ => String::new(),
    };

    let _ = between; // between is used in assemble_file

    Sections {
        before_auto,
        user_content,
        after_user,
    }
}

fn assemble_file(existing: &str, auto_section: &str) -> String {
    if existing.is_empty() {
        // Fresh file: AUTO section + USER section (empty)
        let mut out = String::new();
        out.push_str(auto_section);
        out.push('\n');
        out.push_str(USER_START);
        out.push('\n');
        out.push_str(USER_END);
        out.push('\n');
        return out;
    }

    let sections = split_existing(existing);

    // Reassemble: before_auto + auto_section + between + USER_START + user_content + USER_END + after_user
    let mut out = String::new();
    out.push_str(&sections.before_auto);
    out.push_str(auto_section);

    // Check if there's a USER section in the original
    let has_user_section = existing.contains(USER_START) && existing.contains(USER_END);
    if has_user_section {
        out.push('\n');
        out.push_str(USER_START);
        out.push_str(&sections.user_content);
        out.push_str(USER_END);
        out.push_str(&sections.after_user);
    } else {
        // No USER section existed — add an empty one
        out.push('\n');
        out.push_str(USER_START);
        out.push('\n');
        out.push_str(USER_END);
        out.push('\n');
    }

    out
}

/// The folder name of an agent's workspaces.
///
/// The id comes from the MCP client, so it can hold a separator or be `..`
/// and point outside the workspaces root. A name that cannot do that is used
/// as is, which keeps every existing folder (and the USER section written in
/// it) where it is; any other is percent-encoded byte by byte. `%` counts as
/// unsafe so an encoded name never equals a literal one, and the empty id,
/// which no encoding produces, gets the lone `%`.
///
/// Windows rewrites some names: it drops trailing dots and spaces, so `a.`
/// would share `a`'s folder, and it maps a device name (`con`, `nul.txt`) to
/// the device. Those trailing characters, which also cover `.` and `..`, and
/// the first letter of a device name are encoded on every platform, so a
/// store gives each agent the same folder wherever it runs.
fn workspace_segment(agent_id: &str) -> Cow<'_, str> {
    fn unsafe_char(c: char) -> bool {
        matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '%') || c.is_control()
    }
    fn encode(s: &str, out: &mut String) {
        for b in s.bytes() {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    if agent_id.is_empty() {
        return Cow::Borrowed("%");
    }
    let trailing_from = agent_id.trim_end_matches(['.', ' ']).len();
    let device = is_windows_device_name(agent_id);
    if !device && trailing_from == agent_id.len() && !agent_id.chars().any(unsafe_char) {
        return Cow::Borrowed(agent_id);
    }
    let mut out = String::new();
    for (i, c) in agent_id.char_indices() {
        if unsafe_char(c) || i >= trailing_from || (device && i == 0) {
            encode(c.encode_utf8(&mut [0; 4]), &mut out);
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// The folder name of a project's workspace inside an agent's folder.
///
/// A project id is `<folder name>-hhhh` and is used as is, which keeps every
/// existing folder where it is. Anything that is not exactly one plain path
/// component (an absolute path, `.`, `..`, a separator, a drive prefix, the
/// empty id) could leave the agent's folder, so it is encoded like an agent id.
fn project_segment(project_id: &str) -> Cow<'_, str> {
    let mut components = Path::new(project_id).components();
    let plain = matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(name)), None) if name == project_id
    );
    if plain && !project_id.contains(['/', '\\']) {
        Cow::Borrowed(project_id)
    } else {
        workspace_segment(project_id)
    }
}

/// Whether Windows takes `name` for a device: its part before the first dot,
/// without trailing spaces, is a reserved name in any case.
fn is_windows_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default().trim_end_matches(' ').to_ascii_uppercase();
    match stem.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        _ => match stem.strip_prefix("COM").or_else(|| stem.strip_prefix("LPT")) {
            Some(n) => {
                let mut chars = n.chars();
                matches!((chars.next(), chars.next()), (Some('0'..='9' | '¹' | '²' | '³'), None))
            }
            None => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::MemoryStore;

    #[test]
    fn the_sections_holding_a_text_are_told_apart() {
        let text = "deploy with the vault key";
        let file = |auto: &str, user: &str| {
            format!("# notes\n{AUTO_START}\n{auto}\n{AUTO_END}\n{USER_START}\n{user}\n{USER_END}\n")
        };
        let holds = |auto, user| Holding { auto, user };
        assert_eq!(sections_holding(&file(&format!("- {text}"), "mine"), text), holds(true, false));
        assert_eq!(sections_holding(&file("- other", &format!("I wrote: {text}")), text), holds(false, true));
        assert_eq!(sections_holding(&file(&format!("- {text}"), text), text), holds(true, true));
        assert_eq!(sections_holding(&file("- other", "mine"), text), holds(false, false));
        assert_eq!(sections_holding(&format!("{text} before any mark\n{}", file("", "")), text), holds(false, true));
        assert_eq!(sections_holding(text, text), holds(false, true), "a file without marks is the user's");
    }

        #[tokio::test]
    async fn a_rekey_copies_the_legacy_workspace_and_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let ws = WorkspaceManager::new(dir.path().to_path_buf());
        let legacy = ws.workspace_path("agent-a", Some("src-1a2b"));
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        let content = format!("{AUTO_START}\nauto\n{AUTO_END}\n{USER_START}\nwritten by hand\n{USER_END}\n");
        std::fs::write(&legacy, &content).unwrap();

        ws.adopt_legacy("src-1a2b", "repo-3c4d").await.unwrap();

        let adopted = ws.workspace_path("agent-a", Some("repo-3c4d"));
        assert_eq!(std::fs::read_to_string(&adopted).unwrap(), content);
        assert_eq!(std::fs::read_to_string(&legacy).unwrap(), content, "the legacy file stays as it was");
    }

    #[tokio::test]
    async fn adopting_never_overwrites_the_projects_own_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let ws = WorkspaceManager::new(dir.path().to_path_buf());
        for (project, text) in [("src-1a2b", "legacy"), ("repo-3c4d", "current")] {
            let path = ws.workspace_path("agent-a", Some(project));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
        }

        ws.adopt_legacy("src-1a2b", "repo-3c4d").await.unwrap();

        let read = |p| std::fs::read_to_string(ws.workspace_path("agent-a", Some(p))).unwrap();
        assert_eq!(read("repo-3c4d"), "current");
        assert_eq!(read("src-1a2b"), "legacy");
    }

    #[test]
    fn a_project_id_that_is_not_one_plain_name_stays_inside_the_agents_folder() {
        let root = PathBuf::from("/base/workspaces");
        let ws = WorkspaceManager::new(root.clone());
        let agent_dir = root.join("agent-a");
        let outside = std::env::temp_dir().join("elsewhere");
        let absolute = outside.to_string_lossy().into_owned();
        for id in [absolute.as_str(), "/etc", "..", ".", "../../x", "a/b", "a\\b", "C:x", ""] {
            let path = ws.workspace_path("agent-a", Some(id));
            assert!(path.starts_with(&agent_dir), "{id:?} -> {path:?}");
            assert_eq!(path.strip_prefix(&agent_dir).unwrap().components().count(), 2, "{id:?} -> {path:?}");
        }
    }

    #[test]
    fn a_real_project_id_keeps_its_folder() {
        let root = PathBuf::from("/base/workspaces");
        let ws = WorkspaceManager::new(root.clone());
        for id in ["repo-3c4d", "my app-1a2b", "100%-beef", "café-0001"] {
            assert_eq!(
                ws.workspace_path("agent-a", Some(id)),
                root.join("agent-a").join(id).join("workspace.md"),
                "{id:?}"
            );
        }
    }

    #[tokio::test]
    async fn adopting_never_reads_or_writes_outside_the_agents_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspaces");
        let ws = WorkspaceManager::new(root.clone());
        let legacy = ws.workspace_path("agent-a", Some("src-1a2b"));
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "legacy").unwrap();

        ws.adopt_legacy("src-1a2b", "../../planted").await.unwrap();

        assert!(!dir.path().join("planted").exists());
        let adopted = ws.workspace_path("agent-a", Some("../../planted"));
        assert!(adopted.starts_with(root.join("agent-a")));
        assert_eq!(std::fs::read_to_string(adopted).unwrap(), "legacy");
    }

    #[test]
    fn a_safe_agent_id_is_its_own_segment() {
        for agent in ["claude-code", "cursor", "Claude Code", "código", "agent_1.2"] {
            assert_eq!(workspace_segment(agent), agent);
        }
        let ws = WorkspaceManager::new(PathBuf::from("/base/workspaces"));
        assert_eq!(
            ws.workspace_path("claude-code", Some("proj")),
            PathBuf::from("/base/workspaces").join("claude-code").join("proj").join("workspace.md")
        );
    }

    #[test]
    fn an_unsafe_agent_id_is_encoded_inside_the_root() {
        let root = PathBuf::from("/base/workspaces");
        let ws = WorkspaceManager::new(root.clone());
        for (agent, segment) in [
            ("../escape", "..%2Fescape"),
            ("a/b", "a%2Fb"),
            ("a\\b", "a%5Cb"),
            ("c:", "c%3A"),
            ("..", "%2E%2E"),
            (".", "%2E"),
            ("", "%"),
            ("50%", "50%25"),
            ("tab\there", "tab%09here"),
        ] {
            assert_eq!(workspace_segment(agent), segment, "{agent:?}");
            let path = ws.workspace_path(agent, None);
            assert_eq!(path.parent().and_then(|p| p.parent()).and_then(|p| p.parent()), Some(root.as_path()), "{agent:?}");
        }
    }

    #[test]
    fn a_name_windows_would_rewrite_is_encoded() {
        for (agent, segment) in [
            ("con", "%63on"),
            ("NUL", "%4EUL"),
            ("aux.txt", "%61ux.txt"),
            ("Com1", "%43om1"),
            ("lpt9", "%6Cpt9"),
            ("prn .md", "%70rn .md"),
            ("agent.", "agent%2E"),
            ("agent ", "agent%20"),
            ("agent. .", "agent%2E%20%2E"),
        ] {
            assert_eq!(workspace_segment(agent), segment, "{agent:?}");
        }
        for agent in ["console", "com", "com10", "nulo", "a.b", " lead"] {
            assert_eq!(workspace_segment(agent), agent, "{agent:?}");
        }
    }

    #[test]
    fn windows_naming_rules_keep_agents_apart_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let ws = WorkspaceManager::new(dir.path().to_path_buf());
        for agent in ["agent", "agent.", "agent ", "con", "nul.txt"] {
            let path = ws.workspace_path(agent, None);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, agent).unwrap();
        }
        for agent in ["agent", "agent.", "agent ", "con", "nul.txt"] {
            assert_eq!(std::fs::read_to_string(ws.workspace_path(agent, None)).unwrap(), agent, "{agent:?}");
        }
    }

    #[test]
    fn encoding_never_makes_two_agents_share_a_folder() {
        let agents =
            ["a/b", "a%2Fb", "a%252Fb", "..", "%2E%2E", ".", "%2E", "", "%", "a", "a.", "a%2E", "con", "%63on"];
        let segments: HashSet<String> = agents.iter().map(|a| workspace_segment(a).into_owned()).collect();
        assert_eq!(segments.len(), agents.len(), "{segments:?}");
    }

    #[tokio::test]
    async fn adopting_a_legacy_workspace_keeps_an_encoded_folder() {
        let dir = tempfile::tempdir().unwrap();
        let ws = WorkspaceManager::new(dir.path().to_path_buf());
        let legacy = ws.workspace_path("team/bot", Some("src-1a2b"));
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "notes").unwrap();

        ws.adopt_legacy("src-1a2b", "repo-3c4d").await.unwrap();

        let adopted = ws.workspace_path("team/bot", Some("repo-3c4d"));
        assert_eq!(std::fs::read_to_string(adopted).unwrap(), "notes");
    }

    #[tokio::test]
    async fn adopting_without_any_workspace_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let ws = WorkspaceManager::new(dir.path().join("never-created"));
        ws.adopt_legacy("src-1a2b", "repo-3c4d").await.unwrap();
        assert!(!dir.path().join("never-created").exists());
    }

    #[test]
    fn path_with_project_id() {
        let wm = WorkspaceManager::new(PathBuf::from("/base/workspaces"));
        let p = wm.workspace_path("agent-1", Some("proj-abc"));
        let s = p.to_string_lossy();
        assert!(s.contains("proj-abc"), "path should contain proj-abc: {s}");
        assert!(s.ends_with("workspace.md"), "path should end with workspace.md: {s}");
        assert!(s.contains("agent-1"), "path should contain agent-1: {s}");
    }

    #[test]
    fn path_with_no_project_uses_default_segment() {
        let wm = WorkspaceManager::new(PathBuf::from("/base/workspaces"));
        let p = wm.workspace_path("agent-1", None);
        let s = p.to_string_lossy();
        assert!(s.contains("default"), "path should contain 'default': {s}");
        assert!(s.ends_with("workspace.md"), "path should end with workspace.md: {s}");
    }

    #[test]
    fn workspace_manager_is_send_sync() {
        fn _ss<T: Send + Sync>() {}
        _ss::<WorkspaceManager>();
    }

    #[test]
    fn markers_locate_auto_section_in_existing_file() {
        let content = format!("{}\nsome auto content\n{}", AUTO_START, AUTO_END);
        assert!(content.contains(AUTO_START));
        assert!(content.contains(AUTO_END));
    }

    #[test]
    fn missing_markers_creates_file_with_both_sections() {
        // When existing content has no markers, assemble_file creates fresh file with both sections
        let auto_section = format!("{}\n{}\n", AUTO_START, AUTO_END);
        let result = assemble_file("", &auto_section);
        assert!(result.contains(AUTO_START));
        assert!(result.contains(AUTO_END));
        assert!(result.contains(USER_START));
        assert!(result.contains(USER_END));
    }

    #[tokio::test]
    async fn read_missing_file_returns_ok_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let result = wm.read("agent-1", None).await.unwrap();
        assert_eq!(result, "", "missing file should return empty string");
    }

    #[tokio::test]
    async fn read_existing_file_returns_content() {
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let path = wm.workspace_path("agent-1", None);
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        tokio::fs::write(&path, "hello world").await.unwrap();
        let result = wm.read("agent-1", None).await.unwrap();
        assert_eq!(result, "hello world");
    }

    #[tokio::test]
    async fn user_section_preserved_byte_for_byte() {
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let store = MemoryStore::new_for_test().await.unwrap();
        let path = wm.workspace_path("agent-1", None);
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();

        let user_content = "  my custom notes\n  with trailing space  \n";
        let file_content = format!(
            "{}\n{}\n{}\n{}{}\n",
            AUTO_START, AUTO_END, USER_START, user_content, USER_END
        );
        tokio::fs::write(&path, &file_content).await.unwrap();

        wm.regenerate_auto_section("agent-1", None, &store).await.unwrap();

        // The file was written with USER_START + "\n" + user_content + USER_END.
        // byte-for-byte preservation means the inter-marker slice (which includes the "\n")
        // must be unchanged after regeneration.
        let original_user_start = file_content.find(USER_START).unwrap() + USER_START.len();
        let original_user_end = file_content.find(USER_END).unwrap();
        let original_inter = &file_content[original_user_start..original_user_end];

        let result = tokio::fs::read_to_string(&path).await.unwrap();
        let result_user_start = result.find(USER_START).unwrap() + USER_START.len();
        let result_user_end = result.find(USER_END).unwrap();
        let extracted_user = &result[result_user_start..result_user_end];
        assert_eq!(extracted_user, original_inter, "USER section must be byte-for-byte preserved");
    }

    #[tokio::test]
    async fn zero_tier1_rows_produces_empty_auto_section_with_headers() {
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let store = MemoryStore::new_for_test().await.unwrap();

        wm.regenerate_auto_section("agent-1", None, &store).await.unwrap();

        let path = wm.workspace_path("agent-1", None);
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains(AUTO_START));
        assert!(content.contains(AUTO_END));
        assert!(content.contains("## User Preferences (always active)"), "should have user prefs header");
        // No bullet lines inside the AUTO section inner content
        let auto_inner_start = content.find(AUTO_START).unwrap() + AUTO_START.len();
        let auto_inner_end = content.find(AUTO_END).unwrap();
        let auto_inner = &content[auto_inner_start..auto_inner_end];
        assert!(!auto_inner.contains("\n- "), "should have no bullet lines with zero rows");
    }

    #[tokio::test]
    async fn auto_section_respects_1500_token_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let store = MemoryStore::new_for_test().await.unwrap();

        // Insert 200 architectural_decision rows with importance=10 and tier=1.
        // arch_decisions CAN be trimmed by enforce_byte_cap, so they are the right
        // data type to use when testing the cap. user_preference rows are NEVER trimmed
        // by design (they are always included regardless of size).
        for i in 0..200u32 {
            let content = format!("architectural decision content number {} with some padding text to make it longer than needed for the byte cap test to be meaningful here yes", i);
            let agent = "agent-cap".to_string();
            let content_clone = content.clone();
            store.conn.call(move |c| -> Result<(), rusqlite::Error> {
                c.execute(
                    "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                     created_at, created_by_type, created_by_ref,
                     last_modified_at, last_accessed_at, importance, tier, status)
                     VALUES ('agent', ?1, ?2, 'architectural_decision', 'chat',
                     0, 'user', ?1, 0, 0, 10.0, 1, 'active')",
                    rusqlite::params![agent, content_clone],
                )?;
                Ok(())
            }).await.unwrap();
        }

        wm.regenerate_auto_section("agent-cap", None, &store).await.unwrap();

        let path = wm.workspace_path("agent-cap", None);
        let content = tokio::fs::read_to_string(&path).await.unwrap();

        // Find the AUTO section bytes
        let auto_start = content.find(AUTO_START).unwrap();
        let auto_end = content.find(AUTO_END).unwrap() + AUTO_END.len();
        let auto_bytes = &content[auto_start..auto_end];
        assert!(
            auto_bytes.len() <= AUTO_BYTE_CAP,
            "AUTO section {} bytes exceeds cap {} bytes",
            auto_bytes.len(),
            AUTO_BYTE_CAP
        );
    }

    #[tokio::test]
    async fn creates_directory_and_file_on_first_run() {
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let store = MemoryStore::new_for_test().await.unwrap();

        // The subdirectory does NOT exist yet
        let path = wm.workspace_path("agent-new", None);
        assert!(!path.exists(), "file should not exist before first run");

        wm.regenerate_auto_section("agent-new", None, &store).await.unwrap();

        assert!(path.exists(), "file should exist after regenerate_auto_section");
    }

    // ── what the AUTO section reports having emitted ─────────────────────────

    async fn insert_tier1_memory(
        store: &MemoryStore,
        agent_id: &str,
        content: &str,
        memory_type: &str,
        importance: f64,
    ) -> i64 {
        let agent = agent_id.to_string();
        let content = content.to_string();
        let memory_type = memory_type.to_string();
        store
            .conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                c.execute(
                    "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                     created_at, created_by_type, created_by_ref,
                     last_modified_at, last_accessed_at, importance, tier, status)
                     VALUES ('agent', ?1, ?2, ?3, 'chat', 0, 'user', ?1, 0, 0, ?4, 1, 'active')",
                    rusqlite::params![agent, content, memory_type, importance],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    async fn insert_skill(
        store: &MemoryStore,
        agent_id: &str,
        skill_name: &str,
        trigger: &str,
        tool_sequence_json: &str,
    ) -> i64 {
        use crate::memory::store_insert::{NewProceduralSkillRecord, PromoteOutcome};
        let mut embedding = vec![0.0_f32; 384];
        // A distinct direction per skill: `promote_to_procedural_skill`
        // rejects a zero vector, and identical vectors would make any later
        // ranking assertion meaningless.
        embedding[(skill_name.len() + tool_sequence_json.len()) % 384] = 1.0;
        let outcome = store
            .promote_to_procedural_skill(
                NewProceduralSkillRecord {
                    agent_id: agent_id.to_string(),
                    project_id: None,
                    skill_name: skill_name.to_string(),
                    trigger: trigger.to_string(),
                    tool_sequence_json: tool_sequence_json.to_string(),
                    source_trajectory_ids: vec![1],
                    importance: 8.0,
                    now_ms: 1_700_000_000_000,
                },
                Some(embedding),
            )
            .await
            .unwrap();
        match outcome {
            PromoteOutcome::Inserted(id) => id,
            other => panic!("expected a fresh insert, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn regenerate_reports_the_ids_it_rendered() {
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let store = MemoryStore::new_for_test().await.unwrap();

        let pref = insert_tier1_memory(&store, "agent-emit", "prefiere voseo", "user_preference", 9.0).await;
        let arch =
            insert_tier1_memory(&store, "agent-emit", "hexagonal", "architectural_decision", 9.0).await;
        let weak =
            insert_tier1_memory(&store, "agent-emit", "prefiere tabs", "user_preference", 6.0).await;
        let skill = insert_skill(&store, "agent-emit", "arreglar-test", "un test falla", "[\"read\"]").await;

        let emitted = wm
            .regenerate_auto_section("agent-emit", None, &store)
            .await
            .unwrap();

        assert!(emitted.contains(TableKind::Memory, pref), "the rendered preference must be reported");
        assert!(emitted.contains(TableKind::Memory, arch), "the rendered decision must be reported");
        assert!(emitted.contains(TableKind::Procedural, skill), "the rendered skill must be reported");
        assert!(
            !emitted.contains(TableKind::Memory, weak),
            "importance 6 never reaches the AUTO section, so reporting it as emitted would suppress \
             the only channel that can still deliver it"
        );
        assert_eq!(emitted.len(), 3);
    }

    #[tokio::test]
    async fn a_superseded_row_is_not_in_the_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-superseded";
        let winner = insert_tier1_memory(&store, agent, "prefers tabs", "user_preference", 9.0).await;
        let loser = insert_tier1_memory(&store, agent, "prefers spaces", "user_preference", 9.0).await;
        let decision = insert_tier1_memory(&store, agent, "uses sqlite", "architectural_decision", 9.0).await;
        let replaced = insert_tier1_memory(&store, agent, "uses postgres", "architectural_decision", 9.0).await;
        let disputed = insert_tier1_memory(&store, agent, "prefers dark mode", "user_preference", 9.0).await;
        store
            .conn
            .call(move |c| {
                c.execute(
                    "UPDATE memory SET status = 'superseded', superseded_by = ?2 WHERE id = ?1",
                    rusqlite::params![loser, winner],
                )?;
                c.execute(
                    "UPDATE memory SET status = 'superseded', superseded_by = ?2 WHERE id = ?1",
                    rusqlite::params![replaced, decision],
                )?;
                c.execute(
                    "UPDATE memory SET status = 'disputed', disputed_against = ?2 WHERE id = ?1",
                    rusqlite::params![disputed, format!("[{winner}]")],
                )
            })
            .await
            .unwrap();

        for project in [None, Some("P1")] {
            let emitted = wm.regenerate_auto_section(agent, project, &store).await.unwrap();
            let content = tokio::fs::read_to_string(wm.workspace_path(agent, project)).await.unwrap();

            for (id, text) in [(winner, "prefers tabs"), (decision, "uses sqlite"), (disputed, "prefers dark mode")] {
                assert!(content.contains(text), "{text} missing:\n{content}");
                assert!(emitted.contains(TableKind::Memory, id));
            }
            for (id, text) in [(loser, "prefers spaces"), (replaced, "uses postgres")] {
                assert!(!content.contains(text), "{text} was superseded:\n{content}");
                assert!(!emitted.contains(TableKind::Memory, id));
            }
        }
    }

    #[tokio::test]
    async fn skill_line_carries_trigger_and_tool_sequence() {
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let store = MemoryStore::new_for_test().await.unwrap();

        insert_skill(
            &store,
            "agent-skill-render",
            "arreglar-test-roto",
            "un test falla después de editar",
            "[\"read\",\"edit\",\"test\"]",
        )
        .await;

        wm.regenerate_auto_section("agent-skill-render", None, &store)
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(wm.workspace_path("agent-skill-render", None))
            .await
            .unwrap();

        assert!(content.contains("arreglar-test-roto"), "skill name missing:\n{content}");
        assert!(
            content.contains("un test falla después de editar"),
            "the trigger is what tells the model WHEN to apply the skill:\n{content}"
        );
        assert!(
            content.contains("read → edit → test"),
            "the tool sequence is the actionable part:\n{content}"
        );
    }

    #[tokio::test]
    async fn a_malformed_tool_sequence_degrades_to_name_and_trigger() {
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let store = MemoryStore::new_for_test().await.unwrap();

        insert_skill(&store, "agent-bad-seq", "skill-rara", "cuando pasa X", "not json at all").await;

        wm.regenerate_auto_section("agent-bad-seq", None, &store).await.unwrap();

        let content = tokio::fs::read_to_string(wm.workspace_path("agent-bad-seq", None))
            .await
            .unwrap();
        assert!(content.contains("skill-rara"), "the skill must still be rendered:\n{content}");
        assert!(content.contains("cuando pasa X"));
        assert!(
            !content.contains("not json at all"),
            "an unparseable sequence is dropped, never dumped raw into the prompt:\n{content}"
        );
    }

    // The cap trims from the bottom of the AUTO section, and a trimmed row was
    // NOT delivered. Reporting it as emitted would make the KNN channel skip
    // it too, and the row would reach the model through no channel at all.
    #[tokio::test]
    async fn rows_dropped_by_the_byte_cap_are_not_reported_as_emitted() {
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let store = MemoryStore::new_for_test().await.unwrap();

        let padding = "x".repeat(400);
        let seeded = 40u32;
        for i in 0..seeded {
            insert_tier1_memory(
                &store,
                "agent-cap-emit",
                &format!("decision {i} {padding}"),
                "architectural_decision",
                10.0,
            )
            .await;
        }

        let emitted = wm
            .regenerate_auto_section("agent-cap-emit", None, &store)
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(wm.workspace_path("agent-cap-emit", None))
            .await
            .unwrap();
        let rendered = content.matches("\n- decision ").count();

        assert!(rendered < seeded as usize, "the fixture must actually overflow the cap");
        assert_eq!(
            emitted.len(),
            rendered,
            "emitted ids must count what survived the cap, not what was fetched"
        );
    }

    /// The AUTO section carries a heading only for a channel that something
    /// actually fills.
    ///
    /// It used to end with `## Current Session Focus` and `## Active Workspace
    /// Disputes`, both written into every single prompt with nothing under them
    /// — no producer ever defined what "focus" meant, and live disputes already
    /// reach the model through the prompt's own CONFLICTS block. A permanently
    /// empty heading is not free: it spends tokens telling the model a section
    /// exists and then teaches it that the section is always blank.
    ///
    /// Pinned as an exact set rather than two `!contains` checks so the next
    /// heading added without a producer behind it fails here too.
    #[tokio::test]
    async fn auto_section_has_no_heading_without_a_producer() {
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let store = MemoryStore::new_for_test().await.unwrap();

        wm.regenerate_auto_section("agent-headings", None, &store)
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(wm.workspace_path("agent-headings", None))
            .await
            .unwrap();
        let headings: Vec<&str> = content
            .lines()
            .filter(|l| l.starts_with("## "))
            .collect();

        assert_eq!(
            headings,
            vec![
                "## User Preferences (always active)",
                "## Architectural Decisions",
                "## Procedural Skills",
            ],
            "each AUTO heading must have a query behind it; got:\n{content}"
        );
    }

    #[tokio::test]
    async fn write_failure_returns_workspace_io_error() {
        // A regular file where a directory is expected makes `create_dir_all`
        // fail on every platform.
        let tmp = tempfile::tempdir().unwrap();
        let blocker = tmp.path().join("not-a-dir");
        std::fs::write(&blocker, "").unwrap();
        let wm = WorkspaceManager::new(blocker.join("workspaces"));
        let store = MemoryStore::new_for_test().await.unwrap();
        let result = wm.regenerate_auto_section("agent-1", None, &store).await;
        assert!(
            matches!(result, Err(MemoryError::WorkspaceIo { .. })),
            "expected WorkspaceIo error, got {:?}",
            result
        );
    }

    /// `workspace.md` is rewritten on every `recall_context` and carries the
    /// user's hand-written USER section, so a write that dies halfway must not
    /// cost them the file.
    #[tokio::test]
    async fn a_failed_workspace_write_leaves_the_previous_file_intact() {
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let store = MemoryStore::new_for_test().await.unwrap();
        let path = wm.workspace_path("agent-1", None);
        let dir = path.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        let previous = format!("{USER_START}\nmy own notes\n{USER_END}\n");
        std::fs::write(&path, &previous).unwrap();
        // A directory squatting on the temporary file's name makes its write fail.
        std::fs::create_dir(dir.join("workspace.md.tmp")).unwrap();

        let result = wm.regenerate_auto_section("agent-1", None, &store).await;

        assert!(
            matches!(result, Err(MemoryError::WorkspaceIo { .. })),
            "expected WorkspaceIo error, got {:?}",
            result
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), previous);
        let mut entries: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        entries.sort();
        assert_eq!(entries, vec!["workspace.md", "workspace.md.tmp"]);
    }

    // ── project scoping of the deterministic channel ─────────────────────────

    async fn insert_tier1_memory_with_project(
        store: &MemoryStore,
        agent_id: &str,
        project_id: Option<&str>,
        content: &str,
        memory_type: &str,
        importance: f64,
    ) -> i64 {
        let agent = agent_id.to_string();
        let project = project_id.map(|p| p.to_string());
        let content = content.to_string();
        let memory_type = memory_type.to_string();
        store
            .conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                c.execute(
                    "INSERT INTO memory (scope, agent_id, project_id, content, memory_type, source,
                     created_at, created_by_type, created_by_ref,
                     last_modified_at, last_accessed_at, importance, tier, status)
                     VALUES ('agent', ?1, ?2, ?3, ?4, 'chat', 0, 'user', ?1, 0, 0, ?5, 1, 'active')",
                    rusqlite::params![agent, project, content, memory_type, importance],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    async fn insert_skill_with_project(
        store: &MemoryStore,
        agent_id: &str,
        project_id: Option<&str>,
        skill_name: &str,
    ) -> i64 {
        use crate::memory::store_insert::{NewProceduralSkillRecord, PromoteOutcome};
        let mut embedding = vec![0.0_f32; 384];
        embedding[skill_name.len() % 384] = 1.0;
        let outcome = store
            .promote_to_procedural_skill(
                NewProceduralSkillRecord {
                    agent_id: agent_id.to_string(),
                    project_id: project_id.map(|p| p.to_string()),
                    skill_name: skill_name.to_string(),
                    trigger: format!("trigger-{skill_name}"),
                    // The dedup key is (agent_id, tool_sequence): two rows for the
                    // same agent sharing a sequence collapse into one UPDATE.
                    tool_sequence_json: format!("[\"read\",\"{skill_name}\"]"),
                    source_trajectory_ids: vec![1],
                    importance: 8.0,
                    now_ms: 1_700_000_000_000,
                },
                Some(embedding),
            )
            .await
            .unwrap();
        match outcome {
            PromoteOutcome::Inserted(id) => id,
            other => panic!("expected a fresh insert, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn workspace_top_skills_exclude_foreign_project() {
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let store = MemoryStore::new_for_test().await.unwrap();

        let own = insert_skill_with_project(&store, "agent-skills", Some("P1"), "skill-own").await;
        let foreign =
            insert_skill_with_project(&store, "agent-skills", Some("P2"), "skill-foreign").await;

        let emitted = wm
            .regenerate_auto_section("agent-skills", Some("P1"), &store)
            .await
            .unwrap();

        assert!(
            emitted.contains(TableKind::Procedural, own),
            "the skill belonging to the rendered project must be listed"
        );
        assert!(
            !emitted.contains(TableKind::Procedural, foreign),
            "a skill owned by another project must not occupy a slot in this project's listing"
        );
    }

    #[tokio::test]
    async fn workspace_top_skills_include_null_project() {
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let store = MemoryStore::new_for_test().await.unwrap();

        let unowned =
            insert_skill_with_project(&store, "agent-skills-null", None, "skill-unowned").await;

        let emitted = wm
            .regenerate_auto_section("agent-skills-null", Some("P1"), &store)
            .await
            .unwrap();

        assert!(
            emitted.contains(TableKind::Procedural, unowned),
            "a skill with no project of its own belongs to every project, so scoping must not hide it"
        );
    }

    #[tokio::test]
    async fn workspace_top_skills_unscoped_listing_ignores_project() {
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let store = MemoryStore::new_for_test().await.unwrap();

        let p1 = insert_skill_with_project(&store, "agent-skills-any", Some("P1"), "skill-p1").await;
        let p2 = insert_skill_with_project(&store, "agent-skills-any", Some("P2"), "skill-p2").await;
        let none = insert_skill_with_project(&store, "agent-skills-any", None, "skill-none").await;

        let emitted = wm
            .regenerate_auto_section("agent-skills-any", None, &store)
            .await
            .unwrap();

        for id in [p1, p2, none] {
            assert!(
                emitted.contains(TableKind::Procedural, id),
                "a caller that supplies no project must still see skill {id}"
            );
        }
    }

    #[tokio::test]
    async fn workspace_decisions_include_null_project() {
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let store = MemoryStore::new_for_test().await.unwrap();

        let unowned = insert_tier1_memory_with_project(
            &store,
            "agent-arch",
            None,
            "hexagonal por defecto",
            "architectural_decision",
            9.0,
        )
        .await;

        let emitted = wm
            .regenerate_auto_section("agent-arch", Some("P1"), &store)
            .await
            .unwrap();

        assert!(
            emitted.contains(TableKind::Memory, unowned),
            "a decision recorded without a project applies everywhere, so a scoped workspace must \
             still render it"
        );
    }

    #[tokio::test]
    async fn workspace_decisions_exclude_foreign_project() {
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let store = MemoryStore::new_for_test().await.unwrap();

        let own = insert_tier1_memory_with_project(
            &store,
            "agent-arch-x",
            Some("P1"),
            "decision de este proyecto",
            "architectural_decision",
            9.0,
        )
        .await;
        let foreign = insert_tier1_memory_with_project(
            &store,
            "agent-arch-x",
            Some("P2"),
            "decision de otro proyecto",
            "architectural_decision",
            9.0,
        )
        .await;

        let emitted = wm
            .regenerate_auto_section("agent-arch-x", Some("P1"), &store)
            .await
            .unwrap();

        assert!(emitted.contains(TableKind::Memory, own), "own-project decision must be rendered");
        assert!(
            !emitted.contains(TableKind::Memory, foreign),
            "another project's decision must not be rendered here"
        );
    }

    #[tokio::test]
    async fn workspace_preferences_cross_project_boundaries() {
        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        let store = MemoryStore::new_for_test().await.unwrap();

        let p1 = insert_tier1_memory_with_project(
            &store, "agent-prefs", Some("P1"), "prefiere voseo", "user_preference", 9.0,
        )
        .await;
        let p2 = insert_tier1_memory_with_project(
            &store, "agent-prefs", Some("P2"), "prefiere tabs", "user_preference", 9.0,
        )
        .await;

        let emitted = wm
            .regenerate_auto_section("agent-prefs", Some("P1"), &store)
            .await
            .unwrap();

        for id in [p1, p2] {
            assert!(
                emitted.contains(TableKind::Memory, id),
                "preference {id} describes the person, not the project, so it must survive scoping"
            );
        }
    }
}
