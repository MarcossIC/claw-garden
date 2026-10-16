//! Conventions of the workspace that the compiler does not enforce.

use std::fs;
use std::path::{Path, PathBuf};

/// Whether `line` is an attribute that silences an unused-code lint: an
/// `allow` or `expect`, inner or outer, also behind a `cfg_attr`, naming
/// `dead_code`, `unused` or an `unused_*` lint. The repository's rule is to
/// delete such code, not to hide it.
fn hides_dead_code(line: &str) -> bool {
    let line = line.trim_start();
    let Some(attr) = line.strip_prefix("#![").or_else(|| line.strip_prefix("#[")) else {
        return false;
    };
    ["allow(", "expect("].iter().any(|opener| {
        attr.match_indices(opener).any(|(at, _)| {
            let lints = &attr[at + opener.len()..];
            let lints = &lints[..lints.find(')').unwrap_or(lints.len())];
            lints.split(',').map(str::trim).any(|lint| lint == "dead_code" || lint == "unused" || lint.starts_with("unused_"))
        })
    })
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn hides_dead_code_recognises_the_attribute_forms() {
    for hiding in [
        "#[allow(dead_code)]",
        "    #[allow(dead_code)] // kept for later",
        "#![allow(unused)]",
        "#[allow(unused_imports, unused_variables)]",
        "#[expect(dead_code)]",
        "#[cfg_attr(not(test), allow(dead_code))]",
        "#[allow(clippy::too_many_arguments, dead_code)]",
    ] {
        assert!(hides_dead_code(hiding), "{hiding}");
    }
    for fine in [
        "#[allow(clippy::too_many_arguments)]",
        "#[cfg(test)]",
        "// #[allow(dead_code)] in a comment",
        "/// Never `#[allow(dead_code)]`.",
        "let s = \"#[allow(dead_code)]\";",
        "#[allow(clippy::unused_io_amount)]",
    ] {
        assert!(!hides_dead_code(fine), "{fine}");
    }
}

#[test]
fn no_source_allows_dead_code() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the crates directory");
    let mut files = Vec::new();
    for krate in fs::read_dir(crates).expect("read crates/").flatten() {
        rust_files(&krate.path().join("src"), &mut files);
        rust_files(&krate.path().join("tests"), &mut files);
    }
    assert!(!files.is_empty(), "no sources found under {}", crates.display());

    let mut found = Vec::new();
    for file in &files {
        let source = fs::read_to_string(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        for (n, line) in source.lines().enumerate() {
            if hides_dead_code(line) {
                let shown = file.strip_prefix(crates).unwrap_or(file);
                found.push(format!("{}:{}", shown.display(), n + 1));
            }
        }
    }
    assert!(found.is_empty(), "delete the unused code instead of allowing it:\n{}", found.join("\n"));
}
