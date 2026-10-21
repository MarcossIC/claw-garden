//! `clawg export`: the stored memories, one row per line, in a file.
//!
//! Reads the store the way `clawg secrets` does, without the lock, so it
//! works with or without a daemon. The file holds the text of every memory it
//! takes, so it is created new and, on Unix, readable by its owner only.

use std::io::Write;
use std::path::Path;

use clawg_core::memory::portability::{export, markdown, Export, ExportFilter};

use crate::cli::ExportFormat;

/// Exports the rows of the store in `base_dir` that `filter` takes into
/// `out`, and says what it wrote. `out` must not exist.
pub fn run(base_dir: &Path, out: &Path, filter: &ExportFilter, format: ExportFormat, now_ms: i64) -> Result<String, String> {
    if out.exists() {
        return Err(format!("{} already exists; export writes a new file", out.display()));
    }
    let exported = export(base_dir, filter, now_ms).map_err(|e| e.to_string())?;
    let text = match format {
        ExportFormat::Jsonl => exported.to_jsonl(),
        ExportFormat::Markdown => markdown::render(&exported),
    };
    write_new(out, &text).map_err(|e| format!("could not write {}: {e}", out.display()))?;
    Ok(report(out, &exported, format))
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

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// What was written, and what the file does not hold.
pub fn report(out: &Path, exported: &Export, format: ExportFormat) -> String {
    let mut lines = vec![match format {
        ExportFormat::Jsonl => format!(
            "wrote {}, {}, {}, {} and {} to {}",
            plural(exported.memories.len(), "memory", "memories"),
            plural(exported.projects.len(), "project", "projects"),
            plural(exported.derivations.len(), "derivation", "derivations"),
            plural(exported.pieces.len(), "piece", "pieces"),
            plural(exported.conflicts.len(), "pair", "pairs"),
            out.display()
        ),
        ExportFormat::Markdown => format!(
            "wrote a view of {} to {}; clawg import does not read it",
            plural(exported.memories.len(), "memory", "memories"),
            out.display()
        ),
    }];
    lines.push(
        "the file holds the text of these memories as stored; `clawg secrets` lists the ones that hold a secret"
            .to_string(),
    );
    if exported.outbox_waiting > 0 {
        lines.push(format!(
            "{} waiting in the outbox {} not in the file; export again once the daemon has stored {}",
            plural(exported.outbox_waiting, "save", "saves"),
            if exported.outbox_waiting == 1 { "is" } else { "are" },
            if exported.outbox_waiting == 1 { "it" } else { "them" },
        ));
    }
    if exported.legacy_rows > 0 {
        lines.push(format!(
            "{} of episodic_trajectory and procedural_skill {} not exported",
            plural(exported.legacy_rows, "row", "rows"),
            if exported.legacy_rows == 1 { "is" } else { "are" },
        ));
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
        let err = run(dir.path(), &out, &ExportFilter::default(), ExportFormat::Jsonl, 0).unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "mine");

        let fresh = dir.path().join("fresh.jsonl");
        let err = run(dir.path(), &fresh, &ExportFilter::default(), ExportFormat::Jsonl, 0).unwrap_err();
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
}
