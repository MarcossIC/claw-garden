//! `clawg projects list`: every project, with what leads to it.
//! `clawg projects rename`: name a project and mark it reviewed.
//! `clawg projects add`: let the daemon read a project's files.
//! `clawg projects move`: move memories to another project, or out of every
//! project.
//! `clawg projects merge`: merge one project into another, or undo a merge.
//! `clawg projects keep-apart`: answer that two projects are not one.
//!
//! A bridge only has its own root indexed; any other root the model names is
//! a label for memories. `add` is how the user, not the model, adds one.
//! Everything goes through the running daemon, which owns the store.

use std::path::{Path, PathBuf};

use clawg_core::memory::merge::MergeReport;
use clawg_core::memory::registry::{ListedProject, ListedSuggestion};
use clawg_core::memory::relocate::MoveReport;
use clawg_core::memory::ProjectIdentity;
use clawg_core::preferences::Lang;

use crate::cli::{MergeOpts, MoveOpts};
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
}
