//! Runs the two binaries against each other: a base set goes through
//! `golden-mine synth` and the result is scored by `rag-eval --fake`. Nothing
//! here reads `docs/eval` or a `~/.clawg*` profile; the base is inline and
//! every file lives in a temporary directory.

use std::path::Path;
use std::process::{Command, Output};

const BASE: &str = r#"{
    "corpus": [
        {"ref": "c01", "content": "The `OrderService` owns pricing rules and delegates persistence to `OrderRepository`.", "memory_type": "architectural_decision", "importance": 6.0, "lang": "en"},
        {"ref": "c02", "content": "Gateway authentication moved into `AuthMiddleware`, replacing the legacy session handling.", "memory_type": "architectural_decision", "importance": 7.0, "lang": "en"},
        {"ref": "c03", "content": "Falló la migración `add_index_users` porque la tabla ya tenía bloqueos pendientes.", "memory_type": "tool_error", "importance": 4.0, "lang": "es"},
        {"ref": "c04", "content": "El comando `cargo_build` terminó con errores de enlace durante la compilación incremental.", "memory_type": "tool_error", "importance": 5.0, "lang": "es"}
    ],
    "retrieval_cases": [
        {"query": "how is pricing split from persistence", "corpus_refs": ["c01", "c02"], "expected_refs": ["c01"]},
        {"query": "por qué falló la migración de usuarios", "corpus_refs": ["c03", "c04"], "expected_refs": ["c03"]}
    ],
    "contradiction_pairs": [
        {"a": "the sky is blue", "b": "the sky is not blue", "label": "contradiction", "lang": "en"},
        {"a": "el cielo es azul", "b": "el cielo no es azul", "label": "contradiction", "lang": "es"}
    ]
}"#;

fn synth(base: &Path, out: &Path, seed: u64) -> Output {
    Command::new(env!("CARGO_BIN_EXE_golden-mine"))
        .args(["synth", "--base"])
        .arg(base)
        .arg("--out")
        .arg(out)
        .args(["--seed", &seed.to_string(), "--distractors", "40"])
        .output()
        .expect("golden-mine must start")
}

fn write_base(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("base.json");
    std::fs::write(&path, BASE).unwrap();
    path
}

#[test]
fn the_expanded_set_scores_under_rag_eval_and_reports_every_subset() {
    let dir = tempfile::tempdir().unwrap();
    let base = write_base(dir.path());
    let expanded = dir.path().join("expanded.json");

    let made = synth(&base, &expanded, 7);
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));

    let results_dir = dir.path().join("results");
    let scored = Command::new(env!("CARGO_BIN_EXE_rag-eval"))
        .arg("--fake")
        .arg("--golden")
        .arg(&expanded)
        .arg("--out")
        .arg(&results_dir)
        .output()
        .expect("rag-eval must start");
    assert!(scored.status.success(), "{}", String::from_utf8_lossy(&scored.stderr));

    let file = std::fs::read_dir(&results_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "json"))
        .expect("rag-eval must write a results file");
    let results: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();

    let by_subset = results["retrieval"]["by_subset"].as_array().expect("by_subset must be present");
    let names: Vec<&str> = by_subset.iter().map(|b| b["subset"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["high_importance", "lexical", "multi_ref", "natural"]);
    let natural = by_subset.iter().find(|b| b["subset"] == "natural").unwrap();
    assert_eq!(natural["n_cases"], 2, "the base cases keep the natural label");
}

#[test]
fn the_base_file_is_left_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let base = write_base(dir.path());
    let made = synth(&base, &dir.path().join("expanded.json"), 7);
    assert!(made.status.success());
    assert_eq!(std::fs::read_to_string(&base).unwrap(), BASE);
}

#[test]
fn the_same_seed_gives_identical_bytes_and_another_seed_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let base = write_base(dir.path());
    let (a, b, c) = (dir.path().join("a.json"), dir.path().join("b.json"), dir.path().join("c.json"));
    assert!(synth(&base, &a, 7).status.success());
    assert!(synth(&base, &b, 7).status.success());
    assert!(synth(&base, &c, 8).status.success());

    let read = |p: &Path| std::fs::read(p).unwrap();
    assert_eq!(read(&a), read(&b));
    assert_ne!(read(&a), read(&c));
}

#[test]
fn writing_over_the_base_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let base = write_base(dir.path());
    let made = synth(&base, &base, 7);
    assert!(!made.status.success());
    assert_eq!(std::fs::read_to_string(&base).unwrap(), BASE);
}
