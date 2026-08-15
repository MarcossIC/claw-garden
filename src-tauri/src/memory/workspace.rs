//! Owns the on-disk `workspace.md` file for a (agent_id, project_id) pair:
//! the AUTO section is regenerated from DB queries, the USER section is
//! preserved byte-for-byte.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::memory::{record::TableKind, MemoryError, MemoryStore};

pub(crate) const DEFAULT_PROJECT_SEGMENT: &str = "default";
pub(crate) const AUTO_START: &str = "<!-- AUTO-START -->";
pub(crate) const AUTO_END: &str = "<!-- AUTO-END -->";
pub(crate) const USER_START: &str = "<!-- USER-START -->";
pub(crate) const USER_END: &str = "<!-- USER-END -->";

/// Hard cap on the AUTO section's token count (approximation: 4 bytes/token).
pub(crate) const AUTO_TOKEN_CAP: usize = 1500;
pub(crate) const AUTO_BYTE_CAP: usize = AUTO_TOKEN_CAP * 4;

/// Stateless façade. Holds only the base directory under which all workspaces live.
/// Trivially `Clone + Send + Sync`.
#[derive(Debug, Clone)]
pub(crate) struct WorkspaceManager {
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
    pub(crate) fn new(workspaces_root: PathBuf) -> Self {
        Self { workspaces_root }
    }

    /// Resolves the on-disk path. Pure — no I/O.
    pub(crate) fn workspace_path(&self, agent_id: &str, project_id: Option<&str>) -> PathBuf {
        self.workspaces_root
            .join(agent_id)
            .join(project_id.unwrap_or(DEFAULT_PROJECT_SEGMENT))
            .join("workspace.md")
    }

    /// Regenerates the AUTO section from DB queries. USER section is preserved
    /// byte-for-byte.
    ///
    /// Returns the ids this call actually rendered, so the retrieval channel
    /// can skip them rather than delivering the same rows a second time. They
    /// are reported by the renderer itself, after the byte cap has done its
    /// trimming — a second copy of the selection predicate elsewhere would be
    /// free to drift from what the file really says.
    pub(crate) async fn regenerate_auto_section(
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

        // Write file
        tokio::fs::write(&path, assembled.as_bytes())
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
    /// No producer defines what "focus" means yet, so this is always `None`.
    current_focus: Option<String>,
    /// Always empty: live disputes reach the model through the prompt's own
    /// CONFLICTS block, which queries them directly.
    workspace_disputes: Vec<(i64, i64)>,
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
pub(crate) struct EmittedIds {
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
            // Query 1: user_preference rows with importance >= 8
            let mut stmt = c.prepare(
                "SELECT id, content FROM memory
                 WHERE memory_type = 'user_preference'
                   AND importance >= 8
                   AND agent_id = ?1
                   AND tier = 1
                   AND deleted_at IS NULL
                 ORDER BY importance DESC",
            )?;
            let user_prefs: Vec<MemoryLine> = stmt
                .query_map(rusqlite::params![agent], |row| {
                    Ok(MemoryLine { id: row.get(0)?, content: row.get(1)? })
                })?
                .filter_map(|r| r.ok())
                .collect();

            // Query 2: architectural_decision rows, scoped to project when provided,
            // otherwise all architectural decisions for the agent. importance >= 8.
            let arch_decisions: Vec<MemoryLine> = if let Some(ref pid) = proj {
                let mut stmt2 = c.prepare(
                    "SELECT id, content FROM memory
                     WHERE memory_type = 'architectural_decision'
                       AND importance >= 8
                       AND agent_id = ?1
                       AND project_id = ?2
                       AND tier = 1
                       AND deleted_at IS NULL
                     ORDER BY importance DESC",
                )?;
                let rows: Vec<MemoryLine> = stmt2
                    .query_map(rusqlite::params![agent, pid], |row| {
                        Ok(MemoryLine { id: row.get(0)?, content: row.get(1)? })
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                rows
            } else {
                let mut stmt2 = c.prepare(
                    "SELECT id, content FROM memory
                     WHERE memory_type = 'architectural_decision'
                       AND importance >= 8
                       AND agent_id = ?1
                       AND tier = 1
                       AND deleted_at IS NULL
                     ORDER BY importance DESC",
                )?;
                let rows: Vec<MemoryLine> = stmt2
                    .query_map(rusqlite::params![agent], |row| {
                        Ok(MemoryLine { id: row.get(0)?, content: row.get(1)? })
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                rows
            };

            // Query 3: top-5 procedural skills by access_count
            let mut stmt3 = c.prepare(
                "SELECT id, skill_name, trigger, tool_sequence FROM procedural_skill
                 WHERE agent_id = ?1
                   AND tier = 1
                   AND deleted_at IS NULL
                 ORDER BY access_count DESC
                 LIMIT 5",
            )?;
            let top_skills: Vec<SkillLine> = stmt3
                .query_map(rusqlite::params![agent], |row| {
                    Ok(SkillLine {
                        id: row.get(0)?,
                        skill_name: row.get(1)?,
                        trigger: row.get(2)?,
                        tool_sequence_json: row.get(3)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();

            let current_focus: Option<String> = None;
            let workspace_disputes: Vec<(i64, i64)> = Vec::new();

            Ok(AutoRows {
                user_prefs,
                arch_decisions,
                top_skills,
                current_focus,
                workspace_disputes,
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

/// Renders the AUTO section AND reports which rows ended up inside it.
///
/// The byte cap is applied here, in the same pass, rather than as a post-hoc
/// pass over finished text: a row the cap dropped was not delivered, and
/// reporting it as emitted would make the retrieval channel skip it too —
/// the row would then reach the model through no channel at all.
fn render_auto_section(rows: &AutoRows) -> (String, EmittedIds) {
    let mut kept = rows.clone();
    let mut content = render_rows(&kept);

    // Reverse priority: disputes → focus → skills → decisions. Preferences
    // are never trimmed — they are the standing contract with the user, and
    // dropping one silently changes how the model behaves for a whole
    // session.
    if content.len() > AUTO_BYTE_CAP {
        kept.workspace_disputes.clear();
        content = render_rows(&kept);
    }
    if content.len() > AUTO_BYTE_CAP {
        kept.current_focus = None;
        content = render_rows(&kept);
    }
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
    out.push_str("## Current Session Focus\n");
    if let Some(ref focus) = rows.current_focus {
        out.push_str(&format!("{}\n", focus));
    }
    out.push_str("## Active Workspace Disputes\n");
    for (old_id, new_id) in &rows.workspace_disputes {
        out.push_str(&format!("- conflict: memory:{} vs memory:{}\n", old_id, new_id));
    }
    out.push_str(AUTO_END);
    out.push('\n');
    out
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::MemoryStore;

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

    #[tokio::test]
    async fn write_failure_returns_workspace_io_error() {
        // On Windows, use a path under C:\Windows\System32 or a deeply nested path
        // that should fail due to permissions
        let wm = WorkspaceManager::new(PathBuf::from("C:\\Windows\\System32\\claw-garden-test-forbidden"));
        let store = MemoryStore::new_for_test().await.unwrap();
        let result = wm.regenerate_auto_section("agent-1", None, &store).await;
        assert!(
            matches!(result, Err(MemoryError::WorkspaceIo { .. })),
            "expected WorkspaceIo error, got {:?}",
            result
        );
    }
}
