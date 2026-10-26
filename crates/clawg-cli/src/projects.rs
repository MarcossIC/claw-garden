//! `clawg projects list`: every project, with what leads to it.
//! `clawg projects rename`: name a project and mark it reviewed.
//! `clawg projects add`: let the daemon read a project's files.
//! `clawg projects move`: move memories to another project, or out of every
//! project.
//! `clawg projects merge`: merge one project into another, or undo a merge.
//! `clawg projects keep-apart`: answer that two projects are not one.
//! `clawg projects delete`: delete a project, with a report and a code when it
//! takes its memories, or undo a delete.
//!
//! A bridge only has its own root indexed; any other root the model names is
//! a label for memories. `add` is how the user, not the model, adds one.
//! Everything goes through the running daemon, which owns the store.

use std::path::{Path, PathBuf};

use clawg_core::memory::merge::MergeReport;
use clawg_core::memory::project_delete::{Counts, DeleteReport, KeepReason, Refusal};
use clawg_core::memory::registry::{ListedDelete, ListedProject, ListedSuggestion};
use clawg_core::memory::relocate::MoveReport;
use clawg_core::memory::ProjectIdentity;
use clawg_core::preferences::Lang;

use crate::cli::{DeleteOpts, MergeOpts, MoveOpts};
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
    let listing = client.list_projects().await?;
    Ok(render_listing(lang, &listing.projects, &listing.deletes))
}

/// [`render_list`], closed by the deletes nobody undid or purged: how to undo
/// each, and, when it left deleted rows, how to purge it.
pub fn render_listing(lang: Lang, projects: &[ListedProject], deletes: &[ListedDelete]) -> String {
    let mut lines = vec![render_list(lang, projects)];
    if deletes.is_empty() {
        return lines.remove(0);
    }
    lines.push(
        match lang {
            Lang::En => "deleted projects:",
            Lang::Es => "proyectos borrados:",
        }
        .to_string(),
    );
    for d in deletes {
        let (name, id, n) = (&d.name, &d.project_id, d.delete_id);
        let mut line = match lang {
            Lang::En => format!("{name} ({id}), delete {n}: `clawg projects delete --undo {n}` brings it back"),
            Lang::Es => format!("{name} ({id}), borrado {n}: `clawg projects delete --undo {n}` lo trae de vuelta"),
        };
        if d.rows > 0 {
            let rows = d.rows;
            line.push_str(&match lang {
                Lang::En => format!("; `clawg purge --project-delete {n} --yes` removes its {rows} deleted rows for good"),
                Lang::Es => format!("; `clawg purge --project-delete {n} --yes` saca para siempre sus {rows} filas borradas"),
            });
        }
        if d.back {
            line.push_str(match lang {
                Lang::En => "; a memory saved since brought the project back into use",
                Lang::Es => "; una memoria guardada después hizo que el proyecto volviera a usarse",
            });
        }
        lines.push(line);
    }
    lines.join("\n")
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
        for merged in &p.merged {
            let (name, id) = (&merged.name, &merged.id);
            lines.push(match (merged.merge_id, lang) {
                (Some(n), Lang::En) => format!("  {name} ({id}) was merged into it; `clawg projects merge --undo {n}` takes it back"),
                (Some(n), Lang::Es) => {
                    format!("  {name} ({id}) se unió a este proyecto; `clawg projects merge --undo {n}` lo separa de nuevo")
                }
                (None, Lang::En) => format!("  {name} ({id}) was merged into it"),
                (None, Lang::Es) => format!("  {name} ({id}) se unió a este proyecto"),
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
    if projects.iter().any(|p| !p.suggestions.is_empty()) {
        lines.push(
            match lang {
                Lang::En => {
                    "`clawg projects merge <ID> --into <ID>` joins two projects that are one, \
                     and `clawg projects keep-apart <ID> <ID>` keeps them apart"
                }
                Lang::Es => {
                    "`clawg projects merge <ID> --into <ID>` une dos proyectos que son uno, \
                     y `clawg projects keep-apart <ID> <ID>` los mantiene separados"
                }
            }
            .to_string(),
        );
    }
    lines.join("\n")
}

/// Merges, or undoes a merge, as `opts` asks, and says what it did.
pub async fn merge(lang: Lang, client: &DaemonClient, opts: &MergeOpts) -> Result<String, ClientError> {
    let report = client.merge_projects(&opts.request()).await?;
    Ok(render_merge(lang, &report, opts.undo.is_some()))
}

/// The daemon's own sentence for a refusal (400, 404 or 409); `None` for any
/// other failure.
fn refusal(err: &ClientError) -> Option<(&str, &str)> {
    let ClientError::Daemon { status: 400 | 404 | 409, message, details } = err else {
        return None;
    };
    Some((message.as_str(), details.detail.as_deref().unwrap_or(message)))
}

/// What `clawg projects merge` prints when the daemon refused it.
pub fn merge_refused(lang: Lang, err: &ClientError) -> Option<String> {
    let (code, cause) = refusal(err)?;
    Some(match (code, lang) {
        ("saves_pending", Lang::En) => format!("could not merge now: {cause}"),
        ("saves_pending", Lang::Es) => format!("no se pudo unir ahora: {cause}; se puede volver a intentar en un momento"),
        (_, Lang::En) => format!("could not merge: {cause}"),
        (_, Lang::Es) => format!("no se pudo unir: {cause}"),
    })
}

/// Keeps projects `a` and `b` apart, and says so.
pub async fn keep_apart(lang: Lang, client: &DaemonClient, a: &str, b: &str) -> Result<String, ClientError> {
    let kept = client.keep_apart(a, b).await?;
    let (a, b) = (kept.project_id, kept.other_id);
    Ok(match lang {
        Lang::En => format!("{a} and {b} stay apart; no suggestion that they are one project is made again"),
        Lang::Es => format!("{a} y {b} quedan separados; no se vuelve a sugerir que son el mismo proyecto"),
    })
}

/// What `clawg projects keep-apart` prints when the daemon refused it.
pub fn keep_apart_refused(lang: Lang, err: &ClientError) -> Option<String> {
    let (_, cause) = refusal(err)?;
    Some(match lang {
        Lang::En => format!("could not keep them apart: {cause}"),
        Lang::Es => format!("no se pudieron mantener separados: {cause}"),
    })
}

/// Deletes a project, reports what deleting it takes, or undoes a delete, as
/// `opts` asks. Answers what to print and whether it went through: a report
/// that refused (a project with rows, a code that is no longer the set's)
/// prints and fails.
pub async fn delete(lang: Lang, client: &DaemonClient, opts: &DeleteOpts) -> Result<(String, bool), ClientError> {
    let report = client.delete_projects(&opts.request()).await?;
    Ok((render_delete(lang, &report, opts.undo.is_some()), delete_succeeded(&report)))
}

/// Whether the daemon did what was asked, and not only reported.
pub fn delete_succeeded(report: &DeleteReport) -> bool {
    report.refused.is_none()
}

/// What `clawg projects delete` prints when the daemon refused it, with its
/// own sentence as it came; `None` for any other failure.
pub fn delete_refused(lang: Lang, undo: bool, err: &ClientError) -> Option<String> {
    let (code, cause) = refusal(err)?;
    Some(match (code, undo, lang) {
        ("saves_pending", false, Lang::En) => format!("could not delete now: {cause}"),
        ("saves_pending", true, Lang::En) => format!("could not undo now: {cause}"),
        ("saves_pending", false, Lang::Es) => format!("no se pudo borrar ahora: {cause}; se puede volver a intentar en un momento"),
        ("saves_pending", true, Lang::Es) => format!("no se pudo deshacer ahora: {cause}; se puede volver a intentar en un momento"),
        (_, false, Lang::En) => format!("could not delete: {cause}"),
        (_, true, Lang::En) => format!("could not undo: {cause}"),
        (_, false, Lang::Es) => format!("no se pudo borrar: {cause}"),
        (_, true, Lang::Es) => format!("no se pudo deshacer: {cause}"),
    })
}

/// The rows of each table, as a sentence part: `3 memories, 1 trajectory`.
/// With `deleted`, the rows that were already deleted: `2 deleted memories`.
fn row_counts(lang: Lang, c: &Counts, deleted: bool) -> String {
    let part = |n: usize, en: (&str, &str), es: (&str, &str)| match (lang, deleted) {
        (Lang::En, false) => counted(lang, n, en, es),
        (Lang::En, true) => counted(lang, n, (&format!("deleted {}", en.0), &format!("deleted {}", en.1)), es),
        (Lang::Es, false) => counted(lang, n, en, es),
        (Lang::Es, true) => counted(lang, n, en, (&format!("{} borrada", es.0), &format!("{} borradas", es.1))),
    };
    let parts: Vec<String> = [
        (c.memories, part(c.memories, ("memory", "memories"), ("memoria", "memorias"))),
        (c.trajectories, part(c.trajectories, ("trajectory", "trajectories"), ("trayectoria", "trayectorias"))),
        (c.skills, part(c.skills, ("skill", "skills"), ("habilidad", "habilidades"))),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(_, text)| text)
    .collect();
    parts.join(", ")
}

fn total(c: &Counts) -> usize {
    c.memories + c.trajectories + c.skills
}

/// Why a project is offered to empty another into.
fn candidate_reason(lang: Lang, reason: &str) -> String {
    match (reason, lang) {
        ("same_name", Lang::En) => "they have the same name".to_string(),
        ("same_name", Lang::Es) => "tienen el mismo nombre".to_string(),
        ("same_first_commit" | "root_commit", Lang::En) => "they share their first commit".to_string(),
        ("same_first_commit" | "root_commit", Lang::Es) => "comparten el primer commit".to_string(),
        ("remote_changed", Lang::En) => "the remote of one folder changed".to_string(),
        ("remote_changed", Lang::Es) => "cambió el remoto de una carpeta".to_string(),
        (other, _) => other.to_string(),
    }
}

/// Why a row an undo leaves deleted stays so.
fn keep_reason(lang: Lang, reason: KeepReason) -> &'static str {
    match (reason, lang) {
        (KeepReason::LiveTwin, Lang::En) => "a live memory has its text",
        (KeepReason::LiveTwin, Lang::Es) => "una memoria viva tiene su texto",
        (KeepReason::TopicTaken, Lang::En) => "a live memory holds its topic key",
        (KeepReason::TopicTaken, Lang::Es) => "una memoria viva tiene su clave de tema",
        (KeepReason::HoldsSecret, Lang::En) => "its text holds a secret",
        (KeepReason::HoldsSecret, Lang::Es) => "su texto tiene un secreto",
        (KeepReason::Changed, Lang::En) => "it is not as the delete left it",
        (KeepReason::Changed, Lang::Es) => "ya no está como la dejó el borrado",
        (KeepReason::Tied, Lang::En) => "it is a piece of a save with a piece that stays",
        (KeepReason::Tied, Lang::Es) => "es una pieza de un save con una pieza que queda",
    }
}

fn table_word(lang: Lang, table: &str) -> String {
    match (table, lang) {
        ("memory", Lang::En) => "memory".to_string(),
        ("memory", Lang::Es) => "memoria".to_string(),
        ("episodic_trajectory", Lang::En) => "trajectory".to_string(),
        ("episodic_trajectory", Lang::Es) => "trayectoria".to_string(),
        ("procedural_skill", Lang::En) => "skill".to_string(),
        ("procedural_skill", Lang::Es) => "habilidad".to_string(),
        (other, _) => other.to_string(),
    }
}

/// What `clawg projects delete` prints for `report`, of a delete, of the
/// report that comes before one with its memories, or, with `undo`, of an
/// undo.
pub fn render_delete(lang: Lang, report: &DeleteReport, undo: bool) -> String {
    if undo {
        return render_undo(lang, report);
    }
    let (name, id) = (&report.name, &report.project_id);
    match (report.refused, report.delete_id) {
        (Some(Refusal::NotEmpty), _) => {
            let rows = row_counts(lang, &report.live, false);
            let mut lines = vec![match lang {
                Lang::En => format!("{name} ({id}) has live rows: {rows}, so it is not deleted. To empty it:"),
                Lang::Es => format!("{name} ({id}) tiene filas vivas: {rows}, así que no se borra. Para vaciarlo:"),
            }];
            for c in &report.candidates {
                let (other, other_name, why) = (&c.project_id, &c.name, candidate_reason(lang, &c.reason));
                lines.push(match lang {
                    Lang::En => format!(
                        "`clawg projects merge {id} --into {other} --dry-run` joins it into {other_name} ({other}): {why}"
                    ),
                    Lang::Es => format!(
                        "`clawg projects merge {id} --into {other} --dry-run` lo une a {other_name} ({other}): {why}"
                    ),
                });
            }
            lines.push(match lang {
                Lang::En => format!("`clawg projects move --from-project {id} --to-project <ID>` moves its memories to another project"),
                Lang::Es => format!("`clawg projects move --from-project {id} --to-project <ID>` mueve sus memorias a otro proyecto"),
            });
            lines.push(match lang {
                Lang::En => format!("`clawg projects delete {id} --with-memories` shows what deleting it with its memories takes"),
                Lang::Es => format!("`clawg projects delete {id} --with-memories` muestra lo que se lleva borrarlo con sus memorias"),
            });
            lines.join("\n")
        }
        (Some(Refusal::CodeChanged), _) => {
            let said = match lang {
                Lang::En => "the project changed since that report, so nothing was deleted; this is the new report:",
                Lang::Es => "el proyecto cambió desde ese informe y no se borró nada; este es el nuevo informe:",
            };
            format!("{said}\n{}", render_set(lang, report))
        }
        (None, None) => render_set(lang, report),
        (None, Some(n)) if !report.with_memories => {
            let mut lines = vec![match lang {
                Lang::En => format!("deleted project {name} ({id}) as delete {n}; `clawg projects delete --undo {n}` brings it back"),
                Lang::Es => format!("se borró el proyecto {name} ({id}) como el borrado {n}; `clawg projects delete --undo {n}` lo trae de vuelta"),
            }];
            let left = total(&report.deleted);
            if left > 0 {
                let rows = row_counts(lang, &report.deleted, true);
                lines.push(match (left == 1, lang) {
                    (false, Lang::En) => format!("its {rows} stay; `clawg purge --project-delete {n} --yes` removes them for good"),
                    (true, Lang::En) => format!("its {rows} stays; `clawg purge --project-delete {n} --yes` removes it for good"),
                    (false, Lang::Es) => format!("sus {rows} quedan; `clawg purge --project-delete {n} --yes` las saca para siempre"),
                    (true, Lang::Es) => format!("su {rows} queda; `clawg purge --project-delete {n} --yes` la saca para siempre"),
                });
            }
            lines.join("\n")
        }
        (None, Some(n)) => {
            let rows = row_counts(lang, &report.live, false);
            let one = total(&report.live) == 1;
            let mut lines = vec![match (one, lang) {
                (_, Lang::En) => format!("deleted project {name} ({id}) with its {rows} as delete {n}"),
                (false, Lang::Es) => format!("se borró el proyecto {name} ({id}) con sus {rows} como el borrado {n}"),
                (true, Lang::Es) => format!("se borró el proyecto {name} ({id}) con su {rows} como el borrado {n}"),
            }];
            if let Some(snapshot) = &report.snapshot {
                lines.push(match lang {
                    Lang::En => format!("snapshot before the delete: {snapshot}; `clawg restore` brings it back"),
                    Lang::Es => format!("snapshot de antes de borrar: {snapshot}; `clawg restore` lo trae de vuelta"),
                });
            }
            lines.push(match (one, lang) {
                (false, Lang::En) => format!(
                    "`clawg projects delete --undo {n}` brings them back; `clawg purge --project-delete {n} --yes` removes them for good"
                ),
                (true, Lang::En) => format!(
                    "`clawg projects delete --undo {n}` brings it back; `clawg purge --project-delete {n} --yes` removes it for good"
                ),
                (false, Lang::Es) => format!(
                    "`clawg projects delete --undo {n}` las trae de vuelta; `clawg purge --project-delete {n} --yes` las saca para siempre"
                ),
                (true, Lang::Es) => format!(
                    "`clawg projects delete --undo {n}` la trae de vuelta; `clawg purge --project-delete {n} --yes` la saca para siempre"
                ),
            });
            lines.join("\n")
        }
    }
}

/// The set a delete with its memories would take, and the command that
/// confirms it.
fn render_set(lang: Lang, report: &DeleteReport) -> String {
    let (name, id) = (&report.name, &report.project_id);
    let mut lines = vec![match lang {
        Lang::En => format!("deleting {name} ({id}) with its memories takes: {}", row_counts(lang, &report.live, false)),
        Lang::Es => format!("borrar {name} ({id}) con sus memorias se lleva: {}", row_counts(lang, &report.live, false)),
    }];
    if total(&report.deleted) > 0 {
        let rows = row_counts(lang, &report.deleted, false);
        lines.push(match lang {
            Lang::En => format!("already deleted: {rows}"),
            Lang::Es => format!("ya borradas: {rows}"),
        });
    }
    let tally = |pairs: Vec<(String, usize)>| pairs.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", ");
    if !report.by_type.is_empty() {
        let list = tally(report.by_type.iter().map(|(k, n)| (k.clone(), *n)).collect());
        lines.push(match lang {
            Lang::En => format!("by type: {list}"),
            Lang::Es => format!("por tipo: {list}"),
        });
    }
    if !report.by_author.is_empty() {
        let none = match lang {
            Lang::En => "(none)",
            Lang::Es => "(ninguno)",
        };
        let list = tally(report.by_author.iter().map(|a| (a.agent_id.clone().unwrap_or_else(|| none.to_string()), a.rows)).collect());
        lines.push(match lang {
            Lang::En => format!("by author: {list}"),
            Lang::Es => format!("por autor: {list}"),
        });
    }
    if !report.by_status.is_empty() {
        let list = tally(report.by_status.iter().map(|(k, n)| (k.clone(), *n)).collect());
        lines.push(match lang {
            Lang::En => format!("by status: {list}"),
            Lang::Es => format!("por estado: {list}"),
        });
    }
    if !report.protected.is_empty() {
        let ids = id_list(report.protected.iter().copied());
        lines.push(match lang {
            Lang::En => format!("preferences and decisions among them: {ids}"),
            Lang::Es => format!("preferencias y decisiones entre ellas: {ids}"),
        });
    }
    if report.split_pairs > 0 {
        let n = report.split_pairs;
        lines.push(match lang {
            Lang::En => format!("pairs with a memory of another project, which stay as they are: {n}"),
            Lang::Es => format!("pares con una memoria de otro proyecto, que quedan como están: {n}"),
        });
    }
    if !report.conclusions.is_empty() {
        let ids = id_list(report.conclusions.iter().copied());
        lines.push(match lang {
            Lang::En => format!("conclusions of other projects that lose a source and go to review: {ids}"),
            Lang::Es => format!("conclusiones de otros proyectos que pierden una fuente y pasan a revisión: {ids}"),
        });
    }
    if !report.workspaces.is_empty() {
        let list = report.workspaces.join(", ");
        lines.push(match lang {
            Lang::En => format!("workspace.md files, which are not touched: {list}"),
            Lang::Es => format!("archivos workspace.md, que no se tocan: {list}"),
        });
    }
    if !report.folders.is_empty() {
        let list = report.folders.join(", ");
        lines.push(match lang {
            Lang::En => format!("docs index removed from the folders: {list}"),
            Lang::Es => format!("índice de docs que se quita de las carpetas: {list}"),
        });
    }
    if !report.remotes.is_empty() {
        let list = report.remotes.join(", ");
        lines.push(match lang {
            Lang::En => format!("remotes, which stay linked: {list}"),
            Lang::Es => format!("remotos, que siguen vinculados: {list}"),
        });
    }
    if report.pending > 0 {
        let n = report.pending;
        lines.push(match (n == 1, lang) {
            (true, Lang::En) => "1 save of the project is waiting to be stored; confirming waits for it".to_string(),
            (false, Lang::En) => format!("{n} saves of the project are waiting to be stored; confirming waits for them"),
            (true, Lang::Es) => "1 save del proyecto espera para guardarse; la confirmación lo espera".to_string(),
            (false, Lang::Es) => format!("{n} saves del proyecto esperan para guardarse; la confirmación los espera"),
        });
    }
    for f in &report.failed {
        let (op, agent) = (&f.operation_id, &f.agent_id);
        lines.push(match lang {
            Lang::En => format!(
                "a save that failed would bring the project back if it were retried: {op} (agent {agent}); \
                 `clawg outbox discard {op} --agent {agent}` drops it"
            ),
            Lang::Es => format!(
                "un save que falló haría volver al proyecto si se reintentara: {op} (agente {agent}); \
                 `clawg outbox discard {op} --agent {agent}` lo descarta"
            ),
        });
    }
    for c in &report.candidates {
        let (other, why) = (&c.project_id, candidate_reason(lang, &c.reason));
        let other_name = &c.name;
        lines.push(match lang {
            Lang::En => format!(
                "it could be emptied into another project instead: `clawg projects merge {id} --into {other} --dry-run` ({other_name}: {why})"
            ),
            Lang::Es => format!(
                "se podría vaciar en otro proyecto en cambio: `clawg projects merge {id} --into {other} --dry-run` ({other_name}: {why})"
            ),
        });
    }
    if let Some(code) = &report.code {
        lines.push(match lang {
            Lang::En => format!("to delete it with its memories: `clawg projects delete {id} --with-memories --confirm {code}`"),
            Lang::Es => format!("para borrarlo con sus memorias: `clawg projects delete {id} --with-memories --confirm {code}`"),
        });
    }
    lines.join("\n")
}

/// What an undo, or its dry run, did: the rows that came back and each one
/// that stays deleted, with why.
fn render_undo(lang: Lang, report: &DeleteReport) -> String {
    let (name, id, dry) = (&report.name, &report.project_id, report.dry_run);
    let n = report.delete_id.unwrap_or_default();
    let back = total(&report.restored);
    let rows = row_counts(lang, &report.restored, false);
    let mut lines = vec![if back == 0 {
        match (dry, lang) {
            (true, Lang::En) => format!("would undo delete {n}: {name} ({id}) would be live again, with no rows to restore"),
            (false, Lang::En) => format!("undid delete {n}: {name} ({id}) is live again, with no rows to restore"),
            (true, Lang::Es) => format!("se desharía el borrado {n}: {name} ({id}) volvería a estar vivo, sin filas que restaurar"),
            (false, Lang::Es) => format!("se deshizo el borrado {n}: {name} ({id}) vuelve a estar vivo, sin filas que restaurar"),
        }
    } else {
        let one = back == 1;
        match (dry, one, lang) {
            (true, _, Lang::En) => format!("would undo delete {n}: {rows} would be back in {name} ({id})"),
            (false, true, Lang::En) => format!("undid delete {n}: {rows} is back in {name} ({id})"),
            (false, false, Lang::En) => format!("undid delete {n}: {rows} are back in {name} ({id})"),
            (true, true, Lang::Es) => format!("se desharía el borrado {n}: {rows} volvería a {name} ({id})"),
            (true, false, Lang::Es) => format!("se desharía el borrado {n}: {rows} volverían a {name} ({id})"),
            (false, true, Lang::Es) => format!("se deshizo el borrado {n}: {rows} vuelve a {name} ({id})"),
            (false, false, Lang::Es) => format!("se deshizo el borrado {n}: {rows} vuelven a {name} ({id})"),
        }
    }];
    for k in &report.kept {
        let (table, row, why) = (table_word(lang, &k.table), k.id, keep_reason(lang, k.reason));
        lines.push(match lang {
            Lang::En => format!("stays deleted: {table} #{row}: {why}"),
            Lang::Es => format!("queda borrada: {table} #{row}: {why}"),
        });
    }
    if !report.folders.is_empty() {
        let list = report.folders.join(", ");
        lines.push(match (dry, lang) {
            (false, Lang::En) => format!("docs index turned on again for: {list}; the next refresh rebuilds it"),
            (false, Lang::Es) => format!("índice de docs que se vuelve a activar en: {list}; el próximo refresco lo reconstruye"),
            (true, Lang::En) => format!("docs index would be turned on again for: {list}"),
            (true, Lang::Es) => format!("el índice de docs se volvería a activar en: {list}"),
        });
    }
    if let Some(snapshot) = &report.snapshot {
        lines.push(match lang {
            Lang::En => format!("snapshot before the undo: {snapshot}; `clawg restore` brings it back"),
            Lang::Es => format!("snapshot de antes de deshacer: {snapshot}; `clawg restore` lo trae de vuelta"),
        });
    }
    lines.join("\n")
}

/// `n` of a thing, in the singular or the plural each language gives it.
fn counted(lang: Lang, n: usize, en: (&str, &str), es: (&str, &str)) -> String {
    match (n == 1, lang) {
        (true, Lang::En) => format!("1 {}", en.0),
        (false, Lang::En) => format!("{n} {}", en.1),
        (true, Lang::Es) => format!("1 {}", es.0),
        (false, Lang::Es) => format!("{n} {}", es.1),
    }
}

/// What `clawg projects merge` prints for `report`, of a merge or, with
/// `undo`, of its undo.
pub fn render_merge(lang: Lang, report: &MergeReport, undo: bool) -> String {
    let (s, t, dry) = (&report.source, &report.target, report.dry_run);
    if !undo && !dry && report.merge_id.is_none() {
        return match lang {
            Lang::En => format!("nothing is left to merge from {s} into {t}"),
            Lang::Es => format!("no queda nada de {s} para unir en {t}"),
        };
    }
    let mut lines = Vec::new();
    let memories = counted(lang, report.memories, ("memory", "memories"), ("memoria", "memorias"));
    let (n, one) = (report.merge_id.unwrap_or_default(), report.memories == 1);
    lines.push(match (undo, dry, lang) {
        (false, true, Lang::En) => format!("would merge {s} into {t}: {memories}"),
        (false, false, Lang::En) => format!("merged {s} into {t}: {memories}"),
        (false, true, Lang::Es) => format!("se uniría {s} en {t}: {memories}"),
        (false, false, Lang::Es) => format!("se unió {s} en {t}: {memories}"),
        (true, true, Lang::En) => format!("would undo merge {n}: {memories} back in {s}"),
        (true, false, Lang::En) => format!("undid merge {n}: {memories} back in {s}"),
        (true, true, Lang::Es) => format!("se desharía la unión {n}: {memories} {} a {s}", if one { "volvería" } else { "volverían" }),
        (true, false, Lang::Es) => format!("se deshizo la unión {n}: {memories} {} a {s}", if one { "vuelve" } else { "vuelven" }),
    });
    if !report.superseded.is_empty() {
        let pairs: Vec<String> = report
            .superseded
            .iter()
            .map(|r| match lang {
                Lang::En => format!("#{} by #{}", r.row, r.by),
                Lang::Es => format!("#{} por #{}", r.row, r.by),
            })
            .collect();
        let pairs = pairs.join(", ");
        lines.push(match lang {
            Lang::En => format!("replaced by a memory of the same topic key or text in {t}: {pairs}"),
            Lang::Es => format!("reemplazadas por una memoria de la misma clave o el mismo texto en {t}: {pairs}"),
        });
    }
    if !report.split.is_empty() {
        let ids = id_list(report.split.iter().copied());
        lines.push(match lang {
            Lang::En => format!("saved partly in another project, which keeps its part: {ids}"),
            Lang::Es => format!("guardadas en parte en otro proyecto, que conserva su parte: {ids}"),
        });
    }
    if !report.kept.is_empty() {
        let ids = id_list(report.kept.iter().copied());
        lines.push(match lang {
            Lang::En => format!("moved elsewhere since, so left where they are: {ids}"),
            Lang::Es => format!("se movieron a otro lado después y quedan donde están: {ids}"),
        });
    }
    if report.protected > 0 {
        let n = report.protected;
        lines.push(match lang {
            Lang::En => format!("{n} of them are preferences or decisions"),
            Lang::Es => format!("{n} de ellas son preferencias o decisiones"),
        });
    }
    let also: Vec<String> = [
        (report.trajectories, ("trajectory", "trajectories"), ("trayectoria", "trayectorias")),
        (report.skills, ("skill", "skills"), ("habilidad", "habilidades")),
        (report.reviews, ("synthesize review", "synthesize reviews"), ("revisión de synthesize", "revisiones de synthesize")),
    ]
    .into_iter()
    .filter(|(n, _, _)| *n > 0)
    .map(|(n, en, es)| counted(lang, n, en, es))
    .collect();
    if !also.is_empty() {
        let also = also.join(", ");
        lines.push(match lang {
            Lang::En => format!("also: {also}"),
            Lang::Es => format!("además: {also}"),
        });
    }
    if !report.folders.is_empty() {
        let list = report.folders.join(", ");
        lines.push(match (undo, lang) {
            (false, Lang::En) => format!("folders that lead to {t}: {list}"),
            (false, Lang::Es) => format!("carpetas que llevan a {t}: {list}"),
            (true, Lang::En) => format!("folders that lead to {s} again: {list}"),
            (true, Lang::Es) => format!("carpetas que vuelven a llevar a {s}: {list}"),
        });
    }
    if !report.remotes.is_empty() {
        let list = report.remotes.join(", ");
        lines.push(match (undo, lang) {
            (false, Lang::En) => format!("remotes of {t}: {list}"),
            (false, Lang::Es) => format!("remotos de {t}: {list}"),
            (true, Lang::En) => format!("remotes of {s} again: {list}"),
            (true, Lang::Es) => format!("remotos que vuelven a ser de {s}: {list}"),
        });
    }
    if !report.tombstones.is_empty() {
        let list = report.tombstones.join(", ");
        lines.push(match (undo, lang) {
            (false, Lang::En) => format!("projects merged into {s} before, which lead to {t} now: {list}"),
            (false, Lang::Es) => format!("proyectos que se habían unido en {s} y ahora llevan a {t}: {list}"),
            (true, Lang::En) => format!("projects merged into {s} before, which lead to it again: {list}"),
            (true, Lang::Es) => format!("proyectos que se habían unido en {s} y vuelven a llevar a él: {list}"),
        });
    }
    if report.suggestions > 0 {
        let n = report.suggestions;
        lines.push(match (undo, lang) {
            (false, Lang::En) => format!("suggestions this answers: {n}"),
            (false, Lang::Es) => format!("sugerencias que quedan respondidas: {n}"),
            (true, Lang::En) => format!("suggestions pending again: {n}"),
            (true, Lang::Es) => format!("sugerencias que vuelven a quedar pendientes: {n}"),
        });
    }
    let w = &report.workspaces;
    for path in &w.copied {
        lines.push(match (dry, lang) {
            (true, Lang::En) => format!("workspace.md would be copied to {path}"),
            (false, Lang::En) => format!("workspace.md copied to {path}"),
            (true, Lang::Es) => format!("se copiaría workspace.md a {path}"),
            (false, Lang::Es) => format!("workspace.md copiado a {path}"),
        });
    }
    for [from, to] in &w.kept_beside {
        lines.push(match lang {
            Lang::En => format!("workspace.md left beside the one the target had: {from} and {to}; merge them by hand if both matter"),
            Lang::Es => {
                format!("workspace.md queda junto al que ya tenía el destino: {from} y {to}; si los dos importan, hay que unirlos a mano")
            }
        });
    }
    for path in &w.removed {
        lines.push(match lang {
            Lang::En => format!("workspace.md copy taken back: {path}"),
            Lang::Es => format!("copia de workspace.md que se quita: {path}"),
        });
    }
    for path in &w.kept {
        lines.push(match lang {
            Lang::En => format!("workspace.md copy kept, the user edited it since: {path}"),
            Lang::Es => format!("copia de workspace.md que queda, porque se editó después: {path}"),
        });
    }
    for cause in &w.failed {
        lines.push(match (undo, lang) {
            (false, Lang::En) => format!("workspace.md files not copied: {cause}"),
            (false, Lang::Es) => format!("no se copiaron los workspace.md: {cause}"),
            (true, Lang::En) => format!("workspace.md copies not taken back: {cause}"),
            (true, Lang::Es) => format!("no se quitaron las copias de workspace.md: {cause}"),
        });
    }
    if let Some(snapshot) = &report.snapshot {
        lines.push(match (undo, lang) {
            (false, Lang::En) => format!("snapshot before the merge: {snapshot}; `clawg restore` brings it back"),
            (false, Lang::Es) => format!("snapshot de antes de la unión: {snapshot}; `clawg restore` lo trae de vuelta"),
            (true, Lang::En) => format!("snapshot before the undo: {snapshot}; `clawg restore` brings it back"),
            (true, Lang::Es) => format!("snapshot de antes de deshacer: {snapshot}; `clawg restore` lo trae de vuelta"),
        });
    }
    if let (false, false, Some(id)) = (undo, dry, report.merge_id) {
        lines.push(match lang {
            Lang::En => format!("this is merge {id}; `clawg projects merge --undo {id}` takes it back"),
            Lang::Es => format!("esta es la unión {id}; `clawg projects merge --undo {id}` la deshace"),
        });
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

/// What `clawg projects move` prints when the daemon refused it with a cause
/// of its own beside the code (`project_merged`, `part_of_merge`); `None`
/// otherwise, the refusal's message being that cause already.
pub fn move_refused(lang: Lang, err: &ClientError) -> Option<String> {
    let ClientError::Daemon { status: 409, details, .. } = err else {
        return None;
    };
    let cause = details.detail.as_deref()?;
    Some(match lang {
        Lang::En => format!("could not move: {cause}"),
        Lang::Es => format!("no se pudo mover: {cause}"),
    })
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
             `clawg projects rename <ID> <NAME>` names a project and marks it reviewed\n\
             `clawg projects merge <ID> --into <ID>` joins two projects that are one, \
             and `clawg projects keep-apart <ID> <ID>` keeps them apart"
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
             `clawg projects rename <ID> <NOMBRE>` le pone nombre a un proyecto y lo marca revisado\n\
             `clawg projects merge <ID> --into <ID>` une dos proyectos que son uno, \
             y `clawg projects keep-apart <ID> <ID>` los mantiene separados"
        );
        assert_eq!(render_list(Lang::En, &[]), "no projects yet");
        assert_eq!(render_list(Lang::Es, &[]), "todavía no hay proyectos");
    }

    #[test]
    fn a_listing_names_what_was_merged_into_a_project() {
        let listing: Vec<ListedProject> = serde_json::from_value(serde_json::json!([{
            "id": "web-3c4d", "name": "web", "born": "root", "reviewed": true, "created_at": 1,
            "folders": [{ "root_id": "web-3c4d", "root_path": "/work/web", "remote": null, "index_docs": false, "present": true }],
            "remotes": [], "memories": 12, "suggestions": [],
            "merged": [{ "id": "app-1a2b", "name": "app", "merge_id": 4, "merged_at": 9 }],
        }]))
        .unwrap();

        assert_eq!(
            render_list(Lang::En, &listing),
            "web (web-3c4d): 12 memories\n\
             \x20 folder /work/web\n\
             \x20 app (app-1a2b) was merged into it; `clawg projects merge --undo 4` takes it back"
        );
        assert_eq!(
            render_list(Lang::Es, &listing),
            "web (web-3c4d): 12 memorias\n\
             \x20 carpeta /work/web\n\
             \x20 app (app-1a2b) se unió a este proyecto; `clawg projects merge --undo 4` lo separa de nuevo"
        );
    }

    fn merge_report(report: serde_json::Value) -> MergeReport {
        let mut whole = serde_json::json!({
            "dry_run": false, "merge_id": null, "source": "app-1a2b", "target": "web-3c4d", "move_id": null,
            "memories": 0, "superseded": [], "split": [], "protected": 0, "trajectories": 0, "skills": 0,
            "reviews": 0, "folders": [], "remotes": [], "tombstones": [], "suggestions": 0,
            "workspaces": { "copied": [], "kept_beside": [], "failed": [] }, "snapshot": null,
        });
        whole.as_object_mut().unwrap().extend(report.as_object().unwrap().clone());
        serde_json::from_value(whole).unwrap()
    }

    #[test]
    fn a_merge_report_names_what_moved_and_the_way_back() {
        let merged = merge_report(serde_json::json!({
            "merge_id": 4, "move_id": 9, "memories": 12, "superseded": [{ "row": 7, "by": 40 }], "split": [15],
            "protected": 2, "trajectories": 1, "reviews": 1, "folders": ["app-1a2b"],
            "remotes": ["github.com/acme/app"], "tombstones": ["old-9f9f"], "suggestions": 1,
            "workspaces": {
                "copied": ["/w/a/web-3c4d/workspace.md"],
                "kept_beside": [["/w/b/app-1a2b/workspace.md", "/w/b/web-3c4d/workspace.md"]],
                "failed": [],
            },
            "snapshot": "/b/memory-x-pre-merge.db",
        }));
        assert_eq!(
            render_merge(Lang::En, &merged, false),
            "merged app-1a2b into web-3c4d: 12 memories\n\
             replaced by a memory of the same topic key or text in web-3c4d: #7 by #40\n\
             saved partly in another project, which keeps its part: #15\n\
             2 of them are preferences or decisions\n\
             also: 1 trajectory, 1 synthesize review\n\
             folders that lead to web-3c4d: app-1a2b\n\
             remotes of web-3c4d: github.com/acme/app\n\
             projects merged into app-1a2b before, which lead to web-3c4d now: old-9f9f\n\
             suggestions this answers: 1\n\
             workspace.md copied to /w/a/web-3c4d/workspace.md\n\
             workspace.md left beside the one the target had: /w/b/app-1a2b/workspace.md and /w/b/web-3c4d/workspace.md; \
             merge them by hand if both matter\n\
             snapshot before the merge: /b/memory-x-pre-merge.db; `clawg restore` brings it back\n\
             this is merge 4; `clawg projects merge --undo 4` takes it back"
        );
        assert_eq!(
            render_merge(Lang::Es, &merged, false),
            "se unió app-1a2b en web-3c4d: 12 memorias\n\
             reemplazadas por una memoria de la misma clave o el mismo texto en web-3c4d: #7 por #40\n\
             guardadas en parte en otro proyecto, que conserva su parte: #15\n\
             2 de ellas son preferencias o decisiones\n\
             además: 1 trayectoria, 1 revisión de synthesize\n\
             carpetas que llevan a web-3c4d: app-1a2b\n\
             remotos de web-3c4d: github.com/acme/app\n\
             proyectos que se habían unido en app-1a2b y ahora llevan a web-3c4d: old-9f9f\n\
             sugerencias que quedan respondidas: 1\n\
             workspace.md copiado a /w/a/web-3c4d/workspace.md\n\
             workspace.md queda junto al que ya tenía el destino: /w/b/app-1a2b/workspace.md y /w/b/web-3c4d/workspace.md; \
             si los dos importan, hay que unirlos a mano\n\
             snapshot de antes de la unión: /b/memory-x-pre-merge.db; `clawg restore` lo trae de vuelta\n\
             esta es la unión 4; `clawg projects merge --undo 4` la deshace"
        );

        let dry = merge_report(serde_json::json!({
            "dry_run": true, "memories": 1, "workspaces": { "copied": ["/w/a/web-3c4d/workspace.md"], "kept_beside": [], "failed": [] },
        }));
        assert_eq!(
            render_merge(Lang::En, &dry, false),
            "would merge app-1a2b into web-3c4d: 1 memory\nworkspace.md would be copied to /w/a/web-3c4d/workspace.md"
        );
        assert_eq!(
            render_merge(Lang::Es, &dry, false),
            "se uniría app-1a2b en web-3c4d: 1 memoria\nse copiaría workspace.md a /w/a/web-3c4d/workspace.md"
        );
        let nothing = merge_report(serde_json::json!({}));
        assert_eq!(render_merge(Lang::En, &nothing, false), "nothing is left to merge from app-1a2b into web-3c4d");
        assert_eq!(render_merge(Lang::Es, &nothing, false), "no queda nada de app-1a2b para unir en web-3c4d");

        let undone = merge_report(serde_json::json!({
            "merge_id": 4, "move_id": 11, "memories": 11, "kept": [15], "trajectories": 1, "reviews": 1, "folders": ["app-1a2b"],
            "workspaces": {
                "copied": [], "kept_beside": [], "failed": [],
                "removed": ["/w/a/web-3c4d/workspace.md"], "kept": ["/w/b/web-3c4d/workspace.md"],
            },
            "snapshot": "/b/memory-x-pre-undo.db",
        }));
        assert_eq!(
            render_merge(Lang::En, &undone, true),
            "undid merge 4: 11 memories back in app-1a2b\n\
             moved elsewhere since, so left where they are: #15\n\
             also: 1 trajectory, 1 synthesize review\n\
             folders that lead to app-1a2b again: app-1a2b\n\
             workspace.md copy taken back: /w/a/web-3c4d/workspace.md\n\
             workspace.md copy kept, the user edited it since: /w/b/web-3c4d/workspace.md\n\
             snapshot before the undo: /b/memory-x-pre-undo.db; `clawg restore` brings it back"
        );
        assert_eq!(
            render_merge(Lang::Es, &undone, true),
            "se deshizo la unión 4: 11 memorias vuelven a app-1a2b\n\
             se movieron a otro lado después y quedan donde están: #15\n\
             además: 1 trayectoria, 1 revisión de synthesize\n\
             carpetas que vuelven a llevar a app-1a2b: app-1a2b\n\
             copia de workspace.md que se quita: /w/a/web-3c4d/workspace.md\n\
             copia de workspace.md que queda, porque se editó después: /w/b/web-3c4d/workspace.md\n\
             snapshot de antes de deshacer: /b/memory-x-pre-undo.db; `clawg restore` lo trae de vuelta"
        );
        let dry_undo = merge_report(serde_json::json!({ "dry_run": true, "merge_id": 4, "memories": 1 }));
        assert_eq!(render_merge(Lang::En, &dry_undo, true), "would undo merge 4: 1 memory back in app-1a2b");
        assert_eq!(render_merge(Lang::Es, &dry_undo, true), "se desharía la unión 4: 1 memoria volvería a app-1a2b");
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
    async fn projects_are_merged_and_kept_apart_through_the_daemon() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");
        let (one, two) = (tempfile::tempdir().expect("tempdir"), tempfile::tempdir().expect("tempdir"));
        let a = c.register_project(one.path(), &[], false).await.expect("register").project_id;
        let b = c.register_project(two.path(), &[], false).await.expect("register").project_id;

        let kept = keep_apart(Lang::En, &c, &a, &b).await.expect("keep apart");
        assert_eq!(kept, format!("{a} and {b} stay apart; no suggestion that they are one project is made again"));
        let opts = |source: &str, into: &str| MergeOpts {
            source: Some(source.into()),
            into: Some(into.into()),
            undo: None,
            dry_run: false,
            profile: Default::default(),
        };
        let merged = merge(Lang::En, &c, &opts(&a, &b)).await.expect("merge");
        assert!(merged.starts_with(&format!("merged {a} into {b}: 0 memories\n")), "{merged}");
        assert!(merged.ends_with("`clawg projects merge --undo 1` takes it back"), "{merged}");

        let from_tombstone = ["clawg", "projects", "move", "--from-project", a.as_str(), "--to-none"];
        let moving = match <crate::cli::Cli as clap::Parser>::try_parse_from(from_tombstone).expect("parse").command {
            crate::cli::Command::Projects { action: crate::cli::ProjectsAction::Move(opts) } => opts,
            other => panic!("{other:?}"),
        };
        let AddError::Client(stopped) = move_memories(Lang::En, &c, &moving, one.path()).await.expect_err("from a tombstone") else {
            panic!("a refusal of the daemon");
        };
        assert_eq!(
            move_refused(Lang::En, &stopped),
            Some(format!("could not move: project {a} was merged into {b}: its memories are there now")),
            "the daemon's cause, not only its code"
        );

        let refused = merge(Lang::Es, &c, &opts(&b, &a)).await.expect_err("into a tombstone");
        let said = merge_refused(Lang::Es, &refused).expect("a refusal");
        assert!(said.starts_with("no se pudo unir: project ") && said.contains(&b), "{said}");
        let undo = MergeOpts { source: None, into: None, undo: Some(1), dry_run: false, profile: Default::default() };
        let undone = merge(Lang::En, &c, &undo).await.expect("undo");
        assert!(undone.starts_with(&format!("undid merge 1: 0 memories back in {a}")), "{undone}");
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

    // ── deleting ────────────────────────────────────────────────────────

    fn delete_report(report: serde_json::Value) -> DeleteReport {
        let mut whole = serde_json::json!({ "project_id": "app-1a2b", "name": "App web" });
        whole.as_object_mut().unwrap().extend(report.as_object().unwrap().clone());
        serde_json::from_value(whole).unwrap()
    }

    fn the_report() -> DeleteReport {
        delete_report(serde_json::json!({
            "with_memories": true, "code": "1a2b3c4d",
            "live": { "memories": 3, "trajectories": 1, "skills": 0 },
            "deleted": { "memories": 2, "trajectories": 0, "skills": 0 },
            "by_type": { "semantic_fact": 2, "user_preference": 1 },
            "by_author": [{ "agent_id": null, "rows": 1 }, { "agent_id": "agent-a", "rows": 2 }],
            "by_status": { "active": 3 }, "protected": [2], "split_pairs": 1, "conclusions": [7],
            "workspaces": ["/w/a/app-1a2b/workspace.md"], "folders": ["app-1a2b"], "remotes": ["github.com/acme/app"],
            "pending": 1, "failed": [{ "operation_id": "op-1", "agent_id": "agent-a" }],
            "candidates": [{ "project_id": "web-3c4d", "name": "web", "reason": "same_name" }],
        }))
    }

    #[test]
    fn a_delete_report_names_what_goes_and_the_exact_confirm() {
        assert_eq!(
            render_delete(Lang::En, &the_report(), false),
            "deleting App web (app-1a2b) with its memories takes: 3 memories, 1 trajectory\n\
             already deleted: 2 memories\n\
             by type: semantic_fact 2, user_preference 1\n\
             by author: (none) 1, agent-a 2\n\
             by status: active 3\n\
             preferences and decisions among them: #2\n\
             pairs with a memory of another project, which stay as they are: 1\n\
             conclusions of other projects that lose a source and go to review: #7\n\
             workspace.md files, which are not touched: /w/a/app-1a2b/workspace.md\n\
             docs index removed from the folders: app-1a2b\n\
             remotes, which stay linked: github.com/acme/app\n\
             1 save of the project is waiting to be stored; confirming waits for it\n\
             a save that failed would bring the project back if it were retried: op-1 (agent agent-a); \
             `clawg outbox discard op-1 --agent agent-a` drops it\n\
             it could be emptied into another project instead: \
             `clawg projects merge app-1a2b --into web-3c4d --dry-run` (web: they have the same name)\n\
             to delete it with its memories: `clawg projects delete app-1a2b --with-memories --confirm 1a2b3c4d`"
        );
        let es = render_delete(Lang::Es, &the_report(), false);
        for line in [
            "borrar App web (app-1a2b) con sus memorias se lleva: 3 memorias, 1 trayectoria",
            "ya borradas: 2 memorias",
            "por autor: (ninguno) 1, agent-a 2",
            "preferencias y decisiones entre ellas: #2",
            "1 save del proyecto espera para guardarse; la confirmación lo espera",
            "`clawg outbox discard op-1 --agent agent-a` lo descarta",
            "(web: tienen el mismo nombre)",
        ] {
            assert!(es.contains(line), "{line}\n{es}");
        }
        assert!(
            es.ends_with("para borrarlo con sus memorias: `clawg projects delete app-1a2b --with-memories --confirm 1a2b3c4d`"),
            "{es}"
        );

        // A code that is no longer the set's: the same report, after saying so.
        let stale = delete_report(serde_json::json!({ "with_memories": true, "code": "ffff0000", "refused": "code_changed" }));
        let en = render_delete(Lang::En, &stale, false);
        assert!(en.starts_with("the project changed since that report, so nothing was deleted; this is the new report:\n"), "{en}");
        assert!(en.ends_with("--confirm ffff0000`"), "{en}");
        assert!(render_delete(Lang::Es, &stale, false).starts_with("el proyecto cambió desde ese informe y no se borró nada; este es el nuevo informe:\n"));

        // An empty project, and one deleted with its memories.
        let empty = delete_report(serde_json::json!({
            "delete_id": 4, "deleted": { "memories": 2, "trajectories": 0, "skills": 0 },
        }));
        assert_eq!(
            render_delete(Lang::En, &empty, false),
            "deleted project App web (app-1a2b) as delete 4; `clawg projects delete --undo 4` brings it back\n\
             its 2 deleted memories stay; `clawg purge --project-delete 4 --yes` removes them for good"
        );
        assert_eq!(
            render_delete(Lang::Es, &empty, false),
            "se borró el proyecto App web (app-1a2b) como el borrado 4; `clawg projects delete --undo 4` lo trae de vuelta\n\
             sus 2 memorias borradas quedan; `clawg purge --project-delete 4 --yes` las saca para siempre"
        );
        let done = delete_report(serde_json::json!({
            "with_memories": true, "delete_id": 5, "code": "1a2b3c4d", "snapshot": "/b/memory-x-pre-delete.db",
            "live": { "memories": 3, "trajectories": 0, "skills": 0 },
        }));
        assert_eq!(
            render_delete(Lang::En, &done, false),
            "deleted project App web (app-1a2b) with its 3 memories as delete 5\n\
             snapshot before the delete: /b/memory-x-pre-delete.db; `clawg restore` brings it back\n\
             `clawg projects delete --undo 5` brings them back; `clawg purge --project-delete 5 --yes` removes them for good"
        );
        assert_eq!(
            render_delete(Lang::Es, &done, false),
            "se borró el proyecto App web (app-1a2b) con sus 3 memorias como el borrado 5\n\
             snapshot de antes de borrar: /b/memory-x-pre-delete.db; `clawg restore` lo trae de vuelta\n\
             `clawg projects delete --undo 5` las trae de vuelta; `clawg purge --project-delete 5 --yes` las saca para siempre"
        );
        assert!(!delete_succeeded(&stale) && delete_succeeded(&done) && delete_succeeded(&the_report()));

        // An undo, and what stayed.
        let undone = delete_report(serde_json::json!({
            "delete_id": 5, "snapshot": "/b/memory-x-pre-undo.db", "folders": ["app-1a2b"],
            "restored": { "memories": 2, "trajectories": 1, "skills": 0 },
            "kept": [{ "table": "memory", "id": 9, "reason": "live_twin" }, { "table": "memory", "id": 10, "reason": "tied" }],
        }));
        assert_eq!(
            render_delete(Lang::En, &undone, true),
            "undid delete 5: 2 memories, 1 trajectory are back in App web (app-1a2b)\n\
             stays deleted: memory #9: a live memory has its text\n\
             stays deleted: memory #10: it is a piece of a save with a piece that stays\n\
             docs index turned on again for: app-1a2b; the next refresh rebuilds it\n\
             snapshot before the undo: /b/memory-x-pre-undo.db; `clawg restore` brings it back"
        );
        let dry = delete_report(serde_json::json!({ "dry_run": true, "delete_id": 5, "restored": { "memories": 1, "trajectories": 0, "skills": 0 } }));
        assert_eq!(render_delete(Lang::En, &dry, true), "would undo delete 5: 1 memory would be back in App web (app-1a2b)");
        assert_eq!(render_delete(Lang::Es, &dry, true), "se desharía el borrado 5: 1 memoria volvería a App web (app-1a2b)");
        let es = render_delete(Lang::Es, &undone, true);
        assert!(es.starts_with("se deshizo el borrado 5: 2 memorias, 1 trayectoria vuelven a App web (app-1a2b)\n"), "{es}");
        assert!(es.contains("queda borrada: memoria #9: una memoria viva tiene su texto"), "{es}");
    }

    #[test]
    fn a_refused_delete_names_the_ways_to_empty_it() {
        let report = delete_report(serde_json::json!({
            "refused": "not_empty", "live": { "memories": 3, "trajectories": 1, "skills": 0 },
            "candidates": [
                { "project_id": "web-3c4d", "name": "web", "reason": "same_name" },
                { "project_id": "copia-5e6f", "name": "copia", "reason": "root_commit" },
            ],
        }));
        assert_eq!(
            render_delete(Lang::En, &report, false),
            "App web (app-1a2b) has live rows: 3 memories, 1 trajectory, so it is not deleted. To empty it:\n\
             `clawg projects merge app-1a2b --into web-3c4d --dry-run` joins it into web (web-3c4d): they have the same name\n\
             `clawg projects merge app-1a2b --into copia-5e6f --dry-run` joins it into copia (copia-5e6f): they share their first commit\n\
             `clawg projects move --from-project app-1a2b --to-project <ID>` moves its memories to another project\n\
             `clawg projects delete app-1a2b --with-memories` shows what deleting it with its memories takes"
        );
        assert_eq!(
            render_delete(Lang::Es, &report, false),
            "App web (app-1a2b) tiene filas vivas: 3 memorias, 1 trayectoria, así que no se borra. Para vaciarlo:\n\
             `clawg projects merge app-1a2b --into web-3c4d --dry-run` lo une a web (web-3c4d): tienen el mismo nombre\n\
             `clawg projects merge app-1a2b --into copia-5e6f --dry-run` lo une a copia (copia-5e6f): comparten el primer commit\n\
             `clawg projects move --from-project app-1a2b --to-project <ID>` mueve sus memorias a otro proyecto\n\
             `clawg projects delete app-1a2b --with-memories` muestra lo que se lleva borrarlo con sus memorias"
        );
        assert!(!delete_succeeded(&report));

        // The daemon's own cause, as it came.
        let daemon = |status, message: &str, detail: Option<&str>| ClientError::Daemon {
            status,
            message: message.into(),
            details: crate::client::ErrorDetails { detail: detail.map(str::to_string), ..Default::default() },
        };
        let deleted = daemon(409, "project_deleted", Some("project app-1a2b is deleted, in delete 4: undo it with `clawg projects delete --undo 4`"));
        assert_eq!(
            delete_refused(Lang::En, false, &deleted).as_deref(),
            Some("could not delete: project app-1a2b is deleted, in delete 4: undo it with `clawg projects delete --undo 4`")
        );
        assert_eq!(
            delete_refused(Lang::Es, true, &deleted).as_deref(),
            Some("no se pudo deshacer: project app-1a2b is deleted, in delete 4: undo it with `clawg projects delete --undo 4`")
        );
        let pending = daemon(409, "saves_pending", Some("2 saves of this project are still being stored; try again in a moment"));
        assert_eq!(
            delete_refused(Lang::En, false, &pending).as_deref(),
            Some("could not delete now: 2 saves of this project are still being stored; try again in a moment")
        );
        assert_eq!(
            delete_refused(Lang::Es, false, &pending).as_deref(),
            Some("no se pudo borrar ahora: 2 saves of this project are still being stored; try again in a moment; se puede volver a intentar en un momento")
        );
        assert_eq!(delete_refused(Lang::En, false, &daemon(500, "boom", None)), None);
    }

    #[test]
    fn a_listing_ends_with_the_open_deletes() {
        let deletes: Vec<ListedDelete> = serde_json::from_value(serde_json::json!([
            { "delete_id": 4, "project_id": "gone-1a2b", "name": "gone", "with_memories": true, "deleted_at": 5, "rows": 3, "back": false },
            { "delete_id": 6, "project_id": "empty-3c4d", "name": "empty", "with_memories": false, "deleted_at": 7, "rows": 0, "back": false },
            { "delete_id": 8, "project_id": "used-5e6f", "name": "used", "with_memories": true, "deleted_at": 9, "rows": 2, "back": true },
        ]))
        .unwrap();
        let projects = listing();
        let en = render_listing(Lang::En, &projects, &deletes);
        assert!(en.starts_with(&render_list(Lang::En, &projects)), "{en}");
        assert!(
            en.ends_with(
                "deleted projects:\n\
                 gone (gone-1a2b), delete 4: `clawg projects delete --undo 4` brings it back; \
                 `clawg purge --project-delete 4 --yes` removes its 3 deleted rows for good\n\
                 empty (empty-3c4d), delete 6: `clawg projects delete --undo 6` brings it back\n\
                 used (used-5e6f), delete 8: `clawg projects delete --undo 8` brings it back; \
                 `clawg purge --project-delete 8 --yes` removes its 2 deleted rows for good; \
                 a memory saved since brought the project back into use"
            ),
            "{en}"
        );
        let es = render_listing(Lang::Es, &projects, &deletes);
        assert!(
            es.ends_with(
                "proyectos borrados:\n\
                 gone (gone-1a2b), borrado 4: `clawg projects delete --undo 4` lo trae de vuelta; \
                 `clawg purge --project-delete 4 --yes` saca para siempre sus 3 filas borradas\n\
                 empty (empty-3c4d), borrado 6: `clawg projects delete --undo 6` lo trae de vuelta\n\
                 used (used-5e6f), borrado 8: `clawg projects delete --undo 8` lo trae de vuelta; \
                 `clawg purge --project-delete 8 --yes` saca para siempre sus 2 filas borradas; \
                 una memoria guardada después hizo que el proyecto volviera a usarse"
            ),
            "{es}"
        );
        assert_eq!(render_listing(Lang::En, &projects, &[]), render_list(Lang::En, &projects), "no deletes, no closing");
    }

    #[tokio::test]
    async fn a_project_is_deleted_and_undone_through_the_daemon() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");
        let (one, two) = (tempfile::tempdir().expect("tempdir"), tempfile::tempdir().expect("tempdir"));
        let gone = c.register_project(one.path(), &[], false).await.expect("register").project_id;
        let empty = c.register_project(two.path(), &[], false).await.expect("register").project_id;
        c.save(crate::client::SaveArgs {
            content: "a note of the project to delete".into(),
            agent_id: "agent-a".into(),
            project_id: Some(gone.clone()),
            memory_type: Some("semantic_fact".into()),
            importance: None,
            derived_from: Vec::new(),
            topic_key: None,
            user_stated: false,
        })
        .await
        .expect("save");
        for _ in 0..40 {
            if !c.list(crate::client::ListArgs::default()).await.expect("list").is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let opts = |args: &[&str]| -> DeleteOpts {
            let mut argv = vec!["clawg", "projects", "delete"];
            argv.extend_from_slice(args);
            match <crate::cli::Cli as clap::Parser>::try_parse_from(&argv).expect("parse").command {
                crate::cli::Command::Projects { action: crate::cli::ProjectsAction::Delete(opts) } => opts,
                other => panic!("{other:?}"),
            }
        };

        let (text, ok) = delete(Lang::En, &c, &opts(&[&gone])).await.expect("not empty");
        assert!(!ok && text.contains("has live rows: 1 memory"), "{text}");
        let (report, ok) = delete(Lang::En, &c, &opts(&[&gone, "--with-memories"])).await.expect("report");
        assert!(ok, "{report}");
        let confirm = report.lines().last().and_then(|l| l.split("--confirm ").nth(1)).expect("the exact command").trim_end_matches('`');
        let (stale, ok) = delete(Lang::En, &c, &opts(&[&gone, "--with-memories", "--confirm", "00000000"])).await.expect("stale");
        assert!(!ok && stale.starts_with("the project changed since that report"), "{stale}");
        let (done, ok) = delete(Lang::En, &c, &opts(&[&gone, "--with-memories", "--confirm", confirm])).await.expect("delete");
        assert!(ok && done.starts_with(&format!("deleted project ")) && done.contains("with its 1 memory as delete 1"), "{done}");
        let (emptied, ok) = delete(Lang::En, &c, &opts(&[&empty])).await.expect("empty");
        assert!(ok && emptied.contains("as delete 2;"), "{emptied}");

        let listed = list(Lang::En, &c).await.expect("listing");
        assert!(listed.contains("delete 1: `clawg projects delete --undo 1` brings it back"), "{listed}");
        assert!(listed.contains("delete 2: `clawg projects delete --undo 2` brings it back"), "{listed}");

        let refused = delete(Lang::En, &c, &opts(&[&gone])).await.expect_err("already deleted");
        let said = delete_refused(Lang::En, false, &refused).expect("a refusal");
        assert!(said.starts_with("could not delete: project ") && said.contains("--undo 1"), "{said}");
        let (dry, ok) = delete(Lang::En, &c, &opts(&["--undo", "1", "--dry-run"])).await.expect("dry run");
        assert!(ok && dry.starts_with("would undo delete 1: 1 memory would be back in"), "{dry}");
        let (undone, ok) = delete(Lang::En, &c, &opts(&["--undo", "1"])).await.expect("undo");
        assert!(ok && undone.starts_with("undid delete 1: 1 memory is back in"), "{undone}");
        let (again, _) = (delete_refused(Lang::En, true, &delete(Lang::En, &c, &opts(&["--undo", "1"])).await.expect_err("twice")), ());
        assert!(again.is_some_and(|s| s.starts_with("could not undo: ")));
    }
}
