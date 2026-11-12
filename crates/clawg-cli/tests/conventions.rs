//! Conventions of the workspace that the compiler does not enforce.

use std::fs;
use std::path::{Path, PathBuf};

use clawg_cli::update::{plan, signature};

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

/// V32 moved the rows of these tables into `memory` and dropped them, so no
/// code but the tests that seed a store from before it may name them. The
/// export is the one exception: it reads a store without migrating it, and
/// one from before V32 still has them. A file's tests start at its first
/// `#[cfg(test)]` module.
#[test]
fn no_production_code_names_the_retired_tables() {
    const RETIRED: [&str; 4] = ["episodic_trajectory", "procedural_skill", "episodic_vectors", "procedural_vectors"];
    const ALLOWED: [&str; 1] = ["clawg-core/src/memory/portability/export.rs"];
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the crates directory");
    let mut files = Vec::new();
    for krate in fs::read_dir(crates).expect("read crates/").flatten() {
        rust_files(&krate.path().join("src"), &mut files);
    }
    assert!(!files.is_empty(), "no sources found under {}", crates.display());

    let mut found = Vec::new();
    for file in &files {
        let shown = file.strip_prefix(crates).unwrap_or(file).to_string_lossy().replace('\\', "/");
        if ALLOWED.contains(&shown.as_str()) {
            continue;
        }
        let source = fs::read_to_string(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        found.extend(production_lines_naming(&source, |line| RETIRED.iter().any(|table| line.contains(table))).into_iter().map(|n| format!("{shown}:{n}")));
    }
    assert!(found.is_empty(), "these lines name a table V32 dropped:\n{}", found.join("\n"));
}

/// A failed call on the store's connection is worded in one place,
/// `memory/error.rs`, so no other code of the core matches on the connection's
/// error.
#[test]
fn no_memory_code_translates_a_connection_error_by_hand() {
    const TRANSLATES: &str = "clawg-core/src/memory/error.rs";
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the crates directory");
    let mut files = Vec::new();
    rust_files(&crates.join("clawg-core/src"), &mut files);
    assert!(!files.is_empty(), "no sources found under {}", crates.display());

    let mut found = Vec::new();
    for file in &files {
        let shown = file.strip_prefix(crates).unwrap_or(file).to_string_lossy().replace('\\', "/");
        if shown == TRANSLATES {
            continue;
        }
        let source = fs::read_to_string(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        found.extend(production_lines_naming(&source, |line| line.contains("tokio_rusqlite::Error::")).into_iter().map(|n| format!("{shown}:{n}")));
    }
    assert!(found.is_empty(), "translate a failed call with `failed_call` or a `From` of {TRANSLATES}:\n{}", found.join("\n"));
}

/// The lines, from 1, of the production code of `source` that `names` takes:
/// the code before the first `#[cfg(test)]` followed by a `mod <name> {`. A
/// `mod <name>;` declares a module kept in another file, and ends nothing.
fn production_lines_naming(source: &str, names: impl Fn(&str) -> bool) -> Vec<usize> {
    let lines: Vec<&str> = source.lines().collect();
    lines[..production_end(&lines)].iter().enumerate().filter(|(_, line)| names(line)).map(|(n, _)| n + 1).collect()
}

/// How many of `lines` are production code.
fn production_end(lines: &[&str]) -> usize {
    lines
        .windows(2)
        .position(|pair| pair[0] == "#[cfg(test)]" && pair[1].starts_with("mod ") && pair[1].trim_end().ends_with('{'))
        .unwrap_or(lines.len())
}

/// Whether `line` names `rusqlite` or reads a field or a method called
/// `conn`: `.conn` not followed by a character that continues an identifier,
/// so `.connect_timeout` and `.conn_for_test()` do not count.
fn reaches_the_connection(line: &str) -> bool {
    let continues = |c: char| c.is_ascii_alphanumeric() || c == '_';
    line.contains("rusqlite") || line.match_indices(".conn").any(|(at, found)| !line[at + found.len()..].starts_with(continues))
}

#[test]
fn reaches_the_connection_tells_the_field_from_longer_names() {
    for reaching in ["store.conn", "    .conn", "let c = &store.conn;", "store.conn.call(|c| Ok(()))", "use rusqlite::Connection;", "e: tokio_rusqlite::Error"] {
        assert!(reaches_the_connection(reaching), "{reaching}");
    }
    for fine in ["builder.connect_timeout(wait)", "store.conn_for_test().call(f)", "let connection = open();", ".connected"] {
        assert!(!reaches_the_connection(fine), "{fine}");
    }
}

/// SQLite is the core's: the code of the other crates that a release builds
/// neither names `rusqlite` nor reads the store's connection. Their tests
/// reach it with `MemoryStore::conn_for_test`.
#[test]
fn no_production_code_outside_core_reaches_the_connection() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the crates directory");
    let mut files = Vec::new();
    for krate in ["clawg-cli", "clawg-eval"] {
        rust_files(&crates.join(krate).join("src"), &mut files);
    }
    assert!(!files.is_empty(), "no sources found under {}", crates.display());

    let mut found = Vec::new();
    for file in &files {
        let shown = file.strip_prefix(crates).unwrap_or(file).to_string_lossy().replace('\\', "/");
        let source = fs::read_to_string(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        found.extend(production_lines_naming(&source, reaches_the_connection).into_iter().map(|n| format!("{shown}:{n}")));
    }
    assert!(found.is_empty(), "ask clawg-core for what these lines read from SQLite:\n{}", found.join("\n"));
}

#[test]
fn a_test_module_in_another_file_does_not_end_the_production_code() {
    let source = "#[cfg(test)]\nmod helpers;\n\nuse rusqlite::Connection;\n\n#[cfg(test)]\nmod tests {\n    use rusqlite::params;\n}\n";
    assert_eq!(production_lines_naming(source, reaches_the_connection), [4]);
    assert_eq!(production_lines_naming(&source.replace('\n', "\r\n"), reaches_the_connection), [4]);
}

/// Whether `source` names the module `module` of `clawg_core::memory`: in a
/// path `clawg_core::memory::<module>` that goes on or ends there, or as the
/// start of a path inside a group `clawg_core::memory::{…}`.
fn names_memory_module(source: &str, module: &str) -> bool {
    const ROOT: &str = "clawg_core::memory::";
    source.match_indices(ROOT).any(|(at, _)| {
        let rest = &source[at + ROOT.len()..];
        match rest.strip_prefix('{') {
            Some(group) => top_level_paths(group).iter().any(|path| starts_with_segment(path, module)),
            None => rest.strip_prefix(module).is_some_and(|after| after.starts_with("::") || after.starts_with([';', ',', '}'])),
        }
    })
}

/// The paths of a `use` group whose text after its `{` is `group`, each
/// trimmed, a nested group kept inside its path.
fn top_level_paths(group: &str) -> Vec<&str> {
    let (mut depth, mut from, mut paths) = (0usize, 0, Vec::new());
    for (at, c) in group.char_indices() {
        match c {
            '{' => depth += 1,
            '}' if depth == 0 => {
                paths.push(group[from..at].trim());
                return paths;
            }
            '}' => depth -= 1,
            ',' if depth == 0 => {
                paths.push(group[from..at].trim());
                from = at + 1;
            }
            _ => {}
        }
    }
    paths
}

fn starts_with_segment(path: &str, segment: &str) -> bool {
    path.strip_prefix(segment).is_some_and(|after| after.is_empty() || after.starts_with("::") || after.starts_with(' '))
}

#[test]
fn names_memory_module_reads_paths_and_groups() {
    for naming in [
        "use clawg_core::memory::registry::find_project;",
        "use clawg_core::memory::registry;",
        "let n = clawg_core::memory::registry::own_folder_count(store, id).await?;",
        "use clawg_core::memory::{clock::TimestampMs, registry::{self, Project}, MemoryStore};",
        "use clawg_core::memory::{\n    events::MemoryEvent,\n    registry,\n};",
    ] {
        assert!(names_memory_module(naming, "registry"), "{naming}");
    }
    for other in [
        "use clawg_core::memory::registry_of_something::Thing;",
        "use clawg_core::memory::{MemoryStore, RankedHit};",
        "use clawg_core::memory::{project::{registry, Other}};",
        "use clawg_core::memory::MemoryStore; // the registry",
    ] {
        assert!(!names_memory_module(other, "registry"), "{other}");
    }
}

/// What `clawg_core::memory` shows is what the other crates use: a module
/// none of them names is the core's own, and `pub(crate)` lets the compiler
/// say what in it nothing calls.
#[test]
fn every_public_memory_module_is_named_outside_core() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the crates directory");
    let mut files = Vec::new();
    for used_from in ["clawg-cli/src", "clawg-cli/tests", "clawg-eval/src", "clawg-core/tests"] {
        rust_files(&crates.join(used_from), &mut files);
    }
    let outside: Vec<String> =
        files.iter().map(|file| fs::read_to_string(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()))).collect();
    assert!(!outside.is_empty(), "no sources found under {}", crates.display());

    let root = read_repo_file("crates/clawg-core/src/memory/mod.rs");
    let public = public_memory_modules(&root);
    assert!(!public.is_empty(), "memory/mod.rs shows no module");

    let unnamed: Vec<&str> = public.into_iter().filter(|module| !outside.iter().any(|source| names_memory_module(source, module))).collect();
    assert!(unnamed.is_empty(), "no other crate names these modules of clawg_core::memory, so the root does not show them: {unnamed:?}");
}

/// The modules the root of `memory` shows to the other crates: each
/// `pub mod <module>;` it declares, and each `pub use <folder>::<module>;`
/// that re-exports a module of a folder it declares with `mod <folder>;`.
fn public_memory_modules(root: &str) -> Vec<&str> {
    let folders: Vec<&str> = root.lines().filter_map(|line| line.strip_prefix("mod ")?.strip_suffix(';')).collect();
    let is_module = |name: &str| !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    root.lines()
        .filter_map(|line| {
            if let Some(declared) = line.strip_prefix("pub mod ") {
                return declared.strip_suffix(';');
            }
            let (folder, module) = line.strip_prefix("pub use ")?.strip_suffix(';')?.split_once("::")?;
            (folders.contains(&folder) && is_module(module)).then_some(module)
        })
        .collect()
}

#[test]
fn public_memory_modules_reads_declarations_and_reexports() {
    let root = "pub mod portability;\nmod store;\npub(crate) mod hidden;\npub use store::db;\npub use store::db::MemoryStore;\npub(crate) use store::error;\npub use elsewhere::thing;\n";
    assert_eq!(public_memory_modules(root), ["portability", "db"]);
}

/// The folders of `crates/clawg-core/src/memory`, each with the folders its
/// production code may name. A row names only rows written above it, so the
/// table holds no cycle.
const MEMORY_FOLDERS: [(&str, &[&str]); 10] = [
    ("domain", &[]),
    ("store", &["domain"]),
    ("embedding", &["domain", "store"]),
    ("retrieval", &["domain", "store", "embedding"]),
    ("revision", &["domain", "store", "retrieval"]),
    ("project_docs", &["domain", "store", "embedding", "retrieval"]),
    ("projects", &["domain", "store", "embedding", "retrieval", "revision", "project_docs"]),
    ("ingest", &["domain", "store", "embedding", "retrieval", "revision", "projects"]),
    ("upkeep", &["domain", "store", "embedding", "retrieval", "revision", "project_docs", "projects"]),
    ("portability", &["domain", "store", "embedding", "retrieval", "revision", "projects", "ingest", "upkeep"]),
];

/// The folders that are one module each: the directory is the module.
const ONE_MODULE_FOLDERS: [&str; 2] = ["project_docs", "portability"];

/// The folders whose sources still name a module by the path the root of
/// `memory` re-exports (`crate::memory::chunker`), and not by its folder.
const NAMES_THROUGH_THE_ROOT: [&str; 1] = ["project_docs"];

/// The groups of modules of `memory` that depend on each other in a circle,
/// each inside one folder, with why it stays.
const MEMORY_CYCLES: [(&[&str], &str); 5] = [
    (
        &["store/backup", "store/db", "store/dedup", "store/error", "store/store_lock", "store/tier"],
        "MemoryError carries BackupError and StoreOwner, and the modules that define them, and the ones the store registers its SQL \
         functions from, fail with MemoryError",
    ),
    (
        &["retrieval/listing", "retrieval/pieces", "retrieval/search"],
        "a hit carries the place of its piece in a save, and a save is read back whole from the row of its head",
    ),
    (&["retrieval/prompt_builder", "retrieval/workspace"], "workspace.md is written with the prompt's indentation, and the prompt reads it"),
    (
        &["projects/inspect", "projects/merge", "projects/project_delete", "projects/relocate", "projects/subfolder"],
        "a merge, a move and a delete of a project run each other's steps inside one transaction",
    ),
    (&["projects/project", "projects/registry"], "registering a folder reads the registry's names, and the registry tells a project id by its shape"),
];

/// A module of `memory`: a source file or a directory of them.
struct MemoryModule {
    /// Empty for a module that lies in no folder of the table.
    folder: String,
    name: String,
    files: Vec<PathBuf>,
}

impl MemoryModule {
    fn shown(&self) -> String {
        if self.folder.is_empty() || self.folder == self.name { self.name.clone() } else { format!("{}/{}", self.folder, self.name) }
    }
}

fn is_memory_folder(name: &str) -> bool {
    MEMORY_FOLDERS.iter().any(|(folder, _)| *folder == name)
}

/// The modules under `memory`, the root's own file left out: each entry of a
/// folder of the table, a one-module folder as itself, and whatever else lies
/// beside them as a module in no folder.
fn memory_modules(memory: &Path) -> Vec<MemoryModule> {
    fn entries(dir: &Path) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())).flatten().map(|e| e.path()).collect();
        found.retain(|path| path.file_name().is_some_and(|name| name != "mod.rs") && (path.is_dir() || path.extension().is_some_and(|e| e == "rs")));
        found.sort();
        found
    }
    fn module(folder: &str, path: &Path) -> MemoryModule {
        let name = path.file_stem().expect("a file name").to_string_lossy().into_owned();
        let mut files = Vec::new();
        if path.is_dir() {
            rust_files(path, &mut files);
            files.sort();
        } else {
            files.push(path.to_path_buf());
        }
        MemoryModule { folder: folder.to_string(), name, files }
    }

    let mut modules = Vec::new();
    for path in entries(memory) {
        let name = path.file_stem().expect("a file name").to_string_lossy().into_owned();
        if path.is_dir() && is_memory_folder(&name) && !ONE_MODULE_FOLDERS.contains(&name.as_str()) {
            modules.extend(entries(&path).iter().map(|inside| module(&name, inside)));
        } else {
            modules.push(module(if is_memory_folder(&name) { &name } else { "" }, &path));
        }
    }
    modules
}

/// The paths `text` starts with, each as its segments: one path, or every
/// path of a `{…}` group, a nested group opened under what leads to it. A
/// `self` or a `*` ends a path at what leads to it.
fn leading_paths(text: &str) -> Vec<Vec<String>> {
    fn walk(text: &str, prefix: &[String], out: &mut Vec<Vec<String>>) {
        let text = text.trim_start();
        if let Some(group) = text.strip_prefix('{') {
            for path in top_level_paths(group) {
                walk(path, prefix, out);
            }
            return;
        }
        let end = text.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(text.len());
        let (segment, rest) = text.split_at(end);
        if segment.is_empty() || segment == "self" {
            out.push(prefix.to_vec());
            return;
        }
        let mut path = prefix.to_vec();
        path.push(segment.to_string());
        match rest.strip_prefix("::") {
            Some(rest) => walk(rest, &path, out),
            None => out.push(path),
        }
    }
    let mut out = Vec::new();
    walk(text, &[], &mut out);
    out
}

#[test]
fn leading_paths_opens_groups_under_what_leads_to_them() {
    let paths = |text: &str| -> Vec<String> { leading_paths(text).iter().map(|path| path.join("::")).collect() };
    assert_eq!(paths("store::db::MemoryStore;"), ["store::db::MemoryStore"]);
    assert_eq!(paths("search::reach_predicate!(\"a\", \"?1\")"), ["search::reach_predicate"]);
    assert_eq!(
        paths("{clock::TimestampMs, store::{db::{self, MemoryStore}, error::*},\n    MemoryError};"),
        ["clock::TimestampMs", "store::db", "store::db::MemoryStore", "store::error", "MemoryError"]
    );
}

/// The lines of the production code of `source`, a comment line left empty
/// so that the others keep their numbers.
fn production_code(source: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let code: Vec<&str> = lines[..production_end(&lines)].iter().map(|line| if line.trim_start().starts_with("//") { "" } else { *line }).collect();
    code.join("\n")
}

/// A path under `memory` that the production code of a source names, with
/// the line it is on and whether it is written from the crate's root.
struct MemoryPath {
    line: usize,
    segments: Vec<String>,
    absolute: bool,
}

/// Each path of `code` that starts at `crate::memory::`, and each one that
/// starts at a `super::` of the module `at` (its segments under `memory`).
fn memory_paths(code: &str, at: &[String]) -> Vec<MemoryPath> {
    const ROOT: &str = "crate::memory::";
    const SUPER: &str = "super::";
    let line_of = |offset: usize| code[..offset].matches('\n').count() + 1;
    let mut found = Vec::new();
    for (offset, _) in code.match_indices(ROOT) {
        for segments in leading_paths(&code[offset + ROOT.len()..]) {
            found.push(MemoryPath { line: line_of(offset), segments, absolute: true });
        }
    }
    let continues = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == ':';
    for (offset, _) in code.match_indices(SUPER) {
        if code[..offset].ends_with(continues) {
            continue;
        }
        let mut rest = &code[offset..];
        let mut base = at.to_vec();
        while let Some(after) = rest.strip_prefix(SUPER) {
            rest = after;
            base.pop();
        }
        for tail in leading_paths(rest) {
            found.push(MemoryPath { line: line_of(offset), segments: base.iter().cloned().chain(tail).collect(), absolute: false });
        }
    }
    found
}

#[test]
fn memory_paths_reads_absolute_and_relative_paths() {
    let code = "use crate::memory::{clock::TimestampMs, MemoryStore};\nuse super::token_splitter::TokenSplitter;\nlet x = super::super::vector::to_unit(v);\n";
    let at = ["embedding".to_string(), "chunker".to_string(), "chat".to_string()];
    let read: Vec<(usize, String, bool)> = memory_paths(code, &at).into_iter().map(|p| (p.line, p.segments.join("::"), p.absolute)).collect();
    assert_eq!(
        read,
        [
            (1, "clock::TimestampMs".to_string(), true),
            (1, "MemoryStore".to_string(), true),
            (2, "embedding::chunker::token_splitter::TokenSplitter".to_string(), false),
            (3, "embedding::vector::to_unit".to_string(), false),
        ]
    );
}

/// What each name the root of `memory` re-exports stands for: the last
/// segment of every path of its `use` statements, with the path.
fn root_reexports(root: &str) -> Vec<(String, Vec<String>)> {
    let mut found = Vec::new();
    let mut offset = 0;
    for line in root.split_inclusive('\n') {
        let statement = ["pub use ", "pub(crate) use ", "use "].iter().find_map(|opener| line.strip_prefix(opener));
        if statement.is_some() {
            let from = offset + line.find("use ").expect("the opener") + "use ".len();
            for path in leading_paths(&root[from..]) {
                if let Some(name) = path.last() {
                    found.push((name.clone(), path));
                }
            }
        }
        offset += line.len();
    }
    found
}

/// The module `path` (segments under `memory`) leads to, and whether the
/// path names its folder: `store::db::…`, the module `db` of the folder
/// `store`; `db::…`, the module `db` wherever it lies; `MemoryStore`, the
/// module the root re-exports it from.
fn module_named(modules: &[MemoryModule], reexports: &[(String, Vec<String>)], path: &[String]) -> Option<(usize, bool)> {
    let first = path.first()?;
    if modules.iter().any(|module| module.folder == *first) {
        let name = if ONE_MODULE_FOLDERS.contains(&first.as_str()) { first } else { path.get(1)? };
        return modules.iter().position(|module| module.folder == *first && module.name == *name).map(|at| (at, true));
    }
    if let Some(at) = modules.iter().position(|module| module.name == *first) {
        return Some((at, false));
    }
    let (_, target) = reexports.iter().find(|(name, target)| name == first && target.len() > 1)?;
    module_named(modules, &[], target).map(|(at, _)| (at, false))
}

#[test]
fn module_named_follows_a_folder_a_module_and_a_reexport() {
    let module = |folder: &str, name: &str| MemoryModule { folder: folder.to_string(), name: name.to_string(), files: Vec::new() };
    let modules = [module("store", "db"), module("store", "error"), module("portability", "portability"), module("", "loose")];
    let reexports = root_reexports("pub use store::db;\npub use store::db::MemoryStore;\npub(crate) use store::error::{\n    MemoryError,\n};\n");
    let named = |path: &str| {
        let path: Vec<String> = path.split("::").map(str::to_string).collect();
        module_named(&modules, &reexports, &path).map(|(at, by_folder)| (modules[at].shown(), by_folder))
    };
    assert_eq!(named("store::db::MemoryStore"), Some(("store/db".to_string(), true)));
    assert_eq!(named("portability::import::Journal"), Some(("portability".to_string(), true)));
    assert_eq!(named("db::MemoryStore"), Some(("store/db".to_string(), false)));
    assert_eq!(named("loose::thing"), Some(("loose".to_string(), false)));
    assert_eq!(named("MemoryStore"), Some(("store/db".to_string(), false)));
    assert_eq!(named("MemoryError"), Some(("store/error".to_string(), false)));
    assert_eq!(named("store::MemoryStore"), None);
    assert_eq!(named("nothing"), None);
}

/// Whether `file` is compiled only for the tests: the `mod.rs` beside it, or
/// the one of its directory, declares it under `#[cfg(test)]`.
fn declared_for_tests_only(file: &Path) -> bool {
    let is_root = file.file_name().is_some_and(|name| name == "mod.rs");
    let module = if is_root { file.parent().expect("a directory") } else { file };
    let Some(name) = module.file_stem().map(|stem| stem.to_string_lossy().into_owned()) else { return false };
    let Ok(declarer) = fs::read_to_string(module.with_file_name("mod.rs")) else { return false };
    let lines: Vec<&str> = declarer.lines().collect();
    lines.windows(2).any(|pair| pair[0].trim() == "#[cfg(test)]" && pair[1].trim().trim_start_matches("pub(crate) ").trim_start_matches("pub ") == format!("mod {name};"))
}

/// One module of `memory` naming another in its production code.
struct MemoryDependency {
    from: usize,
    to: usize,
    /// `<file under memory>:<line>`.
    at: String,
    /// The path names the folder of the module it leads to.
    by_folder: bool,
}

/// How a dependency between modules of `memory` is counted. Each source of a
/// module gives its production code (what comes before its first
/// `#[cfg(test)]` module, without the comment lines, and nothing when the
/// file itself is declared under `#[cfg(test)]`). In it, every path that
/// starts at `crate::memory::` or at a `super::` is a dependency on the
/// module it leads to, a name the root re-exports followed to its module.
/// A path into the module it is written in is none. The second list is the
/// `crate::memory::` paths that lead to nothing the tree or the root has.
fn memory_dependencies(memory: &Path, modules: &[MemoryModule]) -> (Vec<MemoryDependency>, Vec<String>) {
    let root = fs::read_to_string(memory.join("mod.rs")).expect("read memory/mod.rs");
    let reexports = root_reexports(&root);
    let (mut found, mut unknown) = (Vec::new(), Vec::new());
    for (from, module) in modules.iter().enumerate() {
        for file in module.files.iter().filter(|file| !declared_for_tests_only(file)) {
            let shown = file.strip_prefix(memory).unwrap_or(file).to_string_lossy().replace('\\', "/");
            let mut at: Vec<String> = shown.trim_end_matches(".rs").split('/').map(str::to_string).collect();
            if at.last().is_some_and(|last| last == "mod") {
                at.pop();
            }
            let source = fs::read_to_string(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
            for path in memory_paths(&production_code(&source), &at) {
                match module_named(modules, &reexports, &path.segments) {
                    Some((to, _)) if to == from => {}
                    Some((to, by_folder)) => found.push(MemoryDependency { from, to, at: format!("{shown}:{}", path.line), by_folder }),
                    None if path.absolute => unknown.push(format!("{shown}:{} crate::memory::{}", path.line, path.segments.join("::"))),
                    None => {}
                }
            }
        }
    }
    (found, unknown)
}

fn memory_dir() -> PathBuf {
    repo_root().join("crates/clawg-core/src/memory")
}

/// The tree of `memory` says what the system does, and a folder reaches only
/// what its row of [`MEMORY_FOLDERS`] lists: the storage knows nothing of
/// the search, and nothing below the ingest knows of it.
#[test]
fn a_memory_folder_names_only_the_folders_its_row_allows() {
    for (at, (folder, allowed)) in MEMORY_FOLDERS.iter().enumerate() {
        let above: Vec<&str> = MEMORY_FOLDERS[..at].iter().map(|(name, _)| *name).collect();
        assert!(allowed.iter().all(|name| above.contains(name)), "{folder} may name only folders written above it: {allowed:?}");
    }

    let memory = memory_dir();
    let modules = memory_modules(&memory);
    let loose: Vec<&str> = modules.iter().filter(|module| module.folder.is_empty()).map(|module| module.name.as_str()).collect();
    assert!(loose.is_empty(), "these modules of memory lie in no folder of the table: {loose:?}");
    let missing: Vec<&str> = MEMORY_FOLDERS.iter().map(|(folder, _)| *folder).filter(|folder| !modules.iter().any(|module| module.folder == *folder)).collect();
    assert!(missing.is_empty(), "memory has no folder {missing:?}");

    let (dependencies, unknown) = memory_dependencies(&memory, &modules);
    assert!(unknown.is_empty(), "these paths lead to no module of memory:\n{}", unknown.join("\n"));

    let allowed = |folder: &str| MEMORY_FOLDERS.iter().find(|(name, _)| *name == folder).map_or(&[][..], |(_, allowed)| *allowed);
    let mut refused = Vec::new();
    let mut through_the_root = Vec::new();
    for dependency in &dependencies {
        let (from, to) = (&modules[dependency.from], &modules[dependency.to]);
        if from.folder != to.folder && !allowed(&from.folder).contains(&to.folder.as_str()) {
            refused.push(format!("{} names {}", dependency.at, to.shown()));
        }
        if !dependency.by_folder && !NAMES_THROUGH_THE_ROOT.contains(&from.folder.as_str()) {
            through_the_root.push(format!("{} names {} without its folder", dependency.at, to.shown()));
        }
    }
    assert!(refused.is_empty(), "a folder of memory names one its row does not allow:\n{}", refused.join("\n"));
    assert!(through_the_root.is_empty(), "write the path with the folder (crate::memory::<folder>::<module>):\n{}", through_the_root.join("\n"));

    let mut unused = Vec::new();
    for (folder, allowed) in MEMORY_FOLDERS {
        for name in allowed {
            if !dependencies.iter().any(|d| modules[d.from].folder == folder && modules[d.to].folder == *name) {
                unused.push(format!("{folder} -> {name}"));
            }
        }
    }
    assert!(unused.is_empty(), "the table allows what no code uses; take it out of the row: {unused:?}");
}

/// The groups of more than one node that reach each other along `edges`,
/// each in order, the groups by their first node.
fn cycles(nodes: usize, edges: &[(usize, usize)]) -> Vec<Vec<usize>> {
    let mut reach = vec![vec![false; nodes]; nodes];
    for &(from, to) in edges {
        reach[from][to] = true;
    }
    for through in 0..nodes {
        let onward = reach[through].clone();
        for row in reach.iter_mut().filter(|row| row[through]) {
            for (to, reached) in onward.iter().enumerate() {
                row[to] |= reached;
            }
        }
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (node, onward) in reach.iter().enumerate() {
        if groups.iter().any(|group| group.contains(&node)) {
            continue;
        }
        let group: Vec<usize> = (node..nodes).filter(|&other| other == node || (onward[other] && reach[other][node])).collect();
        if group.len() > 1 {
            groups.push(group);
        }
    }
    groups
}

#[test]
fn cycles_groups_the_nodes_that_reach_each_other() {
    assert_eq!(cycles(4, &[(0, 1), (1, 2), (2, 3)]), Vec::<Vec<usize>>::new());
    assert_eq!(cycles(6, &[(0, 1), (1, 2), (2, 0), (2, 3), (4, 5), (5, 4), (3, 3)]), [vec![0, 1, 2], vec![4, 5]]);
}

/// No module of `memory` depends on itself through others, but for the
/// groups of [`MEMORY_CYCLES`]. Counted as [`memory_dependencies`] says;
/// a failure prints every group found, so the count can be read off it.
#[test]
fn the_memory_modules_cycle_only_where_listed() {
    let memory = memory_dir();
    let modules = memory_modules(&memory);
    let (dependencies, _) = memory_dependencies(&memory, &modules);
    let edges: Vec<(usize, usize)> = dependencies.iter().map(|d| (d.from, d.to)).collect();
    let mut found: Vec<Vec<String>> = cycles(modules.len(), &edges).iter().map(|group| group.iter().map(|&at| modules[at].shown()).collect()).collect();
    found.sort();
    let mut listed: Vec<Vec<String>> = MEMORY_CYCLES.iter().map(|(group, _)| group.iter().map(|name| name.to_string()).collect()).collect();
    listed.sort();

    let shown = |groups: &[Vec<String>]| groups.iter().map(|group| format!("  {} modules: {}", group.len(), group.join(", "))).collect::<Vec<_>>().join("\n");
    assert!(
        found == listed,
        "of the {} modules of memory, these depend on each other in a circle:\n{}\nand the listed circles are:\n{}",
        modules.len(),
        shown(&found),
        shown(&listed)
    );
}

/// A Windows checkout has CRLF line ends.
#[test]
fn the_production_code_ends_at_the_test_module_with_either_line_end() {
    let source = "use x;\nconst A: &str = \"old_table\";\n\n#[cfg(test)]\nmod tests {\n    const B: &str = \"old_table\";\n}\n";
    let names = |line: &str| line.contains("old_table");
    assert_eq!(production_lines_naming(source, names), [2]);
    assert_eq!(production_lines_naming(&source.replace('\n', "\r\n"), names), [2]);
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

/// Each `key = value` written inside `[section]` of a manifest, the value as
/// the raw text after the first `=`, without a trailing comment.
fn manifest_entries(manifest: &str, section: &str) -> Vec<(String, String)> {
    let mut inside = false;
    let mut entries = Vec::new();
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
            let value = value.split(" #").next().unwrap_or(value);
            entries.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    entries
}

/// The value written for `key` inside `[section]` of a manifest, as the raw
/// text after the `=`, without a trailing comment.
fn manifest_value(manifest: &str, section: &str, key: &str) -> Option<String> {
    manifest_entries(manifest, section).into_iter().find(|(name, _)| name == key).map(|(_, value)| value)
}

#[test]
fn every_package_takes_the_workspace_lints() {
    // Cargo applies [workspace.lints] only to a package whose [lints] says
    // `workspace = true`, and such a package cannot add lints of its own.
    let root = read_repo_file("Cargo.toml");
    let clippy = manifest_entries(&root, "workspace.lints.clippy");
    let level = |lint: &str| clippy.iter().find(|(name, _)| name == lint).map(|(_, value)| value.as_str());
    assert_eq!(level("all"), Some(r#"{ level = "warn", priority = -1 }"#), "the default group, below the lints named after it");
    for lint in ["large_futures", "allow_attributes", "allow_attributes_without_reason"] {
        assert_eq!(level(lint), Some(r#""warn""#), "clippy::{lint}");
    }
    let allowed: Vec<&str> = clippy.iter().filter(|(_, value)| value.contains(r#""allow""#)).map(|(name, _)| name.as_str()).collect();
    assert!(allowed.is_empty(), "an exception is an #[expect] where it applies, not a workspace allow: {allowed:?}");

    let members = manifest_value(&root, "workspace", "members").expect("the workspace lists its members");
    let members: Vec<&str> = members.split('"').skip(1).step_by(2).collect();
    assert!(!members.is_empty(), "no member read from {members:?}");
    for member in members {
        let manifest = read_repo_file(&format!("{member}/Cargo.toml"));
        assert_eq!(manifest_value(&manifest, "lints", "workspace").as_deref(), Some("true"), "{member} takes the workspace lints");
    }
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

// The workflow tests read the files as text, line by line, like the manifest
// tests above: a YAML parser would be a dependency with no other use. A
// reformat that keeps the meaning can break them, and is fixed in the same commit.

/// The lines of a workflow with comments removed: a whole-line comment becomes
/// empty and a trailing ` #` comment is cut.
fn workflow_code(name: &str) -> Vec<String> {
    read_repo_file(&format!(".github/workflows/{name}"))
        .lines()
        .map(|line| {
            if line.trim_start().starts_with('#') {
                String::new()
            } else {
                line.split(" #").next().unwrap_or(line).trim_end().to_string()
            }
        })
        .collect()
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

struct Job {
    name: String,
    lines: Vec<String>,
}

impl Job {
    fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// The value of the job's own `key:` (at the job's key indentation), with the
    /// continuation lines of a folded or literal block joined by spaces.
    fn key(&self, key: &str) -> Option<String> {
        let prefix = format!("{key}:");
        let at = self.lines.iter().position(|l| indent_of(l) == 4 && l.trim_start().starts_with(&prefix))?;
        let mut value = self.lines[at].trim_start()[prefix.len()..].trim().to_string();
        for line in &self.lines[at + 1..] {
            if line.trim().is_empty() {
                continue;
            }
            if indent_of(line) <= 4 {
                break;
            }
            value.push(' ');
            value.push_str(line.trim());
        }
        Some(value)
    }

    /// The steps of the job, each as its own lines.
    fn steps(&self) -> Vec<Vec<String>> {
        let mut steps: Vec<Vec<String>> = Vec::new();
        let mut inside = false;
        for line in &self.lines {
            if indent_of(line) == 4 && line.trim_start().starts_with("steps:") {
                inside = true;
                continue;
            }
            if !inside {
                continue;
            }
            if indent_of(line) == 6 && line.trim_start().starts_with("- ") {
                steps.push(Vec::new());
            }
            if let Some(step) = steps.last_mut() {
                step.push(line.clone());
            }
        }
        steps
    }
}

fn jobs_of(lines: &[String]) -> Vec<Job> {
    let mut jobs: Vec<Job> = Vec::new();
    let mut inside = false;
    for line in lines {
        if line == "jobs:" {
            inside = true;
            continue;
        }
        if !inside || line.trim().is_empty() {
            continue;
        }
        if indent_of(line) == 0 {
            break;
        }
        if indent_of(line) == 2 && line.trim_end().ends_with(':') {
            jobs.push(Job { name: line.trim().trim_end_matches(':').to_string(), lines: Vec::new() });
            continue;
        }
        if let Some(job) = jobs.last_mut() {
            job.lines.push(line.clone());
        }
    }
    jobs
}

fn job<'a>(jobs: &'a [Job], name: &str) -> &'a Job {
    jobs.iter().find(|j| j.name == name).unwrap_or_else(|| panic!("no job `{name}`"))
}

/// The file names in `.github/workflows`, sorted.
fn workflow_files() -> Vec<String> {
    let dir = repo_root().join(".github/workflows");
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| entry.expect("workflow entry").file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".yml") || name.ends_with(".yaml"))
        .collect();
    names.sort();
    names
}

#[test]
fn third_party_actions_are_pinned_to_a_commit() {
    for name in workflow_files() {
        let raw = read_repo_file(&format!(".github/workflows/{name}"));
        for (n, line) in raw.lines().enumerate() {
            let Some(at) = line.find("uses:") else { continue };
            if line.trim_start().starts_with('#') {
                continue;
            }
            let reference = line[at + "uses:".len()..].split_whitespace().next().unwrap_or("");
            if reference.starts_with("./") {
                continue;
            }
            let (_, pin) = reference.split_once('@').unwrap_or_else(|| panic!("{name}:{}: `{reference}` has no @ref", n + 1));
            let is_commit = pin.len() == 40 && pin.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
            assert!(is_commit, "{name}:{}: `{reference}` is not pinned to a full commit SHA", n + 1);
            assert!(line.contains(" # "), "{name}:{}: name the tag the SHA stands for in a trailing comment", n + 1);
        }
    }
}

/// Each `cargo clippy` command among a workflow's lines, joined with the lines
/// that continue it: a trailing `\` in bash, a trailing `` ` `` in PowerShell,
/// the default shell of a Windows runner.
fn clippy_commands(lines: &[String]) -> Vec<String> {
    let mut commands = Vec::new();
    let mut rest = lines.iter();
    while let Some(line) = rest.next() {
        if !line.contains("cargo clippy") {
            continue;
        }
        let mut command = line.trim().to_string();
        while let Some(head) = command.strip_suffix(['\\', '`']) {
            let next = rest.next().map_or("", |l| l.trim());
            command = format!("{} {next}", head.trim_end()).trim_end().to_string();
        }
        commands.push(command);
    }
    commands
}

/// The words of `command` that set a lint level.
fn lint_level_flags(command: &str) -> Vec<&str> {
    let sets_a_level = |word: &&str| {
        ["-A", "-W"].iter().any(|flag| word.starts_with(flag))
            || ["--allow", "--warn", "--cap-lints"].iter().any(|flag| *word == *flag || word.starts_with(&format!("{flag}=")))
    };
    command.split_whitespace().filter(sets_a_level).collect()
}

#[test]
fn clippy_commands_join_what_a_shell_continues() {
    let lines = [
        "      - run: cargo clippy --locked -- -D warnings",
        "      - run: |",
        "          cargo clippy --locked \\",
        "            --workspace \\",
        "            -- -A clippy::all",
        "          cargo test --locked",
        "      - run: cargo clippy `",
        "          --cap-lints=allow",
        "      - run: cargo test \\",
        "          --locked",
        "      - run: cargo clippy \\",
    ]
    .map(String::from);
    assert_eq!(
        clippy_commands(&lines),
        [
            "- run: cargo clippy --locked -- -D warnings",
            "cargo clippy --locked --workspace -- -A clippy::all",
            "- run: cargo clippy --cap-lints=allow",
            "- run: cargo clippy",
        ]
    );
}

#[test]
fn lint_level_flags_tells_a_level_from_other_flags() {
    let levels = "cargo clippy -A clippy::all -Wclippy::pedantic --allow a --warn=b --cap-lints allow --cap-lints=warn";
    assert_eq!(lint_level_flags(levels), ["-A", "-Wclippy::pedantic", "--allow", "--warn=b", "--cap-lints", "--cap-lints=warn"]);

    let fine = lint_level_flags("cargo clippy --fix --allow-dirty --locked --workspace --all-targets -p clawg-cli -- -D warnings");
    assert!(fine.is_empty(), "{fine:?}");
}

#[test]
fn no_workflow_turns_clippy_lints_off() {
    // Which lints run is [workspace.lints.clippy] of the root manifest; a
    // command only makes them fail.
    let mut seen = 0;
    for name in workflow_files() {
        for command in clippy_commands(&workflow_code(&name)) {
            let flags = lint_level_flags(&command);
            assert!(flags.is_empty(), "{name} runs `{command}` with {flags:?}");
            seen += 1;
        }
    }
    assert!(seen > 0, "no workflow runs clippy");
}

#[test]
fn every_checkout_drops_the_token() {
    let mut seen = 0;
    for name in workflow_files() {
        let lines = workflow_code(&name);
        for job in jobs_of(&lines) {
            for step in job.steps().iter().filter(|s| s.iter().any(|l| l.contains("uses: actions/checkout@"))) {
                assert!(
                    step.iter().any(|l| l.trim() == "persist-credentials: false"),
                    "{name}, job `{}`: checkout leaves the token in .git/config: {step:?}",
                    job.name
                );
                seen += 1;
            }
        }
    }
    assert!(seen > 0, "no workflow checks out the repository");
}

#[test]
fn the_release_workflow_publishes_only_for_a_successful_push_to_main() {
    let lines = workflow_code("release.yml");
    let has = |wanted: &str| lines.iter().any(|l| l.trim() == wanted);

    assert!(has("workflow_run:") && has("workflows: [CI]") && has("types: [completed]") && has("branches: [main]"));
    for other in ["pull_request", "pull_request_target", "push:", "workflow_dispatch", "schedule:"] {
        assert!(!lines.iter().any(|l| l.trim_start().starts_with(other)), "release.yml must not be triggered by `{other}`");
    }
    let permissions = lines.iter().position(|l| l == "permissions:").expect("top-level permissions");
    assert_eq!(lines[permissions + 1].trim(), "contents: read");
    let concurrency = lines.iter().position(|l| l == "concurrency:").expect("top-level concurrency");
    assert_eq!(lines[concurrency + 1].trim(), "group: release");
    assert_eq!(lines[concurrency + 2].trim(), "cancel-in-progress: false");

    let jobs = jobs_of(&lines);
    let plan = job(&jobs, "plan");
    let condition = plan.key("if").expect("the plan job is conditional");
    for wanted in [
        "github.event.workflow_run.conclusion == 'success'",
        "github.event.workflow_run.event == 'push'",
        "github.event.workflow_run.head_branch == 'main'",
        "github.event.workflow_run.head_repository.full_name == github.repository",
    ] {
        assert!(condition.contains(wanted), "plan runs without `{wanted}`: {condition}");
    }
    let text = plan.text();
    assert!(text.contains("ref: ${{ github.event.workflow_run.head_sha }}"), "plan must read the commit CI ran on");
    assert!(text.contains("refs/tags/v"), "plan must look for the version's tag");
    for other in jobs.iter().filter(|j| j.name != "plan") {
        assert!(other.key("needs").is_some_and(|n| n.contains("plan")), "job `{}` must wait for plan", other.name);
    }
}

#[test]
fn the_release_workflow_builds_only_the_cli_with_the_dist_mark() {
    let lines = workflow_code("release.yml");
    let jobs = jobs_of(&lines);
    let build = job(&jobs, "build");

    assert!(build.key("needs").is_some_and(|n| n.contains("plan")));
    assert!(build.key("if").is_some_and(|c| c.contains("needs.plan.outputs.publish == 'true'")));
    assert!(!lines.iter().any(|l| l.contains("-latest")), "runners are pinned, not `-latest`");
    for runner in ["os: windows-2022", "os: ubuntu-24.04"] {
        assert!(lines.iter().any(|l| l.trim().trim_start_matches("- ") == runner), "the matrix lacks `{runner}`");
    }
    for target in ["target: x86_64-pc-windows-msvc", "target: x86_64-unknown-linux-gnu"] {
        assert!(lines.iter().any(|l| l.trim() == target), "the matrix lacks `{target}`");
    }

    let builds: Vec<&String> = lines.iter().filter(|l| l.contains("cargo build")).collect();
    assert!(!builds.is_empty(), "release.yml builds nothing");
    for command in &builds {
        assert!(command.contains("-p clawg-cli"), "builds only the CLI: {command}");
        assert!(command.contains("--release") && command.contains("--locked"), "{command}");
        assert!(command.contains("--target ${{ matrix.target }}"), "{command}");
        assert!(!command.contains("--workspace") && !command.contains("--all"), "{command}");
    }
    for step in build.steps().iter().filter(|s| s.iter().any(|l| l.contains("cargo build"))) {
        assert!(
            step.iter().any(|l| l.trim() == "CLAWG_DIST_TARGET: ${{ matrix.target }}"),
            "the build step must set CLAWG_DIST_TARGET: {step:?}"
        );
        // ort-sys downloads ONNX Runtime while building; one failed request
        // must not stop a release.
        let text = step.join("\n");
        assert!(text.contains("for attempt in 1 2 3") && text.contains("sleep"), "the build is retried: {text}");
    }

    let steps = build.steps();
    let smoke = steps
        .iter()
        .find(|s| s.iter().any(|l| l.contains("--version")))
        .expect("a step runs the built binary with --version");
    let smoke = smoke.join("\n");
    assert!(smoke.contains("clawg ${VERSION}"), "the smoke test compares with the planned version");
    assert!(smoke.contains("CLAWG_TEST_"), "the smoke test looks for the test stand-ins in the binary");

    let linkage = steps.iter().find(|s| s.iter().any(|l| l.contains("ldd"))).expect("a step runs ldd");
    let linkage = linkage.join("\n");
    assert!(linkage.contains("libssl") && linkage.contains("libcrypto"));
    assert!(linkage.contains("runner.os == 'Linux'"));
}

#[test]
fn the_release_workflow_publishes_last() {
    let lines = workflow_code("release.yml");
    let jobs = jobs_of(&lines);
    let publish = job(&jobs, "publish");

    let needs = publish.key("needs").expect("publish waits for other jobs");
    assert!(needs.contains("plan") && needs.contains("build"), "publish needs plan and build: {needs}");
    assert!(publish.key("if").is_some_and(|c| c.contains("needs.plan.outputs.publish == 'true'")));

    let only_in_publish = |needle: &str| {
        for other in jobs.iter().filter(|j| j.name != "publish") {
            assert!(!other.text().contains(needle), "`{needle}` appears in job `{}`", other.name);
        }
    };
    only_in_publish("gh release");
    only_in_publish("environment:");
    only_in_publish("contents: write");
    only_in_publish("id-token: write");
    only_in_publish("attestations: write");
    only_in_publish("secrets.");
    assert_eq!(publish.key("environment").as_deref(), Some("release"));

    let text = publish.text();
    assert!(text.contains("gh release create"));
    assert!(text.contains("--target ${TARGET_SHA}") && text.contains("TARGET_SHA: ${{ github.event.workflow_run.head_sha }}"));
    for asset in [
        "dist/clawg-x86_64-pc-windows-msvc.exe",
        "dist/clawg-x86_64-unknown-linux-gnu",
        "dist/install.ps1",
        "dist/SHA256SUMS",
        "dist/SHA256SUMS.minisig",
    ] {
        let uploads = text.lines().filter(|l| l.trim().trim_end_matches('\\').trim() == asset).count();
        assert!(uploads >= 2, "`{asset}` must be attested and uploaded; found {uploads}");
    }

    let position = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("publish lacks `{needle}`"));
    assert!(position("minisign -S") < position("minisign -Vm dist/SHA256SUMS -p release/minisign.pub"));
    assert!(position("minisign -Vm") < position("actions/attest-build-provenance"));
    assert!(position("actions/attest-build-provenance") < position("gh release create"));
}

#[test]
fn the_release_is_drafted_and_published_by_a_repeated_request() {
    let lines = workflow_code("release.yml");
    let jobs = jobs_of(&lines);
    let publish = job(&jobs, "publish");
    let steps = publish.steps();
    let step = |needle: &str| {
        steps
            .iter()
            .find(|s| s.iter().any(|l| l.contains(needle)))
            .unwrap_or_else(|| panic!("no step runs `{needle}`"))
            .join("\n")
    };

    // Without --draft, gh deletes the draft it made when publishing fails, so one
    // 5xx on that last request threw away every upload.
    let create = step("gh release create");
    assert!(create.contains("--draft") && !create.contains("--draft=false"), "the release is created as a draft: {create}");

    let publication = step("--draft=false");
    assert!(publication.contains("gh release edit \"v${VERSION}\""), "{publication}");
    assert!(publication.contains("for attempt in"), "a failed publication is repeated: {publication}");
    assert!(publication.contains("--json isDraft"), "a 5xx can come after GitHub published, so the release is read back: {publication}");

    let cleanup = step("gh release delete");
    for outcome in ["steps.create.outcome == 'failure'", "steps.publish.outcome == 'failure'"] {
        assert!(cleanup.contains(outcome), "the cleanup runs when `{outcome}`: {cleanup}");
    }
    let reads = cleanup.find("--json isDraft").expect("the cleanup reads whether the release is still a draft");
    let deletes = cleanup.find("gh release delete").expect("the cleanup deletes the draft");
    assert!(reads < deletes, "only a draft is deleted: {cleanup}");

    let text = publish.text();
    let position = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("publish lacks `{needle}`"));
    assert!(position("gh release create") < position("gh release edit"));
    assert!(position("gh release edit") < position("gh release delete"));
}

#[test]
fn the_signing_step_reads_the_key_password_from_stdin_without_echo() {
    let lines = workflow_code("release.yml");
    assert!(!lines.iter().any(|l| l.contains("set -x") || l.contains("xtrace")), "the shell trace would print secrets");

    let jobs = jobs_of(&lines);
    let publish = job(&jobs, "publish");
    let steps = publish.steps();
    let signing = steps.iter().find(|s| s.iter().any(|l| l.contains("minisign -S"))).expect("a step signs");

    let pipe = "printf '%s\\n' \"$MINISIGN_PASSWORD\" | minisign -S -s \"$key\" -t \"clawg v${VERSION} SHA256SUMS\" -m SHA256SUMS";
    assert!(signing.iter().any(|l| l.trim() == pipe), "the password goes in on stdin: {signing:?}");
    assert!(!signing.iter().any(|l| l.split_whitespace().any(|w| w == "-W")), "-W would sign without a password");
    assert_eq!(signing.iter().filter(|l| l.contains("$MINISIGN_PASSWORD")).count(), 1, "the password is used once, on stdin");
    assert!(!signing.iter().any(|l| l.contains("echo") && l.contains("MINISIGN_")), "a secret is echoed");
    for secret in ["MINISIGN_SECRET_KEY: ${{ secrets.MINISIGN_SECRET_KEY }}", "MINISIGN_PASSWORD: ${{ secrets.MINISIGN_PASSWORD }}"] {
        assert!(signing.iter().any(|l| l.trim() == secret), "the signing step lacks `{secret}`");
    }
    assert!(signing.iter().any(|l| l.contains("umask 077") || l.contains("chmod 600")), "the key file must be private");
    assert!(signing.iter().any(|l| l.contains("trap") && l.contains("rm -f")), "the key file must be removed at the end");

    for (n, line) in lines.iter().enumerate() {
        let outside = !signing.contains(line);
        assert!(!(outside && line.contains("MINISIGN_PASSWORD")), "release.yml:{}: the password leaves the signing step", n + 1);
        assert!(!(outside && line.contains("MINISIGN_SECRET_KEY")), "release.yml:{}: the key leaves the signing step", n + 1);
        assert!(!(outside && line.contains("secrets.")), "release.yml:{}: a secret outside the signing step", n + 1);
    }
}

/// The quoted entries of the `paths-ignore:` list under `event` in `on:`.
fn ignored_paths(lines: &[String], event: &str) -> Vec<String> {
    let header = format!("{event}:");
    let Some(start) = lines.iter().position(|l| indent_of(l) == 2 && l.trim() == header) else {
        panic!("ci.yml has no `{event}` trigger");
    };
    let section: Vec<&String> = lines[start + 1..]
        .iter()
        .take_while(|l| l.trim().is_empty() || indent_of(l) > 2)
        .collect();
    let list = section
        .iter()
        .position(|l| l.trim() == "paths-ignore:")
        .unwrap_or_else(|| panic!("`{event}` has no paths-ignore"));
    section[list + 1..]
        .iter()
        .take_while(|l| l.trim().starts_with("- "))
        .map(|l| l.trim().trim_start_matches("- ").trim_matches('\'').to_string())
        .collect()
}

#[test]
fn the_ci_workflow_ignores_only_non_code_paths_and_runs_on_both_platforms() {
    let lines = workflow_code("ci.yml");
    let non_code = ["docs/**", "openspec/**", ".claude/**", ".cursor/**", ".gemini/**", "scripts/**", "README.md", "AGENTS.md"];
    for event in ["push", "pull_request"] {
        assert_eq!(ignored_paths(&lines, event), non_code, "paths ignored on {event}");
    }
    let has = |wanted: &str| lines.iter().any(|l| l.trim() == wanted);
    assert!(has("branches: [main]"), "the push trigger is main");
    assert!(has("os: [ubuntu-latest, windows-latest]"), "both platforms run");
    assert!(has("fail-fast: false"), "one platform failing must not stop the other");
    assert!(has("- run: cargo clippy --locked --workspace --all-targets -- -D warnings"));
    assert!(has("- run: cargo test --locked --workspace"));

    // CI is the only trigger of the release and must never hold its secrets.
    for forbidden in ["secrets.", "environment:", "id-token", "contents: write"] {
        assert!(!lines.iter().any(|l| l.contains(forbidden)), "ci.yml must not use `{forbidden}`");
    }
}

/// `X.Y` or `X.Y.Z` as three numbers, a missing patch read as 0.
fn rust_version(text: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<u32> = text.split('.').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    match parts[..] {
        [major, minor] => Some((major, minor, 0)),
        [major, minor, patch] => Some((major, minor, patch)),
        _ => None,
    }
}

/// The highest `rust-version` among the packages in Cargo.lock, today
/// `tree-sitter-language` 0.1.9's. Reading it needs the cargo registry, so it
/// is written here and checked again when a dependency is raised.
const DEPENDENCIES_NEED_RUST: (u32, u32, u32) = (1, 90, 0);

#[test]
fn the_toolchain_is_pinned_once_for_every_workflow() {
    let file = read_repo_file("rust-toolchain.toml");
    let channel = manifest_value(&file, "toolchain", "channel").expect("rust-toolchain.toml names a channel");
    let channel = channel.trim_matches('"');
    let pinned = rust_version(channel).filter(|_| channel.split('.').count() == 3);
    let pinned = pinned.unwrap_or_else(|| panic!("the toolchain is an exact X.Y.Z, not `{channel}`"));
    let components = manifest_value(&file, "toolchain", "components").unwrap_or_default();
    assert!(components.contains("\"clippy\""), "the toolchain brings clippy: {components}");

    let mut seen = 0;
    for name in workflow_files() {
        let lines = workflow_code(&name);
        for job in jobs_of(&lines) {
            for step in job.steps().iter().filter(|s| s.iter().any(|l| l.contains("uses: dtolnay/rust-toolchain@"))) {
                let given = step.iter().find_map(|l| l.trim().strip_prefix("toolchain:")).map(str::trim);
                assert_eq!(given, Some(channel), "{name}, job `{}` installs another toolchain than rust-toolchain.toml", job.name);
                seen += 1;
            }
        }
    }
    assert!(seen > 0, "no workflow installs the toolchain");

    let manifest = read_repo_file("Cargo.toml");
    let declared = manifest_value(&manifest, "workspace.package", "rust-version").expect("the workspace declares rust-version");
    let declared = rust_version(declared.trim_matches('"')).unwrap_or_else(|| panic!("rust-version `{declared}` is not a version"));
    assert!(declared >= DEPENDENCIES_NEED_RUST, "rust-version {declared:?} is below what the dependencies need, {DEPENDENCIES_NEED_RUST:?}");
    assert!(declared <= pinned, "rust-version {declared:?} is above the pinned toolchain {pinned:?}");
}

#[test]
fn only_the_ci_workflow_gates_the_release() {
    // release.yml runs after a workflow named CI; any other workflow with that
    // name would gate, or trigger, a release too.
    for name in workflow_files() {
        let lines = workflow_code(&name);
        let named_ci = lines.iter().any(|l| l.trim() == "name: CI" && indent_of(l) == 0);
        assert_eq!(named_ci, name == "ci.yml", "{name}: only ci.yml is named CI");
    }
}

/// The `env:` value of `key` in a job, as written.
fn job_env(job: &Job, key: &str) -> String {
    let prefix = format!("{key}:");
    job.lines
        .iter()
        .find_map(|l| l.trim().strip_prefix(&prefix).map(|v| v.trim().to_string()))
        .unwrap_or_else(|| panic!("job `{}` sets no `{key}`", job.name))
}

/// Asserts a workflow downloads a tool from `url_start` with an exact version
/// and checks it against a SHA-256 kept in `hash_key`.
fn assert_pinned_download(name: &str, job: &Job, url_key: &str, url_start: &str, version: &str, hash_key: &str) {
    let url = job_env(job, url_key);
    assert!(url.starts_with(url_start), "{name}: {url_key} is `{url}`");
    assert!(url.contains(&format!("/{version}/")), "{name}: {url_key} names version {version}: {url}");
    let hash = job_env(job, hash_key);
    assert!(hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()), "{name}: {hash_key} is a SHA-256: {hash}");
}

fn assert_reads_no_secret(name: &str, lines: &[String]) {
    for forbidden in ["secrets.", "environment:", "id-token", "contents: write"] {
        assert!(!lines.iter().any(|l| l.contains(forbidden)), "{name} must not use `{forbidden}`");
    }
}

#[test]
fn the_advisories_workflow_runs_a_pinned_cargo_deny_apart_from_ci() {
    let name = "advisories.yml";
    let lines = workflow_code(name);
    let has = |wanted: &str| lines.iter().any(|l| l.trim() == wanted);
    for trigger in ["pull_request:", "push:", "schedule:"] {
        assert!(lines.iter().any(|l| indent_of(l) == 2 && l.trim() == trigger), "{name} runs on `{trigger}`");
    }
    for path in ["- '**/Cargo.toml'", "- 'Cargo.lock'", "- 'deny.toml'", "- '.github/workflows/advisories.yml'"] {
        let listed = lines.iter().filter(|l| l.trim() == path).count();
        assert_eq!(listed, 2, "{name}: pull requests and pushes both watch `{path}`");
    }
    assert!(has("branches: [main]"));
    assert_reads_no_secret(name, &lines);

    let jobs = jobs_of(&lines);
    let advisories = job(&jobs, "advisories");
    assert_pinned_download(
        name,
        advisories,
        "CARGO_DENY_URL",
        "https://github.com/EmbarkStudios/cargo-deny/releases/download/",
        "0.20.2",
        "CARGO_DENY_SHA256",
    );
    let text = advisories.text();
    assert!(text.contains("sha256sum --check"), "{name} checks the download before running it");
    let commands = run_commands(advisories);
    assert!(commands.iter().any(|c| c == "cargo deny --locked check advisories"), "{name}: {commands:?}");

    let deny = read_repo_file("deny.toml");
    assert!(deny.lines().any(|l| l.trim() == "[advisories]"), "deny.toml configures the advisories");
    for line in deny.lines().filter(|l| l.contains("id = \"RUSTSEC-")) {
        assert!(line.contains("reason = \""), "an ignored advisory says why: {line}");
    }
}

#[test]
fn the_installer_is_analyzed_with_a_pinned_psscriptanalyzer() {
    let name = "installer.yml";
    let lines = workflow_code(name);
    for path in ["- 'scripts/**'", "- '.github/workflows/installer.yml'"] {
        let listed = lines.iter().filter(|l| l.trim() == path).count();
        assert_eq!(listed, 2, "{name}: pull requests and pushes both watch `{path}`");
    }
    assert_reads_no_secret(name, &lines);

    let jobs = jobs_of(&lines);
    let analyze = job(&jobs, "analyze");
    assert!(analyze.key("runs-on").is_some_and(|r| r.starts_with("windows-")), "{name} runs on Windows");
    assert_pinned_download(
        name,
        analyze,
        "PSSA_URL",
        "https://github.com/PowerShell/PSScriptAnalyzer/releases/download/",
        "1.25.0",
        "PSSA_SHA256",
    );
    let text = analyze.text();
    let position = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("{name} lacks `{needle}`"));
    assert!(position("Get-FileHash -Algorithm SHA256") < position("Import-Module"), "{name} checks the module before loading it");
    assert!(text.contains("Invoke-ScriptAnalyzer -Path scripts -Recurse -Settings scripts/PSScriptAnalyzerSettings.psd1"));
    assert!(text.contains("if ($findings.Count -gt 0) { throw"), "{name}: any finding fails the run");

    let settings = read_repo_file("scripts/PSScriptAnalyzerSettings.psd1");
    let settings: Vec<&str> = settings.lines().map(str::trim).collect();
    for (n, rule) in settings.iter().enumerate().filter(|(_, l)| l.starts_with("'PS")) {
        assert!(n > 0 && settings[n - 1].starts_with('#'), "the excluded rule {rule} says why in a comment above it");
    }
}

#[test]
fn dependabot_updates_the_pinned_actions_on_develop() {
    let config = read_repo_file(".github/dependabot.yml");
    let lines: Vec<&str> = config.lines().map(str::trim).filter(|l| !l.starts_with('#')).collect();
    let ecosystems: Vec<&str> = lines.iter().filter_map(|l| l.strip_prefix("- package-ecosystem:")).map(str::trim).collect();
    assert_eq!(ecosystems, ["github-actions"], "crates are left to the Advisories workflow");
    for wanted in ["directory: /", "target-branch: develop", "interval: monthly"] {
        assert!(lines.contains(&wanted), "dependabot.yml lacks `{wanted}`");
    }
}

/// The command of every `run:` step of a job, single-line ones only.
fn run_commands(job: &Job) -> Vec<String> {
    job.steps()
        .iter()
        .flat_map(|step| step.iter())
        .filter_map(|l| {
            let l = l.trim().trim_start_matches("- ");
            l.strip_prefix("run: ").map(|c| c.trim().to_string())
        })
        .collect()
}

#[test]
fn the_ci_workflow_checks_the_locked_production_build() {
    let lines = workflow_code("ci.yml");
    let jobs = jobs_of(&lines);
    let commands = run_commands(job(&jobs, "test"));

    // A Cargo.lock that trails Cargo.toml must fail here, not in release.yml.
    for command in commands.iter().filter(|c| c.starts_with("cargo ")) {
        assert!(command.contains(" --locked"), "ci.yml runs `{command}` without --locked");
    }
    // Without --all-targets no dev-dependency, and so no `testing`, reaches
    // clawg-core: the configuration release.yml builds.
    for wanted in ["cargo clippy --locked -p clawg-cli -- -D warnings", "cargo test --locked -p clawg-core --lib build_info"] {
        assert!(commands.iter().any(|c| c == wanted), "ci.yml lacks `{wanted}`: {commands:?}");
    }

    // Pull requests only read the cache, which cache.yml writes from main, so
    // nothing a pull request builds is restored into a run on main.
    let cache = rust_cache_step(job(&jobs, "test")).expect("ci.yml caches the build");
    assert!(cache.iter().any(|l| l.trim() == "save-if: false"), "a pull request writes the cache: {cache:?}");
}

/// The `Swatinem/rust-cache` step of a job.
fn rust_cache_step(job: &Job) -> Option<Vec<String>> {
    job.steps().into_iter().find(|s| s.iter().any(|l| l.contains("uses: Swatinem/rust-cache@")))
}

/// The step of a job whose `run:` is `command`.
fn step_running(job: &Job, command: &str) -> Vec<String> {
    let wanted = format!("run: {command}");
    job.steps()
        .into_iter()
        .find(|s| s.iter().any(|l| l.trim().trim_start_matches("- ") == wanted))
        .unwrap_or_else(|| panic!("job `{}` does not run `{command}`", job.name))
}

#[test]
fn the_ci_workflow_runs_the_suite_only_for_pull_requests() {
    // A push to main merges a pull request whose run tested the same content.
    // release.yml waits for a successful CI run of that push, which a run whose
    // jobs were all skipped is not, so one job runs there and builds nothing.
    let lines = workflow_code("ci.yml");
    let jobs = jobs_of(&lines);
    let test = job(&jobs, "test");
    assert_eq!(test.key("if").as_deref(), Some("github.event_name == 'pull_request'"), "the suite runs for pull requests alone");
    let merged = job(&jobs, "merged");
    assert_eq!(merged.key("if").as_deref(), Some("github.event_name == 'push'"), "a push to main runs `merged`");
    assert!(!merged.steps().is_empty(), "`merged` runs a step");
    assert!(!merged.text().contains("cargo ") && !merged.text().contains("uses:"), "`merged` builds nothing");

    let build_info = step_running(test, "cargo test --locked -p clawg-core --lib build_info");
    assert!(build_info.iter().any(|l| l.trim() == "if: runner.os == 'Linux'"), "build_info has no platform code: {build_info:?}");
}

#[test]
fn the_cache_workflow_builds_from_main_what_ci_tests() {
    // rust-cache keys a cache by job id, runner, CARGO_/RUST environment and
    // lock, never by workflow, so this `test` job writes the cache ci.yml's
    // `test` job reads, building the same targets without running them.
    let name = "cache.yml";
    let lines = workflow_code(name);
    for trigger in ["push:", "schedule:", "workflow_dispatch:"] {
        assert!(lines.iter().any(|l| indent_of(l) == 2 && l.trim() == trigger), "{name} runs on `{trigger}`");
    }
    assert!(!lines.iter().any(|l| l.trim_start().starts_with("pull_request")), "{name} never runs for a pull request");
    assert!(lines.iter().any(|l| l.trim() == "branches: [main]"), "{name} runs on main");
    assert_reads_no_secret(name, &lines);

    let ci_lines = workflow_code("ci.yml");
    let ci_jobs = jobs_of(&ci_lines);
    let ci = job(&ci_jobs, "test");
    let jobs = jobs_of(&lines);
    let cache = job(&jobs, "test");
    assert_eq!(cache.key("runs-on"), ci.key("runs-on"));
    let os = |job: &Job| job.lines.iter().find(|l| l.trim().starts_with("os: [")).map(|l| l.trim().to_string());
    assert_eq!(os(cache), os(ci), "{name} builds on the platforms CI tests");
    for key in ["CARGO_TERM_COLOR", "CARGO_INCREMENTAL"] {
        assert_eq!(job_env(cache, key), job_env(ci, key), "{name}: `{key}` is part of the cache key");
    }

    let expected: Vec<String> = run_commands(ci)
        .into_iter()
        .map(|c| if c.starts_with("cargo test ") { format!("{c} --no-run") } else { c })
        .collect();
    assert_eq!(run_commands(cache), expected, "{name} builds what ci.yml tests");
    let build_info = step_running(cache, "cargo test --locked -p clawg-core --lib build_info --no-run");
    assert!(build_info.iter().any(|l| l.trim() == "if: runner.os == 'Linux'"), "{name} builds build_info where CI tests it");

    let step = rust_cache_step(cache).expect("cache.yml caches the build");
    assert!(step.iter().any(|l| l.trim() == "save-if: ${{ github.ref == 'refs/heads/main' }}"), "the cache is written from any ref: {step:?}");
}

#[test]
fn the_readme_documents_the_install_and_update_path() {
    let readme = read_repo_file("README.md");
    for (wanted, why) in [
        ("irm https://github.com/MarcossIC/claw-garden/releases/latest/download/install.ps1 | iex", "the one-line install"),
        ("glibc 2.39", "the oldest glibc the Linux binary runs on"),
        ("SmartScreen", "the warning of an unsigned executable"),
        ("Smart App Control", "the policy that can block it"),
        ("minisign -Vm SHA256SUMS", "how to verify what was downloaded"),
        ("release/minisign.pub", "the repository key"),
        ("release/minisign-next.pub", "the backup key"),
        ("no protege", "the backup key does not protect binaries that never update"),
        ("## Actualizar", "the update section"),
        ("clawg update --check", "how to ask whether there is a newer release"),
        ("clawg update", "how to update"),
        ("clawg stop", "the first step of going back by hand"),
        (".old-", "the previous binary that going back by hand puts back"),
        ("clawg restore", "how to give the store back its snapshot"),
        ("pre-migration", "the snapshot taken before a migration"),
    ] {
        assert!(readme.contains(wanted), "README.md lacks {why}: `{wanted}`");
    }
}

/// `install.ps1` and `clawg update` set the installed binary aside under one
/// name, and each deletes what the other left behind.
#[test]
fn the_installer_and_update_set_the_binary_aside_under_one_name() {
    let script = read_repo_file("scripts/install.ps1");
    for (wanted, why) in [
        ("$target = Join-Path $InstallDir 'clawg.exe'", "the installed binary"),
        (r#"$aside = "$target.old-$(Get-Date -Format yyyyMMddHHmmssfff)""#, "the name it is set aside under"),
        ("-Filter 'clawg.exe.old-*'", "the cleanup of what earlier runs set aside"),
    ] {
        assert!(script.contains(wanted), "install.ps1 lacks {why}: `{wanted}`");
    }

    let by_update = plan::aside_name("clawg.exe", 1_791_827_476_123);
    let stamp = by_update.strip_prefix("clawg.exe.old-").expect("the prefix install.ps1 deletes");
    assert!(
        stamp.len() == "yyyyMMddHHmmssfff".len() && stamp.bytes().all(|b| b.is_ascii_digit()),
        "{by_update}"
    );
    let by_script = vec!["clawg.exe.old-20261012145116123".to_string()];
    assert_eq!(plan::leftovers(&by_script, "clawg.exe"), by_script, "update deletes what install.ps1 set aside");
}

/// The tests of `source` that open a runtime on disk and never close it. Read
/// as text, like the manifests above: one `.close().await` is enough, so a test
/// with two runtimes can still leave one open.
fn tests_leaving_a_store_open(source: &str) -> Vec<&str> {
    source
        .split("#[tokio::test")
        .skip(1)
        .filter(|test| test.contains("MemoryRuntime::open(") && !test.contains(".close().await"))
        .filter_map(|test| test.split_once("fn ")?.1.split_once('('))
        .map(|(name, _)| name)
        .collect()
}

#[test]
fn tests_leaving_a_store_open_names_each_test_without_a_close() {
    let source = r#"
#[tokio::test]
async fn stays_in_memory() {
    let (rt, _rx) = MemoryRuntime::new_for_test(base).await;
}

#[tokio::test]
async fn closes_what_it_opens() {
    let (rt, _rx) = MemoryRuntime::open(base, StoreHolder::Daemon).await.unwrap();
    rt.close().await;
}

#[tokio::test]
async fn only_shuts_down() {
    let (rt, rx) = MemoryRuntime::open(base, StoreHolder::Daemon).await.unwrap();
    rt.shutdown(start(&rt, rx)).await;
}

#[tokio::test(start_paused = true)]
async fn only_drops_it() {
    let (rt, _rx) = MemoryRuntime::open(base, StoreHolder::Daemon).await.unwrap();
}
"#;
    assert_eq!(tests_leaving_a_store_open(source), ["only_shuts_down", "only_drops_it"]);
}

#[test]
fn an_integration_test_closes_the_store_it_opens_on_disk() {
    // Dropping a runtime only signals its connection's thread, and on Windows
    // `TempDir` gives up at once on a file still open: the store stays in the
    // system temp dir and no test fails, so nothing else would say it.
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the crates directory");
    let mut files = Vec::new();
    for krate in fs::read_dir(crates).expect("read crates/").flatten() {
        rust_files(&krate.path().join("tests"), &mut files);
    }
    let this_file = Path::new(file!()).file_name().expect("this file has a name");
    files.retain(|file| file.file_name() != Some(this_file));
    assert!(!files.is_empty(), "no integration tests found under {}", crates.display());

    let mut found = Vec::new();
    for file in &files {
        let source = fs::read_to_string(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        for name in tests_leaving_a_store_open(&source) {
            found.push(format!("{}: {name}", file.strip_prefix(crates).unwrap_or(file).display()));
        }
    }
    assert!(found.is_empty(), "call MemoryRuntime::close before the test ends:\n{}", found.join("\n"));
}

/// The base64 line of a minisign `.pub`, which names the key whatever its
/// comment says.
fn key_line(public_key: &str) -> &str {
    public_key.lines().nth(1).unwrap_or_default().trim()
}

#[test]
fn the_release_keys_are_two_distinct_minisign_public_keys() {
    let mut key_lines = Vec::new();
    for file in ["release/minisign.pub", "release/minisign-next.pub"] {
        let text = read_repo_file(file);
        assert_eq!(text.lines().count(), 2, "{file} must be a comment line and a key line");
        assert!(
            text.lines().next().unwrap_or_default().starts_with("untrusted comment:"),
            "{file} must start with the `untrusted comment:` line of a minisign key"
        );
        let key = key_line(&text);
        assert_eq!(key.len(), 56, "{file} must hold a 56 character minisign key");
        assert!(key.starts_with("RW"), "{file} must hold an Ed25519 minisign key (`RW...`)");
        key_lines.push(key.to_string());
    }
    assert_ne!(key_lines[0], key_lines[1], "the primary and the backup release keys must differ");
}

#[test]
fn the_embedded_key_list_has_the_primary_and_backup_keys_and_not_the_test_key() {
    let primary = read_repo_file("release/minisign.pub");
    let backup = read_repo_file("release/minisign-next.pub");
    let test_key = read_repo_file("crates/clawg-cli/src/update/testdata/fixture.pub");

    assert_eq!(signature::EMBEDDED_KEYS, [primary.as_str(), backup.as_str()]);
    assert!(
        signature::EMBEDDED_KEYS.iter().all(|key| key_line(key) != key_line(&test_key)),
        "a release must never trust the key the tests sign with"
    );
}
