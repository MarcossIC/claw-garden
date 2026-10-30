//! `--docs`: index a project's documentation and score how well it answers
//! the questions of a golden set written about it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clawg_core::eval_api::{doc_paths, is_code_path, EvalDocHit, EvalDocIndex, EvalDocReport, EvalEmbedder};
use serde::{Deserialize, Serialize};

use crate::cli::Cli;
use crate::report::{Mode, ReportError};
use crate::{EXIT_GOLDEN_INVALID, EXIT_OK, EXIT_RESULTS_EXISTS, EXIT_RUNTIME_FAILURE};

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
    #[error("question '{id}' expects {path}, which quotes a question of the golden set")]
    ExpectedQuotes { id: String, path: String },
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

/// Lower case with every run of whitespace folded to one space, so a quote
/// that a table or a wrapped line split still reads as one.
fn folded(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// The paths, in the order given, of the docs that quote a question of
/// `cases`. Such a doc shares every word of the question by construction, so
/// it would rank first and measure the copy rather than the answer.
pub fn quoting_docs(cases: &[DocsCase], docs: &[(String, String)]) -> Vec<String> {
    let questions: Vec<String> = cases.iter().map(|c| folded(&c.question)).collect();
    docs.iter()
        .filter(|(_, text)| {
            let text = folded(text);
            questions.iter().any(|q| text.contains(q.as_str()))
        })
        .map(|(path, _)| path.clone())
        .collect()
}

/// Fails on an expected doc that quotes a question: leaving it out would
/// leave the question without its answer, and keeping it would score a copy.
pub fn check_quotes(cases: &[DocsCase], left_out: &[String]) -> Result<(), DocsGoldenError> {
    for case in cases {
        if let Some(quoting) = case.expected.iter().find(|e| left_out.contains(&e.path)) {
            return Err(DocsGoldenError::ExpectedQuotes { id: case.id.clone(), path: quoting.path.clone() });
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
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

fn left_out_line(left_out: &[String]) -> String {
    match left_out.len() {
        0 => "left out 0 docs quoting a question\n".to_string(),
        1 => format!("left out 1 doc quoting a question: {}\n", left_out[0]),
        n => format!("left out {n} docs quoting a question: {}\n", left_out.join(", ")),
    }
}

pub fn render(report: &EvalDocReport, took: Duration, left_out: &[String], results: &[CaseResult<'_>]) -> String {
    let all: Vec<&CaseResult<'_>> = results.iter().collect();
    let mut out = format!(
        "indexed {} docs into {} passages ({} skipped) in {:.1} s\n{}{}",
        report.docs,
        report.passages,
        report.skipped,
        took.as_secs_f64(),
        left_out_line(left_out),
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

/// Which run produced a results file.
pub struct RunStamp {
    pub date: String,
    pub mode: Mode,
    pub tag: Option<String>,
    pub model_id: String,
    pub git_sha: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DocsCorpus {
    pub docs: usize,
    pub passages: usize,
    pub skipped: usize,
    /// The docs that quote a question, which the index never saw.
    pub left_out: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DocsAggregate {
    pub all: DocsScore,
    /// Absent when no question is of that kind.
    pub docs: Option<DocsScore>,
    pub code: Option<DocsScore>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseRank {
    pub id: String,
    pub code: bool,
    /// From 1, `None` past the depth MRR reads.
    pub rank: Option<usize>,
    pub top: Option<String>,
}

/// A docs run as its results file holds it: ranks, counts and paths, never a
/// question's or a passage's text.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DocsResults {
    pub kind: &'static str,
    pub date: String,
    pub mode: Mode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub model_id: String,
    pub git_sha: String,
    pub golden_sha256: String,
    pub cases: usize,
    pub corpus: DocsCorpus,
    pub index_seconds: f64,
    pub aggregate: DocsAggregate,
    pub per_case: Vec<CaseRank>,
}

pub const DOCS_KIND: &str = "docs";

impl DocsResults {
    pub fn new(
        stamp: RunStamp,
        golden_sha256: String,
        report: &EvalDocReport,
        left_out: &[String],
        took: Duration,
        results: &[CaseResult<'_>],
    ) -> Self {
        let score_of = |code: Option<bool>| {
            let ranks: Vec<Option<usize>> =
                results.iter().filter(|r| code.is_none_or(|c| r.code == c)).map(|r| r.rank).collect();
            (!ranks.is_empty()).then(|| score(&ranks))
        };
        Self {
            kind: DOCS_KIND,
            date: stamp.date,
            mode: stamp.mode,
            tag: stamp.tag,
            model_id: stamp.model_id,
            git_sha: stamp.git_sha,
            golden_sha256,
            cases: results.len(),
            corpus: DocsCorpus {
                docs: report.docs,
                passages: report.passages,
                skipped: report.skipped,
                left_out: left_out.to_vec(),
            },
            index_seconds: took.as_secs_f64(),
            aggregate: DocsAggregate {
                all: score_of(None).unwrap_or(score(&[])),
                docs: score_of(Some(false)),
                code: score_of(Some(true)),
            },
            per_case: results
                .iter()
                .map(|r| CaseRank { id: r.case.id.clone(), code: r.code, rank: r.rank, top: r.top.clone() })
                .collect(),
        }
    }
}

/// `{out_dir}/{date}-{mode}-docs[-{tag}].json`, apart from a memory run's
/// file of the same day.
pub fn results_path(out_dir: &Path, date: &str, mode: Mode, tag: Option<&str>) -> PathBuf {
    let tag = match tag {
        Some(tag) => format!("{DOCS_KIND}-{tag}"),
        None => DOCS_KIND.to_string(),
    };
    out_dir.join(crate::report::results_filename(date, mode, Some(&tag)))
}

/// Writes `results`, refusing an existing file unless `force`.
pub fn write_results(results: &DocsResults, out_dir: &Path, force: bool) -> Result<PathBuf, ReportError> {
    std::fs::create_dir_all(out_dir).map_err(|e| ReportError::CreateDir { path: out_dir.to_path_buf(), source: e })?;
    let path = results_path(out_dir, &results.date, results.mode, results.tag.as_deref());
    if path.exists() && !force {
        return Err(ReportError::AlreadyExists { path });
    }
    let json = serde_json::to_string_pretty(results)?;
    std::fs::write(&path, json).map_err(|e| ReportError::Write { path: path.clone(), source: e })?;
    Ok(path)
}

/// Runs `--docs` end to end and returns the exit code.
pub async fn run(cli: &Cli, embedder: &EvalEmbedder, base_dir: &Path) -> i32 {
    let golden = match std::fs::read_to_string(&cli.golden_path)
        .map_err(|e| DocsGoldenError::Read(cli.golden_path.display().to_string(), e))
    {
        Ok(text) => text,
        Err(e) => {
            eprintln!("docs golden set validation failed: {e}");
            return EXIT_GOLDEN_INVALID;
        }
    };
    let cases = match parse(&golden) {
        Ok(cases) => cases,
        Err(e) => {
            eprintln!("docs golden set validation failed: {e}");
            return EXIT_GOLDEN_INVALID;
        }
    };
    // Only a real run writes a file: the fake embedder measures plumbing,
    // and a file of its numbers would only mislead a later diff. Checked
    // before indexing, which takes minutes with the real model.
    let stamp = (cli.mode == crate::cli::Mode::Real).then(|| RunStamp {
        date: crate::current_date_string(),
        mode: Mode::Real,
        tag: cli.tag.clone(),
        model_id: embedder.model_id().to_string(),
        git_sha: crate::report::git_sha(),
    });
    if let Some(stamp) = &stamp {
        let path = results_path(&cli.out_dir, &stamp.date, stamp.mode, stamp.tag.as_deref());
        if path.exists() && !cli.force {
            println!("results file already exists: {}", path.display());
            return EXIT_RESULTS_EXISTS;
        }
    }
    let output = Output { stamp, golden_sha256: clawg_core::eval_api::sha256_hex(golden.as_bytes()) };
    let corpus = match doc_paths(&cli.docs_root) {
        Ok(paths) => paths,
        Err(e) => {
            eprintln!("cannot list the docs of {}: {e}", cli.docs_root.display());
            return EXIT_RUNTIME_FAILURE;
        }
    };
    // A doc that is not UTF-8 quotes nothing: the index skips it as well.
    let texts: Vec<(String, String)> = corpus
        .iter()
        .filter_map(|path| std::fs::read_to_string(cli.docs_root.join(path)).ok().map(|text| (path.clone(), text)))
        .collect();
    let left_out = quoting_docs(&cases, &texts);
    if let Err(e) = check_corpus(&cases, &corpus).and_then(|()| check_quotes(&cases, &left_out)) {
        eprintln!("docs golden set validation failed: {e}");
        return EXIT_GOLDEN_INVALID;
    }

    let scratch = std::env::temp_dir().join(format!("rag-eval-docs-{}", uuid::Uuid::new_v4()));
    run_in(cli, embedder, base_dir, &cases, &left_out, output, &scratch).await
}

/// Measures `cases` with an index built in `scratch`, which is removed when
/// the measurement ends, passed or failed.
async fn run_in(
    cli: &Cli,
    embedder: &EvalEmbedder,
    base_dir: &Path,
    cases: &[DocsCase],
    left_out: &[String],
    output: Output,
    scratch: &Path,
) -> i32 {
    let code = measure(cli, embedder, base_dir, cases, left_out, output, scratch).await;
    crate::seed::remove_scratch(scratch).await;
    code
}

async fn measure(
    cli: &Cli,
    embedder: &EvalEmbedder,
    base_dir: &Path,
    cases: &[DocsCase],
    left_out: &[String],
    output: Output,
    scratch: &Path,
) -> i32 {
    if let Err(e) = std::fs::create_dir_all(scratch) {
        eprintln!("cannot create {}: {e}", scratch.display());
        return EXIT_RUNTIME_FAILURE;
    }
    // Passages are capped by the tokenizer of the model that embeds them,
    // as in the daemon; the fake embedder has none and caps them by bytes.
    let tokenizer_dir = match cli.mode {
        crate::cli::Mode::Real => base_dir,
        crate::cli::Mode::Fake => scratch,
    };
    let index = match EvalDocIndex::open(scratch, embedder, tokenizer_dir).await {
        Ok(index) => index,
        Err(e) => {
            eprintln!("{e}");
            return EXIT_RUNTIME_FAILURE;
        }
    };
    let code = measure_with(cli, &index, cases, left_out, output).await;
    if let Err(e) = index.close().await {
        eprintln!("warning: {e}");
    }
    code
}

/// Where a run's results go besides stdout.
struct Output {
    /// Set for a run that writes a results file.
    stamp: Option<RunStamp>,
    golden_sha256: String,
}

/// Indexes the root but `left_out` into `index`, asks it every question,
/// prints the report and writes the results file when `output` asks for one.
async fn measure_with(
    cli: &Cli,
    index: &EvalDocIndex,
    cases: &[DocsCase],
    left_out: &[String],
    output: Output,
) -> i32 {
    let started = Instant::now();
    let report = match index.index(&cli.docs_root, left_out).await {
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
    print!("{}", render(&report, took, left_out, &results));
    let Some(stamp) = output.stamp else {
        return EXIT_OK;
    };
    let file = DocsResults::new(stamp, output.golden_sha256, &report, left_out, took, &results);
    match write_results(&file, &cli.out_dir, cli.force) {
        Ok(path) => {
            println!("results written to {}", path.display());
            EXIT_OK
        }
        Err(ReportError::AlreadyExists { path }) => {
            println!("results file already exists: {}", path.display());
            EXIT_RESULTS_EXISTS
        }
        Err(e) => {
            eprintln!("failed to write results: {e}");
            EXIT_RUNTIME_FAILURE
        }
    }
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

    #[tokio::test]
    async fn a_docs_run_removes_its_index() {
        let root = crate::test_temp::temp_path("docs-root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("README.md"), "# Proyecto\n## Apagado\nEl daemon se detiene con clawg stop.\n").unwrap();
        let args = ["rag-eval", "--fake", "--docs", "--root", &root.to_string_lossy()].map(str::to_string);
        let Ok(crate::cli::ParseOutcome::Run(cli)) = crate::cli::parse(args.into_iter()) else {
            panic!("the docs arguments parse");
        };
        let cases = parse(r#"[{"id": "q1", "question": "daemon stop", "expected": [{"path": "README.md"}]}]"#).unwrap();
        let scratch = crate::test_temp::temp_path("docs-scratch");

        let output = Output { stamp: None, golden_sha256: String::new() };
        let code = run_in(&cli, &EvalEmbedder::fake(), &root, &cases, &[], output, &scratch).await;

        assert_eq!(code, EXIT_OK);
        assert!(!scratch.exists(), "the index is removed when the run ends");
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
    fn a_doc_quotes_a_question_across_lines_and_case() {
        let cases = parse(r#"[{"id": "q1", "question": "¿Qué pasa con los saves si paro el daemon?", "expected": [{"path": "a.md"}]}]"#)
            .unwrap();
        let quoted = |text: &str| !quoting_docs(&cases, &[("x.md".to_string(), text.to_string())]).is_empty();

        assert!(quoted("| q1 | ¿qué pasa con los saves si paro el daemon? | 4 |"));
        assert!(quoted("La pregunta era ¿Qué pasa con los\n  saves si paro\tel daemon? y no salió."));
        assert!(!quoted("¿Qué pasa con los saves?"), "a part of the question is not a quote");
        assert!(!quoted("Los saves pendientes se vuelcan al apagar el daemon."));
    }

    #[test]
    fn quoting_docs_names_only_the_docs_that_quote() {
        let cases = parse(
            r#"[{"id": "q1", "question": "how do I stop it?", "expected": [{"path": "README.md"}]},
                {"id": "q2", "question": "where is the data?", "expected": [{"path": "README.md"}]}]"#,
        )
        .unwrap();
        let docs = [
            ("README.md".to_string(), "# Stop\nRun clawg stop.\n".to_string()),
            ("notes/verify.md".to_string(), "| q2 | Where is the data? | - |\n".to_string()),
            ("notes/other.md".to_string(), "how do I stop\nit?\n".to_string()),
        ];

        assert_eq!(quoting_docs(&cases, &docs), ["notes/verify.md", "notes/other.md"]);
    }

    #[test]
    fn an_expected_doc_that_quotes_a_question_fails_validation() {
        let cases = parse(r#"[{"id": "q1", "question": "x?", "expected": [{"path": "a.md"}, {"path": "notes.md"}]}]"#).unwrap();

        match check_quotes(&cases, &["notes.md".to_string()]) {
            Err(DocsGoldenError::ExpectedQuotes { id, path }) => assert_eq!((id.as_str(), path.as_str()), ("q1", "notes.md")),
            other => panic!("expected ExpectedQuotes, got {other:?}"),
        }
        assert!(check_quotes(&cases, &["other.md".to_string()]).is_ok());
    }

    #[test]
    fn the_report_names_the_docs_it_left_out() {
        let cases = parse(r#"[{"id": "q1", "question": "first?", "expected": [{"path": "a.md"}]}]"#).unwrap();
        let results = [CaseResult { case: &cases[0], rank: Some(1), top: None, code: false }];
        let report = EvalDocReport { docs: 1, passages: 1, skipped: 0 };

        let none = render(&report, Duration::ZERO, &[], &results);
        let one = render(&report, Duration::ZERO, &["notes/verify.md".to_string()], &results);
        let two = render(&report, Duration::ZERO, &["a/x.md".to_string(), "b/y.md".to_string()], &results);

        assert!(none.contains("\nleft out 0 docs quoting a question\n"), "{none}");
        assert!(one.contains("\nleft out 1 doc quoting a question: notes/verify.md\n"), "{one}");
        assert!(two.contains("\nleft out 2 docs quoting a question: a/x.md, b/y.md\n"), "{two}");
    }

    fn two_results<'a>(cases: &'a [DocsCase]) -> DocsResults {
        let results = [
            CaseResult { case: &cases[0], rank: Some(2), top: Some("a.md > Otro".into()), code: false },
            CaseResult { case: &cases[1], rank: None, top: Some("src/b.rs".into()), code: true },
        ];
        DocsResults::new(
            RunStamp { date: "2026-10-30".into(), mode: Mode::Real, tag: None, model_id: "m1".into(), git_sha: "abc1234".into() },
            "f00d".into(),
            &EvalDocReport { docs: 2, passages: 5, skipped: 1 },
            &["notes/verify.md".to_string()],
            Duration::from_millis(2500),
            &results,
        )
    }

    fn two_cases() -> Vec<DocsCase> {
        parse(
            r#"[{"id": "q1", "question": "first?", "expected": [{"path": "a.md"}]},
                {"id": "q2", "question": "second?", "expected": [{"path": "src/b.rs"}]}]"#,
        )
        .unwrap()
    }

    #[test]
    fn docs_results_carry_ranks_and_no_text() {
        let cases = two_cases();
        let json = serde_json::to_value(two_results(&cases)).unwrap();

        assert_eq!(json["kind"], "docs");
        assert_eq!(json["mode"], "real");
        assert_eq!(json["golden_sha256"], "f00d");
        assert_eq!(json["cases"], 2);
        assert_eq!(json["corpus"], serde_json::json!({"docs": 2, "passages": 5, "skipped": 1, "left_out": ["notes/verify.md"]}));
        assert_eq!(json["index_seconds"], 2.5);
        assert_eq!(json["aggregate"]["all"]["hit_at_3"], 0.5);
        assert_eq!(json["aggregate"]["docs"]["mrr"], 0.5);
        assert_eq!(json["aggregate"]["code"]["hit_at_5"], 0.0);
        assert_eq!(
            json["per_case"],
            serde_json::json!([
                {"id": "q1", "code": false, "rank": 2, "top": "a.md > Otro"},
                {"id": "q2", "code": true, "rank": null, "top": "src/b.rs"}
            ])
        );
        assert!(json.get("tag").is_none());
        let text = json.to_string();
        assert!(!text.contains("first?"), "no question text: {text}");
    }

    #[test]
    fn docs_results_name_their_file_and_refuse_to_overwrite() {
        let cases = two_cases();
        let out = crate::test_temp::temp_path("docs-results");
        let mut results = two_results(&cases);

        let path = write_results(&results, &out, false).unwrap();
        assert_eq!(path, out.join("2026-10-30-real-docs.json"));
        assert!(matches!(
            write_results(&results, &out, false),
            Err(crate::report::ReportError::AlreadyExists { .. })
        ));
        assert!(write_results(&results, &out, true).is_ok(), "--force overwrites");

        results.tag = Some("lex2".into());
        assert_eq!(results_path(&out, &results.date, results.mode, results.tag.as_deref()), out.join("2026-10-30-real-docs-lex2.json"));
        assert_eq!(write_results(&results, &out, false).unwrap(), out.join("2026-10-30-real-docs-lex2.json"));
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

        let text = render(&report, Duration::from_millis(1500), &[], &results);

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

        let text = render(&EvalDocReport { docs: 3, passages: 3, skipped: 0 }, Duration::ZERO, &[], &results);

        assert!(text.contains("  docs  Hit@1 0.00  Hit@3 0.00  Hit@5 0.00  MRR@10 0.00  over 1 questions\n"), "{text}");
        assert!(text.contains("  code  Hit@1 0.50  Hit@3 1.00  Hit@5 1.00  MRR@10 0.75  over 2 questions\n"), "{text}");
    }

    #[test]
    fn the_shipped_golden_set_names_docs_this_workspace_has() {
        let text = std::fs::read_to_string(crate::cli::default_docs_golden_path()).unwrap();
        let cases = parse(&text).unwrap();
        let root = crate::cli::default_docs_root();
        let corpus = doc_paths(&root).unwrap();
        check_corpus(&cases, &corpus).unwrap();
        let texts: Vec<(String, String)> = corpus
            .iter()
            .filter_map(|p| std::fs::read_to_string(root.join(p)).ok().map(|t| (p.clone(), t)))
            .collect();
        check_quotes(&cases, &quoting_docs(&cases, &texts)).unwrap();
        assert_eq!(cases.len(), 15);
    }
}
