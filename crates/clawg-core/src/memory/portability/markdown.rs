//! A Markdown view of an export, to read without clawg. Nothing reads it back.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::memory::backup::civil_from_days;
use crate::memory::portability::export::Export;
use crate::memory::portability::format::{MemoryLine, ProjectLine};
use crate::memory::store_insert::MemoryType;

/// The rows of `export` by project, then by type, then by id.
pub fn render(export: &Export) -> String {
    let mut out = String::from("# clawg memories\n\n");
    let _ = writeln!(
        out,
        "Exported {} by clawg {}: {} memories. A view to read; `clawg import` reads the JSON Lines export.",
        stamp(export.header.exported_at),
        export.header.clawg,
        export.memories.len()
    );

    let mut groups: BTreeMap<(usize, String), Vec<&MemoryLine>> = BTreeMap::new();
    for m in &export.memories {
        let key = match &m.project_id {
            Some(id) => match export.projects.iter().position(|p| &p.id == id) {
                Some(i) => (i, id.clone()),
                None => (export.projects.len(), id.clone()),
            },
            None => (usize::MAX, String::new()),
        };
        groups.entry(key).or_default().push(m);
    }

    for ((_, project_id), rows) in groups {
        let title = if project_id.is_empty() {
            "No project".to_string()
        } else {
            match export.projects.iter().find(|p| p.id == project_id) {
                Some(ProjectLine { id, root_path: Some(root), .. }) => format!("{id} — {root}"),
                Some(p) => p.name.clone().map_or_else(|| p.id.clone(), |name| format!("{} — {name}", p.id)),
                None => project_id.clone(),
            }
        };
        let _ = write!(out, "\n## {title}\n");
        for ty in MemoryType::ALL {
            let of_type: Vec<&&MemoryLine> = rows.iter().filter(|m| m.memory_type == ty.as_str()).collect();
            if of_type.is_empty() {
                continue;
            }
            let _ = write!(out, "\n### {}\n", ty.as_str());
            for m in of_type {
                row(&mut out, export, m);
            }
        }
    }
    out
}

fn row(out: &mut String, export: &Export, m: &MemoryLine) {
    let mut state = m.status.clone();
    if let Some(by) = m.superseded_by {
        let _ = write!(state, " by #{by}");
    }
    if let Some(at) = m.deleted_at {
        let _ = write!(state, ", deleted {}", stamp(at));
    }
    let _ = write!(out, "\n#### #{} · {} · {}\n\n", m.id, stamp(m.created_at), state);

    let mut facts = vec![match &m.agent_id {
        Some(agent) => format!("agent {agent}"),
        None => format!("{} scope", m.scope),
    }];
    if let Some(key) = &m.topic_key {
        facts.push(format!("topic {key}"));
    }
    if m.source == "synthesis" {
        let sources: Vec<String> =
            export.derivations.iter().filter(|d| d.memory == m.id).map(|d| format!("#{}", d.source)).collect();
        facts.push(format!("conclusion from {}", sources.join(", ")));
    }
    if let Some(p) = export.pieces.iter().find(|p| p.memory == m.id) {
        let of = export.pieces.iter().filter(|q| q.save == p.save).count();
        facts.push(format!("piece {} of {of}", p.piece));
    }
    let _ = write!(out, "_{}_\n\n", facts.join(" · "));
    for line in m.content.lines() {
        let _ = writeln!(out, "> {line}");
    }
}

/// `YYYY-MM-DD HH:MM UTC`.
fn stamp(ms: i64) -> String {
    let (days, ms_of_day) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {:02}:{:02} UTC", ms_of_day / 3_600_000, ms_of_day / 60_000 % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::portability::export::{read, ExportFilter};
    use crate::memory::portability::fixtures::{other_project, project, seed};
    use crate::memory::MemoryStore;

    #[tokio::test]
    async fn markdown_groups_rows_by_project_and_type() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed(&store).await;
        let filter = ExportFilter { include_deleted: true, ..Default::default() };
        let export = store.conn.call(move |c| Ok::<_, rusqlite::Error>(read(c, &filter, 0))).await.unwrap().unwrap();
        let text = render(&export);

        let at = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("{needle:?} missing in\n{text}"));
        let (first, second) = if project() < other_project() { (project(), other_project()) } else { (other_project(), project()) };
        assert!(at(&format!("## {first}")) < at(&format!("## {second}")));
        assert!(at(&format!("## {second}")) < at("## No project"));
        assert!(text.contains(&format!("## {} — /work/repo", project())));
        assert!(text.starts_with("# clawg memories\n\nExported 1970-01-01 00:00 UTC by clawg"));

        let repo = &text[at(&format!("## {} — /work/repo", project()))..];
        assert!(repo.find("### semantic_fact").unwrap() < repo.find("### user_preference").unwrap());
        assert!(text.contains("#### #1 · 2025-06-15 15:06 UTC · superseded by #2\n\n_agent claude-code · topic storage/engine_\n\n> Storage engine: Postgres\n"));
        assert!(text.contains("#### #11 · 2025-06-15 15:06 UTC · active, deleted 2025-08-24"), "{text}");
        assert!(text.contains("conclusion from #2, #3"));
        assert!(text.contains("piece 2 of 2"));
        assert!(text.contains("_project scope_"));
    }
}
