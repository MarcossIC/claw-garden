//! Hand-rolled argument parsing for the retrieval-evaluation binary. No new
//! dependency is pulled in for this: the flag surface is small and fixed, and
//! a parser this size stays more readable written out directly than behind a
//! builder API whose main job would be parsing two mutually exclusive flags
//! and four optional ones.

use std::path::PathBuf;

/// Which embedder backs a run. Exactly one of `--fake`/`--real` is required
/// unless `--help` was given; the two are mutually exclusive with each
/// other, not merely with themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Fake,
    Real,
}

// `PartialEq` only, not `Eq`: `sweep_imp`/`sweep_cos_floor` hold `f64`, which
// has no total order and therefore no `Eq` impl.
#[derive(Debug, Clone, PartialEq)]
pub struct Cli {
    pub mode: Mode,
    pub check_only: bool,
    pub golden_path: PathBuf,
    pub out_dir: PathBuf,
    pub force: bool,
    /// Audit the golden set's own fitness in addition to scoring retrieval,
    /// and fail the run if it cannot carry a claim. Additive rather than a
    /// third mode: the audit reads the rankings the normal run already
    /// produces, so making it exclusive with `--fake`/`--real` would mean
    /// computing them twice to answer two questions about the same run.
    pub audit: bool,
    /// Distinguishes this run's results file from another of the same
    /// date+mode — see [`crate::report::results_filename`]. Its whole reason
    /// to exist is keeping `--force` away from a baseline: without it, two
    /// real runs taken on one day collide on `{date}-{mode}.json`, and
    /// `--force` becomes the tempting fix that overwrites the very file the
    /// second run was supposed to be compared against.
    pub tag: Option<String>,
    /// Each repetition of `--sweep-imp <min>:<max>` — the importance-factor
    /// compression range `ImportanceFactor::spanning` derives `(base,
    /// divisor)` from. Empty by default: building the empty-vec-means-one-
    /// default-point cartesian product is main.rs's job, not this parser's —
    /// it only reports what was actually typed.
    pub sweep_imp: Vec<(f64, f64)>,
    /// Each repetition of `--sweep-cos-floor <f>`, one candidate cosine floor
    /// per entry. Empty by default, same reasoning as `sweep_imp`.
    pub sweep_cos_floor: Vec<f64>,
    /// Each repetition of `--sweep-lexical <n>`, one candidate lexical-channel
    /// budget per entry — `0` means the channel is off for that
    /// point. Empty by default, same reasoning as `sweep_imp`/`sweep_cos_floor`:
    /// building the "empty axis leaves the value untouched" cartesian product
    /// is `main.rs`'s job, this parser only reports what was actually typed.
    pub sweep_lexical: Vec<usize>,
    /// `--fts-tokenizer <name>` — swaps `memory_fts`'s tokenizer for this run
    /// only, via `EvalStore::recreate_fts_with_tokenizer`. `None` by default,
    /// same shape as `--tag`: a single optional value, not a sweep axis of
    /// its own — allowed values are validated where the DDL is actually
    /// built, not here, the same "parser reports what was typed, the callee
    /// validates" split `--golden`/`--out` already follow for paths.
    pub fts_tokenizer: Option<String>,
    /// `--follow-up`: run the synthetic Ceiling/Floor/Treatment arms instead
    /// of the normal single-arm scoring pass. Mutually exclusive with every
    /// sweep axis (`--sweep-imp`, `--sweep-cos-floor`, `--sweep-lexical`) and
    /// with `--fts-tokenizer` — the sweep varies scoring config, this varies
    /// query TEXT, and crossing them would multiply runs while making the
    /// resolvability gate per-point instead of per-run.
    pub follow_up: bool,
    /// `--agent-view`: ingest the corpus through the real pipeline and score
    /// the text the MCP tools return, instead of the store's own ranking.
    /// Mutually exclusive with `--follow-up` and every sweep flag — those vary
    /// the ranking, this measures what an agent sees of a single fixed one.
    pub agent_view: bool,
    /// `--docs`: index a project's documentation and score the questions of
    /// the docs golden set against it, instead of seeding memories. Mutually
    /// exclusive with `--audit`, `--follow-up`, `--agent-view` and every
    /// sweep flag, which all measure memory retrieval.
    pub docs: bool,
    /// `--root <dir>`, the project `--docs` indexes.
    pub docs_root: PathBuf,
    /// `--lexical-only`: rank every case by the lexical channel alone (bm25,
    /// no vectors), as a floor the full ranking has to beat. Mutually
    /// exclusive with the other arms and every sweep flag.
    pub lexical_only: bool,
    /// `--background <snapshot>`: a snapshot `clawg backup` wrote, copied and
    /// measured under the golden corpus. The sweep flags vary the ranking over
    /// it; the modes that measure the golden corpus alone, and a tokenizer
    /// swap that would empty its text index, are refused with it.
    pub background: Option<PathBuf>,
    /// `--keep-store`: leave the store a run seeds on disk and name it, to
    /// open the database behind a number. Without it the run removes it.
    pub keep_store: bool,
}

/// What a parse attempt produces. `--help` is not an error: it takes
/// precedence over every other flag (including a missing or conflicting
/// mode) and short-circuits the moment it is seen. `--diff` short-circuits
/// the same way and for the same reason: comparing two existing results
/// files needs neither a golden set nor a mode, so requiring either one
/// first would make `--diff` depend on inputs it never reads.
// `PartialEq` only — see the note on `Cli` above; `ParseOutcome::Run` wraps one.
#[derive(Debug, PartialEq)]
pub enum ParseOutcome {
    Help,
    Run(Cli),
    Diff { a: PathBuf, b: PathBuf, fail_on_regression: bool },
    /// `--lang-detect`: label the golden rows' language, with neither a
    /// model nor a store, so no mode is needed.
    LangDetect { golden: PathBuf },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CliError {
    #[error("unknown flag '{0}'")]
    UnknownFlag(String),
    #[error("flag '{0}' requires a value")]
    MissingValue(String),
    #[error("exactly one of --fake or --real is required (see --help)")]
    MissingMode,
    #[error("--fake and --real are mutually exclusive")]
    ConflictingMode,
    #[error("invalid --sweep-imp value '{0}', expected '<min>:<max>' (e.g. '0.55:1.0')")]
    InvalidSweepImp(String),
    #[error("invalid --sweep-cos-floor value '{0}', expected a floating-point number")]
    InvalidSweepCosFloor(String),
    #[error("invalid --sweep-lexical value '{0}', expected a non-negative integer")]
    InvalidSweepLexical(String),
    #[error("--follow-up is mutually exclusive with the sweep flags (--sweep-imp, --sweep-cos-floor, --sweep-lexical, --fts-tokenizer)")]
    FollowUpWithSweep,
    #[error("--agent-view is mutually exclusive with --follow-up and the sweep flags (--sweep-imp, --sweep-cos-floor, --sweep-lexical, --fts-tokenizer)")]
    AgentViewWithOtherMode,
    #[error("--docs is mutually exclusive with --audit, --follow-up, --agent-view and the sweep flags (--sweep-imp, --sweep-cos-floor, --sweep-lexical, --fts-tokenizer)")]
    DocsWithOtherMode,
    #[error("--root is only meaningful with --docs")]
    RootWithoutDocs,
    #[error("--lexical-only is mutually exclusive with --agent-view, --follow-up, --docs and the sweep flags (--sweep-imp, --sweep-cos-floor, --sweep-lexical, --fts-tokenizer)")]
    LexicalOnlyWithOtherMode,
    #[error("--fail-on-regression is only meaningful with --diff")]
    FailOnRegressionWithoutDiff,
    #[error("--background is mutually exclusive with --docs, --lang-detect, --fts-tokenizer, --follow-up, --audit and --lexical-only")]
    BackgroundWithOtherMode,
    #[error("--keep-store is mutually exclusive with --background, --agent-view and --docs, which always remove their store")]
    KeepStoreWithOtherMode,
}

pub const HELP_TEXT: &str = "\
rag-eval - retrieval-quality measurement harness for Claw Garden's memory subsystem

USAGE:
    rag-eval (--fake | --real) [OPTIONS]

MODES (exactly one required):
    --fake              Seed and score using the deterministic FakeEmbedder (no model, no network)
    --real              Seed and score using the real multilingual-e5-small model

OPTIONS:
    --check-only        Valid only with --real: verify the model is cached without downloading it
                         or running the harness. Combined with --fake this is a harmless no-op.
    --audit             Also audit whether the golden set can carry a claim at all
                         (saturation, query-independence of the ranking, and whether
                         queries restate their own answers). Exits 5 if it cannot.
    --golden <path>     Override the default golden set (docs/eval/golden.json)
    --out <dir>         Override the default results directory (docs/eval/results/)
    --tag <slug>        Distinguish this run's results file from another of the same
                         date+mode: {date}-{mode}-{tag}.json instead of {date}-{mode}.json
    --sweep-imp <min>:<max>
                         Repeatable. Adds one importance-factor compression range to the
                         sweep grid. Zero repetitions leaves importance at its default range.
    --sweep-cos-floor <f>
                         Repeatable. Adds one candidate cosine floor to the sweep grid.
                         Zero repetitions leaves cosine re-scaling off (None).
    --sweep-lexical <n>
                         Repeatable. Adds one candidate lexical-channel budget to the
                         sweep grid; 0 means the channel is off for that point.
                         Zero repetitions leaves the lexical channel at its default.
    --fts-tokenizer <name>
                         Swaps memory_fts's tokenizer for this run only (allowed:
                         'unicode61 remove_diacritics 2', 'trigram'). Absent by default,
                         which leaves the store on the tokenizer the migration applied.
                         A run that passes this writes to its own results filename and
                         records the tokenizer in the file, so two runs differing only
                         in this flag stay comparable side by side.
    --force             Allow overwriting an existing results file for the same date+config
    --keep-store        Leave the store a run seeds in the system temp directory and print
                         its path, to open the database behind a number. Without it the run
                         removes the store when it ends, passed or failed. Mutually exclusive
                         with --background, --agent-view and --docs, which always remove theirs.
    --diff <a.json> <b.json>
                         Compare two existing results files' retrieval.per_case and
                         similarity.per_case, joined on query, reporting better/worse/net
                         and the exact paired p-value per metric per block (with a bootstrap
                         interval of the mean change for mrr and ndcg@10). When both files
                         come from --agent-view, also search.found, context.found and the
                         tokens of each tool. When the cases carry query_lang, each block is
                         also cut by the language pair of query and row (such as es>en),
                         over the cases with that pair in both files; the pair rows are
                         informative and never enter primary: nor --fail-on-regression.
                         Ends with whether each primary metric (recall@5, search.found)
                         moved distinguishably at p < 0.05. When a file was measured over
                         --background it names each side's snapshot and search time, and warns
                         when both searched one snapshot more than a day apart (the exit code
                         stays the metrics'). Needs neither --fake/--real nor a golden set;
                         short-circuits like --help.
    --fail-on-regression With --diff, on either side of it: exit 6 when a primary metric
                         (retrieval recall@5, agent_view search.found) gets worse with
                         p < 0.05. The informative metrics never fail the run.
    --follow-up          Run the synthetic Ceiling/Floor/Treatment arms instead of the
                         normal single-arm pass, measuring whether conversational context
                         helps a content-free follow-up retrieve the right document.
                         Mutually exclusive with every sweep flag and --fts-tokenizer.
    --agent-view         Ingest the corpus through the real save pipeline, answer every case
                         through the daemon's own search and context endpoints, and score the
                         text an agent would actually receive: whether the expected rows are
                         visible, their token cost and their latency. Writes
                         {date}-{mode}-agent-view.json. Mutually exclusive with --follow-up
                         and every sweep flag.
    --docs              Index a project's documentation (Markdown and manifests) into a
                         temporary store and score the questions of the docs golden set
                         (crates/clawg-eval/golden/project-docs.json unless --golden says
                         otherwise): Hit@1/3/5 and MRR@10, printed. Writes no results file.
                         Mutually exclusive with --audit, --follow-up, --agent-view and
                         every sweep flag.
    --root <dir>        With --docs: the project to index (default: this workspace)
    --lexical-only      Rank every case by the lexical channel alone: the same FTS5 match and
                         bm25 order the full search uses, with no vectors and no scoring. A
                         floor for the full ranking; writes {date}-{mode}-lexical-only.json,
                         which --diff compares with any other run. Mutually exclusive with
                         --agent-view, --follow-up, --docs and every sweep flag.
    --background <snapshot>
                         Measure over a copy of a snapshot `clawg backup` wrote, with the golden
                         corpus seeded on top and every search reaching every agent's rows, as
                         the daemon does. The snapshot is only read; the copy, in the system temp
                         directory, is migrated if its schema is older and removed when the run
                         ends. A snapshot with commits in a -wal beside it, damaged, of a newer
                         or diverged schema, or embedded by another model is refused with exit 1.
                         A hit from the snapshot keeps its place in ranked_refs as bg, and the
                         results name the snapshot (path, sha256, contract, schema, the time of
                         the search and its rows by type, agent, language, source and project)
                         and fold bg-<first 8 hex of its sha256> into the tag. Mutually exclusive
                         with --docs, --lang-detect, --fts-tokenizer, --follow-up, --audit and
                         --lexical-only; the sweep flags are accepted, and with --agent-view the
                         corpus is saved through the ingest on top of the copy.
    --lang-detect       Print as JSON how clawg's language detector labels the corpus rows
                         that carry a lang (en or es): by label, in total and for the rows
                         of eight words or more, with the share right, crossed and
                         undecided. Counts only, no row text. Needs neither --fake/--real
                         nor a store; reads --golden (default: docs/eval/golden.json).
    --help, -h          Print this message and exit

EXIT CODES:
    0    success, or --check-only confirmed the model is ready
    1    runtime failure (I/O, embedding, database, or seeding error)
    2    golden set failed validation (with --docs: also when it names a doc the root lacks)
    3    --real --check-only and the model is not cached
    4    a results file for this date+config already exists and --force was not given
    5    --audit and the golden set is not fit to measure against
    6    --diff --fail-on-regression and a primary metric got worse with p < 0.05
";

/// Default golden-set path, resolved relative to this crate's own manifest
/// directory rather than the process's current directory: `docs/eval/` lives
/// two levels above `crates/clawg-eval`, and a binary invoked via `cargo run`
/// from an unexpected working directory must still find it.
pub fn default_golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("docs")
        .join("eval")
        .join("golden.json")
}

/// The docs golden set, versioned with this crate: it asks about this
/// repository's own public docs.
pub fn default_docs_golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("golden").join("project-docs.json")
}

/// The workspace this crate belongs to, the project `--docs` indexes by default.
pub fn default_docs_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Default results directory, resolved the same CWD-independent way as
/// [`default_golden_path`].
pub fn default_out_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("docs")
        .join("eval")
        .join("results")
}

/// Parses an argv-shaped iterator. The first element (the program name, the
/// same thing `std::env::args()` yields at index 0) is skipped unconditionally.
pub fn parse<I: Iterator<Item = String>>(mut args: I) -> Result<ParseOutcome, CliError> {
    args.next(); // argv[0], not a flag

    let mut mode: Option<Mode> = None;
    let mut check_only = false;
    let mut golden_path: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut force = false;
    let mut audit = false;
    let mut tag: Option<String> = None;
    let mut sweep_imp: Vec<(f64, f64)> = Vec::new();
    let mut sweep_cos_floor: Vec<f64> = Vec::new();
    let mut sweep_lexical: Vec<usize> = Vec::new();
    let mut fts_tokenizer: Option<String> = None;
    let mut follow_up = false;
    let mut agent_view = false;
    let mut docs = false;
    let mut docs_root: Option<PathBuf> = None;
    let mut fail_on_regression = false;
    let mut lexical_only = false;
    let mut lang_detect = false;
    let mut background: Option<PathBuf> = None;
    let mut keep_store = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(ParseOutcome::Help),
            "--lang-detect" => lang_detect = true,
            "--follow-up" => follow_up = true,
            "--agent-view" => agent_view = true,
            "--docs" => docs = true,
            "--fail-on-regression" => fail_on_regression = true,
            "--lexical-only" => lexical_only = true,
            "--root" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::MissingValue("--root".to_string()))?;
                docs_root = Some(PathBuf::from(value));
            }
            "--diff" => {
                let a = args
                    .next()
                    .ok_or_else(|| CliError::MissingValue("--diff".to_string()))?;
                let b = args
                    .next()
                    .ok_or_else(|| CliError::MissingValue("--diff".to_string()))?;
                let fail_on_regression = fail_on_regression || args.any(|rest| rest == "--fail-on-regression");
                return Ok(ParseOutcome::Diff { a: PathBuf::from(a), b: PathBuf::from(b), fail_on_regression });
            }
            "--fake" => {
                if matches!(mode, Some(Mode::Real)) {
                    return Err(CliError::ConflictingMode);
                }
                mode = Some(Mode::Fake);
            }
            "--real" => {
                if matches!(mode, Some(Mode::Fake)) {
                    return Err(CliError::ConflictingMode);
                }
                mode = Some(Mode::Real);
            }
            "--check-only" => check_only = true,
            "--force" => force = true,
            "--keep-store" => keep_store = true,
            "--audit" => audit = true,
            "--golden" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::MissingValue("--golden".to_string()))?;
                golden_path = Some(PathBuf::from(value));
            }
            "--out" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::MissingValue("--out".to_string()))?;
                out_dir = Some(PathBuf::from(value));
            }
            "--tag" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::MissingValue("--tag".to_string()))?;
                tag = Some(value);
            }
            "--sweep-imp" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::MissingValue("--sweep-imp".to_string()))?;
                let (min_str, max_str) = value
                    .split_once(':')
                    .ok_or_else(|| CliError::InvalidSweepImp(value.clone()))?;
                let min: f64 = min_str.parse().map_err(|_| CliError::InvalidSweepImp(value.clone()))?;
                let max: f64 = max_str.parse().map_err(|_| CliError::InvalidSweepImp(value.clone()))?;
                sweep_imp.push((min, max));
            }
            "--sweep-cos-floor" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::MissingValue("--sweep-cos-floor".to_string()))?;
                let floor: f64 =
                    value.parse().map_err(|_| CliError::InvalidSweepCosFloor(value.clone()))?;
                sweep_cos_floor.push(floor);
            }
            "--sweep-lexical" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::MissingValue("--sweep-lexical".to_string()))?;
                let budget: usize =
                    value.parse().map_err(|_| CliError::InvalidSweepLexical(value.clone()))?;
                sweep_lexical.push(budget);
            }
            "--fts-tokenizer" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::MissingValue("--fts-tokenizer".to_string()))?;
                fts_tokenizer = Some(value);
            }
            "--background" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::MissingValue("--background".to_string()))?;
                background = Some(PathBuf::from(value));
            }
            other => return Err(CliError::UnknownFlag(other.to_string())),
        }
    }

    if background.is_some() && (docs || lang_detect || fts_tokenizer.is_some() || follow_up || audit || lexical_only) {
        return Err(CliError::BackgroundWithOtherMode);
    }
    if keep_store && (background.is_some() || agent_view || docs) {
        return Err(CliError::KeepStoreWithOtherMode);
    }
    if lang_detect {
        return Ok(ParseOutcome::LangDetect { golden: golden_path.unwrap_or_else(default_golden_path) });
    }
    let mode = mode.ok_or(CliError::MissingMode)?;

    // `--follow-up` varies query TEXT; the sweep axes vary scoring CONFIG.
    // Crossing them would multiply runs and turn the resolvability gate
    // per-point instead of per-run — rejected before any run starts, same
    // spirit as the mode conflict check above.
    if follow_up
        && (!sweep_imp.is_empty()
            || !sweep_cos_floor.is_empty()
            || !sweep_lexical.is_empty()
            || fts_tokenizer.is_some())
    {
        return Err(CliError::FollowUpWithSweep);
    }

    if agent_view
        && (follow_up
            || !sweep_imp.is_empty()
            || !sweep_cos_floor.is_empty()
            || !sweep_lexical.is_empty()
            || fts_tokenizer.is_some())
    {
        return Err(CliError::AgentViewWithOtherMode);
    }

    if docs
        && (audit
            || follow_up
            || agent_view
            || !sweep_imp.is_empty()
            || !sweep_cos_floor.is_empty()
            || !sweep_lexical.is_empty()
            || fts_tokenizer.is_some())
    {
        return Err(CliError::DocsWithOtherMode);
    }
    if lexical_only
        && (follow_up
            || agent_view
            || docs
            || !sweep_imp.is_empty()
            || !sweep_cos_floor.is_empty()
            || !sweep_lexical.is_empty()
            || fts_tokenizer.is_some())
    {
        return Err(CliError::LexicalOnlyWithOtherMode);
    }
    if fail_on_regression {
        return Err(CliError::FailOnRegressionWithoutDiff);
    }
    if docs_root.is_some() && !docs {
        return Err(CliError::RootWithoutDocs);
    }

    let default_golden = if docs { default_docs_golden_path } else { default_golden_path };
    Ok(ParseOutcome::Run(Cli {
        mode,
        check_only,
        golden_path: golden_path.unwrap_or_else(default_golden),
        out_dir: out_dir.unwrap_or_else(default_out_dir),
        force,
        audit,
        tag,
        sweep_imp,
        sweep_cos_floor,
        sweep_lexical,
        fts_tokenizer,
        follow_up,
        agent_view,
        docs,
        docs_root: docs_root.unwrap_or_else(default_docs_root),
        lexical_only,
        background,
        keep_store,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> impl Iterator<Item = String> {
        std::iter::once("rag-eval".to_string())
            .chain(items.iter().map(|s| s.to_string()))
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn lang_detect_needs_no_mode() {
        assert_eq!(
            parse(argv(&["--lang-detect", "--golden", "x.json"])),
            Ok(ParseOutcome::LangDetect { golden: PathBuf::from("x.json") })
        );
        assert_eq!(parse(argv(&["--lang-detect"])), Ok(ParseOutcome::LangDetect { golden: default_golden_path() }));
    }

    fn run_cli(items: &[&str]) -> Cli {
        match parse(argv(items)).expect("parse must succeed") {
            ParseOutcome::Run(cli) => cli,
            ParseOutcome::Help => panic!("expected Run, got Help"),
            ParseOutcome::Diff { a, b, .. } => panic!("expected Run, got Diff({a:?}, {b:?})"),
            ParseOutcome::LangDetect { golden } => panic!("expected Run, got LangDetect({golden:?})"),
        }
    }

    #[test]
    fn background_parses() {
        assert_eq!(run_cli(&["--fake", "--background", "snap.db"]).background, Some(PathBuf::from("snap.db")));
        assert_eq!(run_cli(&["--fake"]).background, None);
        let swept = run_cli(&["--real", "--background", "snap.db", "--sweep-cos-floor", "0.8", "--sweep-lexical", "0"]);
        assert_eq!(swept.sweep_cos_floor, vec![0.8]);
        assert_eq!(parse(argv(&["--fake", "--background"])), Err(CliError::MissingValue("--background".to_string())));
        let viewed = run_cli(&["--fake", "--agent-view", "--background", "snap.db"]);
        assert!(viewed.agent_view && viewed.background.is_some(), "agent-view measures over a background too");
    }

    #[test]
    fn background_with_docs_is_a_usage_error() {
        for other in [
            &["--docs"][..],
            &["--lang-detect"],
            &["--fts-tokenizer", "trigram"],
            &["--follow-up"],
            &["--audit"],
            &["--lexical-only"],
        ] {
            let args: Vec<&str> = ["--fake", "--background", "snap.db"].iter().chain(other).copied().collect();
            assert_eq!(parse(argv(&args)), Err(CliError::BackgroundWithOtherMode), "{other:?}");
        }
    }

    #[test]
    fn keep_store_parses() {
        assert!(run_cli(&["--fake", "--keep-store"]).keep_store);
        assert!(!run_cli(&["--fake"]).keep_store, "a run removes the store it seeds unless asked to keep it");
    }

    #[test]
    fn keep_store_with_a_mode_that_always_removes_its_store_is_a_usage_error() {
        for other in [&["--background", "snap.db"][..], &["--agent-view"], &["--docs"]] {
            let args: Vec<&str> = ["--fake", "--keep-store"].iter().chain(other).copied().collect();
            assert_eq!(parse(argv(&args)), Err(CliError::KeepStoreWithOtherMode), "{other:?}");
        }
    }

    #[test]
    fn fake_alone_uses_defaults() {
        let cli = run_cli(&["--fake"]);
        assert_eq!(cli.mode, Mode::Fake);
        assert!(!cli.check_only);
        assert!(!cli.force);
        assert!(!cli.audit, "auditing must be opt-in: it can fail an otherwise fine run");
        assert_eq!(cli.golden_path, default_golden_path());
        assert_eq!(cli.out_dir, default_out_dir());
    }

    #[test]
    fn audit_composes_with_either_mode_rather_than_replacing_it() {
        assert!(run_cli(&["--fake", "--audit"]).audit);
        let cli = run_cli(&["--audit", "--real"]);
        assert!(cli.audit);
        assert_eq!(cli.mode, Mode::Real);
    }

    #[test]
    fn real_with_check_only_and_force() {
        let cli = run_cli(&["--real", "--check-only", "--force"]);
        assert_eq!(cli.mode, Mode::Real);
        assert!(cli.check_only);
        assert!(cli.force);
    }

    #[test]
    fn tag_is_none_by_default() {
        assert_eq!(run_cli(&["--fake"]).tag, None);
    }

    #[test]
    fn tag_is_captured_verbatim_when_given() {
        let cli = run_cli(&["--real", "--tag", "pre-prefix"]);
        assert_eq!(cli.tag, Some("pre-prefix".to_string()));
    }

    #[test]
    fn tag_without_a_value_is_an_error() {
        let err = parse(argv(&["--fake", "--tag"])).expect_err("missing value must error");
        assert_eq!(err, CliError::MissingValue("--tag".to_string()));
    }

    #[test]
    fn golden_and_out_overrides_are_captured_verbatim() {
        let cli = run_cli(&["--fake", "--golden", "/tmp/g.json", "--out", "/tmp/out"]);
        assert_eq!(cli.golden_path, PathBuf::from("/tmp/g.json"));
        assert_eq!(cli.out_dir, PathBuf::from("/tmp/out"));
    }

    #[test]
    fn flag_order_does_not_matter() {
        let cli = run_cli(&["--out", "/tmp/out", "--force", "--fake", "--golden", "/tmp/g.json"]);
        assert_eq!(cli.mode, Mode::Fake);
        assert!(cli.force);
        assert_eq!(cli.golden_path, PathBuf::from("/tmp/g.json"));
        assert_eq!(cli.out_dir, PathBuf::from("/tmp/out"));
    }

    #[test]
    fn repeating_the_same_mode_flag_is_not_a_conflict() {
        let cli = run_cli(&["--fake", "--fake"]);
        assert_eq!(cli.mode, Mode::Fake);
    }

    #[test]
    fn missing_mode_is_an_error() {
        let err = parse(argv(&["--force"])).expect_err("missing mode must error");
        assert_eq!(err, CliError::MissingMode);
    }

    #[test]
    fn conflicting_modes_is_an_error() {
        let err = parse(argv(&["--fake", "--real"])).expect_err("conflicting modes must error");
        assert_eq!(err, CliError::ConflictingMode);
        // Order reversed still conflicts.
        let err = parse(argv(&["--real", "--fake"])).expect_err("conflicting modes must error");
        assert_eq!(err, CliError::ConflictingMode);
    }

    #[test]
    fn unknown_flag_is_an_error() {
        let err = parse(argv(&["--fake", "--bogus"])).expect_err("unknown flag must error");
        assert_eq!(err, CliError::UnknownFlag("--bogus".to_string()));
    }

    #[test]
    fn golden_without_a_value_is_an_error() {
        let err = parse(argv(&["--fake", "--golden"])).expect_err("missing value must error");
        assert_eq!(err, CliError::MissingValue("--golden".to_string()));
    }

    #[test]
    fn out_without_a_value_is_an_error() {
        let err = parse(argv(&["--fake", "--out"])).expect_err("missing value must error");
        assert_eq!(err, CliError::MissingValue("--out".to_string()));
    }

    #[test]
    fn help_short_and_long_both_short_circuit() {
        assert_eq!(parse(argv(&["--help"])).unwrap(), ParseOutcome::Help);
        assert_eq!(parse(argv(&["-h"])).unwrap(), ParseOutcome::Help);
    }

    #[test]
    fn help_short_circuits_even_without_a_mode() {
        // --help must never be rejected as "missing mode": it is checked as
        // it is encountered, not after the whole line is parsed.
        assert_eq!(parse(argv(&["--help"])).unwrap(), ParseOutcome::Help);
    }

    #[test]
    fn help_short_circuits_before_a_later_conflict_would_fire() {
        assert_eq!(
            parse(argv(&["--fake", "--help", "--real"])).unwrap(),
            ParseOutcome::Help
        );
    }

    #[test]
    fn diff_flag_captures_both_paths_and_short_circuits_without_a_mode() {
        // No --fake/--real given: --diff must not fall through to
        // MissingMode, because comparing two results files needs neither.
        assert_eq!(
            parse(argv(&["--diff", "/tmp/a.json", "/tmp/b.json"])).unwrap(),
            diff_of("/tmp/a.json", "/tmp/b.json", false)
        );
    }

    #[test]
    fn diff_short_circuits_before_a_later_conflict_would_fire() {
        assert_eq!(
            parse(argv(&["--fake", "--diff", "/tmp/a.json", "/tmp/b.json", "--real"])).unwrap(),
            diff_of("/tmp/a.json", "/tmp/b.json", false)
        );
    }

    fn diff_of(a: &str, b: &str, fail_on_regression: bool) -> ParseOutcome {
        ParseOutcome::Diff { a: PathBuf::from(a), b: PathBuf::from(b), fail_on_regression }
    }

    #[test]
    fn fail_on_regression_parses_on_either_side_of_diff() {
        assert_eq!(
            parse(argv(&["--diff", "/tmp/a.json", "/tmp/b.json", "--fail-on-regression"])).unwrap(),
            diff_of("/tmp/a.json", "/tmp/b.json", true)
        );
        assert_eq!(
            parse(argv(&["--fail-on-regression", "--diff", "/tmp/a.json", "/tmp/b.json"])).unwrap(),
            diff_of("/tmp/a.json", "/tmp/b.json", true)
        );
    }

    #[test]
    fn lexical_only_conflicts_with_the_other_arms() {
        assert!(run_cli(&["--fake", "--lexical-only"]).lexical_only);
        assert!(!run_cli(&["--fake"]).lexical_only);
        for other in [
            &["--agent-view"][..],
            &["--follow-up"],
            &["--docs"],
            &["--sweep-imp", "0.5:1.0"],
            &["--sweep-cos-floor", "0.8"],
            &["--sweep-lexical", "0"],
            &["--fts-tokenizer", "trigram"],
        ] {
            let args: Vec<&str> = ["--fake", "--lexical-only"].iter().chain(other).copied().collect();
            let err = parse(argv(&args)).expect_err("--lexical-only is an arm of its own");
            assert!(
                matches!(err, CliError::LexicalOnlyWithOtherMode | CliError::DocsWithOtherMode | CliError::AgentViewWithOtherMode),
                "{other:?}: {err:?}"
            );
        }
    }

    #[test]
    fn fail_on_regression_without_diff_is_an_error() {
        let err = parse(argv(&["--fake", "--fail-on-regression"])).expect_err("the flag needs --diff");
        assert_eq!(err, CliError::FailOnRegressionWithoutDiff);
    }

    #[test]
    fn diff_missing_the_first_path_is_an_error() {
        let err = parse(argv(&["--diff"])).expect_err("missing first value must error");
        assert_eq!(err, CliError::MissingValue("--diff".to_string()));
    }

    #[test]
    fn diff_missing_the_second_path_is_an_error() {
        let err = parse(argv(&["--diff", "/tmp/a.json"])).expect_err("missing second value must error");
        assert_eq!(err, CliError::MissingValue("--diff".to_string()));
    }

    #[test]
    fn sweep_imp_is_empty_by_default() {
        assert!(run_cli(&["--fake"]).sweep_imp.is_empty());
    }

    #[test]
    fn sweep_cos_floor_is_empty_by_default() {
        assert!(run_cli(&["--fake"]).sweep_cos_floor.is_empty());
    }

    #[test]
    fn sweep_imp_is_repeatable_and_parses_min_max_pairs() {
        let cli = run_cli(&["--fake", "--sweep-imp", "0.55:1.0", "--sweep-imp", "0.7:1.0"]);
        assert_eq!(cli.sweep_imp, vec![(0.55, 1.0), (0.7, 1.0)]);
    }

    #[test]
    fn sweep_cos_floor_is_repeatable_and_parses_each_value() {
        let cli = run_cli(&["--fake", "--sweep-cos-floor", "0.8625", "--sweep-cos-floor", "0.8793"]);
        assert_eq!(cli.sweep_cos_floor, vec![0.8625, 0.8793]);
    }

    #[test]
    fn sweep_imp_bad_format_is_an_error() {
        let err = parse(argv(&["--fake", "--sweep-imp", "notapair"])).expect_err("bad format must error");
        assert_eq!(err, CliError::InvalidSweepImp("notapair".to_string()));
    }

    #[test]
    fn sweep_imp_non_numeric_bound_is_an_error() {
        let err =
            parse(argv(&["--fake", "--sweep-imp", "abc:1.0"])).expect_err("non-numeric bound must error");
        assert_eq!(err, CliError::InvalidSweepImp("abc:1.0".to_string()));
    }

    #[test]
    fn sweep_imp_without_a_value_is_an_error() {
        let err = parse(argv(&["--fake", "--sweep-imp"])).expect_err("missing value must error");
        assert_eq!(err, CliError::MissingValue("--sweep-imp".to_string()));
    }

    #[test]
    fn sweep_cos_floor_non_numeric_is_an_error() {
        let err = parse(argv(&["--fake", "--sweep-cos-floor", "abc"]))
            .expect_err("non-numeric floor must error");
        assert_eq!(err, CliError::InvalidSweepCosFloor("abc".to_string()));
    }

    #[test]
    fn sweep_cos_floor_without_a_value_is_an_error() {
        let err = parse(argv(&["--fake", "--sweep-cos-floor"])).expect_err("missing value must error");
        assert_eq!(err, CliError::MissingValue("--sweep-cos-floor".to_string()));
    }

    #[test]
    fn sweep_lexical_is_empty_by_default() {
        assert!(run_cli(&["--fake"]).sweep_lexical.is_empty());
    }

    #[test]
    fn sweep_lexical_is_repeatable_and_parses_each_value() {
        let cli = run_cli(&["--fake", "--sweep-lexical", "0", "--sweep-lexical", "20"]);
        assert_eq!(cli.sweep_lexical, vec![0, 20]);
    }

    #[test]
    fn sweep_lexical_non_numeric_is_an_error() {
        let err = parse(argv(&["--fake", "--sweep-lexical", "abc"]))
            .expect_err("non-numeric budget must error");
        assert_eq!(err, CliError::InvalidSweepLexical("abc".to_string()));
    }

    #[test]
    fn sweep_lexical_negative_is_an_error() {
        // usize rejects a leading '-' at parse time, same error path as any
        // other non-numeric value — no separate "negative" variant needed.
        let err = parse(argv(&["--fake", "--sweep-lexical", "-1"]))
            .expect_err("negative budget must error");
        assert_eq!(err, CliError::InvalidSweepLexical("-1".to_string()));
    }

    #[test]
    fn sweep_lexical_without_a_value_is_an_error() {
        let err = parse(argv(&["--fake", "--sweep-lexical"])).expect_err("missing value must error");
        assert_eq!(err, CliError::MissingValue("--sweep-lexical".to_string()));
    }

    #[test]
    fn fts_tokenizer_is_none_by_default() {
        assert_eq!(run_cli(&["--fake"]).fts_tokenizer, None);
    }

    #[test]
    fn fts_tokenizer_is_captured_verbatim_when_given() {
        let cli = run_cli(&["--real", "--fts-tokenizer", "trigram"]);
        assert_eq!(cli.fts_tokenizer, Some("trigram".to_string()));
    }

    #[test]
    fn fts_tokenizer_without_a_value_is_an_error() {
        let err = parse(argv(&["--fake", "--fts-tokenizer"])).expect_err("missing value must error");
        assert_eq!(err, CliError::MissingValue("--fts-tokenizer".to_string()));
    }

    // --- --follow-up -----------------------------------------------------

    #[test]
    fn follow_up_is_false_by_default() {
        assert!(!run_cli(&["--fake"]).follow_up);
    }

    #[test]
    fn follow_up_alone_parses_true() {
        assert!(run_cli(&["--fake", "--follow-up"]).follow_up);
    }

    #[test]
    fn follow_up_with_sweep_lexical_is_an_error() {
        let err = parse(argv(&["--fake", "--follow-up", "--sweep-lexical", "20"]))
            .expect_err("--follow-up + --sweep-lexical must be rejected");
        assert_eq!(err, CliError::FollowUpWithSweep);
    }

    #[test]
    fn follow_up_with_sweep_imp_is_an_error() {
        let err = parse(argv(&["--fake", "--follow-up", "--sweep-imp", "0.55:1.0"]))
            .expect_err("--follow-up + --sweep-imp must be rejected");
        assert_eq!(err, CliError::FollowUpWithSweep);
    }

    #[test]
    fn follow_up_with_sweep_cos_floor_is_an_error() {
        let err = parse(argv(&["--fake", "--follow-up", "--sweep-cos-floor", "0.86"]))
            .expect_err("--follow-up + --sweep-cos-floor must be rejected");
        assert_eq!(err, CliError::FollowUpWithSweep);
    }

    #[test]
    fn follow_up_with_fts_tokenizer_is_an_error() {
        let err = parse(argv(&["--fake", "--follow-up", "--fts-tokenizer", "trigram"]))
            .expect_err("--follow-up + --fts-tokenizer must be rejected");
        assert_eq!(err, CliError::FollowUpWithSweep);
    }

    #[test]
    fn follow_up_order_does_not_matter_for_the_conflict_check() {
        // The sweep flag typed BEFORE --follow-up must still be caught —
        // this is a post-parse check over the final state, not a per-token one.
        let err = parse(argv(&["--fake", "--sweep-lexical", "20", "--follow-up"]))
            .expect_err("order must not matter for the conflict check");
        assert_eq!(err, CliError::FollowUpWithSweep);
    }

    #[test]
    fn follow_up_composes_with_audit_and_force() {
        let cli = run_cli(&["--fake", "--follow-up", "--audit", "--force"]);
        assert!(cli.follow_up);
        assert!(cli.audit);
        assert!(cli.force);
    }

    // --- --agent-view ----------------------------------------------------

    #[test]
    fn agent_view_is_false_by_default() {
        assert!(!run_cli(&["--fake"]).agent_view);
    }

    #[test]
    fn agent_view_alone_parses_true() {
        assert!(run_cli(&["--real", "--agent-view"]).agent_view);
    }

    #[test]
    fn agent_view_with_follow_up_is_an_error() {
        let err = parse(argv(&["--fake", "--agent-view", "--follow-up"]))
            .expect_err("--agent-view + --follow-up must be rejected");
        assert_eq!(err, CliError::AgentViewWithOtherMode);
    }

    #[test]
    fn agent_view_with_every_sweep_flag_is_an_error() {
        for extra in [
            ["--sweep-imp", "0.55:1.0"],
            ["--sweep-cos-floor", "0.86"],
            ["--sweep-lexical", "20"],
            ["--fts-tokenizer", "trigram"],
        ] {
            let err = parse(argv(&["--fake", "--agent-view", extra[0], extra[1]]))
                .expect_err("--agent-view + a sweep flag must be rejected");
            assert_eq!(err, CliError::AgentViewWithOtherMode, "{extra:?}");
        }
    }

    #[test]
    fn agent_view_order_does_not_matter_for_the_conflict_check() {
        let err = parse(argv(&["--fake", "--follow-up", "--agent-view"]))
            .expect_err("order must not matter for the conflict check");
        assert_eq!(err, CliError::AgentViewWithOtherMode);
    }

    #[test]
    fn agent_view_composes_with_tag_and_force() {
        let cli = run_cli(&["--fake", "--agent-view", "--tag", "baseline", "--force"]);
        assert!(cli.agent_view);
        assert_eq!(cli.tag, Some("baseline".to_string()));
        assert!(cli.force);
    }

    #[test]
    fn the_help_text_documents_agent_view() {
        assert!(HELP_TEXT.contains("--agent-view"));
    }

    #[test]
    fn the_diff_help_describes_the_pair_cut() {
        let diff_help = HELP_TEXT
            .split("--diff <a.json> <b.json>")
            .nth(1)
            .and_then(|rest| rest.split("--fail-on-regression With").next())
            .expect("a --diff entry");
        assert!(diff_help.contains("query_lang"), "{diff_help}");
        assert!(diff_help.contains("language pair of query and row"), "{diff_help}");
        assert!(diff_help.contains("never enter primary: nor --fail-on-regression"), "{diff_help}");
    }

    #[test]
    fn check_only_combined_with_fake_parses_cleanly() {
        // Not an error at the parse layer: this combination is treated
        // as a harmless no-op resolved by main's control flow, not a parse
        // failure.
        let cli = run_cli(&["--fake", "--check-only"]);
        assert_eq!(cli.mode, Mode::Fake);
        assert!(cli.check_only);
    }

    #[test]
    fn docs_reads_its_own_golden_set_and_this_workspace_by_default() {
        let cli = run_cli(&["--fake", "--docs"]);
        assert!(cli.docs);
        assert_eq!(cli.golden_path, default_docs_golden_path());
        assert_eq!(cli.docs_root, default_docs_root());
        assert!(!run_cli(&["--fake"]).docs);
    }

    #[test]
    fn docs_takes_a_root_and_a_golden_set() {
        let cli = run_cli(&["--real", "--docs", "--root", "/tmp/repo", "--golden", "/tmp/q.json"]);
        assert_eq!(cli.docs_root, PathBuf::from("/tmp/repo"));
        assert_eq!(cli.golden_path, PathBuf::from("/tmp/q.json"));
    }

    #[test]
    fn root_without_docs_is_an_error() {
        let err = parse(argv(&["--fake", "--root", "/tmp/repo"])).expect_err("--root needs --docs");
        assert_eq!(err, CliError::RootWithoutDocs);
    }

    #[test]
    fn docs_with_another_mode_is_an_error() {
        for extra in [
            &["--audit"][..],
            &["--follow-up"],
            &["--agent-view"],
            &["--sweep-lexical", "0"],
            &["--sweep-imp", "0.5:1.0"],
            &["--sweep-cos-floor", "0.1"],
            &["--fts-tokenizer", "trigram"],
        ] {
            let mut args = vec!["--fake", "--docs"];
            args.extend_from_slice(extra);
            assert_eq!(parse(argv(&args)).expect_err("must conflict"), CliError::DocsWithOtherMode, "{extra:?}");
        }
    }

    #[test]
    fn the_help_text_documents_docs() {
        assert!(HELP_TEXT.contains("--docs"));
        assert!(HELP_TEXT.contains("--root <dir>"));
    }
}
