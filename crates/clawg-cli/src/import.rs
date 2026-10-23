//! `clawg import`: a file `clawg export` wrote, into the running daemon's
//! store, which embeds each memory again.

use std::path::{Path, PathBuf};

use clawg_core::memory::identity::resolve_project_root;
use clawg_core::memory::portability::{ImportReport, MAX_IMPORT_BYTES};
use clawg_core::preferences::Lang;

use crate::cli::RootMap;
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
    #[error(transparent)]
    Client(#[from] ClientError),
}

/// Sends `file` to the daemon and says what it did. Each NEW root is resolved
/// here, as a bridge resolves its own; each OLD root is taken as written.
pub async fn run(
    lang: Lang,
    client: &DaemonClient,
    file: &Path,
    dry_run: bool,
    agent: Option<&str>,
    map_roots: &[RootMap],
) -> Result<String, ImportError> {
    let read = |source| ImportError::Read { path: file.to_path_buf(), source };
    let size = std::fs::metadata(file).map_err(read)?.len();
    if size > MAX_IMPORT_BYTES as u64 {
        return Err(ImportError::TooLarge { path: file.to_path_buf(), size });
    }
    let text = std::fs::read_to_string(file).map_err(read)?;
    let roots: Vec<(String, String)> = map_roots
        .iter()
        .map(|m| (m.old.clone(), resolve_project_root(&m.new).to_string_lossy().into_owned()))
        .collect();
    let report = client.import(&text, dry_run, agent, &roots).await?;
    Ok(render(lang, &report))
}

fn counts(map: &std::collections::BTreeMap<String, usize>, empty: &str) -> String {
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
    let (by_type, by_state, by_project, none) = match lang {
        Lang::En => ("by type", "by state", "by project", "(none)"),
        Lang::Es => ("por tipo", "por estado", "por proyecto", "(ninguno)"),
    };
    if r.inserted > 0 {
        lines.push(format!("  {by_type}: {}", counts(&r.by_type, "")));
        lines.push(format!("  {by_state}: {}", counts(&r.by_state, "")));
        lines.push(format!("  {by_project}: {}", counts(&r.by_project, none)));
    }
    if r.present > 0 {
        lines.push(match lang {
            Lang::En => format!("already here: {}", r.present),
            Lang::Es => format!("ya estaban aquí: {}", r.present),
        });
    }
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
    if let Some(snapshot) = &r.snapshot {
        lines.push(match lang {
            Lang::En => format!("the store before the import is in {snapshot}; `clawg restore` brings it back"),
            Lang::Es => format!("el store de antes de la importación está en {snapshot}; `clawg restore` lo trae de vuelta"),
        });
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tests::{publish, start_daemon};
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

        let dry = run(Lang::En, &client, &file, true, Some("b"), &maps).await.expect("dry run");
        assert!(dry.starts_with("would import 1 of 1 memories in the file\n"), "{dry}");
        assert!(dry.contains(&format!("  by project: {project} 1")), "{dry}");
        assert!(dry.contains("not imported:\n  line 4: memory:"), "{dry}");
        assert!(!dry.contains("restore"), "{dry}");

        let done = run(Lang::En, &client, &file, false, Some("b"), &maps).await.expect("import");
        assert!(done.starts_with("imported 1 of 1 memories in the file\n  by type: semantic_fact 1\n  by state: live 1\n"), "{done}");
        assert!(done.contains("-pre-import.db; `clawg restore` brings it back"), "{done}");

        let again = run(Lang::En, &client, &file, false, Some("b"), &maps).await.expect("again");
        assert!(again.starts_with("imported 0 of 1 memories in the file\nalready here: 1\n"), "{again}");
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

    #[tokio::test]
    async fn a_missing_file_is_named_before_the_daemon_is_asked() {
        let c = DaemonClient::new("http://127.0.0.1:1", "token", std::env::temp_dir());
        let err = run(Lang::En, &c, Path::new("/no/such/export.jsonl"), false, None, &[]).await.unwrap_err();
        assert!(matches!(err, ImportError::Read { .. }), "{err:?}");
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
