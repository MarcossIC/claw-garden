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
            // How `save_memory` stores an explicit memory with `user_stated`.
            created_by_type: if memory.user_stated { "user" } else { "agent" }.into(),
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

/// The memories an arm with memory is seeded with: the noise and the task's,
/// from the newest to the oldest and, at one age, the noise first, so that a
/// task's own memory is not always in the same place.
pub fn seeded(task: &[SeedMemory], noise: &[SeedMemory]) -> Vec<SeedMemory> {
    let mut all: Vec<SeedMemory> = noise.iter().chain(task).cloned().collect();
    all.sort_by_key(|m| m.age_days);
    all
}

/// `memories`, as [`seeded`] orders them, as a native memory file: one section
/// per memory, its text whole, from the oldest to the newest as an agent that
/// appends its notes leaves it.
pub fn native_memory(memories: &[SeedMemory]) -> String {
    let mut oldest_first: Vec<&SeedMemory> = memories.iter().collect();
    // Stable, so at one age the noise stays before the task's memory.
    oldest_first.sort_by_key(|m| std::cmp::Reverse(m.age_days));
    let mut out = String::from("# Memory\n");
    for memory in oldest_first {
        out.push('\n');
        out.push_str(memory.content.trim_end());
        out.push('\n');
    }
    out
}

/// How many lines of a native memory file the client loads at startup.
const NATIVE_LOADED_LINES: usize = 200;

/// How many bytes of it, when they come before those lines.
const NATIVE_LOADED_BYTES: usize = 25_000;

/// What the client puts in the context of a session of a native memory file
/// `text`: its first 200 lines or its first 25,000 bytes, whichever it reaches
/// first, as the client documents it.
pub fn native_loaded(text: &str) -> &str {
    let by_lines = text.match_indices('\n').nth(NATIVE_LOADED_LINES - 1).map_or(text.len(), |(at, _)| at + 1);
    let mut end = by_lines.min(NATIVE_LOADED_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
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
            user_stated: false,
            rule: None,
        }
    }

    #[test]
    fn a_seeded_memory_the_person_stated_is_the_persons() {
        let mut stated = memory("Name counters with _n", "user_preference", 30, 1);
        stated.user_stated = true;
        let text = export(&[stated, memory("Chose SQLite", "architectural_decision", 0, 1)], "claude-code", "/w", "f", 0);
        let by: Vec<String> = text
            .lines()
            .filter_map(|l| match serde_json::from_str(l).unwrap() {
                Line::Memory(m) => Some(m.created_by_type),
                _ => None,
            })
            .collect();
        assert_eq!(by, ["user", "agent"]);
    }

    #[test]
    fn the_seed_goes_from_the_newest_memory_to_the_oldest() {
        let task = [memory("task old", "semantic_fact", 120, 1), memory("task new", "semantic_fact", 10, 1)];
        let noise = [memory("noise mid", "semantic_fact", 60, 1), memory("noise new", "semantic_fact", 10, 1), memory("noise oldest", "semantic_fact", 200, 1)];
        let order: Vec<String> = seeded(&task, &noise).into_iter().map(|m| m.content).collect();
        assert_eq!(order, ["noise new", "task new", "noise mid", "task old", "noise oldest"], "at one age the noise goes first");
        assert!(seeded(&[], &[]).is_empty());
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
    fn the_native_memory_goes_from_the_oldest_memory_to_the_newest() {
        let task = [memory("task old", "semantic_fact", 120, 1), memory("task new", "semantic_fact", 10, 1)];
        let noise = [memory("noise mid", "semantic_fact", 60, 1), memory("noise new", "semantic_fact", 10, 1), memory("noise oldest", "semantic_fact", 200, 1)];
        let text = native_memory(&seeded(&task, &noise));
        let order: Vec<&str> = text.lines().filter(|l| !l.is_empty() && *l != "# Memory").collect();
        assert_eq!(order, ["noise oldest", "task old", "noise mid", "noise new", "task new"], "at one age the noise goes first");
    }

    #[test]
    fn the_client_loads_the_first_200_lines_or_25_kb_of_the_native_memory() {
        let lines = |n: usize| (1..=n).map(|i| format!("line {i}\n")).collect::<String>();
        assert_eq!(native_loaded("# Memory\n\nshort\n"), "# Memory\n\nshort\n");
        assert_eq!(native_loaded(""), "");

        let long = lines(900);
        let loaded = native_loaded(&long);
        assert_eq!(loaded.lines().count(), 200, "the lines cut first");
        assert!(loaded.ends_with("line 200\n") && !loaded.contains("line 201\n"));
        assert!(long.starts_with(loaded));
        let exact = lines(200);
        assert_eq!(native_loaded(&exact), exact, "200 lines are all loaded");

        let wide = format!("{}\n{}", "x".repeat(24_000), "y".repeat(5_000));
        let loaded = native_loaded(&wide);
        assert_eq!(loaded.len(), 25_000, "the bytes cut first: a line is cut where the limit falls");
        assert!(loaded.ends_with('y'));

        let euros = "€".repeat(9_000);
        let loaded = native_loaded(&euros);
        assert_eq!(loaded.len(), 24_999, "never in the middle of a character");
    }

    #[test]
    fn the_slug_keeps_letters_and_digits() {
        assert_eq!(project_slug("/tmp/run-1/work"), "-tmp-run-1-work");
        assert_eq!(project_slug(r"C:\runs\a b"), "C--runs-a-b");
    }
}
