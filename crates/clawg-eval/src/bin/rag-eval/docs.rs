//! `--docs`: index a project's documentation and score how well it answers
//! the questions of a golden set written about it.

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

use clawg_core::eval_api::{doc_paths, is_code_path, EvalDocHit, EvalDocIndex, EvalDocReport, EvalEmbedder};
use serde::Deserialize;

use crate::cli::{Cli, Mode};
use crate::{EXIT_GOLDEN_INVALID, EXIT_OK, EXIT_RUNTIME_FAILURE};

/// How deep each question's ranking is read: MRR counts a first hit up to here.
const RANKED: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocsCase {
    pub id: String,
    pub question: String,
    pub expected: Vec<ExpectedDoc>,
}

/// A passage that answers a question: any passage of `path`, or only those
/// of its section `heading` when one is given.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedDoc {
    pub path: String,
    #[serde(default)]
    pub heading: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DocsGoldenError {
    #[error("cannot read {0}: {1}")]
    Read(String, std::io::Error),
    #[error("not a docs golden set: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("the golden set has no questions")]
    Empty,
    #[error("question id '{0}' appears twice")]
    DuplicateId(String),
    #[error("question '{0}' is blank")]
    BlankQuestion(String),
    #[error("question '{0}' expects no passage")]
    NoExpected(String),
    #[error("question '{id}' expects {path}, which the root has no doc at")]
    MissingDoc { id: String, path: String },
}

/// Parses a golden set and checks it on its own terms.
pub fn parse(text: &str) -> Result<Vec<DocsCase>, DocsGoldenError> {
    let cases: Vec<DocsCase> = serde_json::from_str(text)?;
    if cases.is_empty() {
        return Err(DocsGoldenError::Empty);
    }
    let mut ids = HashSet::new();
    for case in &cases {
        if !ids.insert(case.id.as_str()) {
            return Err(DocsGoldenError::DuplicateId(case.id.clone()));
        }
        if case.question.trim().is_empty() {
            return Err(DocsGoldenError::BlankQuestion(case.id.clone()));
        }
        if case.expected.is_empty() {
            return Err(DocsGoldenError::NoExpected(case.id.clone()));
        }
    }
    Ok(cases)
}

/// Fails on an expected doc the corpus lacks, so a renamed doc breaks the
/// measurement instead of quietly lowering it.
pub fn check_corpus(cases: &[DocsCase], corpus: &[String]) -> Result<(), DocsGoldenError> {
    let corpus: HashSet<&str> = corpus.iter().map(String::as_str).collect();
    for case in cases {
        if let Some(missing) = case.expected.iter().find(|e| !corpus.contains(e.path.as_str())) {
            return Err(DocsGoldenError::MissingDoc { id: case.id.clone(), path: missing.path.clone() });
        }
    }
    Ok(())
}

pub fn matches(hit: &EvalDocHit, expected: &ExpectedDoc) -> bool {
    hit.path == expected.path
        && expected.heading.as_deref().map_or(true, |h| hit.breadcrumb.rsplit(" > ").next() == Some(h))
}

/// The rank, from 1, of the first hit that answers the question.
pub fn first_rank(hits: &[EvalDocHit], expected: &[ExpectedDoc]) -> Option<usize> {
    hits.iter().position(|h| expected.iter().any(|e| matches(h, e))).map(|i| i + 1)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DocsScore {
    pub hit_at_1: f64,
    pub hit_at_3: f64,
    pub hit_at_5: f64,
    pub mrr: f64,
}

pub fn score(ranks: &[Option<usize>]) -> DocsScore {
    let n = ranks.len().max(1) as f64;
    let within = |k: usize| ranks.iter().filter(|r| r.is_some_and(|r| r <= k)).count() as f64 / n;
    DocsScore {
        hit_at_1: within(1),
        hit_at_3: within(3),
        hit_at_5: within(5),
        mrr: ranks.iter().map(|r| r.map_or(0.0, |r| 1.0 / r as f64)).sum::<f64>() / n,
    }
}

/// One question's outcome, as the report lists it.
pub struct CaseResult<'a> {
    pub case: &'a DocsCase,
    pub rank: Option<usize>,
    /// The breadcrumb of the top hit, to show what was found instead.
    pub top: Option<String>,
    /// Whether the question is about code, by the first path it expects.
    pub code: bool,
}

fn score_line(results: &[&CaseResult<'_>]) -> String {
    let ranks: Vec<Option<usize>> = results.iter().map(|r| r.rank).collect();
    let s = score(&ranks);
    format!(
        "Hit@1 {:.2}  Hit@3 {:.2}  Hit@5 {:.2}  MRR@{RANKED} {:.2}  over {} questions\n",
        s.hit_at_1,
        s.hit_at_3,
        s.hit_at_5,
        s.mrr,
        results.len(),
    )
}

pub fn render(report: &EvalDocReport, took: Duration, results: &[CaseResult<'_>]) -> String {
    let all: Vec<&CaseResult<'_>> = results.iter().collect();
    let mut out = format!(
        "indexed {} docs into {} passages ({} skipped) in {:.1} s\n{}",
        report.docs,
        report.passages,
        report.skipped,
        took.as_secs_f64(),
        score_line(&all),
    );
    let (code, docs): (Vec<_>, Vec<_>) = all.iter().partition(|r| r.code);
    if !code.is_empty() && !docs.is_empty() {
        out.push_str(&format!("  docs  {}", score_line(&docs)));
        out.push_str(&format!("  code  {}", score_line(&code)));
    }
    out.push('\n');
    for r in results {
        let rank = r.rank.map_or("-".to_string(), |n| n.to_string());
        out.push_str(&format!("{rank:>3}  {}  {}\n", r.case.id, r.case.question));
        if r.rank != Some(1) {
            let expected: Vec<String> = r
                .case
                .expected
                .iter()
                .map(|e| match &e.heading {
                    Some(h) => format!("{} > … > {h}", e.path),
                    None => e.path.clone(),
                })
                .collect();
            out.push_str(&format!("     expected {}\n", expected.join(" or ")));
            out.push_str(&format!("     top      {}\n", r.top.as_deref().unwrap_or("(nothing)")));
        }
    }
    out
}

/// Runs `--docs` end to end and returns the exit code.
pub async fn run(cli: &Cli, embedder: &EvalEmbedder, base_dir: &Path) -> i32 {
    let cases = match std::fs::read_to_string(&cli.golden_path)
        .map_err(|e| DocsGoldenError::Read(cli.golden_path.display().to_string(), e))
        .and_then(|text| parse(&text))
    {
        Ok(cases) => cases,
        Err(e) => {
            eprintln!("docs golden set validation failed: {e}");
            return EXIT_GOLDEN_INVALID;
        }
    };
    let corpus = match doc_paths(&cli.docs_root) {
        Ok(paths) => paths,
        Err(e) => {
            eprintln!("cannot list the docs of {}: {e}", cli.docs_root.display());
            return EXIT_RUNTIME_FAILURE;
        }
    };
    if let Err(e) = check_corpus(&cases, &corpus) {
        eprintln!("docs golden set validation failed: {e}");
        return EXIT_GOLDEN_INVALID;
    }

    let scratch = std::env::temp_dir().join(format!("rag-eval-docs-{}", uuid::Uuid::new_v4()));
    let code = measure(cli, embedder, base_dir, &cases, &scratch).await;
    let _ = std::fs::remove_dir_all(&scratch);
    code
}

async fn measure(cli: &Cli, embedder: &EvalEmbedder, base_dir: &Path, cases: &[DocsCase], scratch: &Path) -> i32 {
    if let Err(e) = std::fs::create_dir_all(scratch) {
        eprintln!("cannot create {}: {e}", scratch.display());
        return EXIT_RUNTIME_FAILURE;
    }
    // Passages are capped by the tokenizer of the model that embeds them,
    // as in the daemon; the fake embedder has none and caps them by bytes.
    let tokenizer_dir = match cli.mode {
        Mode::Real => base_dir,
        Mode::Fake => scratch,
    };
    let index = match EvalDocIndex::open(scratch, embedder, tokenizer_dir).await {
        Ok(index) => index,
        Err(e) => {
            eprintln!("{e}");
            return EXIT_RUNTIME_FAILURE;
        }
    };
    let started = Instant::now();
    let report = match index.index(&cli.docs_root).await {
        Ok(report) => report,
        Err(e) => {
            eprintln!("indexing {} failed: {e}", cli.docs_root.display());
            return EXIT_RUNTIME_FAILURE;
        }
    };
    let took = started.elapsed();

    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let hits = match index.search(&case.question, RANKED).await {
            Ok(hits) => hits,
            Err(e) => {
                eprintln!("question '{}' failed: {e}", case.id);
                return EXIT_RUNTIME_FAILURE;
            }
        };
        results.push(CaseResult {
            case,
            rank: first_rank(&hits, &case.expected),
            top: hits.first().map(|h| h.breadcrumb.clone()),
            code: is_code_path(&case.expected[0].path),
        });
    }
    print!("{}", render(&report, took, &results));
    EXIT_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(path: &str, breadcrumb: &str) -> EvalDocHit {
        EvalDocHit { path: path.to_string(), breadcrumb: breadcrumb.to_string() }
    }

    fn expect(path: &str, heading: Option<&str>) -> ExpectedDoc {
        ExpectedDoc { path: path.to_string(), heading: heading.map(str::to_string) }
    }

    #[test]
    fn the_docs_golden_set_parses_and_rejects_unknown_fields() {
        let ok = r#"[{"id": "q1", "question": "how do I stop it?", "expected": [{"path": "README.md", "heading": "Stop"}, {"path": "a.md"}]}]"#;
        let cases = parse(ok).unwrap();
        assert_eq!(cases[0].expected, [expect("README.md", Some("Stop")), expect("a.md", None)]);

        let unknown_case = r#"[{"id": "q1", "question": "x", "expected": [{"path": "a.md"}], "lang": "en"}]"#;
        assert!(matches!(parse(unknown_case), Err(DocsGoldenError::Parse(_))));
        let unknown_expected = r#"[{"id": "q1", "question": "x", "expected": [{"path": "a.md", "line": 3}]}]"#;
        assert!(matches!(parse(unknown_expected), Err(DocsGoldenError::Parse(_))));
    }

    #[test]
    fn a_golden_set_must_be_well_formed() {
        assert!(matches!(parse("[]"), Err(DocsGoldenError::Empty)));
        let twice = r#"[{"id": "q1", "question": "x", "expected": [{"path": "a.md"}]},
                        {"id": "q1", "question": "y", "expected": [{"path": "a.md"}]}]"#;
        assert!(matches!(parse(twice), Err(DocsGoldenError::DuplicateId(id)) if id == "q1"));
        let blank = r#"[{"id": "q1", "question": "  ", "expected": [{"path": "a.md"}]}]"#;
        assert!(matches!(parse(blank), Err(DocsGoldenError::BlankQuestion(_))));
        let none = r#"[{"id": "q1", "question": "x", "expected": []}]"#;
        assert!(matches!(parse(none), Err(DocsGoldenError::NoExpected(_))));
    }

    #[test]
    fn hits_count_when_path_and_heading_match() {
        let h = hit("docs/guia.md", "docs/guia.md > Guía > Instalación");
        assert!(matches(&h, &expect("docs/guia.md", None)));
        assert!(matches(&h, &expect("docs/guia.md", Some("Instalación"))));
        assert!(!matches(&h, &expect("docs/guia.md", Some("Guía"))), "only the section's own heading counts");
        assert!(!matches(&h, &expect("README.md", Some("Instalación"))));
        assert!(!matches(&hit("Cargo.toml", "Cargo.toml"), &expect("Cargo.toml", Some("deps"))));

        let hits = [hit("a.md", "a.md > Uso"), hit("b.md", "b.md > Uso"), hit("a.md", "a.md > Instalación")];
        assert_eq!(first_rank(&hits, &[expect("a.md", Some("Instalación"))]), Some(3));
        assert_eq!(first_rank(&hits, &[expect("c.md", None), expect("b.md", None)]), Some(2));
        assert_eq!(first_rank(&hits, &[expect("c.md", None)]), None);
    }

    #[test]
    fn an_expected_doc_missing_from_the_corpus_fails_validation() {
        let cases = parse(r#"[{"id": "q1", "question": "x", "expected": [{"path": "README.md"}, {"path": "gone.md"}]}]"#).unwrap();
        let corpus = vec!["README.md".to_string()];
        match check_corpus(&cases, &corpus) {
            Err(DocsGoldenError::MissingDoc { id, path }) => assert_eq!((id.as_str(), path.as_str()), ("q1", "gone.md")),
            other => panic!("expected MissingDoc, got {other:?}"),
        }
        assert!(check_corpus(&cases[..0], &corpus).is_ok());
    }

    #[test]
    fn the_score_counts_hits_within_each_depth_and_the_reciprocal_rank() {
        let s = score(&[Some(1), Some(3), Some(5), None]);
        assert_eq!(s.hit_at_1, 0.25);
        assert_eq!(s.hit_at_3, 0.5);
        assert_eq!(s.hit_at_5, 0.75);
        assert!((s.mrr - (1.0 + 1.0 / 3.0 + 1.0 / 5.0) / 4.0).abs() < 1e-12);
    }

    #[test]
    fn the_report_names_what_a_miss_found_instead() {
        let cases = parse(
            r#"[{"id": "q1", "question": "first?", "expected": [{"path": "a.md"}]},
                {"id": "q2", "question": "second?", "expected": [{"path": "b.md", "heading": "Uso"}]}]"#,
        )
        .unwrap();
        let results = [
            CaseResult { case: &cases[0], rank: Some(1), top: Some("a.md".into()), code: false },
            CaseResult { case: &cases[1], rank: None, top: Some("a.md > Otro".into()), code: false },
        ];
        let report = EvalDocReport { docs: 2, passages: 7, skipped: 1 };

        let text = render(&report, Duration::from_millis(1500), &results);

        assert!(text.starts_with("indexed 2 docs into 7 passages (1 skipped) in 1.5 s\n"), "{text}");
        assert!(text.contains("Hit@1 0.50  Hit@3 0.50  Hit@5 0.50  MRR@10 0.50  over 2 questions"), "{text}");
        assert!(text.contains("  1  q1  first?\n  -  q2  second?\n     expected b.md > … > Uso\n     top      a.md > Otro\n"), "{text}");
        assert!(!text.contains("  docs "), "with no code questions there is nothing to split: {text}");
    }

    #[test]
    fn the_report_scores_docs_and_code_apart() {
        let cases = parse(
            r#"[{"id": "q1", "question": "a?", "expected": [{"path": "a.md"}]},
                {"id": "q2", "question": "b?", "expected": [{"path": "src/b.rs"}]},
                {"id": "q3", "question": "c?", "expected": [{"path": "src/c.rs"}]}]"#,
        )
        .unwrap();
        let results = [
            CaseResult { case: &cases[0], rank: None, top: None, code: false },
            CaseResult { case: &cases[1], rank: Some(1), top: None, code: true },
            CaseResult { case: &cases[2], rank: Some(2), top: None, code: true },
        ];

        let text = render(&EvalDocReport { docs: 3, passages: 3, skipped: 0 }, Duration::ZERO, &results);

        assert!(text.contains("  docs  Hit@1 0.00  Hit@3 0.00  Hit@5 0.00  MRR@10 0.00  over 1 questions\n"), "{text}");
        assert!(text.contains("  code  Hit@1 0.50  Hit@3 1.00  Hit@5 1.00  MRR@10 0.75  over 2 questions\n"), "{text}");
    }

    #[test]
    fn the_shipped_golden_set_names_docs_this_workspace_has() {
        let text = std::fs::read_to_string(crate::cli::default_docs_golden_path()).unwrap();
        let cases = parse(&text).unwrap();
        let corpus = doc_paths(&crate::cli::default_docs_root()).unwrap();
        check_corpus(&cases, &corpus).unwrap();
        assert_eq!(cases.len(), 15);
    }
}
