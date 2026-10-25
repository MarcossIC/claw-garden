//! `clawg projects list`: every project, with what leads to it.
//! `clawg projects rename`: name a project and mark it reviewed.
//! `clawg projects add`: let the daemon read a project's files.
//! `clawg projects move`: move memories to another project, or out of every
//! project.
//!
//! A bridge only has its own root indexed; any other root the model names is
//! a label for memories. `add` is how the user, not the model, adds one.
//! Everything goes through the running daemon, which owns the store.

use std::path::{Path, PathBuf};

use clawg_core::memory::registry::{ListedProject, ListedSuggestion};
use clawg_core::memory::relocate::MoveReport;
use clawg_core::memory::ProjectIdentity;
use clawg_core::preferences::Lang;

use crate::cli::MoveOpts;
use crate::client::{ClientError, DaemonClient};

#[derive(Debug, thiserror::Error)]
pub enum AddError {
    #[error("{} is not a directory", .0.display())]
    NotADirectory(PathBuf),
    #[error(transparent)]
    Client(#[from] ClientError),
}

/// Every project the daemon has, as [`render_list`] prints it.
pub async fn list(lang: Lang, client: &DaemonClient) -> Result<String, ClientError> {
    Ok(render_list(lang, &client.list_projects().await?))
}

/// Names project `id` and marks it reviewed, and says so.
pub async fn rename(lang: Lang, client: &DaemonClient, id: &str, name: &str) -> Result<String, ClientError> {
    let project = client.rename_project(id, name).await?;
    let (id, name) = (project.id, project.name);
    Ok(match lang {
        Lang::En => format!("project {id} is now named \"{name}\""),
        Lang::Es => format!("el proyecto {id} ahora se llama \"{name}\""),
    })
}

/// What `clawg projects rename` prints when the daemon refused the name or
/// does not know the project, with the daemon's own sentence as it came;
/// `None` for any other failure.
pub fn rename_refused(lang: Lang, id: &str, err: &ClientError) -> Option<String> {
    let ClientError::Daemon { status: 400 | 404 | 409, message, details } = err else {
        return None;
    };
    let cause = details.detail.as_deref().unwrap_or(message);
    Some(match lang {
        Lang::En => format!("could not rename {id}: {cause}"),
        Lang::Es => format!("no se pudo renombrar {id}: {cause}"),
    })
}

/// How a project nobody reviewed started.
fn born(lang: Lang, born: &str) -> &str {
    match (born, lang) {
        ("root", Lang::En) => "registered from a folder",
        ("root", Lang::Es) => "registrado desde una carpeta",
        ("adopted", Lang::En) => "adopted from a memory saved under its id",
        ("adopted", Lang::Es) => "adoptado de una memoria guardada con su id",
        ("import", Lang::En) => "created by an import",
        ("import", Lang::Es) => "creado por un import",
        ("migrated", Lang::En) => "from before projects",
        ("migrated", Lang::Es) => "de antes de los proyectos",
        (other, _) => other,
    }
}

/// Why two projects may be one.
fn reason(lang: Lang, suggestion: &ListedSuggestion) -> &str {
    match (suggestion.reason.as_str(), lang) {
        ("root_commit", Lang::En) => "they share their first commit",
        ("root_commit", Lang::Es) => "comparten el primer commit",
        ("remote_changed", Lang::En) => "the remote of one folder changed",
        ("remote_changed", Lang::Es) => "cambió el remoto de una carpeta",
        (other, _) => other,
    }
}

/// What `clawg projects list` prints: each project with how many memories it
/// holds and, when nobody reviewed it, what it started from; then its
/// folders, marking those no longer there, its remotes and the suggestions
/// nobody answered.
pub fn render_list(lang: Lang, projects: &[ListedProject]) -> String {
    if projects.is_empty() {
        return match lang {
            Lang::En => "no projects yet",
            Lang::Es => "todavía no hay proyectos",
        }
        .to_string();
    }
    let mut lines = Vec::new();
    for p in projects {
        let (name, id, n) = (&p.project.name, &p.project.id, p.memories);
        let memories = match (n == 1, lang) {
            (true, Lang::En) => "1 memory".to_string(),
            (false, Lang::En) => format!("{n} memories"),
            (true, Lang::Es) => "1 memoria".to_string(),
            (false, Lang::Es) => format!("{n} memorias"),
        };
        let born = born(lang, &p.project.born);
        lines.push(match (p.project.reviewed, lang) {
            (true, _) => format!("{name} ({id}): {memories}"),
            (false, Lang::En) => format!("{name} ({id}): {memories}, not reviewed: {born}"),
            (false, Lang::Es) => format!("{name} ({id}): {memories}, sin revisar: {born}"),
        });
        if p.folders.is_empty() {
            lines.push(
                match lang {
                    Lang::En => "  no folder",
                    Lang::Es => "  sin carpeta",
                }
                .to_string(),
            );
        }
        for folder in &p.folders {
            let path = &folder.root_path;
            lines.push(match (folder.present, lang) {
                (true, Lang::En) => format!("  folder {path}"),
                (false, Lang::En) => format!("  folder {path}, no longer there"),
                (true, Lang::Es) => format!("  carpeta {path}"),
                (false, Lang::Es) => format!("  carpeta {path}, ya no está"),
            });
        }
        for remote in &p.remotes {
            lines.push(match lang {
                Lang::En => format!("  remote {remote}"),
                Lang::Es => format!("  remoto {remote}"),
            });
        }
        for suggestion in &p.suggestions {
            let other = match &suggestion.other_name {
                Some(name) => format!("{name} ({})", suggestion.other_id),
                None => suggestion.other_id.clone(),
            };
            let why = reason(lang, suggestion);
            lines.push(match lang {
                Lang::En => format!("  may be the same project as {other}: {why}"),
                Lang::Es => format!("  puede ser el mismo proyecto que {other}: {why}"),
            });
        }
    }
    if projects.iter().any(|p| !p.project.reviewed || !p.suggestions.is_empty()) {
        lines.push(
            match lang {
                Lang::En => "`clawg projects rename <ID> <NAME>` names a project and marks it reviewed",
                Lang::Es => "`clawg projects rename <ID> <NOMBRE>` le pone nombre a un proyecto y lo marca revisado",
            }
            .to_string(),
        );
    }
    lines.join("\n")
}

/// Registers the project `path` is in so the daemon may read its files, and
/// says which root and id that was.
pub async fn add(lang: Lang, client: &DaemonClient, path: &Path) -> Result<String, AddError> {
    let project = ProjectIdentity::resolve(path);
    if !project.root.is_dir() {
        return Err(AddError::NotADirectory(project.root));
    }
    let legacy = project.legacy_ids();
    let id = client.register_project(&project.root, &legacy, true).await?.project_id;
    let root = project.root.display();
    Ok(match lang {
        Lang::En => format!("added {root} as project {id}; the daemon is indexing its files in the background"),
        Lang::Es => format!("se agregó {root} como el proyecto {id}; el daemon está indexando sus archivos en segundo plano"),
    })
}

/// Sends the move `opts` asks for and says what it did. `--to PATH` is
/// registered first, without letting the daemon read its files, and must be
/// a directory; `--here` is the project of `cwd`.
pub async fn move_memories(lang: Lang, client: &DaemonClient, opts: &MoveOpts, cwd: &Path) -> Result<String, AddError> {
    let to = match &opts.to {
        Some(path) => {
            let project = ProjectIdentity::resolve(path);
            if !project.root.is_dir() {
                return Err(AddError::NotADirectory(project.root));
            }
            let legacy = project.legacy_ids();
            Some(client.register_project(&project.root, &legacy, false).await?.project_id)
        }
        None => None,
    };
    let here = if opts.here {
        let project = ProjectIdentity::resolve(cwd);
        Some(client.register_project(&project.root, &project.legacy_ids(), false).await?.project_id)
    } else {
        None
    };
    let report = client.move_memories(&opts.request(here, to)).await?;
    Ok(render_move(lang, &report))
}

/// Where a memory moves to, with the preposition Spanish fuses with it.
fn place(lang: Lang, project: &Option<String>) -> String {
    match (project, lang) {
        (Some(p), Lang::En) => format!("to project {p}"),
        (Some(p), Lang::Es) => format!("al proyecto {p}"),
        (None, Lang::En) => "to no project".to_string(),
        (None, Lang::Es) => "a ningún proyecto".to_string(),
    }
}

fn id_list(ids: impl IntoIterator<Item = i64>) -> String {
    ids.into_iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(", ")
}

/// What `clawg projects move` prints for `report`. Why a unit stayed is the
/// daemon's own sentence, shown as it came.
pub fn render_move(lang: Lang, report: &MoveReport) -> String {
    let mut lines = Vec::new();
    if report.moved.is_empty() {
        lines.push(
            match (report.dry_run, lang) {
                (true, Lang::En) => "nothing would move",
                (false, Lang::En) => "nothing moved",
                (true, Lang::Es) => "no se movería nada",
                (false, Lang::Es) => "no se movió nada",
            }
            .to_string(),
        );
    } else {
        let mut by_target: std::collections::BTreeMap<String, Vec<i64>> = std::collections::BTreeMap::new();
        for m in &report.moved {
            by_target.entry(place(lang, &m.to_project)).or_default().push(m.id);
        }
        for (target, ids) in by_target {
            let (n, ids) = (ids.len(), id_list(ids));
            lines.push(match (report.dry_run, n == 1, lang) {
                (true, true, Lang::En) => format!("would move 1 memory {target}: {ids}"),
                (true, false, Lang::En) => format!("would move {n} memories {target}: {ids}"),
                (false, true, Lang::En) => format!("moved 1 memory {target}: {ids}"),
                (false, false, Lang::En) => format!("moved {n} memories {target}: {ids}"),
                (true, true, Lang::Es) => format!("se movería 1 memoria {target}: {ids}"),
                (true, false, Lang::Es) => format!("se moverían {n} memorias {target}: {ids}"),
                (false, true, Lang::Es) => format!("se movió 1 memoria {target}: {ids}"),
                (false, false, Lang::Es) => format!("se movieron {n} memorias {target}: {ids}"),
            });
        }
    }
    if !report.pulled_in.is_empty() {
        let (n, ids) = (report.pulled_in.len(), id_list(report.pulled_in.iter().copied()));
        lines.push(match lang {
            Lang::En => format!("{n} of them were not selected but go with a save or topic history that was: {ids}"),
            Lang::Es => format!("{n} de ellas no se eligieron, pero van con un save o un historial de tema que sí: {ids}"),
        });
    }
    if report.moved.iter().any(|m| m.to_project.is_none()) {
        lines.push(
            match lang {
                Lang::En => "a memory in no project is found from every project",
                Lang::Es => "una memoria sin proyecto se encuentra desde cualquier proyecto",
            }
            .to_string(),
        );
    }
    if report.protected > 0 {
        let n = report.protected;
        lines.push(match lang {
            Lang::En => format!("{n} of them are preferences or decisions"),
            Lang::Es => format!("{n} de ellas son preferencias o decisiones"),
        });
    }
    if !report.unchanged.is_empty() {
        let ids = id_list(report.unchanged.iter().copied());
        lines.push(match lang {
            Lang::En => format!("already there: {ids}"),
            Lang::Es => format!("ya estaban ahí: {ids}"),
        });
    }
    for skipped in &report.skipped {
        lines.push(match lang {
            Lang::En => format!("left where it is: {skipped}"),
            Lang::Es => format!("queda donde está: {skipped}"),
        });
    }
    if !report.split_pairs.is_empty() {
        let pairs: Vec<String> = report.split_pairs.iter().map(|[a, b]| format!("#{a}/#{b}")).collect();
        let pairs = pairs.join(", ");
        lines.push(match lang {
            Lang::En => format!("pairs to judge now across two projects: {pairs}"),
            Lang::Es => format!("pares para juzgar que ahora quedan entre dos proyectos: {pairs}"),
        });
    }
    if let Some(snapshot) = &report.snapshot {
        lines.push(match lang {
            Lang::En => format!("snapshot before the move: {snapshot}; `clawg restore` brings it back"),
            Lang::Es => format!("snapshot de antes del movimiento: {snapshot}; `clawg restore` lo trae de vuelta"),
        });
    }
    if let Some(id) = report.move_id {
        lines.push(match lang {
            Lang::En => format!("this is move {id}; `clawg projects move --undo {id}` moves it back"),
            Lang::Es => format!("este es el movimiento {id}; `clawg projects move --undo {id}` lo deshace"),
        });
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
            render_move(Lang::En, &report),
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
            render_move(Lang::En, &report),
            "would move 1 memory to no project: #1\na memory in no project is found from every project"
        );
        assert_eq!(render_move(Lang::En, &MoveReport { dry_run: true, ..MoveReport::default() }), "nothing would move");
    }

    #[test]
    fn the_move_report_speaks_spanish() {
        let report = MoveReport {
            move_id: Some(3),
            moved: vec![moved(1, Some("repo-1a2b")), moved(2, Some("repo-1a2b")), moved(6, None)],
            pulled_in: vec![2],
            unchanged: vec![5],
            split_pairs: vec![[7, 1]],
            protected: 1,
            snapshot: Some("/data/backups/memory-x-pre-move.db".into()),
            ..MoveReport::default()
        };
        assert_eq!(
            render_move(Lang::Es, &report),
            "se movió 1 memoria a ningún proyecto: #6\n\
             se movieron 2 memorias al proyecto repo-1a2b: #1, #2\n\
             1 de ellas no se eligieron, pero van con un save o un historial de tema que sí: #2\n\
             una memoria sin proyecto se encuentra desde cualquier proyecto\n\
             1 de ellas son preferencias o decisiones\n\
             ya estaban ahí: #5\n\
             pares para juzgar que ahora quedan entre dos proyectos: #7/#1\n\
             snapshot de antes del movimiento: /data/backups/memory-x-pre-move.db; `clawg restore` lo trae de vuelta\n\
             este es el movimiento 3; `clawg projects move --undo 3` lo deshace"
        );
        assert_eq!(render_move(Lang::Es, &MoveReport { dry_run: true, ..MoveReport::default() }), "no se movería nada");
    }

    fn listing() -> Vec<ListedProject> {
        let folder = |path: &str, present: bool| {
            serde_json::json!({ "root_id": "x-0000", "root_path": path, "remote": null, "index_docs": false, "present": present })
        };
        let suggestion = |id: &str, name: &str| {
            serde_json::json!({ "other_id": id, "other_name": name, "reason": "root_commit", "created_at": 1 })
        };
        serde_json::from_value(serde_json::json!([
            {
                "id": "app-1a2b", "name": "App web", "born": "root", "reviewed": true, "created_at": 1,
                "folders": [folder("/work/app", true), folder("/work/app-old", false)],
                "remotes": ["github.com/acme/app"], "memories": 12,
                "suggestions": [suggestion("copia-3c4d", "copia")],
            },
            {
                "id": "copia-3c4d", "name": "copia", "born": "root", "reviewed": false, "created_at": 2,
                "folders": [folder("/work/copia", true)], "remotes": [], "memories": 0,
                "suggestions": [suggestion("app-1a2b", "App web")],
            },
            {
                "id": "loose-1a2b", "name": "loose", "born": "adopted", "reviewed": false, "created_at": 3,
                "folders": [], "remotes": [], "memories": 1, "suggestions": [],
            },
        ]))
        .unwrap()
    }

    #[test]
    fn a_listing_marks_what_was_created_alone_and_the_suggestions() {
        assert_eq!(
            render_list(Lang::En, &listing()),
            "App web (app-1a2b): 12 memories\n\
             \x20 folder /work/app\n\
             \x20 folder /work/app-old, no longer there\n\
             \x20 remote github.com/acme/app\n\
             \x20 may be the same project as copia (copia-3c4d): they share their first commit\n\
             copia (copia-3c4d): 0 memories, not reviewed: registered from a folder\n\
             \x20 folder /work/copia\n\
             \x20 may be the same project as App web (app-1a2b): they share their first commit\n\
             loose (loose-1a2b): 1 memory, not reviewed: adopted from a memory saved under its id\n\
             \x20 no folder\n\
             `clawg projects rename <ID> <NAME>` names a project and marks it reviewed"
        );
        assert_eq!(
            render_list(Lang::Es, &listing()),
            "App web (app-1a2b): 12 memorias\n\
             \x20 carpeta /work/app\n\
             \x20 carpeta /work/app-old, ya no está\n\
             \x20 remoto github.com/acme/app\n\
             \x20 puede ser el mismo proyecto que copia (copia-3c4d): comparten el primer commit\n\
             copia (copia-3c4d): 0 memorias, sin revisar: registrado desde una carpeta\n\
             \x20 carpeta /work/copia\n\
             \x20 puede ser el mismo proyecto que App web (app-1a2b): comparten el primer commit\n\
             loose (loose-1a2b): 1 memoria, sin revisar: adoptado de una memoria guardada con su id\n\
             \x20 sin carpeta\n\
             `clawg projects rename <ID> <NOMBRE>` le pone nombre a un proyecto y lo marca revisado"
        );
        assert_eq!(render_list(Lang::En, &[]), "no projects yet");
        assert_eq!(render_list(Lang::Es, &[]), "todavía no hay proyectos");
    }

    #[tokio::test]
    async fn projects_are_renamed_and_listed_through_the_daemon() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");
        let repo = tempfile::tempdir().expect("tempdir");
        let id = c.register_project(repo.path(), &[], false).await.expect("register").project_id;

        let renamed = rename(Lang::En, &c, &id, " App web ").await.expect("rename");

        assert_eq!(renamed, format!("project {id} is now named \"App web\""));
        let listed = list(Lang::En, &c).await.expect("list");
        assert!(listed.starts_with(&format!("App web ({id}): 0 memories\n")), "{listed}");
        let refused = rename(Lang::Es, &c, "a/b?c", "x").await.expect_err("no such project");
        assert_eq!(
            rename_refused(Lang::Es, "a/b?c", &refused).as_deref(),
            Some("no se pudo renombrar a/b?c: no project has the id a/b?c"),
            "the id travels as one segment: {refused:?}"
        );
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
        let id = c.register_project(&root, &[], false).await.expect("register as a label").project_id;
        let refused = c.search_docs(&id, "How the other project ships", None).await.expect_err("not accepted");
        assert!(matches!(refused, ClientError::Daemon { status: 403, .. }), "{refused:?}");

        let out = add(Lang::En, &c, &root.join("docs")).await.expect("add");

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

        let dry = move_memories(Lang::En, &c, &opts(&["--id", &selected, "--to", &to, "--dry-run"]), repo.path()).await.expect("dry run");
        assert!(dry.starts_with(&format!("would move 1 memory to project {}", ProjectIdentity::resolve(&root).id)), "{dry}");

        let done = move_memories(Lang::En, &c, &opts(&["--id", &selected, "--to", &to]), repo.path()).await.expect("move");
        assert!(done.contains("this is move 1"), "{done}");
        let here = move_memories(Lang::En, &c, &opts(&["--here", "--to-none", "--dry-run"]), &root).await.expect("here");
        assert!(here.contains(&format!("#{id}")), "{here}");

        let undone = move_memories(Lang::En, &c, &opts(&["--undo", "1"]), repo.path()).await.expect("undo");
        assert!(undone.starts_with(&format!("moved 1 memory to no project: #{id}")), "{undone}");
    }

    #[tokio::test]
    async fn adding_a_path_that_is_not_a_directory_is_refused() {
        // Nothing listens here: the path is refused before the daemon is asked.
        let c = DaemonClient::new("http://127.0.0.1:1", "token", std::env::temp_dir());
        let dir = tempfile::tempdir().expect("tempdir");

        let err = add(Lang::En, &c, &dir.path().join("gone")).await.expect_err("missing");

        assert!(matches!(err, AddError::NotADirectory(_)), "{err:?}");
    }
}
