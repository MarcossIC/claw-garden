//! `clawg projects add`: let the daemon read a project's files.
//! `clawg projects move`: move memories to another project, or out of every
//! project.
//!
//! A bridge only has its own root indexed; any other root the model names is
//! a label for memories. This command is how the user, not the model, adds
//! one. Registering and moving go through the running daemon, which owns the
//! store.

use std::path::{Path, PathBuf};

use clawg_core::memory::relocate::MoveReport;
use clawg_core::memory::ProjectIdentity;

use crate::cli::MoveOpts;
use crate::client::{ClientError, DaemonClient};

#[derive(Debug, thiserror::Error)]
pub enum AddError {
    #[error("{} is not a directory", .0.display())]
    NotADirectory(PathBuf),
    #[error(transparent)]
    Client(#[from] ClientError),
}

/// Registers the project `path` is in so the daemon may read its files, and
/// says which root and id that was.
pub async fn add(client: &DaemonClient, path: &Path) -> Result<String, AddError> {
    let project = ProjectIdentity::resolve(path);
    if !project.root.is_dir() {
        return Err(AddError::NotADirectory(project.root));
    }
    let legacy: Vec<String> = Some(project.legacy_id.clone()).filter(|l| *l != project.id).into_iter().collect();
    let (id, _) = client.register_project(&project.root, &legacy, true).await?;
    Ok(format!(
        "added {} as project {id}; the daemon is indexing its files in the background",
        project.root.display()
    ))
}

/// Sends the move `opts` asks for and says what it did. `--to PATH` is
/// registered first, without letting the daemon read its files, and must be
/// a directory; `--here` is the project of `cwd`.
pub async fn move_memories(client: &DaemonClient, opts: &MoveOpts, cwd: &Path) -> Result<String, AddError> {
    let to = match &opts.to {
        Some(path) => {
            let project = ProjectIdentity::resolve(path);
            if !project.root.is_dir() {
                return Err(AddError::NotADirectory(project.root));
            }
            let legacy: Vec<String> =
                Some(project.legacy_id.clone()).filter(|l| *l != project.id).into_iter().collect();
            Some(client.register_project(&project.root, &legacy, false).await?.0)
        }
        None => None,
    };
    let here = opts.here.then(|| ProjectIdentity::resolve(cwd).id);
    let report = client.move_memories(&opts.request(here, to)).await?;
    Ok(render_move(&report))
}

fn place(project: &Option<String>) -> String {
    match project {
        Some(p) => format!("project {p}"),
        None => "no project".to_string(),
    }
}

fn id_list(ids: impl IntoIterator<Item = i64>) -> String {
    ids.into_iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(", ")
}

/// What `clawg projects move` prints for `report`.
pub fn render_move(report: &MoveReport) -> String {
    let mut lines = Vec::new();
    let verb = if report.dry_run { "would move" } else { "moved" };
    if report.moved.is_empty() {
        lines.push(if report.dry_run { "nothing would move".to_string() } else { "nothing moved".to_string() });
    } else {
        let mut by_target: std::collections::BTreeMap<String, Vec<i64>> = std::collections::BTreeMap::new();
        for m in &report.moved {
            by_target.entry(place(&m.to_project)).or_default().push(m.id);
        }
        for (target, ids) in by_target {
            let count = if ids.len() == 1 { "1 memory".to_string() } else { format!("{} memories", ids.len()) };
            lines.push(format!("{verb} {count} to {target}: {}", id_list(ids)));
        }
    }
    if !report.pulled_in.is_empty() {
        lines.push(format!(
            "{} of them were not selected but go with a save or topic history that was: {}",
            report.pulled_in.len(),
            id_list(report.pulled_in.iter().copied())
        ));
    }
    if report.moved.iter().any(|m| m.to_project.is_none()) {
        lines.push("a memory in no project is found from every project".to_string());
    }
    if report.protected > 0 {
        lines.push(format!("{} of them are preferences or decisions", report.protected));
    }
    if !report.unchanged.is_empty() {
        lines.push(format!("already there: {}", id_list(report.unchanged.iter().copied())));
    }
    for skipped in &report.skipped {
        lines.push(format!("left where it is: {skipped}"));
    }
    if !report.split_pairs.is_empty() {
        let pairs: Vec<String> = report.split_pairs.iter().map(|[a, b]| format!("#{a}/#{b}")).collect();
        lines.push(format!("pairs to judge now across two projects: {}", pairs.join(", ")));
    }
    if let Some(snapshot) = &report.snapshot {
        lines.push(format!("snapshot before the move: {snapshot}; `clawg restore` brings it back"));
    }
    if let Some(id) = report.move_id {
        lines.push(format!("this is move {id}; `clawg projects move --undo {id}` moves it back"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tests::{publish, start_daemon};
    use clawg_core::memory::relocate::{MovedRow, Refusal, Skipped};

    fn moved(id: i64, to: Option<&str>) -> MovedRow {
        serde_json::from_value(serde_json::json!({ "id": id, "from_project": null, "to_project": to })).unwrap()
    }

    #[test]
    fn a_move_report_names_what_moved_what_stayed_and_the_way_back() {
        let report = MoveReport {
            move_id: Some(3),
            moved: vec![moved(1, Some("repo-1a2b")), moved(2, Some("repo-1a2b"))],
            pulled_in: vec![2],
            unchanged: vec![5],
            skipped: vec![Skipped { id: 4, refusal: Refusal::LiveTwin { row: 4, twin: 9 } }],
            split_pairs: vec![[7, 1]],
            protected: 1,
            snapshot: Some("/data/backups/memory-x-pre-move.db".into()),
            ..MoveReport::default()
        };

        assert_eq!(
            render_move(&report),
            "moved 2 memories to project repo-1a2b: #1, #2\n\
             1 of them were not selected but go with a save or topic history that was: #2\n\
             1 of them are preferences or decisions\n\
             already there: #5\n\
             left where it is: memory #4 cannot move: #4 holds the same text as #9, live in the target; \
             delete one of them and move again\n\
             pairs to judge now across two projects: #7/#1\n\
             snapshot before the move: /data/backups/memory-x-pre-move.db; `clawg restore` brings it back\n\
             this is move 3; `clawg projects move --undo 3` moves it back"
        );
    }

    #[test]
    fn a_dry_run_says_what_would_move() {
        let report = MoveReport { dry_run: true, moved: vec![moved(1, None)], ..MoveReport::default() };

        assert_eq!(
            render_move(&report),
            "would move 1 memory to no project: #1\na memory in no project is found from every project"
        );
        assert_eq!(render_move(&MoveReport { dry_run: true, ..MoveReport::default() }), "nothing would move");
    }

    #[tokio::test]
    async fn adding_a_project_lets_its_docs_be_searched() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");
        let repo = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(repo.path().join(".git")).unwrap();
        let root = ProjectIdentity::resolve(repo.path()).root;
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(root.join("README.md"), "# Other\nHow the other project ships.\n").unwrap();
        let (id, _) = c.register_project(&root, &[], false).await.expect("register as a label");
        let refused = c.search_docs(&id, "How the other project ships", None).await.expect_err("not accepted");
        assert!(matches!(refused, ClientError::Daemon { status: 403, .. }), "{refused:?}");

        let out = add(&c, &root.join("docs")).await.expect("add");

        assert_eq!(
            out,
            format!("added {} as project {id}; the daemon is indexing its files in the background", root.display())
        );
        let found = c.search_docs(&id, "How the other project ships", None).await.expect("search");
        assert_eq!(found.hits.first().map(|h| h.path.as_str()), Some("README.md"), "{:?}", found.hits);
    }

    #[tokio::test]
    async fn projects_move_moves_a_memory_and_undoes_it() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");
        let repo = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(repo.path().join(".git")).unwrap();
        let root = ProjectIdentity::resolve(repo.path()).root;
        c.save(crate::client::SaveArgs {
            content: "a note filed nowhere".into(),
            agent_id: "agent-a".into(),
            project_id: None,
            memory_type: Some("semantic_fact".into()),
            importance: None,
            derived_from: Vec::new(),
            topic_key: None,
            user_stated: false,
        })
        .await
        .expect("save");
        let mut rows = Vec::new();
        for _ in 0..40 {
            rows = c.list(crate::client::ListArgs::default()).await.expect("list");
            if !rows.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let id = rows.first().expect("the save landed").id;
        let opts = |args: &[&str]| -> MoveOpts {
            let mut argv = vec!["clawg", "projects", "move"];
            argv.extend_from_slice(args);
            match <crate::cli::Cli as clap::Parser>::try_parse_from(&argv).expect("parse").command {
                crate::cli::Command::Projects { action: crate::cli::ProjectsAction::Move(opts) } => opts,
                other => panic!("{other:?}"),
            }
        };
        let to = root.to_string_lossy().into_owned();
        let selected = id.to_string();

        let dry = move_memories(&c, &opts(&["--id", &selected, "--to", &to, "--dry-run"]), repo.path()).await.expect("dry run");
        assert!(dry.starts_with(&format!("would move 1 memory to project {}", ProjectIdentity::resolve(&root).id)), "{dry}");

        let done = move_memories(&c, &opts(&["--id", &selected, "--to", &to]), repo.path()).await.expect("move");
        assert!(done.contains("this is move 1"), "{done}");
        let here = move_memories(&c, &opts(&["--here", "--to-none", "--dry-run"]), &root).await.expect("here");
        assert!(here.contains(&format!("#{id}")), "{here}");

        let undone = move_memories(&c, &opts(&["--undo", "1"]), repo.path()).await.expect("undo");
        assert!(undone.starts_with(&format!("moved 1 memory to no project: #{id}")), "{undone}");
    }

    #[tokio::test]
    async fn adding_a_path_that_is_not_a_directory_is_refused() {
        // Nothing listens here: the path is refused before the daemon is asked.
        let c = DaemonClient::new("http://127.0.0.1:1", "token", std::env::temp_dir());
        let dir = tempfile::tempdir().expect("tempdir");

        let err = add(&c, &dir.path().join("gone")).await.expect_err("missing");

        assert!(matches!(err, AddError::NotADirectory(_)), "{err:?}");
    }
}
