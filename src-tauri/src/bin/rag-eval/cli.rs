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
    Diff(PathBuf, PathBuf),
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
}

pub const HELP_TEXT: &str = "\
rag-eval - retrieval-quality measurement harness for agent-office's memory subsystem

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
    --diff <a.json> <b.json>
                         Compare two existing results files' retrieval.per_case and
                         similarity.per_case, joined on query, reporting better/worse/net
                         per metric per block. Needs neither --fake/--real nor a golden set;
                         short-circuits like --help.
    --follow-up          Run the synthetic Ceiling/Floor/Treatment arms instead of the
                         normal single-arm pass, measuring whether conversational context
                         helps a content-free follow-up retrieve the right document.
                         Mutually exclusive with every sweep flag and --fts-tokenizer.
    --help, -h          Print this message and exit

EXIT CODES:
    0    success, or --check-only confirmed the model is ready
    1    runtime failure (I/O, embedding, database, or seeding error)
    2    golden set failed validation
    3    --real --check-only and the model is not cached
    4    a results file for this date+config already exists and --force was not given
    5    --audit and the golden set is not fit to measure against
";

/// Default golden-set path, resolved relative to this crate's own manifest
/// directory rather than the process's current directory: `docs/eval/` lives
/// one level above `src-tauri`, and a binary invoked via `cargo run` from an
/// unexpected working directory must still find it.
pub fn default_golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("docs")
        .join("eval")
        .join("golden.json")
}

/// Default results directory, resolved the same CWD-independent way as
/// [`default_golden_path`].
pub fn default_out_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(ParseOutcome::Help),
            "--follow-up" => follow_up = true,
            "--diff" => {
                let a = args
                    .next()
                    .ok_or_else(|| CliError::MissingValue("--diff".to_string()))?;
                let b = args
                    .next()
                    .ok_or_else(|| CliError::MissingValue("--diff".to_string()))?;
                return Ok(ParseOutcome::Diff(PathBuf::from(a), PathBuf::from(b)));
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
            other => return Err(CliError::UnknownFlag(other.to_string())),
        }
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

    Ok(ParseOutcome::Run(Cli {
        mode,
        check_only,
        golden_path: golden_path.unwrap_or_else(default_golden_path),
        out_dir: out_dir.unwrap_or_else(default_out_dir),
        force,
        audit,
        tag,
        sweep_imp,
        sweep_cos_floor,
        sweep_lexical,
        fts_tokenizer,
        follow_up,
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

    fn run_cli(items: &[&str]) -> Cli {
        match parse(argv(items)).expect("parse must succeed") {
            ParseOutcome::Run(cli) => cli,
            ParseOutcome::Help => panic!("expected Run, got Help"),
            ParseOutcome::Diff(a, b) => panic!("expected Run, got Diff({a:?}, {b:?})"),
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
            ParseOutcome::Diff(PathBuf::from("/tmp/a.json"), PathBuf::from("/tmp/b.json"))
        );
    }

    #[test]
    fn diff_short_circuits_before_a_later_conflict_would_fire() {
        assert_eq!(
            parse(argv(&["--fake", "--diff", "/tmp/a.json", "/tmp/b.json", "--real"])).unwrap(),
            ParseOutcome::Diff(PathBuf::from("/tmp/a.json"), PathBuf::from("/tmp/b.json"))
        );
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

    #[test]
    fn check_only_combined_with_fake_parses_cleanly() {
        // Not an error at the parse layer: the spec treats this combination
        // as a harmless no-op resolved by main's control flow, not a parse
        // failure.
        let cli = run_cli(&["--fake", "--check-only"]);
        assert_eq!(cli.mode, Mode::Fake);
        assert!(cli.check_only);
    }
}
