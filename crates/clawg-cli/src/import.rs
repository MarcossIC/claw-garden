//! `clawg import`: a file `clawg export` wrote, or another memory tool's
//! export, into the running daemon's store, which embeds each memory again.

use std::path::{Path, PathBuf};

use clawg_core::memory::ProjectIdentity;
use clawg_core::memory::portability::translate::ForeignReport;
use clawg_core::memory::portability::{ImportReport, KeepReason, MapTarget, RegistryUndo, UndoReport, MAX_IMPORT_BYTES};
use clawg_core::preferences::Lang;

use crate::cli::{ImportSource, MapTo, ProjectMap, RootMap};
use crate::client::{ClientError, DaemonClient};

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("could not read {}: {source}", .path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{} is {size} bytes; at most {MAX_IMPORT_BYTES} are imported", .path.display())]
    TooLarge { path: PathBuf, size: u64 },
    #[error("--project-map {name}={}: not a directory", .path.display())]
    NotADirectory { name: String, path: PathBuf },
    #[error(transparent)]
    Client(#[from] ClientError),
}

/// The maps `clawg import` sends: the roots of a clawg export, or the
/// projects of another tool's.
#[derive(Debug, Clone, Copy)]
pub struct Maps<'a> {
    pub roots: &'a [RootMap],
    pub projects: &'a [ProjectMap],
}

/// Sends `file`, an export of `from`, to the daemon and says what it did.
/// Each NEW root and each PATH is resolved here, as a bridge resolves its
/// own, and a PATH that is not a directory stops it before the file is read;
/// each OLD root is taken as written.
pub async fn run(
    lang: Lang,
    client: &DaemonClient,
    file: &Path,
    from: ImportSource,
    dry_run: bool,
    agent: Option<&str>,
    maps: Maps<'_>,
) -> Result<String, ImportError> {
    for m in maps.projects {
        if let MapTo::Path(path) = &m.to {
            if !path.is_dir() {
                return Err(ImportError::NotADirectory { name: m.name.clone(), path: path.clone() });
            }
        }
    }
    let read = |source| ImportError::Read { path: file.to_path_buf(), source };
    let size = std::fs::metadata(file).map_err(read)?.len();
    if size > MAX_IMPORT_BYTES as u64 {
        return Err(ImportError::TooLarge { path: file.to_path_buf(), size });
    }
    let text = std::fs::read_to_string(file).map_err(read)?;
    let (roots, projects) = targets(maps);
    let report = client.import(&text, from.as_str(), dry_run, agent, &roots, &projects).await?;
    Ok(render(lang, &report))
}

/// What the maps send: each path as its root and the path itself, which a
/// declared subfolder leads to its project, as a bridge resolves its own; a
/// project by the reference given, which the daemon resolves.
fn targets(maps: Maps<'_>) -> (Vec<(String, MapTarget)>, Vec<(String, MapTarget)>) {
    let folder = |path: &Path| {
        let identity = ProjectIdentity::resolve(path);
        MapTarget::Folder {
            root: identity.root.to_string_lossy().into_owned(),
            path: Some(identity.path.to_string_lossy().into_owned()),
        }
    };
    let roots = maps.roots.iter().map(|m| (m.old.clone(), folder(&m.new))).collect();
    let projects = maps
        .projects
        .iter()
        .map(|m| {
            let target = match &m.to {
                MapTo::Path(path) => folder(path),
                MapTo::Project(reference) => MapTarget::Project { project: reference.clone() },
            };
            (m.name.clone(), target)
        })
        .collect();
    (roots, projects)
}

fn counts<N: std::fmt::Display>(map: &std::collections::BTreeMap<String, N>, empty: &str) -> String {
    map.iter()
        .map(|(k, n)| format!("{} {n}", if k.is_empty() { empty } else { k }))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The report, one fact per line, in `lang`. Row ids are the file's; `#N` is
/// a row here. Types, states and the reasons the daemon gives stay as they are.
pub fn render(lang: Lang, r: &ImportReport) -> String {
    let (inserted, rows) = (r.inserted, r.rows);
    let mut lines = vec![match (r.dry_run, lang) {
        (true, Lang::En) => format!("would import {inserted} of {rows} memories in the file"),
        (false, Lang::En) => format!("imported {inserted} of {rows} memories in the file"),
        (true, Lang::Es) => format!("se importarían {inserted} de {rows} memorias del archivo"),
        (false, Lang::Es) => format!("se importaron {inserted} de {rows} memorias del archivo"),
    }];
    let (by_type, by_state, by_project, by_lang, none) = match lang {
        Lang::En => ("by type", "by state", "by project", "by language", "(none)"),
        Lang::Es => ("por tipo", "por estado", "por proyecto", "por idioma", "(ninguno)"),
    };
    if r.inserted > 0 {
        lines.push(format!("  {by_type}: {}", counts(&r.by_type, "")));
        lines.push(format!("  {by_state}: {}", counts(&r.by_state, "")));
        lines.push(format!("  {by_project}: {}", counts(&r.by_project, none)));
        if !r.by_lang.is_empty() {
            lines.push(format!("  {by_lang}: {}", counts(&r.by_lang, "")));
        }
    }
    if r.present > 0 {
        lines.push(match lang {
            Lang::En => format!("already here: {}", r.present),
            Lang::Es => format!("ya estaban aquí: {}", r.present),
        });
    }
    if !r.present_elsewhere.is_empty() {
        lines.push(match lang {
            Lang::En => format!("already here, moved to another project: {}", counts(&r.present_elsewhere, none)),
            Lang::Es => format!("ya estaban aquí, movidas a otro proyecto: {}", counts(&r.present_elsewhere, none)),
        });
    }
    render_projects(lang, r, &mut lines);
    let (line, memory) = match lang {
        Lang::En => ("line", "memory"),
        Lang::Es => ("línea", "memoria"),
    };
    if !r.merged.is_empty() {
        lines.push(
            match lang {
                Lang::En => "the same text was already live here, so these were not added again:",
                Lang::Es => "el mismo texto ya estaba vivo aquí, así que estas no se agregaron de nuevo:",
            }
            .into(),
        );
        for m in &r.merged {
            lines.push(match lang {
                Lang::En => format!("  {line} {}: {memory} {} is #{}", m.line, m.id, m.into),
                Lang::Es => format!("  {line} {}: la {memory} {} es #{}", m.line, m.id, m.into),
            });
        }
    }
    if !r.topic_taken.is_empty() {
        lines.push(
            match lang {
                Lang::En => "their topic key is held by a live memory here, so they were stored superseded by it:",
                Lang::Es => "su clave de tema la tiene una memoria viva aquí, así que se guardaron reemplazadas por ella:",
            }
            .into(),
        );
        for t in &r.topic_taken {
            lines.push(match lang {
                Lang::En => format!(
                    "  {line} {}: {memory} {} ({}) is #{}, superseded by #{}",
                    t.line, t.id, t.topic_key, t.stored_as, t.by
                ),
                Lang::Es => format!(
                    "  {line} {}: la {memory} {} ({}) es #{}, reemplazada por #{}",
                    t.line, t.id, t.topic_key, t.stored_as, t.by
                ),
            });
        }
    }
    if r.redacted > 0 {
        lines.push(match lang {
            Lang::En => format!("secrets masked in {}", r.redacted),
            Lang::Es => format!("secretos enmascarados en {}", r.redacted),
        });
    }
    if !r.invalid.is_empty() {
        lines.push(
            match lang {
                Lang::En => "not imported:",
                Lang::Es => "no se importaron:",
            }
            .into(),
        );
        lines.extend(r.invalid.iter().map(|i| format!("  {line} {}: {}", i.line, i.reason)));
    }
    if !r.dropped_links.is_empty() {
        lines.push(
            match lang {
                Lang::En => "links left out:",
                Lang::Es => "vínculos que quedaron afuera:",
            }
            .into(),
        );
        lines.extend(r.dropped_links.iter().map(|i| format!("  {line} {}: {}", i.line, i.reason)));
    }
    if let Some(foreign) = &r.foreign {
        render_foreign(lang, foreign, &mut lines);
    }
    if let Some(snapshot) = &r.snapshot {
        lines.push(match lang {
            Lang::En => format!("the store before the import is in {snapshot}; `clawg restore` brings it back"),
            Lang::Es => format!("el store de antes de la importación está en {snapshot}; `clawg restore` lo trae de vuelta"),
        });
    }
    if let Some(n) = r.import_id {
        lines.push(match lang {
            Lang::En => format!("this is import {n}; `clawg import --undo {n}` takes it back and keeps what changed since"),
            Lang::Es => format!("esta es la importación {n}; `clawg import --undo {n}` la deshace y conserva lo que cambió desde entonces"),
        });
    }
    if !r.dry_run && (r.projects.iter().any(|p| p.created) || !r.suggestions.is_empty()) {
        lines.push(match lang {
            Lang::En => "`clawg projects list` shows the new projects and the suggestions to answer".to_string(),
            Lang::Es => "`clawg projects list` muestra los proyectos nuevos y las sugerencias por responder".to_string(),
        });
    }
    lines.join("\n")
}

/// The identifier serde gives `value`: a step, a reason.
fn identifier<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

/// Where each project of the file went, the suggestions it left and the
/// subfolders it declared. The step that decided and the reasons stay the
/// daemon's identifiers.
fn render_projects(lang: Lang, r: &ImportReport, lines: &mut Vec<String>) {
    let heading = match lang {
        Lang::En => "projects of the file:",
        Lang::Es => "proyectos del archivo:",
    };
    list(
        lines,
        heading,
        r.projects.iter().map(|p| {
            let name = p.name.as_ref().map(|n| format!(" ({n})")).unwrap_or_default();
            let Some(to) = &p.to else {
                return match lang {
                    Lang::En => format!("{}{name}: to none, a merge of the source whose target is not here", p.id),
                    Lang::Es => format!("{}{name}: a ninguno, una unión de la fuente cuyo destino no está aquí", p.id),
                };
            };
            let by = p.by.as_ref().map(identifier).unwrap_or_default();
            let mut line = match lang {
                Lang::En => format!("{}{name} → {to} by {by}", p.id),
                Lang::Es => format!("{}{name} → {to} por {by}", p.id),
            };
            if p.created {
                line.push_str(match lang {
                    Lang::En => ", created",
                    Lang::Es => ", creado",
                });
            }
            if p.revived {
                line.push_str(match lang {
                    Lang::En => ", deleted here and back in use",
                    Lang::Es => ", borrado aquí y de nuevo en uso",
                });
            }
            line
        }),
    );
    let heading = match (r.dry_run, lang) {
        (false, Lang::En) => "suggestions that two projects may be one:",
        (true, Lang::En) => "suggestions it would leave that two projects may be one:",
        (false, Lang::Es) => "sugerencias de que dos proyectos pueden ser uno:",
        (true, Lang::Es) => "sugerencias que dejaría de que dos proyectos pueden ser uno:",
    };
    list(lines, heading, r.suggestions.iter().map(|s| format!("{} ~ {}: {}", s.project_id, s.other_id, s.reason)));
    let heading = match lang {
        Lang::En => "subfolders the file declares:",
        Lang::Es => "subcarpetas que el archivo declara:",
    };
    list(
        lines,
        heading,
        r.subfolders.iter().map(|d| {
            let repository = d.repository_here.as_deref().unwrap_or(&d.repository_id);
            let project = d.project_here.as_deref().unwrap_or(&d.project_id);
            match (&d.reason, lang) {
                (None, _) => format!("{repository} {} → {project}", d.subpath),
                (Some(reason), Lang::En) => format!("{repository} {} → {project}: not declared, {reason}", d.subpath),
                (Some(reason), Lang::Es) => format!("{repository} {} → {project}: no se declaró, {reason}", d.subpath),
            }
        }),
    );
}

/// Asks the daemon to take import `import_id` back and says what it did.
pub async fn undo(lang: Lang, client: &DaemonClient, import_id: i64, dry_run: bool) -> Result<String, ImportError> {
    Ok(render_undo(lang, &client.undo_import(import_id, dry_run).await?))
}

fn ids(ids: &[i64]) -> String {
    ids.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(", ")
}

/// What an undo did, one fact per line, in `lang`. Why a memory stayed is
/// the daemon's identifier.
pub fn render_undo(lang: Lang, r: &UndoReport) -> String {
    let (n, removed) = (r.import_id, r.removed.len());
    let nothing = r.removed.is_empty()
        && r.restored.is_empty()
        && r.kept.is_empty()
        && r.unfilled.is_empty()
        && r.links == 0
        && r.projects.is_empty()
        && r.remotes.is_empty()
        && r.registry == RegistryUndo::default();
    if nothing {
        return match lang {
            Lang::En => format!("nothing of import {n} is left to take back"),
            Lang::Es => format!("no queda nada de la importación {n} para deshacer"),
        };
    }
    let mut lines = vec![match (r.dry_run, lang) {
        (true, Lang::En) => format!("would take back import {n}: remove {removed} memories"),
        (false, Lang::En) => format!("took back import {n}: removed {removed} memories"),
        (true, Lang::Es) => format!("se desharía la importación {n}: se quitarían {removed} memorias"),
        (false, Lang::Es) => format!("se deshizo la importación {n}: se quitaron {removed} memorias"),
    }];
    if !r.restored.is_empty() {
        lines.push(match lang {
            Lang::En => format!("memories it had replaced, live again: {}", ids(&r.restored)),
            Lang::Es => format!("memorias que había reemplazado, vivas de nuevo: {}", ids(&r.restored)),
        });
    }
    let heading = match lang {
        Lang::En => "kept, because they changed or something that stays needs them:",
        Lang::Es => "se quedan, porque cambiaron o algo que queda las necesita:",
    };
    list(&mut lines, heading, r.kept.iter().map(|k| format!("#{}: {}", k.id, reason(k.reason))));
    if !r.unfilled.is_empty() {
        lines.push(match lang {
            Lang::En => format!("links it had filled, emptied again in: {}", ids(&r.unfilled)),
            Lang::Es => format!("vínculos que había completado, vacíos de nuevo en: {}", ids(&r.unfilled)),
        });
    }
    if r.links > 0 {
        lines.push(match lang {
            Lang::En => format!("pairs, derivations and pieces it had added, removed: {}", r.links),
            Lang::Es => format!("pares, derivaciones y piezas que había agregado, quitados: {}", r.links),
        });
    }
    if !r.projects.is_empty() {
        lines.push(match lang {
            Lang::En => format!("roots it had recorded, removed: {}", r.projects.join(", ")),
            Lang::Es => format!("raíces que había registrado, quitadas: {}", r.projects.join(", ")),
        });
    }
    if !r.remotes.is_empty() {
        lines.push(match lang {
            Lang::En => format!("remotes it had recorded, removed: {}", r.remotes.join(", ")),
            Lang::Es => format!("remotos que había registrado, quitados: {}", r.remotes.join(", ")),
        });
    }
    render_registry(lang, &r.registry, &mut lines);
    if let Some(snapshot) = &r.snapshot {
        lines.push(match lang {
            Lang::En => format!("the store before the undo is in {snapshot}; `clawg restore` brings it back"),
            Lang::Es => format!("el store de antes de deshacer está en {snapshot}; `clawg restore` lo trae de vuelta"),
        });
    }
    lines.join("\n")
}

/// The identifier the daemon gives `reason`.
fn reason(reason: KeepReason) -> String {
    identifier(&reason)
}

/// What an undo took back of the registry of projects, and what stayed with
/// the daemon's reason.
fn render_registry(lang: Lang, r: &RegistryUndo, lines: &mut Vec<String>) {
    let pairs = |pairs: &[(String, String)], joint: &str| pairs.iter().map(|(a, b)| format!("{a}{joint}{b}")).collect::<Vec<_>>().join(", ");
    if !r.projects.is_empty() {
        lines.push(match lang {
            Lang::En => format!("projects it had created, deleted: {}", r.projects.join(", ")),
            Lang::Es => format!("proyectos que había creado, borrados: {}", r.projects.join(", ")),
        });
    }
    if !r.deleted_again.is_empty() {
        lines.push(match lang {
            Lang::En => format!("deleted projects it had brought back, deleted again: {}", r.deleted_again.join(", ")),
            Lang::Es => format!("proyectos borrados que había traído de vuelta, borrados de nuevo: {}", r.deleted_again.join(", ")),
        });
    }
    if !r.suggestions.is_empty() {
        lines.push(match lang {
            Lang::En => format!("suggestions it had left, removed: {}", pairs(&r.suggestions, " ~ ")),
            Lang::Es => format!("sugerencias que había dejado, quitadas: {}", pairs(&r.suggestions, " ~ ")),
        });
    }
    if !r.subfolders.is_empty() {
        lines.push(match lang {
            Lang::En => format!("subfolders it had declared, removed: {}", pairs(&r.subfolders, " ")),
            Lang::Es => format!("subcarpetas que había declarado, quitadas: {}", pairs(&r.subfolders, " ")),
        });
    }
    let heading = match lang {
        Lang::En => "kept in the registry, because something uses them or someone answered or reviewed them:",
        Lang::Es => "se quedan en el registro, porque algo las usa o alguien las respondió o revisó:",
    };
    list(
        lines,
        heading,
        r.kept.iter().map(|k| match &k.other {
            Some(other) => format!("{} {} {other}: {}", k.kind, k.id, identifier(&k.reason)),
            None => format!("{} {}: {}", k.kind, k.id, identifier(&k.reason)),
        }),
    );
}

/// `heading` and an indented line per item, unless there is none.
fn list(lines: &mut Vec<String>, heading: &str, items: impl IntoIterator<Item = String>) {
    let items: Vec<String> = items.into_iter().map(|i| format!("  {i}")).collect();
    if !items.is_empty() {
        lines.push(heading.to_owned());
        lines.extend(items);
    }
}

/// What an import of another tool's export did with its notes, each named by
/// its origin, after the lines every import gives.
fn render_foreign(lang: Lang, f: &ForeignReport, lines: &mut Vec<String>) {
    let none = match lang {
        Lang::En => "(none)",
        Lang::Es => "(ninguno)",
    };
    lines.push(match lang {
        Lang::En => format!(
            "from {} {}: notes {}, memories {}, notes in more than one memory {}",
            f.source, f.version, f.notes, f.rows, f.split_notes
        ),
        Lang::Es => format!(
            "de {} {}: notas {}, memorias {}, notas en más de una memoria {}",
            f.source, f.version, f.notes, f.rows, f.split_notes
        ),
    });
    let heading = match lang {
        Lang::En => "notes by type of the source:",
        Lang::Es => "notas por tipo de la fuente:",
    };
    list(
        lines,
        heading,
        f.by_source_type.iter().map(|t| match &t.source_type {
            Some(source_type) => format!("{} {source_type} → {}: {}", t.kind, t.memory_type, t.notes),
            None => format!("{} → {}: {}", t.kind, t.memory_type, t.notes),
        }),
    );
    let heading = match lang {
        Lang::En => "notes by project of the source:",
        Lang::Es => "notas por proyecto de la fuente:",
    };
    list(
        lines,
        heading,
        f.by_source_project.iter().map(|p| {
            let personal = if p.personal { " (personal)" } else { "" };
            let to = match (&p.project_id, &p.root) {
                (Some(id), Some(root)) => format!("{id} ({root})"),
                (Some(id), None) => id.clone(),
                (None, _) => none.to_owned(),
            };
            format!("{}{personal} → {to}: {}", p.project.as_deref().unwrap_or(none), p.notes)
        }),
    );
    if !f.unmapped.is_empty() {
        lines.push(match lang {
            Lang::En => format!("projects of the source no --project-map names, stored in none: {}", f.unmapped.join(", ")),
            Lang::Es => format!(
                "proyectos de la fuente que ningún --project-map nombra, guardados en ninguno: {}",
                f.unmapped.join(", ")
            ),
        });
    }
    if !f.unused_map.is_empty() {
        lines.push(match lang {
            Lang::En => format!("--project-map names no note has: {}", f.unused_map.join(", ")),
            Lang::Es => format!("nombres de --project-map que ninguna nota tiene: {}", f.unused_map.join(", ")),
        });
    }
    if f.masked_notes > 0 {
        lines.push(match lang {
            Lang::En => format!("notes with a secret, masked whole: {}", f.masked_notes),
            Lang::Es => format!("notas con un secreto, enmascaradas enteras: {}", f.masked_notes),
        });
    }
    let heading = match lang {
        Lang::En => "topic keys clawg does not take, rewritten:",
        Lang::Es => "claves de tema que clawg no acepta, reescritas:",
    };
    list(lines, heading, f.rewritten_keys.iter().map(|k| format!("{} → {}", k.from, k.to)));
    let heading = match lang {
        Lang::En => "topic keys two projects of the source share in one project here, qualified:",
        Lang::Es => "claves de tema que dos proyectos de la fuente comparten en un proyecto de aquí, calificadas:",
    };
    list(lines, heading, f.qualified_keys.iter().map(|k| format!("{} → {}", k.from, k.to)));
    let (heading, by) = match lang {
        Lang::En => ("notes stored superseded by a newer note of their key:", "by"),
        Lang::Es => ("notas guardadas reemplazadas por una nota más nueva de su clave:", "por"),
    };
    list(lines, heading, f.superseded_in_file.iter().map(|s| format!("{} {by} {}", s.origin, s.by)));
    let (heading, into) = match lang {
        Lang::En => ("notes of the text of an older note of their project of the source, stored in it:", "in"),
        Lang::Es => ("notas del texto de una nota más vieja de su proyecto de la fuente, guardadas en ella:", "en"),
    };
    list(lines, heading, f.absorbed.iter().map(|a| format!("{} {into} {}", a.origin, a.into)));
    if !f.present.is_empty() {
        lines.push(match lang {
            Lang::En => format!("notes already here, by the project they are in: {}", counts(&f.present, none)),
            Lang::Es => format!("notas que ya estaban aquí, por el proyecto donde están: {}", counts(&f.present, none)),
        });
    }
    let heading = match lang {
        Lang::En => "notes the source rewrote, stored as the newer version of the one here:",
        Lang::Es => "notas que la fuente reescribió, guardadas como la versión más nueva de la de aquí:",
    };
    list(lines, heading, f.revised.iter().cloned());
    let heading = match lang {
        Lang::En => "older versions of notes here, stored superseded by them:",
        Lang::Es => "versiones más viejas de notas de aquí, guardadas reemplazadas por ellas:",
    };
    list(lines, heading, f.older_versions.iter().cloned());
    let heading = match lang {
        Lang::En => "notes the source deleted, left here as they are:",
        Lang::Es => "notas que la fuente borró, dejadas aquí como están:",
    };
    list(lines, heading, f.deleted_at_source.iter().cloned());
    let heading = match lang {
        Lang::En => "notes dated after the import or without a date, stored with the time of the import:",
        Lang::Es => "notas con una fecha posterior a la importación o sin fecha, guardadas con la hora de la importación:",
    };
    list(lines, heading, f.clamped_dates.iter().cloned());
    if f.deleted > 0 {
        lines.push(match lang {
            Lang::En => format!("notes the source had deleted, stored deleted: {}", f.deleted),
            Lang::Es => format!("notas que la fuente tenía borradas, guardadas borradas: {}", f.deleted),
        });
    }
    if !f.relations.is_empty() {
        lines.push(match lang {
            Lang::En => format!("relations stored as pairs: {}", counts(&f.relations, "")),
            Lang::Es => format!("relaciones guardadas como pares: {}", counts(&f.relations, "")),
        });
    }
    if !f.relations_left_out.is_empty() {
        lines.push(match lang {
            Lang::En => format!("relations left out: {}", counts(&f.relations_left_out, "")),
            Lang::Es => format!("relaciones que quedaron afuera: {}", counts(&f.relations_left_out, "")),
        });
    }
    let heading = match lang {
        Lang::En => "notes whose session id is not kept:",
        Lang::Es => "notas cuyo id de sesión no se guarda:",
    };
    list(lines, heading, f.sessions_left_out.iter().cloned());
    let heading = match lang {
        Lang::En => "notes without text, not stored:",
        Lang::Es => "notas sin texto, no se guardaron:",
    };
    list(lines, heading, f.without_text.iter().cloned());
    if !f.unplaced.is_empty() {
        lines.push(match lang {
            Lang::En => format!("in the export with no place here, by field: {}", counts(&f.unplaced, "")),
            Lang::Es => format!("en el export sin lugar aquí, por campo: {}", counts(&f.unplaced, "")),
        });
    }
    if f.rewritten_at_source > 0 {
        lines.push(match lang {
            Lang::En => format!("notes the source rewrote without exporting their earlier versions: {}", f.rewritten_at_source),
            Lang::Es => format!("notas que la fuente reescribió sin exportar sus versiones anteriores: {}", f.rewritten_at_source),
        });
    }
    if f.discarded_sessions > 0 {
        lines.push(match lang {
            Lang::En => format!("sessions without a summary, which give no note: {}", f.discarded_sessions),
            Lang::Es => format!("sesiones sin resumen, que no dan nota: {}", f.discarded_sessions),
        });
    }
    let (heading, piece, is) = match lang {
        Lang::En => ("the same text was already live here, so these pieces were not added again:", "piece", "is"),
        Lang::Es => ("el mismo texto ya estaba vivo aquí, así que estas piezas no se agregaron de nuevo:", "pieza", "es"),
    };
    list(lines, heading, f.merged.iter().map(|m| format!("{} {piece} {} {is} #{}", m.origin, m.piece, m.into)));
    let (heading, superseded) = match lang {
        Lang::En => ("their topic key is held by a live memory here, so they were stored superseded by it:", "superseded by"),
        Lang::Es => ("su clave de tema la tiene una memoria viva aquí, así que se guardaron reemplazadas por ella:", "reemplazada por"),
    };
    list(lines, heading, f.topic_taken.iter().map(|t| format!("{} ({}) {superseded} #{}", t.origin, t.topic_key, t.by)));
    let heading = match lang {
        Lang::En => "not imported:",
        Lang::Es => "no se importaron:",
    };
    list(lines, heading, f.left_out.iter().map(|l| format!("{}: {}", l.origin, l.reason)));
    let (heading, belongs, notes) = match lang {
        Lang::En => ("projects whose id a registration gave another root during the import, not imported:", "belongs to", "notes"),
        Lang::Es => ("proyectos cuyo id un registro le dio a otra raíz durante la importación, no se importaron:", "es de", "notas"),
    };
    list(
        lines,
        heading,
        f.taken_projects.iter().map(|t| format!("{} ({}) {belongs} {}, {notes} {}", t.project, t.root, t.belongs_to, t.notes)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tests::{publish, start_daemon};
    use clawg_core::memory::portability::KeptRow;
    use clawg_core::memory::portability::format::Invalid;

    const FILE: &str = concat!(
        r#"{"kind":"header","format":"clawg-memories","version":1,"exported_at":0,"clawg":"0.0.0"}"#,
        "\n",
        r#"{"kind":"project","id":"repo-0000","root_path":"/far/away/repo"}"#,
        "\n",
        r#"{"kind":"memory","id":4,"scope":"agent","agent_id":"a","project_id":"repo-0000","content":"Deploys go out on Tuesdays","memory_type":"semantic_fact","source":"explicit","created_at":10,"created_by_type":"agent","last_modified_at":10,"last_accessed_at":20,"access_count":3,"importance":5.0,"tier":3,"status":"active"}"#,
        "\n",
        r#"{"kind":"memory","id":5}"#,
        "\n",
    );

    #[tokio::test]
    async fn import_report_lists_counts_and_invalid_lines() {
        let d = start_daemon().await;
        publish(&d);
        let client = DaemonClient::from_data_dir(d.dir.path()).expect("client");
        let file = d.dir.path().join("memories.jsonl");
        std::fs::write(&file, FILE).unwrap();
        let here = tempfile::tempdir().unwrap();
        let maps = [RootMap { old: "/far/away/repo".into(), new: here.path().to_path_buf() }];
        let project = clawg_core::memory::ProjectIdentity::resolve(here.path()).id;
        let maps = Maps { roots: &maps, projects: &[] };
        let send = |dry_run| run(Lang::En, &client, &file, ImportSource::Clawg, dry_run, Some("b"), maps);

        let dry = send(true).await.expect("dry run");
        assert!(dry.starts_with("would import 1 of 1 memories in the file\n"), "{dry}");
        assert!(dry.contains(&format!("  by project: {project} 1")), "{dry}");
        assert!(dry.contains("not imported:\n  line 4: memory:"), "{dry}");
        assert!(!dry.contains("restore"), "{dry}");

        let done = send(false).await.expect("import");
        assert!(done.starts_with("imported 1 of 1 memories in the file\n  by type: semantic_fact 1\n  by state: live 1\n"), "{done}");
        assert!(done.contains("-pre-import.db; `clawg restore` brings it back"), "{done}");
        assert!(done.contains(&format!("projects of the file:\n  repo-0000 → {project} by map, created\n")), "{done}");
        assert!(
            done.ends_with(
                "\nthis is import 1; `clawg import --undo 1` takes it back and keeps what changed since\n\
                 `clawg projects list` shows the new projects and the suggestions to answer"
            ),
            "{done}"
        );

        let again = send(false).await.expect("again");
        assert!(again.starts_with("imported 0 of 1 memories in the file\nalready here: 1\n"), "{again}");
        assert!(!again.contains("--undo"), "an import that wrote nothing has no number: {again}");

        let would = undo(Lang::En, &client, 1, true).await.expect("dry undo");
        assert_eq!(
            would,
            format!("would take back import 1: remove 1 memories\nroots it had recorded, removed: {project}\nprojects it had created, deleted: {project}")
        );
        let undone = undo(Lang::En, &client, 1, false).await.expect("undo");
        assert!(undone.starts_with("took back import 1: removed 1 memories\n"), "{undone}");
        assert!(undone.contains("-pre-undo.db; `clawg restore` brings it back"), "{undone}");
        let nothing = undo(Lang::En, &client, 1, false).await.expect("undo again");
        assert_eq!(nothing, "nothing of import 1 is left to take back");
        let unknown = undo(Lang::En, &client, 9, false).await.unwrap_err();
        assert!(unknown.to_string().contains("no import 9"), "{unknown}");
    }

    fn undo_report() -> UndoReport {
        UndoReport {
            import_id: 3,
            removed: vec![10, 11],
            restored: vec![4],
            kept: vec![KeptRow { id: 12, reason: KeepReason::Edited }, KeptRow { id: 13, reason: KeepReason::Tied }],
            unfilled: vec![2],
            links: 1,
            projects: vec!["repo-1a2b".into()],
            remotes: vec!["example.com/team/repo".into()],
            snapshot: Some("/b/memory-x-pre-undo.db".into()),
            ..Default::default()
        }
    }

    #[test]
    fn render_undo_names_what_it_took_back_and_what_stayed() {
        assert_eq!(
            render_undo(Lang::En, &undo_report()),
            "took back import 3: removed 2 memories\n\
             memories it had replaced, live again: #4\n\
             kept, because they changed or something that stays needs them:\n  #12: edited\n  #13: tied\n\
             links it had filled, emptied again in: #2\n\
             pairs, derivations and pieces it had added, removed: 1\n\
             roots it had recorded, removed: repo-1a2b\n\
             remotes it had recorded, removed: example.com/team/repo\n\
             the store before the undo is in /b/memory-x-pre-undo.db; `clawg restore` brings it back"
        );
        let report = ImportReport { rows: 1, inserted: 1, import_id: Some(3), ..Default::default() };
        assert!(render(Lang::En, &report).ends_with("this is import 3; `clawg import --undo 3` takes it back and keeps what changed since"));
    }

    #[test]
    fn the_undo_report_speaks_spanish() {
        assert_eq!(
            render_undo(Lang::Es, &undo_report()),
            "se deshizo la importación 3: se quitaron 2 memorias\n\
             memorias que había reemplazado, vivas de nuevo: #4\n\
             se quedan, porque cambiaron o algo que queda las necesita:\n  #12: edited\n  #13: tied\n\
             vínculos que había completado, vacíos de nuevo en: #2\n\
             pares, derivaciones y piezas que había agregado, quitados: 1\n\
             raíces que había registrado, quitadas: repo-1a2b\n\
             remotos que había registrado, quitados: example.com/team/repo\n\
             el store de antes de deshacer está en /b/memory-x-pre-undo.db; `clawg restore` lo trae de vuelta"
        );
        let dry = UndoReport { import_id: 3, dry_run: true, removed: vec![10], ..Default::default() };
        assert_eq!(render_undo(Lang::Es, &dry), "se desharía la importación 3: se quitarían 1 memorias");
        assert_eq!(render_undo(Lang::Es, &UndoReport { import_id: 3, ..Default::default() }), "no queda nada de la importación 3 para deshacer");
        let report = ImportReport { rows: 1, inserted: 1, import_id: Some(3), ..Default::default() };
        assert!(render(Lang::Es, &report).ends_with("esta es la importación 3; `clawg import --undo 3` la deshace y conserva lo que cambió desde entonces"));
    }

    #[test]
    fn render_names_merged_taken_and_dropped_rows() {
        let report = ImportReport {
            rows: 3,
            inserted: 1,
            by_type: [("semantic_fact".to_string(), 1)].into(),
            by_state: [("superseded".to_string(), 1)].into(),
            by_project: [(String::new(), 1)].into(),
            merged: vec![clawg_core::memory::portability::import::Merged { line: 3, id: 2, into: 40 }],
            topic_taken: vec![clawg_core::memory::portability::import::TopicTaken {
                line: 4,
                id: 3,
                topic_key: "api/port".into(),
                stored_as: 41,
                by: 7,
            }],
            redacted: 1,
            dropped_links: vec![Invalid { line: 9, reason: "the pair 3-1 names a row that is not in the file".into() }],
            ..Default::default()
        };
        assert_eq!(
            render(Lang::En, &report),
            "imported 1 of 3 memories in the file\n  by type: semantic_fact 1\n  by state: superseded 1\n  by project: (none) 1\n\
             the same text was already live here, so these were not added again:\n  line 3: memory 2 is #40\n\
             their topic key is held by a live memory here, so they were stored superseded by it:\n  line 4: memory 3 (api/port) is #41, superseded by #7\n\
             secrets masked in 1\nlinks left out:\n  line 9: the pair 3-1 names a row that is not in the file"
        );
    }

    #[test]
    fn the_maps_are_sent_with_their_raw_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(dir.path().join("apps/web")).unwrap();
        let root = clawg_core::memory::identity::strip_verbatim(&std::fs::canonicalize(dir.path()).unwrap());
        let roots = [RootMap { old: "/far/mono".into(), new: dir.path().to_path_buf() }];
        let projects = [
            ProjectMap { name: "web".into(), to: MapTo::Path(dir.path().join("apps/web")) },
            ProjectMap { name: "ideas".into(), to: MapTo::Project("Ideas".into()) },
        ];

        let (roots, projects) = targets(Maps { roots: &roots, projects: &projects });

        let text = |p: &Path| p.to_string_lossy().into_owned();
        assert_eq!(
            serde_json::to_value((roots, projects)).unwrap(),
            serde_json::json!([
                [["/far/mono", { "root": text(&root), "path": text(&root) }]],
                [["web", { "root": text(&root), "path": text(&root.join("apps").join("web")) }], ["ideas", { "project": "Ideas" }]],
            ])
        );
    }

    fn placed_report() -> ImportReport {
        use clawg_core::memory::portability::{By, ImportedProject, ImportedSubfolder, ImportedSuggestion};
        let went = |id: &str, name: Option<&str>, to: Option<&str>, by: Option<By>, created, revived| ImportedProject {
            id: id.into(),
            name: name.map(str::to_owned),
            to: to.map(str::to_owned),
            by,
            created,
            revived,
        };
        ImportReport {
            rows: 2,
            inserted: 1,
            present: 0,
            present_elsewhere: [("other-3c4d".to_string(), 1)].into(),
            by_type: [("semantic_fact".to_string(), 1)].into(),
            by_state: [("live".to_string(), 1)].into(),
            by_project: [("ideas-1a2b".to_string(), 1)].into(),
            import_id: Some(4),
            projects: vec![
                went("app-0000", Some("app"), Some("app-1a2b"), Some(By::Remote), false, false),
                went("ideas-1a2b", Some("Ideas"), Some("ideas-1a2b"), Some(By::New), true, false),
                went("gone-5e6f", None, Some("gone-5e6f"), Some(By::Id), false, true),
                went("x-7a8b", Some("x"), None, None, false, false),
            ],
            suggestions: vec![ImportedSuggestion { project_id: "ideas-1a2b".into(), other_id: "ideas-9c0d".into(), reason: "similar_name".into() }],
            subfolders: vec![
                ImportedSubfolder {
                    repository_id: "mono-0000".into(),
                    subpath: "apps/web".into(),
                    project_id: "web-0000".into(),
                    repository_here: Some("mono-1a2b".into()),
                    project_here: Some("web-1a2b".into()),
                    written: true,
                    reason: None,
                },
                ImportedSubfolder {
                    repository_id: "mono-0000".into(),
                    subpath: "apps/api".into(),
                    project_id: "api-0000".into(),
                    repository_here: Some("mono-1a2b".into()),
                    project_here: Some("api-1a2b".into()),
                    written: false,
                    reason: Some("declared_here".into()),
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn an_import_report_names_where_each_project_went() {
        assert_eq!(
            render(Lang::En, &placed_report()),
            "imported 1 of 2 memories in the file\n  by type: semantic_fact 1\n  by state: live 1\n  by project: ideas-1a2b 1\n\
             already here, moved to another project: other-3c4d 1\n\
             projects of the file:\n\
             \x20 app-0000 (app) → app-1a2b by remote\n\
             \x20 ideas-1a2b (Ideas) → ideas-1a2b by new, created\n\
             \x20 gone-5e6f → gone-5e6f by id, deleted here and back in use\n\
             \x20 x-7a8b (x): to none, a merge of the source whose target is not here\n\
             suggestions that two projects may be one:\n  ideas-1a2b ~ ideas-9c0d: similar_name\n\
             subfolders the file declares:\n  mono-1a2b apps/web → web-1a2b\n  mono-1a2b apps/api → api-1a2b: not declared, declared_here\n\
             this is import 4; `clawg import --undo 4` takes it back and keeps what changed since\n\
             `clawg projects list` shows the new projects and the suggestions to answer"
        );
        assert_eq!(
            render(Lang::Es, &placed_report()),
            "se importaron 1 de 2 memorias del archivo\n  por tipo: semantic_fact 1\n  por estado: live 1\n  por proyecto: ideas-1a2b 1\n\
             ya estaban aquí, movidas a otro proyecto: other-3c4d 1\n\
             proyectos del archivo:\n\
             \x20 app-0000 (app) → app-1a2b por remote\n\
             \x20 ideas-1a2b (Ideas) → ideas-1a2b por new, creado\n\
             \x20 gone-5e6f → gone-5e6f por id, borrado aquí y de nuevo en uso\n\
             \x20 x-7a8b (x): a ninguno, una unión de la fuente cuyo destino no está aquí\n\
             sugerencias de que dos proyectos pueden ser uno:\n  ideas-1a2b ~ ideas-9c0d: similar_name\n\
             subcarpetas que el archivo declara:\n  mono-1a2b apps/web → web-1a2b\n  mono-1a2b apps/api → api-1a2b: no se declaró, declared_here\n\
             esta es la importación 4; `clawg import --undo 4` la deshace y conserva lo que cambió desde entonces\n\
             `clawg projects list` muestra los proyectos nuevos y las sugerencias por responder"
        );
        let plain = ImportReport { rows: 1, inserted: 1, import_id: Some(3), ..Default::default() };
        assert!(!render(Lang::En, &plain).contains("projects list"), "nothing new to show");
        let dry = ImportReport { dry_run: true, import_id: None, ..placed_report() };
        for lang in [Lang::En, Lang::Es] {
            assert!(!render(lang, &dry).contains("projects list"), "a dry run creates and leaves nothing to list");
        }
    }

    #[test]
    fn render_undo_names_what_it_took_back_from_the_registry() {
        use clawg_core::memory::portability::{KeptEntry, RegistryKeep};
        let report = UndoReport {
            import_id: 4,
            registry: RegistryUndo {
                projects: vec!["ideas-1a2b".into(), "web-1a2b".into()],
                deleted_again: vec!["gone-5e6f".into()],
                suggestions: vec![("ideas-1a2b".into(), "ideas-9c0d".into())],
                subfolders: vec![("mono-1a2b".into(), "apps/web".into())],
                kept: vec![
                    KeptEntry { kind: "project".into(), id: "mono-1a2b".into(), other: None, reason: RegistryKeep::Reviewed },
                    KeptEntry { kind: "suggestion".into(), id: "a-1111".into(), other: Some("b-2222".into()), reason: RegistryKeep::Answered },
                ],
            },
            ..Default::default()
        };
        assert_eq!(
            render_undo(Lang::En, &report),
            "took back import 4: removed 0 memories\n\
             projects it had created, deleted: ideas-1a2b, web-1a2b\n\
             deleted projects it had brought back, deleted again: gone-5e6f\n\
             suggestions it had left, removed: ideas-1a2b ~ ideas-9c0d\n\
             subfolders it had declared, removed: mono-1a2b apps/web\n\
             kept in the registry, because something uses them or someone answered or reviewed them:\n\
             \x20 project mono-1a2b: reviewed\n  suggestion a-1111 b-2222: answered"
        );
        assert_eq!(
            render_undo(Lang::Es, &report),
            "se deshizo la importación 4: se quitaron 0 memorias\n\
             proyectos que había creado, borrados: ideas-1a2b, web-1a2b\n\
             proyectos borrados que había traído de vuelta, borrados de nuevo: gone-5e6f\n\
             sugerencias que había dejado, quitadas: ideas-1a2b ~ ideas-9c0d\n\
             subcarpetas que había declarado, quitadas: mono-1a2b apps/web\n\
             se quedan en el registro, porque algo las usa o alguien las respondió o revisó:\n\
             \x20 project mono-1a2b: reviewed\n  suggestion a-1111 b-2222: answered"
        );
    }

    #[tokio::test]
    async fn a_missing_file_is_named_before_the_daemon_is_asked() {
        let c = DaemonClient::new("http://127.0.0.1:1", "token", std::env::temp_dir());
        let maps = Maps { roots: &[], projects: &[] };
        let err = run(Lang::En, &c, Path::new("/no/such/export.jsonl"), ImportSource::Clawg, false, None, maps).await.unwrap_err();
        assert!(matches!(err, ImportError::Read { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn a_project_map_must_name_a_directory() {
        let c = DaemonClient::new("http://127.0.0.1:1", "token", std::env::temp_dir());
        let dir = tempfile::tempdir().unwrap();
        let not_a_dir = dir.path().join("notes.txt");
        std::fs::write(&not_a_dir, "notes").unwrap();
        let projects = [
            ProjectMap { name: "app".into(), to: MapTo::Path(dir.path().to_path_buf()) },
            ProjectMap { name: "notes".into(), to: MapTo::Path(not_a_dir.clone()) },
        ];
        let maps = Maps { roots: &[], projects: &projects };
        let missing = Path::new("/no/such/export.json");
        let err = run(Lang::En, &c, missing, ImportSource::Engram, false, None, maps).await.unwrap_err();
        assert!(matches!(&err, ImportError::NotADirectory { name, .. } if name == "notes"), "before the file is read: {err:?}");
        assert_eq!(err.to_string(), format!("--project-map notes={}: not a directory", not_a_dir.display()));
    }

    /// A report of an import from another tool with something in every field.
    fn foreign_report() -> ImportReport {
        use clawg_core::memory::portability::translate::{
            AbsorbedNote, KeyChange, LeftOut, MergedNote, ProjectCount, SupersededNote, TakenKey, TakenProject, TypeCount,
        };
        let count = |project: Option<&str>, personal, root: Option<&str>, project_id: Option<&str>, notes| ProjectCount {
            project: project.map(str::to_owned),
            personal,
            root: root.map(str::to_owned),
            project_id: project_id.map(str::to_owned),
            notes,
        };
        ImportReport {
            rows: 4,
            inserted: 3,
            present: 1,
            by_type: [("semantic_fact".to_string(), 3)].into(),
            by_state: [("live".to_string(), 3)].into(),
            by_project: [(String::new(), 3)].into(),
            by_lang: [("en".to_string(), 2), ("es".to_string(), 1)].into(),
            snapshot: Some("/b/memory-x-pre-import.db".into()),
            foreign: Some(ForeignReport {
                source: "engram".into(),
                version: "0.2.0".into(),
                notes: 5,
                by_source_type: vec![
                    TypeCount { kind: "observation".into(), source_type: Some("bugfix".into()), memory_type: "semantic_fact".into(), notes: 4 },
                    TypeCount { kind: "prompt".into(), source_type: None, memory_type: "chat_message".into(), notes: 1 },
                ],
                by_source_project: vec![
                    count(None, false, None, None, 2),
                    count(Some("app"), false, Some("/work/app"), Some("app-1a2b"), 2),
                    count(Some("app"), true, None, None, 1),
                ],
                unmapped: vec!["notes".into()],
                unused_map: vec!["old".into()],
                rows: 4,
                split_notes: 1,
                masked_notes: 1,
                rewritten_keys: vec![KeyChange { from: "Sdd X".into(), to: "sdd-x-0123456789".into() }],
                qualified_keys: vec![KeyChange { from: "sdd/x".into(), to: "app/sdd/x".into() }],
                superseded_in_file: vec![SupersededNote { origin: "engram:a".into(), by: "engram:b".into() }],
                absorbed: vec![AbsorbedNote { origin: "engram:c".into(), into: "engram:d".into() }],
                clamped_dates: vec!["engram:e".into()],
                deleted: 1,
                sessions_left_out: vec!["engram:f".into()],
                without_text: vec!["engram:g".into()],
                unplaced: [("observation.tool_name".to_string(), 2)].into(),
                rewritten_at_source: 3,
                discarded_sessions: 4,
                merged: vec![MergedNote { origin: "engram:h".into(), piece: 2, into: 40 }],
                topic_taken: vec![TakenKey { origin: "engram:i".into(), topic_key: "api/port".into(), by: 7 }],
                taken_projects: vec![TakenProject { project: "web".into(), root: "/work/web".into(), belongs_to: "/other/web".into(), notes: 2 }],
                left_out: vec![LeftOut { origin: "engram:j".into(), reason: "the store changed while importing; import the file again".into() }],
                present: [("app-1a2b".to_string(), 1)].into(),
                revised: vec!["engram:k".into()],
                older_versions: vec!["engram:l".into()],
                deleted_at_source: vec!["engram:m".into()],
                relations: [("compatible".to_string(), 2), ("escalated".to_string(), 1)].into(),
                relations_left_out: [("orphaned".to_string(), 1)].into(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn render_names_what_a_foreign_import_did() {
        assert_eq!(
            render(Lang::En, &foreign_report()),
            "imported 3 of 4 memories in the file\n  by type: semantic_fact 3\n  by state: live 3\n  by project: (none) 3\n  by language: en 2, es 1\n\
             already here: 1\n\
             from engram 0.2.0: notes 5, memories 4, notes in more than one memory 1\n\
             notes by type of the source:\n  observation bugfix → semantic_fact: 4\n  prompt → chat_message: 1\n\
             notes by project of the source:\n  (none) → (none): 2\n  app → app-1a2b (/work/app): 2\n  app (personal) → (none): 1\n\
             projects of the source no --project-map names, stored in none: notes\n\
             --project-map names no note has: old\n\
             notes with a secret, masked whole: 1\n\
             topic keys clawg does not take, rewritten:\n  Sdd X → sdd-x-0123456789\n\
             topic keys two projects of the source share in one project here, qualified:\n  sdd/x → app/sdd/x\n\
             notes stored superseded by a newer note of their key:\n  engram:a by engram:b\n\
             notes of the text of an older note of their project of the source, stored in it:\n  engram:c in engram:d\n\
             notes already here, by the project they are in: app-1a2b 1\n\
             notes the source rewrote, stored as the newer version of the one here:\n  engram:k\n\
             older versions of notes here, stored superseded by them:\n  engram:l\n\
             notes the source deleted, left here as they are:\n  engram:m\n\
             notes dated after the import or without a date, stored with the time of the import:\n  engram:e\n\
             notes the source had deleted, stored deleted: 1\n\
             relations stored as pairs: compatible 2, escalated 1\n\
             relations left out: orphaned 1\n\
             notes whose session id is not kept:\n  engram:f\n\
             notes without text, not stored:\n  engram:g\n\
             in the export with no place here, by field: observation.tool_name 2\n\
             notes the source rewrote without exporting their earlier versions: 3\n\
             sessions without a summary, which give no note: 4\n\
             the same text was already live here, so these pieces were not added again:\n  engram:h piece 2 is #40\n\
             their topic key is held by a live memory here, so they were stored superseded by it:\n  engram:i (api/port) superseded by #7\n\
             not imported:\n  engram:j: the store changed while importing; import the file again\n\
             projects whose id a registration gave another root during the import, not imported:\n  web (/work/web) belongs to /other/web, notes 2\n\
             the store before the import is in /b/memory-x-pre-import.db; `clawg restore` brings it back"
        );
    }

    #[test]
    fn the_foreign_report_speaks_spanish() {
        assert_eq!(
            render(Lang::Es, &foreign_report()),
            "se importaron 3 de 4 memorias del archivo\n  por tipo: semantic_fact 3\n  por estado: live 3\n  por proyecto: (ninguno) 3\n  por idioma: en 2, es 1\n\
             ya estaban aquí: 1\n\
             de engram 0.2.0: notas 5, memorias 4, notas en más de una memoria 1\n\
             notas por tipo de la fuente:\n  observation bugfix → semantic_fact: 4\n  prompt → chat_message: 1\n\
             notas por proyecto de la fuente:\n  (ninguno) → (ninguno): 2\n  app → app-1a2b (/work/app): 2\n  app (personal) → (ninguno): 1\n\
             proyectos de la fuente que ningún --project-map nombra, guardados en ninguno: notes\n\
             nombres de --project-map que ninguna nota tiene: old\n\
             notas con un secreto, enmascaradas enteras: 1\n\
             claves de tema que clawg no acepta, reescritas:\n  Sdd X → sdd-x-0123456789\n\
             claves de tema que dos proyectos de la fuente comparten en un proyecto de aquí, calificadas:\n  sdd/x → app/sdd/x\n\
             notas guardadas reemplazadas por una nota más nueva de su clave:\n  engram:a por engram:b\n\
             notas del texto de una nota más vieja de su proyecto de la fuente, guardadas en ella:\n  engram:c en engram:d\n\
             notas que ya estaban aquí, por el proyecto donde están: app-1a2b 1\n\
             notas que la fuente reescribió, guardadas como la versión más nueva de la de aquí:\n  engram:k\n\
             versiones más viejas de notas de aquí, guardadas reemplazadas por ellas:\n  engram:l\n\
             notas que la fuente borró, dejadas aquí como están:\n  engram:m\n\
             notas con una fecha posterior a la importación o sin fecha, guardadas con la hora de la importación:\n  engram:e\n\
             notas que la fuente tenía borradas, guardadas borradas: 1\n\
             relaciones guardadas como pares: compatible 2, escalated 1\n\
             relaciones que quedaron afuera: orphaned 1\n\
             notas cuyo id de sesión no se guarda:\n  engram:f\n\
             notas sin texto, no se guardaron:\n  engram:g\n\
             en el export sin lugar aquí, por campo: observation.tool_name 2\n\
             notas que la fuente reescribió sin exportar sus versiones anteriores: 3\n\
             sesiones sin resumen, que no dan nota: 4\n\
             el mismo texto ya estaba vivo aquí, así que estas piezas no se agregaron de nuevo:\n  engram:h pieza 2 es #40\n\
             su clave de tema la tiene una memoria viva aquí, así que se guardaron reemplazadas por ella:\n  engram:i (api/port) reemplazada por #7\n\
             no se importaron:\n  engram:j: the store changed while importing; import the file again\n\
             proyectos cuyo id un registro le dio a otra raíz durante la importación, no se importaron:\n  web (/work/web) es de /other/web, notas 2\n\
             el store de antes de la importación está en /b/memory-x-pre-import.db; `clawg restore` lo trae de vuelta"
        );
    }

    #[test]
    fn the_import_report_speaks_spanish() {
        let report = ImportReport {
            rows: 3,
            inserted: 1,
            present: 1,
            by_type: [("semantic_fact".to_string(), 1)].into(),
            by_state: [("live".to_string(), 1)].into(),
            by_project: [(String::new(), 1)].into(),
            merged: vec![clawg_core::memory::portability::import::Merged { line: 3, id: 2, into: 40 }],
            topic_taken: vec![clawg_core::memory::portability::import::TopicTaken {
                line: 4,
                id: 3,
                topic_key: "api/port".into(),
                stored_as: 41,
                by: 7,
            }],
            redacted: 1,
            invalid: vec![Invalid { line: 5, reason: "memory: missing field `id`".into() }],
            snapshot: Some("/b/memory-x-pre-import.db".into()),
            ..Default::default()
        };
        assert_eq!(
            render(Lang::Es, &report),
            "se importaron 1 de 3 memorias del archivo\n  por tipo: semantic_fact 1\n  por estado: live 1\n  por proyecto: (ninguno) 1\n\
             ya estaban aquí: 1\n\
             el mismo texto ya estaba vivo aquí, así que estas no se agregaron de nuevo:\n  línea 3: la memoria 2 es #40\n\
             su clave de tema la tiene una memoria viva aquí, así que se guardaron reemplazadas por ella:\n  línea 4: la memoria 3 (api/port) es #41, reemplazada por #7\n\
             secretos enmascarados en 1\nno se importaron:\n  línea 5: memory: missing field `id`\n\
             el store de antes de la importación está en /b/memory-x-pre-import.db; `clawg restore` lo trae de vuelta"
        );
        let dry = render(Lang::Es, &ImportReport { dry_run: true, rows: 2, ..Default::default() });
        assert_eq!(dry, "se importarían 0 de 2 memorias del archivo");
    }
}
