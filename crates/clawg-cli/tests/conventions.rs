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

/// Lines of a manifest that turn on clawg-core's `testing` feature outside
/// `[dev-dependencies]`. That feature lets a binary read `CLAWG_TEST_HOME`
/// and `CLAWG_TEST_EMBEDDER`, so a release build must never get it.
fn testing_outside_dev_dependencies(manifest: &str) -> Vec<usize> {
    let mut section = "";
    let mut found = Vec::new();
    for (n, line) in manifest.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            section = line;
            continue;
        }
        let enables = line.contains("\"testing\"") || line.contains("/testing\"");
        if enables && !section.ends_with("dev-dependencies]") {
            found.push(n + 1);
        }
    }
    found
}

#[test]
fn testing_outside_dev_dependencies_reads_the_section() {
    let manifest = r#"
[dependencies]
clawg-core = { workspace = true, features = ["testing"] }
[dev-dependencies]
clawg-core = { workspace = true, features = ["testing"] }
[target.'cfg(unix)'.dev-dependencies]
clawg-core = { workspace = true, features = ["testing"] }
[features]
testing = []
default = ["testing"]
local = ["clawg-core/testing"]
"#;
    assert_eq!(testing_outside_dev_dependencies(manifest), [3, 10, 11]);
}

#[test]
fn only_dev_dependencies_turn_on_the_test_stand_ins() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let crates = root.join("crates");
    let mut manifests = vec![root.join("Cargo.toml")];
    for krate in fs::read_dir(&crates).expect("read crates/").flatten() {
        manifests.push(krate.path().join("Cargo.toml"));
    }

    let mut found = Vec::new();
    for manifest in manifests.iter().filter(|m| m.is_file()) {
        let text = fs::read_to_string(manifest).unwrap_or_else(|e| panic!("read {}: {e}", manifest.display()));
        for n in testing_outside_dev_dependencies(&text) {
            found.push(format!("{}:{n}", manifest.display()));
        }
    }
    assert!(found.is_empty(), "`testing` belongs only in [dev-dependencies]:\n{}", found.join("\n"));
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read_repo_file(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The value written for `key` inside `[section]` of a manifest, as the raw
/// text after the `=`, without a trailing comment.
fn manifest_value(manifest: &str, section: &str, key: &str) -> Option<String> {
    let mut inside = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == format!("[{section}]");
            continue;
        }
        if !inside {
            continue;
        }
        if let Some((name, value)) = line.split_once('=') {
            if name.trim() == key {
                let value = value.split(" #").next().unwrap_or(value);
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

#[test]
fn the_manifest_carries_the_publication_metadata() {
    let manifest = read_repo_file("Cargo.toml");
    let value = |key: &str| manifest_value(&manifest, "workspace.package", key).unwrap_or_default();

    assert_eq!(value("license"), "\"MIT\"");
    assert_eq!(value("repository"), "\"https://github.com/MarcossIC/claw-garden\"");
    assert_eq!(value("authors"), "[\"The Claw Garden contributors\"]");

    let license = read_repo_file("LICENSE");
    assert!(license.starts_with("MIT License"), "LICENSE is the MIT text");
    assert!(license.contains("The Claw Garden contributors"), "LICENSE names the holder");
}

#[test]
fn reqwest_uses_rustls_with_the_system_roots() {
    let manifest = read_repo_file("Cargo.toml");
    let declaration = manifest_value(&manifest, "workspace.dependencies", "reqwest").expect("reqwest is declared");

    assert!(declaration.contains("default-features = false"), "{declaration}");
    for feature in ["json", "stream", "rustls-tls-native-roots", "charset", "http2", "system-proxy"] {
        assert!(declaration.contains(&format!("\"{feature}\"")), "reqwest needs `{feature}`: {declaration}");
    }
    for feature in ["default-tls", "native-tls"] {
        assert!(!declaration.contains(feature), "reqwest must not enable `{feature}`: {declaration}");
    }
}
