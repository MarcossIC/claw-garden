//! Which files of a project are its documentation, and finding them on disk.
//!
//! [`classify`] decides from a relative path alone; [`walk`] and [`read`] do
//! the I/O around it.

use std::path::{Path, PathBuf};

use sha2::Digest;

use super::code::CodeLang;

/// A file larger than this is skipped: past it a "doc" is generated output or
/// a data dump, and its passages would crowd out the prose around them.
pub const MAX_DOC_BYTES: u64 = 256 * 1024;

/// How many docs one project indexes at most, taken in path order.
pub const MAX_DOCS: usize = 2_000;

const MANIFESTS: &[&str] = &["Cargo.toml", "package.json", "pyproject.toml", "go.mod"];

/// Whether a bridge started in `root` may have its files read without the
/// user adding it: only a project, which `has_marker` says, and never the
/// home directory or a filesystem root, which hold other people's packages
/// and the user's private notes even when they carry a `.git`. Pure; `home`
/// is compared as given, so the caller resolves it as it resolved `root`.
pub fn auto_indexable(root: &Path, home: Option<&Path>, has_marker: bool) -> bool {
    has_marker && root.parent().is_some() && home != Some(root)
}

/// Whether `root` holds a `.git` (a directory, or the file a worktree has) or
/// a manifest [`classify`] reads.
pub fn has_project_marker(root: &Path) -> bool {
    root.join(".git").exists() || MANIFESTS.iter().any(|m| root.join(m).is_file())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    Markdown,
    Manifest,
    Code(CodeLang),
}

/// Whether the file at `rel_path` (relative to the project root, `/`-separated)
/// is indexed, and as which kind.
pub fn classify(rel_path: &str) -> Option<DocKind> {
    let name = rel_path.rsplit('/').next()?;
    if MANIFESTS.contains(&name) {
        return Some(DocKind::Manifest);
    }
    let (_, ext) = name.rsplit_once('.')?;
    match ext.to_ascii_lowercase().as_str() {
        "md" | "mdx" | "markdown" => Some(DocKind::Markdown),
        _ => CodeLang::from_extension(ext).map(DocKind::Code),
    }
}

/// A doc found on disk, not read yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocFile {
    /// Relative to the root, `/`-separated on every platform.
    pub path: String,
    pub abs: PathBuf,
    pub kind: DocKind,
}

#[derive(Debug, Default)]
pub struct Walk {
    /// In path order, at most [`MAX_DOCS`].
    pub docs: Vec<DocFile>,
    /// Docs left out for their size or past [`MAX_DOCS`].
    pub skipped: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum CorpusError {
    #[error("project root {0} is not a directory")]
    NotADirectory(PathBuf),
}

/// The docs under `root`, honouring the ignore rules a git checkout of it
/// would: `.gitignore`, `.ignore` and `.git/info/exclude`, even with no `.git`
/// there. Hidden directories count (`.claude/`, `.github/`), `.git` itself and
/// symbolic links do not.
pub fn walk(root: &Path) -> Result<Walk, CorpusError> {
    if !root.is_dir() {
        return Err(CorpusError::NotADirectory(root.to_path_buf()));
    }
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .parents(false)
        .ignore(true)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(false)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build();

    let mut found: Vec<(DocFile, u64)> = Vec::new();
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Some(path) = relative(root, entry.path()) else { continue };
        let Some(kind) = classify(&path) else { continue };
        let size = entry.metadata().map(|m| m.len()).unwrap_or(u64::MAX);
        found.push((DocFile { path, abs: entry.into_path(), kind }, size));
    }
    // The walker's order depends on the directory listing; sorting is what
    // makes the cap pick the same docs on every run and every platform.
    found.sort_by(|a, b| a.0.path.cmp(&b.0.path));

    let mut walk = Walk::default();
    for (doc, size) in found {
        if size > MAX_DOC_BYTES || walk.docs.len() >= MAX_DOCS {
            walk.skipped += 1;
        } else {
            walk.docs.push(doc);
        }
    }
    Ok(walk)
}

/// A doc's text and the sha256 of its bytes, or `None` when it is not UTF-8
/// (or can no longer be read).
pub fn read(doc: &DocFile) -> Option<(String, String)> {
    let bytes = std::fs::read(&doc.abs).ok()?;
    let hash = sha256_hex(&bytes);
    String::from_utf8(bytes).ok().map(|text| (text, hash))
}

fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
    Some(parts?.join("/"))
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_indexable_wants_a_project_that_is_not_home() {
        let home = Path::new("/home/u");
        assert!(!auto_indexable(home, Some(home), true), "home, even with a .git");
        assert!(!auto_indexable(Path::new("/"), Some(home), true), "a filesystem root");
        assert!(!auto_indexable(Path::new("/home/u/Documents"), Some(home), false), "no marker");
        assert!(auto_indexable(Path::new("/home/u/repo"), Some(home), true));
        assert!(auto_indexable(Path::new("/srv/repo"), None, true), "no home to compare with");
    }

    #[test]
    fn a_git_or_a_manifest_marks_a_project() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!has_project_marker(dir.path()));
        std::fs::write(dir.path().join("README.md"), "# notes\n").unwrap();
        assert!(!has_project_marker(dir.path()), "a doc alone is not a project");
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        assert!(has_project_marker(dir.path()));

        let worktree = tempfile::tempdir().unwrap();
        std::fs::write(worktree.path().join(".git"), "gitdir: /elsewhere\n").unwrap();
        assert!(has_project_marker(worktree.path()), "a .git file, as in a worktree");
    }

    #[test]
    fn classify_takes_markdown_in_any_case() {
        assert_eq!(classify("README.md"), Some(DocKind::Markdown));
        assert_eq!(classify("docs/guia.MD"), Some(DocKind::Markdown));
        assert_eq!(classify("site/page.mdx"), Some(DocKind::Markdown));
        assert_eq!(classify("notes.markdown"), Some(DocKind::Markdown));
    }

    #[test]
    fn classify_takes_manifests_by_exact_name() {
        assert_eq!(classify("Cargo.toml"), Some(DocKind::Manifest));
        assert_eq!(classify("crates/a/Cargo.toml"), Some(DocKind::Manifest));
        assert_eq!(classify("web/package.json"), Some(DocKind::Manifest));
        assert_eq!(classify("pyproject.toml"), Some(DocKind::Manifest));
        assert_eq!(classify("go.mod"), Some(DocKind::Manifest));
        assert_eq!(classify("cargo.toml"), None);
        assert_eq!(classify("rustfmt.toml"), None);
    }

    #[test]
    fn classify_takes_code_by_extension() {
        assert_eq!(classify("src/main.rs"), Some(DocKind::Code(CodeLang::Rust)));
        assert_eq!(classify("app/models.PY"), Some(DocKind::Code(CodeLang::Python)));
        assert_eq!(classify("web/api.ts"), Some(DocKind::Code(CodeLang::TypeScript)));
        assert_eq!(classify("web/App.tsx"), Some(DocKind::Code(CodeLang::Tsx)));
        assert_eq!(classify("web/app.js"), None);
    }

    #[test]
    fn classify_leaves_out_everything_else() {
        assert_eq!(classify("src/main.c"), None);
        assert_eq!(classify("README"), None);
        assert_eq!(classify("docs/md"), None);
        assert_eq!(classify("image.md.png"), None);
    }

    fn write(root: &Path, rel: &str, bytes: &[u8]) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn paths(walk: &Walk) -> Vec<&str> {
        walk.docs.iter().map(|d| d.path.as_str()).collect()
    }

    #[test]
    fn the_walk_honours_gitignore_and_keeps_hidden_docs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "README.md", b"# Readme");
        write(root, "docs/guia.MD", b"# Guia");
        write(root, ".claude/CLAUDE.md", b"# Claude");
        write(root, "crates/a/Cargo.toml", b"[package]");
        write(root, "src/main.rs", b"fn main() {}");
        write(root, "target/x.md", b"# built");
        write(root, ".gitignore", b"target/\n");
        write(root, ".git/HEAD", b"ref: refs/heads/main");
        write(root, ".git/notes.md", b"# not a doc");

        let walk = walk(root).unwrap();

        assert_eq!(paths(&walk), [".claude/CLAUDE.md", "README.md", "crates/a/Cargo.toml", "docs/guia.MD", "src/main.rs"]);
        assert_eq!(walk.docs[2].kind, DocKind::Manifest);
        assert_eq!(walk.skipped, 0);
    }

    #[test]
    fn oversized_and_binary_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "big.md", &vec![b'a'; MAX_DOC_BYTES as usize + 1]);
        write(root, "binary.md", &[0xff, 0xfe, 0x00, 0x80]);
        write(root, "ok.md", "# Café".as_bytes());

        let walk = walk(root).unwrap();

        assert_eq!(paths(&walk), ["binary.md", "ok.md"]);
        assert_eq!(walk.skipped, 1, "the oversized one is skipped by size");
        assert_eq!(read(&walk.docs[0]), None, "the binary one is skipped when read");
        let (text, hash) = read(&walk.docs[1]).unwrap();
        assert_eq!(text, "# Café");
        assert_eq!(hash, sha256_hex("# Café".as_bytes()));
    }

    #[test]
    fn a_root_that_is_not_a_directory_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(walk(&dir.path().join("gone")), Err(CorpusError::NotADirectory(_))));
    }

    #[test]
    fn the_hash_is_lowercase_hex_sha256() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
