//! `--docs`: index a project's documentation and score how well it answers
//! the questions of a golden set written about it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clawg_core::eval_api::{
    doc_paths, is_code_path, session_threads, EvalCpu, EvalDocHit, EvalDocIndex, EvalDocReport, EvalEmbedder, EvalError,
    EvalIndexRun, EvalMemo, EvalMemoReport,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cli::{memo_dir, Cli};
use crate::diff::{metric_line, verdict, MetricDiff, METRIC_HEADER};
use crate::index_cost::{memo_line, padding_warning, IndexBlock, Phases, Tokens};
use crate::report::{Mode, ReportError};
use crate::stats::{paired_bootstrap_ci, BOOTSTRAP_RESAMPLES, BOOTSTRAP_SEED};
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
    #[error("question '{id}' expects {path} > {heading}, which no passage of that doc ends in")]
    MissingHeading { id: String, path: String, heading: String },
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

/// Fails on an expected heading that no passage of its doc ends in. A heading
/// renamed since, or one whose section holds only subsections, leaves the
/// question no passage that can answer it and lowers the measurement in
/// silence. `crumbs` holds, by path, the breadcrumbs of each doc expected
/// under a heading.
pub fn check_headings(cases: &[DocsCase], crumbs: &HashMap<String, Vec<String>>) -> Result<(), DocsGoldenError> {
    for case in cases {
        for expected in &case.expected {
            let Some(heading) = &expected.heading else { continue };
            let ends_in = |crumb: &String| leaf(crumb) == Some(heading.as_str());
            if !crumbs.get(&expected.path).is_some_and(|c| c.iter().any(ends_in)) {
                return Err(DocsGoldenError::MissingHeading {
                    id: case.id.clone(),
                    path: expected.path.clone(),
                    heading: heading.clone(),
                });
            }
        }
    }
    Ok(())
}

/// The breadcrumbs of every doc a case expects under a heading, as `index`
/// splits it.
fn expected_breadcrumbs(
    index: &EvalDocIndex,
    root: &Path,
    cases: &[DocsCase],
) -> Result<HashMap<String, Vec<String>>, EvalError> {
    let mut crumbs = HashMap::new();
    for expected in cases.iter().flat_map(|c| &c.expected).filter(|e| e.heading.is_some()) {
        if !crumbs.contains_key(&expected.path) {
            crumbs.insert(expected.path.clone(), index.breadcrumbs(root, &expected.path)?);
        }
    }
    Ok(crumbs)
}

/// The heading a passage sits directly under: the last part of its breadcrumb.
fn leaf(breadcrumb: &str) -> Option<&str> {
    breadcrumb.rsplit(" > ").next()
}

pub fn matches(hit: &EvalDocHit, expected: &ExpectedDoc) -> bool {
    hit.path == expected.path && expected.heading.as_deref().map_or(true, |h| leaf(&hit.breadcrumb) == Some(h))
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

/// What only a run that writes a results file has to report.
pub struct RealRun<'a> {
    pub block: &'a IndexBlock,
    /// `None` when the run used no memo.
    pub memo: Option<&'a EvalMemoReport>,
}

pub fn render(
    report: &EvalDocReport,
    took: Duration,
    left_out: &[String],
    results: &[CaseResult<'_>],
    phases: &Phases,
    real: Option<RealRun<'_>>,
) -> String {
    let all: Vec<&CaseResult<'_>> = results.iter().collect();
    let mut out = format!(
        "indexed {} docs into {} passages ({} skipped) in {:.1} s\n{}{}{}{}{}",
        report.docs,
        report.passages,
        report.skipped,
        took.as_secs_f64(),
        phases.line(),
        real.as_ref().and_then(|r| r.block.tokens.as_ref()).map_or(String::new(), Tokens::line),
        real.as_ref().map_or(String::new(), |r| memo_line(r.memo) + &r.block.by_area.lines()),
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
    pub index: IndexBlock,
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
        index: IndexBlock,
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
            index,
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

/// What a diff reads of a docs results file.
#[derive(Deserialize)]
struct DocsFile {
    golden_sha256: String,
    corpus: LeftOut,
    per_case: Vec<CaseRank>,
}

#[derive(Deserialize)]
struct LeftOut {
    left_out: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DocsDiffError {
    #[error("failed to read '{}': {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("'{}' is not valid JSON: {source}", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("not a docs results file: {0}")]
    Shape(serde_json::Error),
    #[error("one file is a docs run and the other is not; a docs run compares only with another")]
    Mixed,
}

/// The paired movement of the four scores over the questions both runs asked.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScoreDiff {
    pub joined: usize,
    pub hit_1: MetricDiff,
    pub hit_3: MetricDiff,
    pub hit_5: MetricDiff,
    pub mrr: MetricDiff,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DocsDiff {
    pub all: ScoreDiff,
    /// Absent when neither run asked a question of that kind.
    pub docs: Option<ScoreDiff>,
    pub code: Option<ScoreDiff>,
    pub entered_top_3: Vec<String>,
    pub left_top_3: Vec<String>,
    pub one_file_only: Vec<String>,
    /// Why the two runs may not have measured the same thing.
    pub warnings: Vec<String>,
}

fn is_docs(value: &Value) -> bool {
    value.get("kind").and_then(Value::as_str) == Some(DOCS_KIND)
}

fn within(rank: Option<usize>, k: usize) -> bool {
    rank.is_some_and(|r| r <= k)
}

fn reciprocal(rank: Option<usize>) -> f64 {
    rank.map_or(0.0, |r| 1.0 / r as f64)
}

fn tally(m: &mut MetricDiff, a: f64, b: f64) {
    if b > a {
        m.better += 1;
    } else if b < a {
        m.worse += 1;
    }
    m.net = m.better as i64 - m.worse as i64;
}

fn score_diff(pairs: &[(&CaseRank, &CaseRank)]) -> ScoreDiff {
    let mut d = ScoreDiff { joined: pairs.len(), ..Default::default() };
    let flag = |f: bool| if f { 1.0 } else { 0.0 };
    for (a, b) in pairs {
        tally(&mut d.hit_1, flag(within(a.rank, 1)), flag(within(b.rank, 1)));
        tally(&mut d.hit_3, flag(within(a.rank, 3)), flag(within(b.rank, 3)));
        tally(&mut d.hit_5, flag(within(a.rank, 5)), flag(within(b.rank, 5)));
        tally(&mut d.mrr, reciprocal(a.rank), reciprocal(b.rank));
    }
    let deltas: Vec<f64> = pairs.iter().map(|(a, b)| reciprocal(b.rank) - reciprocal(a.rank)).collect();
    d.mrr.ci = Some(paired_bootstrap_ci(&deltas, BOOTSTRAP_RESAMPLES, BOOTSTRAP_SEED));
    d
}

/// Diffs two results files when they are docs runs; `None` when neither is,
/// so the caller diffs them as memory runs.
pub fn diff_files(a_path: &Path, b_path: &Path) -> Result<Option<DocsDiff>, DocsDiffError> {
    let read = |path: &Path| -> Result<Value, DocsDiffError> {
        let text = std::fs::read_to_string(path).map_err(|e| DocsDiffError::Read { path: path.to_path_buf(), source: e })?;
        serde_json::from_str(&text).map_err(|e| DocsDiffError::Parse { path: path.to_path_buf(), source: e })
    };
    diff_values(&read(a_path)?, &read(b_path)?)
}

/// Joins the questions of two docs runs by id, the second run against the first.
pub fn diff_values(a: &Value, b: &Value) -> Result<Option<DocsDiff>, DocsDiffError> {
    match (is_docs(a), is_docs(b)) {
        (false, false) => return Ok(None),
        (true, true) => {}
        _ => return Err(DocsDiffError::Mixed),
    }
    let a: DocsFile = serde_json::from_value(a.clone()).map_err(DocsDiffError::Shape)?;
    let b: DocsFile = serde_json::from_value(b.clone()).map_err(DocsDiffError::Shape)?;
    let in_b: HashMap<&str, &CaseRank> = b.per_case.iter().map(|c| (c.id.as_str(), c)).collect();
    let in_a: HashSet<&str> = a.per_case.iter().map(|c| c.id.as_str()).collect();
    let pairs: Vec<(&CaseRank, &CaseRank)> =
        a.per_case.iter().filter_map(|c| in_b.get(c.id.as_str()).map(|other| (c, *other))).collect();
    let part = |code: bool| {
        let part: Vec<_> = pairs.iter().copied().filter(|(_, b)| b.code == code).collect();
        (!part.is_empty()).then(|| score_diff(&part))
    };
    let mut warnings = Vec::new();
    if a.golden_sha256 != b.golden_sha256 {
        warnings.push(format!(
            "the golden sets differ ({} and {}); only the questions both asked are joined",
            a.golden_sha256, b.golden_sha256
        ));
    }
    if a.corpus.left_out != b.corpus.left_out {
        warnings.push(format!(
            "the docs left out differ ([{}] and [{}]), so the runs searched different corpora",
            a.corpus.left_out.join(", "),
            b.corpus.left_out.join(", ")
        ));
    }
    let ids = |keep: &dyn Fn(&CaseRank, &CaseRank) -> bool| -> Vec<String> {
        pairs.iter().filter(|(a, b)| keep(a, b)).map(|(a, _)| a.id.clone()).collect()
    };
    Ok(Some(DocsDiff {
        all: score_diff(&pairs),
        docs: part(false),
        code: part(true),
        entered_top_3: ids(&|a, b| !within(a.rank, 3) && within(b.rank, 3)),
        left_top_3: ids(&|a, b| within(a.rank, 3) && !within(b.rank, 3)),
        one_file_only: a
            .per_case
            .iter()
            .filter(|c| !in_b.contains_key(c.id.as_str()))
            .chain(b.per_case.iter().filter(|c| !in_a.contains(c.id.as_str())))
            .map(|c| c.id.clone())
            .collect(),
        warnings,
    }))
}

/// Hit@3 got worse by more than chance explains.
pub fn regressed(diff: &DocsDiff) -> bool {
    let m = &diff.all.hit_3;
    m.worse > m.better && m.p_value() < crate::stats::ALPHA
}

pub fn render_diff(diff: &DocsDiff) -> String {
    let mut out = String::from("rag-eval diff (docs)\n");
    for w in &diff.warnings {
        out.push_str(&format!("  warning: {w}\n"));
    }
    let rows = |out: &mut String, name: &str, d: &ScoreDiff| {
        out.push_str(&format!("  {name} ({} joined):\n{METRIC_HEADER}", d.joined));
        for (metric, m) in [("hit@1", &d.hit_1), ("hit@3", &d.hit_3), ("hit@5", &d.hit_5), ("mrr@10", &d.mrr)] {
            out.push_str(&metric_line(metric, m));
        }
    };
    rows(&mut out, "all", &diff.all);
    if let Some(d) = &diff.docs {
        rows(&mut out, "docs", d);
    }
    if let Some(d) = &diff.code {
        rows(&mut out, "code", d);
    }
    let list = |out: &mut String, what: &str, ids: &[String]| {
        if !ids.is_empty() {
            out.push_str(&format!("  {what}: {}\n", ids.join(", ")));
        }
    };
    list(&mut out, "entered the top 3", &diff.entered_top_3);
    list(&mut out, "left the top 3", &diff.left_top_3);
    list(&mut out, "in one file only, counted in no row", &diff.one_file_only);
    out.push_str(&format!("  primary:\n    {}\n", verdict("docs hit@3", &diff.all.hit_3)));
    out
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
    // Read before the index is timed, so the memo's seconds stay out of it.
    let memo = memo_dir(cli).map(|dir| EvalMemo::open(embedder, dir));
    if let Some(warning) = memo.as_ref().and_then(EvalMemo::warning) {
        eprintln!("warning: {warning}");
    }
    let memoized = memo.as_ref().map(EvalMemo::embedder);
    let index = match EvalDocIndex::open(scratch, memoized.as_ref().unwrap_or(embedder), tokenizer_dir).await {
        Ok(index) => index,
        Err(e) => {
            eprintln!("{e}");
            return EXIT_RUNTIME_FAILURE;
        }
    };
    let code = measure_with(cli, embedder, &index, memo.as_ref(), cases, left_out, output).await;
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

/// The `index` block of a real run: the machine, the phases and the tokens
/// of the passages `index` holds. A failure is already printed; its exit
/// code comes back.
async fn index_block(
    index: &EvalDocIndex,
    embedder: &EvalEmbedder,
    run: &EvalIndexRun,
    memo: Option<&EvalMemoReport>,
) -> Result<IndexBlock, i32> {
    let padded_ran = embedder.passage_tokens_ran();
    let lengths = index.passage_lengths().await.map_err(|e| {
        eprintln!("could not count the tokens of the passages: {e}");
        EXIT_RUNTIME_FAILURE
    })?;
    let tokens = lengths.as_ref().map(|l| Tokens::of(l, padded_ran));
    let from_memo = memo.map_or(0, |m| m.from_memo);
    if let Some(warning) = tokens.as_ref().and_then(|t| padding_warning(t, from_memo)) {
        eprintln!("{warning}");
    }
    let threads = session_threads().map_err(|e| {
        eprintln!("{e}");
        EXIT_RUNTIME_FAILURE
    })?;
    Ok(IndexBlock::new(run, lengths.as_ref(), memo, EvalCpu::this(), threads, cfg!(debug_assertions), tokens))
}

/// Indexes the root but `left_out` into `index`, asks it every question,
/// prints the report and writes the results file when `output` asks for one.
async fn measure_with(
    cli: &Cli,
    embedder: &EvalEmbedder,
    index: &EvalDocIndex,
    memo: Option<&EvalMemo>,
    cases: &[DocsCase],
    left_out: &[String],
    output: Output,
) -> i32 {
    // Checked before indexing, which with the real model takes minutes.
    let checked = match expected_breadcrumbs(index, &cli.docs_root, cases) {
        Ok(crumbs) => check_headings(cases, &crumbs),
        Err(e) => {
            eprintln!("{e}");
            return EXIT_RUNTIME_FAILURE;
        }
    };
    if let Err(e) = checked {
        eprintln!("docs golden set validation failed: {e}");
        return EXIT_GOLDEN_INVALID;
    }

    let started = Instant::now();
    let indexed = index.index(&cli.docs_root, left_out).await;
    let took = started.elapsed();
    // What the run embedded is kept whether or not it finished; the read
    // warning was already said.
    let memo_report = memo.map(|m| {
        let finished = m.finish();
        for warning in finished.warnings.iter().filter(|w| Some(w.as_str()) != m.warning()) {
            eprintln!("warning: {warning}");
        }
        finished
    });
    let run = match indexed {
        Ok(run) => run,
        Err(e) => {
            eprintln!("indexing {} failed: {e}", cli.docs_root.display());
            return EXIT_RUNTIME_FAILURE;
        }
    };
    let report = run.report;
    // Only a run that writes a file counts tokens; the padding the embedder
    // ran is read before the questions embed anything.
    let block = match &output.stamp {
        Some(_) => match index_block(index, embedder, &run, memo_report.as_ref()).await {
            Ok(block) => Some(block),
            Err(code) => return code,
        },
        None => None,
    };

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
    print!("{}", render(&report, took, left_out, &results, &Phases::of(&run), block.as_ref().map(|block| RealRun { block, memo: memo_report.as_ref() })));
    let (Some(stamp), Some(block)) = (output.stamp, block) else {
        return EXIT_OK;
    };
    let file = DocsResults::new(stamp, output.golden_sha256, &report, left_out, took, &results, block);
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
    use clawg_core::eval_api::{
        EvalCpu, EvalDocLengths, EvalDocTiming, EvalIndexRun, EvalMemoReport, EvalPassageLengths, EvalPhases,
    };

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
    fn an_expected_heading_no_passage_ends_in_fails_validation() {
        let cases = parse(
            r#"[{"id": "q1", "question": "a?", "expected": [{"path": "a.md", "heading": "Uso"}, {"path": "src/b.rs"}]},
                {"id": "q2", "question": "b?", "expected": [{"path": "a.md", "heading": "Uso"}, {"path": "a.md", "heading": "Guía"}]}]"#,
        )
        .unwrap();
        let crumbs = HashMap::from([("a.md".to_string(), vec!["a.md > Guía > Uso".to_string()])]);

        assert!(check_headings(&cases[..1], &crumbs).is_ok(), "an expected doc without a heading is not looked up");
        match check_headings(&cases, &crumbs) {
            Err(DocsGoldenError::MissingHeading { id, path, heading }) => {
                assert_eq!((id.as_str(), path.as_str(), heading.as_str()), ("q2", "a.md", "Guía"))
            }
            other => panic!("expected MissingHeading, got {other:?}"),
        }
    }

    #[test]
    fn the_report_names_the_docs_it_left_out() {
        let cases = parse(r#"[{"id": "q1", "question": "first?", "expected": [{"path": "a.md"}]}]"#).unwrap();
        let results = [CaseResult { case: &cases[0], rank: Some(1), top: None, code: false }];
        let report = EvalDocReport { docs: 1, passages: 1, skipped: 0 };

        let none = render(&report, Duration::ZERO, &[], &results, &Phases::default(), None);
        let one = render(&report, Duration::ZERO, &["notes/verify.md".to_string()], &results, &Phases::default(), None);
        let two = render(&report, Duration::ZERO, &["a/x.md".to_string(), "b/y.md".to_string()], &results, &Phases::default(), None);

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
            sample_block(None),
        )
    }

    fn doc_timing(path: &str, code: bool, passages: usize, ms: [u64; 6]) -> EvalDocTiming {
        let [read, mask, split, lookup, embed, write] = ms.map(Duration::from_millis);
        EvalDocTiming { path: path.to_string(), code, passages, phases: EvalPhases { read, mask, split, lookup, embed, write } }
    }

    /// 2 docs, 5 passages, in quarter seconds so every sum is exact in binary.
    fn sample_run() -> EvalIndexRun {
        EvalIndexRun {
            report: EvalDocReport { docs: 2, passages: 5, skipped: 1 },
            scan: Duration::from_millis(500),
            remove: Duration::from_millis(250),
            docs: vec![
                doc_timing("secret/folder/guide.md", false, 3, [250, 500, 1000, 0, 3000, 750]),
                doc_timing("crates/hidden/src/lib.rs", true, 2, [250, 250, 500, 250, 1000, 250]),
            ],
        }
    }

    fn sample_block(memo: Option<&EvalMemoReport>) -> IndexBlock {
        IndexBlock::new(&sample_run(), None, memo, EvalCpu::this(), 8, true, None)
    }

    fn memo_of(from_memo: usize, embedded: usize) -> EvalMemoReport {
        EvalMemoReport {
            file: "memo.bin".to_string(),
            used: true,
            from_memo,
            embedded,
            seconds: 0.5,
            warnings: vec!["a warning that is not for the file".to_string()],
        }
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
    fn the_index_block_reports_the_memo_and_the_phases() {
        let run = EvalIndexRun { report: EvalDocReport { docs: 2, passages: 100, skipped: 0 }, ..sample_run() };
        let with = serde_json::to_value(IndexBlock::new(&run, None, Some(&memo_of(30, 70)), EvalCpu::this(), 8, true, None)).unwrap();
        let without = serde_json::to_value(IndexBlock::new(&run, None, None, EvalCpu::this(), 8, false, None)).unwrap();

        assert_eq!(with["memo"], serde_json::json!({"used": true, "from_memo": 30, "embedded": 70, "seconds": 0.5}));
        assert_eq!(without["memo"], serde_json::json!({"used": false, "from_memo": 0, "embedded": 100, "seconds": 0.0}));
        assert_eq!(
            with["phases"],
            serde_json::json!({"scan": 0.5, "read": 0.5, "mask": 0.75, "split": 1.5, "lookup": 0.25, "embed": 4.0, "write": 1.25}),
            "the removal counts as a write"
        );
        assert_eq!(with["phases"], without["phases"]);
        assert_eq!((with["build"].as_str(), without["build"].as_str()), (Some("debug"), Some("release")));
        let cpu = EvalCpu::this();
        assert_eq!(with["machine"]["os"], cpu.os.as_str());
        assert_eq!(with["machine"]["arch"], cpu.arch.as_str());
        assert_eq!(with["machine"]["avx2"], cpu.avx2);
        assert_eq!(with["machine"]["threads"], 8);
    }

    #[test]
    fn the_index_block_holds_no_text_and_no_path() {
        let text = serde_json::to_string(&sample_block(Some(&memo_of(1, 4)))).unwrap();

        for secret in ["secret", "folder", "guide", "crates", "hidden", "lib.rs", "memo.bin", "warning"] {
            assert!(!text.contains(secret), "{secret} leaked into {text}");
        }
    }

    #[test]
    fn index_seconds_and_corpus_keep_their_shape() {
        let cases = two_cases();
        let json = serde_json::to_value(two_results(&cases)).unwrap();

        assert!(json["index_seconds"].is_f64(), "{}", json["index_seconds"]);
        let mut corpus: Vec<&str> = json["corpus"].as_object().unwrap().keys().map(String::as_str).collect();
        corpus.sort();
        assert_eq!(corpus, ["docs", "left_out", "passages", "skipped"]);
        assert!(json["corpus"]["docs"].is_u64() && json["corpus"]["left_out"].is_array());
        assert_eq!(json["index"]["build"], "debug");
        let keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
        let at = |k: &str| keys.iter().position(|x| *x == k).unwrap();
        assert!(at("index_seconds") < at("index") && at("index") < at("aggregate"), "{keys:?}");
    }

    #[test]
    fn a_results_file_without_an_index_block_still_diffs() {
        let old = docs_file("g", &[], &[("q1", false, Some(1)), ("q2", false, Some(4))]);
        let cases = two_cases();
        let new = serde_json::to_value(two_results(&cases)).unwrap();
        assert!(new.get("index").is_some());

        let diff = diff_values(&old, &new).unwrap().expect("two docs files");

        assert_eq!(diff.all.joined, 2);
        assert!(!render_diff(&diff).contains("index"), "no time is compared");
    }

    #[test]
    fn a_results_file_with_an_index_block_still_diffs_against_an_old_one() {
        let old = docs_file("g", &[], &[("q1", false, Some(1)), ("q2", false, Some(4))]);
        let cases = two_cases();
        let new = serde_json::to_value(two_results(&cases)).unwrap();

        let diff = diff_values(&new, &old).unwrap().expect("two docs files");

        assert_eq!(diff.all.joined, 2);
        assert!(!render_diff(&diff).contains("index"), "no time is compared");
    }

    #[test]
    fn the_report_shows_the_phases_after_its_first_line() {
        let cases = two_cases();
        let results = [CaseResult { case: &cases[0], rank: Some(1), top: None, code: false }];
        let phases = Phases::of(&sample_run());

        let text = render(&EvalDocReport { docs: 2, passages: 5, skipped: 1 }, Duration::from_millis(2500), &[], &results, &phases, None);

        assert!(text.starts_with("indexed 2 docs into 5 passages (1 skipped) in 2.5 s\nphases: scan 0.50 s  read 0.50 s"), "{text}");
        assert!(text.contains("embed 4.00 s  write 1.25 s\nleft out 0 docs"), "{text}");
    }

    fn lengths_of(docs: &[&[usize]]) -> EvalPassageLengths {
        EvalPassageLengths {
            docs: docs
                .iter()
                .enumerate()
                .map(|(i, lengths)| EvalDocLengths { path: format!("doc{i}.md"), lengths: lengths.to_vec() })
                .collect(),
            count_seconds: 0.25,
        }
    }

    #[test]
    fn the_index_block_carries_the_tokens_of_a_real_run() {
        let tokens = Tokens::of(&lengths_of(&[&[10, 40, 25]]), 120.0);
        let block = IndexBlock::new(&sample_run(), None, None, EvalCpu::this(), 8, true, Some(tokens));

        let json = serde_json::to_value(&block).unwrap();

        assert_eq!(
            json["tokens"],
            serde_json::json!({
                "real": 75, "padded": 120, "padded_ran": 120,
                "padded_sorted_in_doc": 120, "padded_sorted_all": 120, "count_seconds": 0.25
            })
        );
        let text = serde_json::to_string(&json).unwrap();
        assert!(!text.contains("doc0"), "no path: {text}");
        let results = [CaseResult { case: &two_cases()[0], rank: Some(1), top: None, code: false }];
        let report = render(&EvalDocReport { docs: 2, passages: 5, skipped: 1 }, Duration::ZERO, &[], &results, &Phases::default(), Some(RealRun { block: &block, memo: None }));
        assert!(report.contains("tokens: real 75  padded 120  ran 120  sorted in doc 120  sorted in run 120  counted in 0.25 s\n"), "{report}");
    }

    #[test]
    fn the_report_prints_one_line_per_area_of_a_real_run() {
        let block = sample_block(None);
        let cases = two_cases();
        let results = [CaseResult { case: &cases[0], rank: Some(1), top: None, code: false }];
        let report = EvalDocReport { docs: 2, passages: 5, skipped: 1 };

        let real = render(&report, Duration::ZERO, &[], &results, &Phases::default(), Some(RealRun { block: &block, memo: None }));
        let fake = render(&report, Duration::ZERO, &[], &results, &Phases::default(), None);

        assert!(real.contains("
  docs: 1 docs, 3 passages (0 over one batch)  read 0.25 s"), "{real}");
        assert!(real.contains("
  code: 1 docs, 2 passages (0 over one batch)  read 0.25 s"), "{real}");
        assert!(real.contains("
  archive: 0 docs, 0 passages"), "{real}");
        assert!(!fake.contains("archive:"), "{fake}");
    }

    fn real_docs_cli(extra: &[&str], root: &Path, cache: &Path) -> Cli {
        let mut args = vec!["rag-eval".to_string(), "--real".into(), "--docs".into(), "--root".into(), root.to_string_lossy().into()];
        args.extend(extra.iter().map(|a| a.to_string()));
        let Ok(crate::cli::ParseOutcome::Run(mut cli)) = crate::cli::parse(args.into_iter()) else {
            panic!("the docs arguments parse");
        };
        cli.cache_dir = cache.to_path_buf();
        cli
    }

    fn docs_root_with_one_doc() -> crate::test_temp::TempPath {
        let root = crate::test_temp::temp_path("memo-root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("README.md"), "# Proyecto\n## Apagado\nEl daemon se detiene con clawg stop.\n").unwrap();
        root
    }

    fn one_case() -> Vec<DocsCase> {
        parse(r#"[{"id": "q1", "question": "daemon stop", "expected": [{"path": "README.md"}]}]"#).unwrap()
    }

    /// The name and bytes of every file in `dir`, or `None` when it is not there.
    fn folder_bytes(dir: &Path) -> Option<Vec<(String, Vec<u8>)>> {
        let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
            .ok()?
            .map(|e| {
                let e = e.unwrap();
                (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap())
            })
            .collect();
        files.sort();
        Some(files)
    }

    async fn run_with(cli: &Cli, root: &Path) -> i32 {
        let scratch = crate::test_temp::temp_path("memo-scratch");
        let output = Output { stamp: None, golden_sha256: String::new() };
        run_in(cli, &EvalEmbedder::fake(), root, &one_case(), &[], output, &scratch).await
    }

    #[tokio::test]
    async fn a_real_docs_run_keeps_its_vectors_in_the_cache_folder() {
        let root = docs_root_with_one_doc();
        let cache = crate::test_temp::temp_path("memo-cache");
        let cli = real_docs_cli(&[], &root, &cache);

        assert_eq!(run_with(&cli, &root).await, EXIT_OK);
        let first = folder_bytes(&cache).expect("the run created the folder");
        assert_eq!(run_with(&cli, &root).await, EXIT_OK);

        assert_eq!(first.len(), 1, "one memo per contract");
        assert!(first[0].0.starts_with("docs-vectors-") && first[0].0.ends_with(".bin"), "{}", first[0].0);
        assert_eq!(folder_bytes(&cache), Some(first), "a second run adds nothing, so it rewrites nothing");
    }

    #[tokio::test]
    async fn no_cache_leaves_the_folder_byte_for_byte() {
        let root = docs_root_with_one_doc();
        let cache = crate::test_temp::temp_path("memo-cache");
        let absent = crate::test_temp::temp_path("memo-absent");
        assert_eq!(run_with(&real_docs_cli(&[], &root, &cache), &root).await, EXIT_OK);
        let before = folder_bytes(&cache);
        assert!(before.as_ref().is_some_and(|f| !f.is_empty()));

        assert_eq!(run_with(&real_docs_cli(&["--no-cache"], &root, &cache), &root).await, EXIT_OK);
        assert_eq!(run_with(&real_docs_cli(&["--no-cache"], &root, &absent), &root).await, EXIT_OK);

        assert_eq!(folder_bytes(&cache), before);
        assert!(!absent.exists(), "a folder that was not there is not created");
    }

    #[tokio::test]
    async fn docs_mode_with_the_fake_embedder_never_touches_the_memo() {
        let root = docs_root_with_one_doc();
        let absent = crate::test_temp::temp_path("memo-absent");
        let mut cli = real_docs_cli(&[], &root, &absent);
        cli.mode = crate::cli::Mode::Fake;

        assert_eq!(run_with(&cli, &root).await, EXIT_OK);

        assert!(!absent.exists());
    }

    #[test]
    fn the_docs_report_names_the_memo_file() {
        let block = sample_block(Some(&memo_of(30, 70)));
        let cases = two_cases();
        let results = [CaseResult { case: &cases[0], rank: Some(1), top: None, code: false }];
        let report = EvalDocReport { docs: 2, passages: 5, skipped: 1 };
        let at = |memo: Option<&EvalMemoReport>| {
            render(&report, Duration::ZERO, &[], &results, &Phases::default(), Some(RealRun { block: &block, memo }))
        };

        let with = at(Some(&memo_of(30, 70)));
        let without = at(None);

        assert!(with.contains("vector memo memo.bin: 30 from the memo, 70 embedded, 0.50 s\n"), "{with}");
        assert!(without.contains("vector memo: not used (--no-cache)\n"), "{without}");
        assert!(!with.contains("a warning that is not for the file"), "{with}");
    }

    #[test]
    fn a_fake_run_reports_no_tokens_and_no_memo() {
        let block = IndexBlock::new(&sample_run(), None, None, EvalCpu::this(), 8, true, None);
        let cases = two_cases();
        let results = [CaseResult { case: &cases[0], rank: Some(1), top: None, code: false }];

        let json = serde_json::to_value(&block).unwrap();
        let report = render(&EvalDocReport { docs: 2, passages: 5, skipped: 1 }, Duration::ZERO, &[], &results, &Phases::default(), None);
        let counted = render(&EvalDocReport { docs: 2, passages: 5, skipped: 1 }, Duration::ZERO, &[], &results, &Phases::default(), Some(RealRun { block: &block, memo: None }));

        assert!(json["tokens"].is_null(), "{json}");
        assert_eq!(json["memo"], serde_json::json!({"used": false, "from_memo": 0, "embedded": 5, "seconds": 0.0}));
        assert!(!report.contains("tokens") && !report.contains("memo"), "{report}");
        assert!(!counted.contains("tokens:"), "a block with no tokens prints none: {counted}");
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

    fn docs_file(sha: &str, left_out: &[&str], ranks: &[(&str, bool, Option<usize>)]) -> serde_json::Value {
        let cases: Vec<_> = ranks
            .iter()
            .map(|(id, code, rank)| serde_json::json!({"id": id, "code": code, "rank": rank, "top": null}))
            .collect();
        serde_json::json!({"kind": "docs", "golden_sha256": sha, "corpus": {"left_out": left_out}, "per_case": cases})
    }

    #[test]
    fn a_docs_diff_counts_moves_and_names_the_top_3_changes() {
        let a = docs_file("g", &[], &[("q1", false, Some(1)), ("q2", false, Some(4)), ("q3", false, None), ("q9", false, Some(1))]);
        let b = docs_file("g", &[], &[("q1", false, Some(5)), ("q2", false, Some(2)), ("q3", false, Some(8))]);

        let diff = diff_values(&a, &b).unwrap().expect("two docs files");

        assert_eq!((diff.all.hit_3.better, diff.all.hit_3.worse), (1, 1));
        assert_eq!(diff.all.hit_3.p_value(), 1.0);
        assert_eq!((diff.all.hit_1.better, diff.all.hit_1.worse), (0, 1));
        assert_eq!((diff.all.hit_5.better, diff.all.hit_5.worse), (0, 0), "q1 and q2 stay within 5, q3 stays out");
        assert_eq!((diff.all.mrr.better, diff.all.mrr.worse), (2, 1));
        assert!(diff.all.mrr.ci.is_some());
        assert_eq!(diff.entered_top_3, ["q2"]);
        assert_eq!(diff.left_top_3, ["q1"]);
        assert_eq!(diff.one_file_only, ["q9"]);
        assert!(diff.warnings.is_empty());
        let text = render_diff(&diff);
        assert!(text.contains("entered the top 3: q2\n") && text.contains("left the top 3: q1\n"), "{text}");
        assert!(text.contains("in one file only, counted in no row: q9\n"), "{text}");
        assert!(text.contains("docs hit@3: not distinguishable"), "{text}");
    }

    #[test]
    fn a_docs_diff_splits_docs_and_code() {
        let a = docs_file("g", &[], &[("d1", false, Some(1)), ("c1", true, None)]);
        let b = docs_file("g", &[], &[("d1", false, Some(1)), ("c1", true, Some(2))]);

        let diff = diff_values(&a, &b).unwrap().unwrap();

        assert_eq!(diff.docs.map(|d| (d.joined, d.hit_3.better)), Some((1, 0)));
        assert_eq!(diff.code.map(|c| (c.joined, c.hit_3.better)), Some((1, 1)));
        assert!(render_diff(&diff).contains("  code (1 joined):\n"));

        let only_docs = docs_file("g", &[], &[("d1", false, Some(1))]);
        let diff = diff_values(&only_docs, &only_docs).unwrap().unwrap();
        assert!(diff.code.is_none(), "no code question, no code row");
    }

    #[test]
    fn a_docs_diff_warns_when_the_corpus_differs() {
        let a = docs_file("g1", &[], &[("q1", false, Some(1))]);
        let b = docs_file("g2", &["notes/verify.md"], &[("q1", false, Some(1))]);

        let diff = diff_values(&a, &b).unwrap().unwrap();

        assert_eq!(diff.warnings.len(), 2, "{:?}", diff.warnings);
        assert!(render_diff(&diff).contains("warning: the docs left out differ"));
        assert!(diff_values(&serde_json::json!({"retrieval": {}}), &serde_json::json!({})).unwrap().is_none(), "two memory runs are not docs runs");
        assert!(matches!(diff_values(&a, &serde_json::json!({})), Err(DocsDiffError::Mixed)));
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

        let text = render(&report, Duration::from_millis(1500), &[], &results, &Phases::default(), None);

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

        let text = render(&EvalDocReport { docs: 3, passages: 3, skipped: 0 }, Duration::ZERO, &[], &results, &Phases::default(), None);

        assert!(text.contains("  docs  Hit@1 0.00  Hit@3 0.00  Hit@5 0.00  MRR@10 0.00  over 1 questions\n"), "{text}");
        assert!(text.contains("  code  Hit@1 0.50  Hit@3 1.00  Hit@5 1.00  MRR@10 0.75  over 2 questions\n"), "{text}");
    }

    #[tokio::test]
    async fn the_shipped_golden_set_names_docs_this_workspace_has() {
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

        let store = crate::test_temp::temp_path("shipped-golden");
        std::fs::create_dir_all(&store).unwrap();
        let index = EvalDocIndex::open(&store, &EvalEmbedder::fake(), &store).await.unwrap();
        let crumbs = expected_breadcrumbs(&index, &root, &cases);
        index.close().await.unwrap();
        check_headings(&cases, &crumbs.unwrap()).unwrap();

        let code = cases.iter().filter(|c| is_code_path(&c.expected[0].path)).count();
        let docs = cases.len() - code;
        assert!(cases.len() >= 40 && docs >= 15 && code >= 15, "{docs} questions of docs and {code} of code");
    }
}
