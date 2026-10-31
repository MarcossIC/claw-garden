//! The same seeded facts in the two shapes the arms read them in: a
//! `clawg-memories` export that `clawg import` loads with its dates, accesses
//! and tier, and a `MEMORY.md` for the client's native memory.

use clawg_core::eval_api::earned_tier;
use clawg_core::memory::portability::format::{Filter, Header, Line, MemoryLine, ProjectLine, FORMAT, VERSION};

use crate::task::SeedMemory;

const DAY_MS: i64 = 86_400_000;

/// The id the export gives the fixture's project; the import places its rows
/// in the project registered for `root`.
pub const PROJECT_ID: &str = "agent-eval-fixture";

/// The export of `memories`, saved by `agent` in the project rooted at `root`,
/// each `age_days` before `now_ms`.
pub fn export(memories: &[SeedMemory], agent: &str, root: &str, name: &str, now_ms: i64) -> String {
    let mut lines = vec![
        Line::Header(Header {
            format: FORMAT.into(),
            version: VERSION,
            exported_at: now_ms,
            clawg: env!("CARGO_PKG_VERSION").into(),
            embedding_model: None,
            filter: Filter::default(),
        }),
        Line::Project(ProjectLine {
            id: PROJECT_ID.into(),
            root_path: Some(root.into()),
            name: Some(name.into()),
            ..Default::default()
        }),
    ];
    for (i, memory) in memories.iter().enumerate() {
        // Rows of one age stay apart so their order is the file's.
        let created_at = now_ms - i64::from(memory.age_days) * DAY_MS + i as i64;
        lines.push(Line::Memory(MemoryLine {
            id: i as i64 + 1,
            scope: "agent".into(),
            agent_id: Some(agent.into()),
            project_id: Some(PROJECT_ID.into()),
            content: memory.content.clone(),
            memory_type: memory.memory_type.clone(),
            source: "explicit".into(),
            created_at,
            created_by_type: "agent".into(),
            created_by_ref: String::new(),
            source_session_id: None,
            last_modified_at: created_at,
            last_modified_by: None,
            update_count: 0,
            last_accessed_at: created_at,
            access_count: memory.access_count,
            importance: memory.importance,
            tier: earned_tier(memory.importance, memory.access_count, &memory.memory_type),
            status: "active".into(),
            superseded_by: None,
            supersede_cause: None,
            dispute_count: 0,
            disputed_against: Vec::new(),
            last_disputed_at: None,
            deleted_at: None,
            topic_key: memory.topic_key.clone(),
            revived_from: None,
            lang: None,
            origin_project: None,
        }));
    }
    lines
        .iter()
        .map(|line| serde_json::to_string(line).expect("an export line serializes") + "\n")
        .collect()
}

/// `memories` as a native memory file: one section per memory, its text whole.
pub fn native_memory(memories: &[SeedMemory]) -> String {
    let mut out = String::from("# Memory\n");
    for memory in memories {
        out.push('\n');
        out.push_str(memory.content.trim_end());
        out.push('\n');
    }
    out
}

/// The directory name the client keeps a project's native memory under: the
/// absolute path of its working directory with every character that is not
/// a letter or a digit turned into `-`.
pub fn project_slug(workdir: &str) -> String {
    workdir.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawg_core::memory::portability::format::Line;

    fn memory(content: &str, memory_type: &str, age_days: u32, access_count: i64) -> SeedMemory {
        SeedMemory {
            content: content.into(),
            memory_type: memory_type.into(),
            importance: 5.0,
            age_days,
            access_count,
            topic_key: Some("deploy/port".into()),
        }
    }

    #[test]
    fn the_export_dates_each_memory_and_gives_it_the_tier_the_store_would() {
        let now = 1_800_000_000_000;
        let text = export(
            &[memory("Staging uses port 48213", "semantic_fact", 40, 1), memory("Chose SQLite", "architectural_decision", 0, 3)],
            "claude-code",
            "/runs/1/work",
            "tally",
            now,
        );
        let lines: Vec<Line> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 4);
        let Line::Header(header) = &lines[0] else { panic!("the header goes first") };
        assert_eq!(header.format, FORMAT);
        let Line::Project(project) = &lines[1] else { panic!("then the project") };
        assert_eq!(project.root_path.as_deref(), Some("/runs/1/work"));
        let Line::Memory(old) = &lines[2] else { panic!("then the memories") };
        assert_eq!(old.created_at, now - 40 * DAY_MS);
        assert_eq!(old.last_accessed_at, old.created_at);
        assert_eq!((old.tier, old.status.as_str()), (3, "active"));
        assert_eq!(old.project_id.as_deref(), Some(PROJECT_ID));
        assert_eq!(old.topic_key.as_deref(), Some("deploy/port"));
        let Line::Memory(decision) = &lines[3] else { panic!() };
        assert_eq!((decision.id, decision.tier, decision.created_at), (2, 1, now + 1));
    }

    #[test]
    fn the_native_memory_holds_every_text_whole() {
        let text = native_memory(&[memory("Staging uses port 48213\nbecause 8080 is taken.", "semantic_fact", 0, 1), memory("Chose SQLite", "architectural_decision", 0, 1)]);
        assert_eq!(text, "# Memory\n\nStaging uses port 48213\nbecause 8080 is taken.\n\nChose SQLite\n");
    }

    #[test]
    fn the_slug_keeps_letters_and_digits() {
        assert_eq!(project_slug("/tmp/run-1/work"), "-tmp-run-1-work");
        assert_eq!(project_slug(r"C:\runs\a b"), "C--runs-a-b");
    }
}
