//! `clawg export`: the stored memories, one row per line, in a file.
//!
//! Reads the store the way `clawg secrets` does, without the lock, so it
//! works with or without a daemon. The file holds the text of every memory it
//! takes, so it is created new and, on Unix, readable by its owner only.

use std::io::Write;
use std::path::Path;

use clawg_core::memory::portability::{export, markdown, Export, ExportFilter};
use clawg_core::preferences::Lang;

use crate::cli::ExportFormat;

/// Exports the rows of the store in `base_dir` that `filter` takes into
/// `out`, and says what it wrote in `lang`. `out` must not exist.
pub fn run(
    lang: Lang,
    base_dir: &Path,
    out: &Path,
    filter: &ExportFilter,
    format: ExportFormat,
    now_ms: i64,
) -> Result<String, String> {
    if out.exists() {
        return Err(match lang {
            Lang::En => format!("{} already exists; export writes a new file", out.display()),
            Lang::Es => format!("{} ya existe; export escribe un archivo nuevo", out.display()),
        });
    }
    let exported = export(base_dir, filter, now_ms).map_err(|e| e.to_string())?;
    let text = match format {
        ExportFormat::Jsonl => exported.to_jsonl(),
        ExportFormat::Markdown => markdown::render(&exported),
    };
    write_new(out, &text).map_err(|e| match lang {
        Lang::En => format!("could not write {}: {e}", out.display()),
        Lang::Es => format!("no se pudo escribir {}: {e}", out.display()),
    })?;
    Ok(report(lang, out, &Written::of(&exported), format))
}

/// Creates `path`, failing when it exists, owner-only on Unix, and writes
/// `text` through to disk. A file left half written is removed.
fn write_new(path: &Path, text: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    let written = file.write_all(text.as_bytes()).and_then(|()| file.sync_all());
    if written.is_err() {
        drop(file);
        let _ = std::fs::remove_file(path);
    }
    written
}

/// How many of each thing an export holds, and what it left out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Written {
    pub memories: usize,
    pub projects: usize,
    pub derivations: usize,
    pub pieces: usize,
    pub pairs: usize,
    pub outbox_waiting: usize,
    pub legacy_rows: usize,
}

impl Written {
    pub fn of(exported: &Export) -> Self {
        Written {
            memories: exported.memories.len(),
            projects: exported.projects.len(),
            derivations: exported.derivations.len(),
            pieces: exported.pieces.len(),
            pairs: exported.conflicts.len(),
            outbox_waiting: exported.outbox_waiting,
            legacy_rows: exported.legacy_rows,
        }
    }
}

/// `n` and the noun that agrees with it.
fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// What was written, and what the file does not hold.
pub fn report(lang: Lang, out: &Path, w: &Written, format: ExportFormat) -> String {
    let out = out.display();
    let mut lines = vec![match (format, lang) {
        (ExportFormat::Jsonl, Lang::En) => format!(
            "wrote {}, {}, {}, {} and {} to {out}",
            plural(w.memories, "memory", "memories"),
            plural(w.projects, "project", "projects"),
            plural(w.derivations, "derivation", "derivations"),
            plural(w.pieces, "piece", "pieces"),
            plural(w.pairs, "pair", "pairs"),
        ),
        (ExportFormat::Jsonl, Lang::Es) => format!(
            "se escribieron {}, {}, {}, {} y {} en {out}",
            plural(w.memories, "memoria", "memorias"),
            plural(w.projects, "proyecto", "proyectos"),
            plural(w.derivations, "derivación", "derivaciones"),
            plural(w.pieces, "pieza", "piezas"),
            plural(w.pairs, "par", "pares"),
        ),
        (ExportFormat::Markdown, Lang::En) => format!(
            "wrote a view of {} to {out}; clawg import does not read it",
            plural(w.memories, "memory", "memories"),
        ),
        (ExportFormat::Markdown, Lang::Es) => format!(
            "se escribió una vista de {} en {out}; clawg import no la lee",
            plural(w.memories, "memoria", "memorias"),
        ),
    }];
    lines.push(
        match lang {
            Lang::En => "the file holds the text of these memories as stored; `clawg secrets` lists the ones that hold a secret",
            Lang::Es => {
                "el archivo tiene el texto de estas memorias tal como están guardadas; `clawg secrets` lista las que \
                 guardan un secreto"
            }
        }
        .to_string(),
    );
    if w.outbox_waiting > 0 {
        let one = w.outbox_waiting == 1;
        lines.push(match lang {
            Lang::En => format!(
                "{} waiting in the outbox {} not in the file; export again once the daemon has stored {}",
                plural(w.outbox_waiting, "save", "saves"),
                if one { "is" } else { "are" },
                if one { "it" } else { "them" },
            ),
            Lang::Es => format!(
                "{} que {} en el outbox no {} en el archivo; exporta de nuevo cuando el daemon {} haya guardado",
                plural(w.outbox_waiting, "save", "saves"),
                if one { "espera" } else { "esperan" },
                if one { "está" } else { "están" },
                if one { "lo" } else { "los" },
            ),
        });
    }
    if w.legacy_rows > 0 {
        let one = w.legacy_rows == 1;
        lines.push(match lang {
            Lang::En => format!(
                "{} of episodic_trajectory and procedural_skill {} not exported",
                plural(w.legacy_rows, "row", "rows"),
                if one { "is" } else { "are" },
            ),
            Lang::Es => format!(
                "{} de episodic_trajectory y procedural_skill no se {}",
                plural(w.legacy_rows, "fila", "filas"),
                if one { "exportó" } else { "exportaron" },
            ),
        });
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_refuses_an_existing_file_and_writes_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("memories.jsonl");
        std::fs::write(&out, "mine").unwrap();
        let err = run(Lang::En, dir.path(), &out, &ExportFilter::default(), ExportFormat::Jsonl, 0).unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "mine");

        let fresh = dir.path().join("fresh.jsonl");
        let err = run(Lang::En, dir.path(), &fresh, &ExportFilter::default(), ExportFormat::Jsonl, 0).unwrap_err();
        assert!(err.contains("no memory store"), "{err}");
        assert!(!fresh.exists(), "nothing is written without a store");

        write_new(&fresh, "rows\n").unwrap();
        assert_eq!(std::fs::read_to_string(&fresh).unwrap(), "rows\n");
        assert!(write_new(&fresh, "again").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&fresh).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn the_export_report_agrees_with_its_numbers_in_both_languages() {
        let out = Path::new("/x/m.jsonl");
        let one = Written { memories: 1, projects: 1, derivations: 1, pieces: 1, pairs: 1, outbox_waiting: 1, legacy_rows: 1 };
        let many = Written { memories: 2, projects: 0, derivations: 3, pieces: 4, pairs: 5, outbox_waiting: 2, legacy_rows: 2 };
        assert_eq!(
            report(Lang::En, out, &one, ExportFormat::Jsonl),
            "wrote 1 memory, 1 project, 1 derivation, 1 piece and 1 pair to /x/m.jsonl\n\
             the file holds the text of these memories as stored; `clawg secrets` lists the ones that hold a secret\n\
             1 save waiting in the outbox is not in the file; export again once the daemon has stored it\n\
             1 row of episodic_trajectory and procedural_skill is not exported"
        );
        assert_eq!(
            report(Lang::Es, out, &one, ExportFormat::Jsonl),
            "se escribieron 1 memoria, 1 proyecto, 1 derivación, 1 pieza y 1 par en /x/m.jsonl\n\
             el archivo tiene el texto de estas memorias tal como están guardadas; `clawg secrets` lista las que guardan un secreto\n\
             1 save que espera en el outbox no está en el archivo; exporta de nuevo cuando el daemon lo haya guardado\n\
             1 fila de episodic_trajectory y procedural_skill no se exportó"
        );
        let es = report(Lang::Es, out, &many, ExportFormat::Jsonl);
        assert!(es.starts_with("se escribieron 2 memorias, 0 proyectos, 3 derivaciones, 4 piezas y 5 pares en /x/m.jsonl\n"), "{es}");
        assert!(es.contains("2 saves que esperan en el outbox no están en el archivo; exporta de nuevo cuando el daemon los haya guardado"), "{es}");
        assert!(es.ends_with("2 filas de episodic_trajectory y procedural_skill no se exportaron"), "{es}");
        assert_eq!(
            report(Lang::Es, out, &Written { memories: 2, ..Written::default() }, ExportFormat::Markdown).lines().next(),
            Some("se escribió una vista de 2 memorias en /x/m.jsonl; clawg import no la lee")
        );
        let dir = tempfile::tempdir().unwrap();
        let taken = dir.path().join("taken.jsonl");
        std::fs::write(&taken, "mine").unwrap();
        let err = run(Lang::Es, dir.path(), &taken, &ExportFilter::default(), ExportFormat::Jsonl, 0).unwrap_err();
        assert!(err.ends_with("ya existe; export escribe un archivo nuevo"), "{err}");
    }
}
