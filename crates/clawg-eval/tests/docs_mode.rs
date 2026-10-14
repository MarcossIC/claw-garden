//! Runs `rag-eval --fake --docs` over a small project in a temporary
//! directory. Nothing here reads a `~/.clawg*` profile or this repository.

use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn rag_eval_docs(root: &Path, golden: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rag-eval"))
        .args(["--fake", "--docs", "--root"])
        .arg(root)
        .arg("--golden")
        .arg(golden)
        .output()
        .expect("rag-eval must start")
}

fn project(root: &Path) {
    write(root, "README.md", "# Proyecto\n## Instalación\nCompilá con cargo build.\n## Apagado\nEl daemon se detiene con clawg stop.\n");
    write(root, "docs/backups.md", "# Copias\nCada migración escribe un snapshot antes de tocar el store.\n");
    write(root, "Cargo.toml", "[package]\nname = \"demo\"\n");
    write(root, "src/main.rs", "fn main() {}\n");
}

#[test]
fn docs_mode_runs_end_to_end_with_the_fake_embedder() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    project(&root);
    let golden = dir.path().join("golden.json");
    std::fs::write(
        &golden,
        r#"[
            {"id": "q1", "question": "daemon stop", "expected": [{"path": "README.md", "heading": "Apagado"}]},
            {"id": "q2", "question": "snapshot migración", "expected": [{"path": "docs/backups.md"}]}
        ]"#,
    )
    .unwrap();

    let out = rag_eval_docs(&root, &golden);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "stdout: {stdout}\nstderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.starts_with("indexed 3 docs into 4 passages (0 skipped)"), "{stdout}");
    assert!(stdout.contains("Hit@1 1.00  Hit@3 1.00  Hit@5 1.00  MRR@10 1.00  over 2 questions"), "{stdout}");
}

#[test]
fn docs_mode_refuses_a_golden_set_naming_a_doc_the_root_lacks() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    project(&root);
    let golden = dir.path().join("golden.json");
    std::fs::write(&golden, r#"[{"id": "q1", "question": "x", "expected": [{"path": "docs/gone.md"}]}]"#).unwrap();

    let out = rag_eval_docs(&root, &golden);

    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("docs/gone.md"));
}
