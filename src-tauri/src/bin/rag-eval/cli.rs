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

#[derive(Debug, Clone, PartialEq, Eq)]
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
}

/// What a parse attempt produces. `--help` is not an error: it takes
/// precedence over every other flag (including a missing or conflicting
/// mode) and short-circuits the moment it is seen.
#[derive(Debug, PartialEq, Eq)]
pub enum ParseOutcome {
    Help,
    Run(Cli),
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
    --force             Allow overwriting an existing results file for the same date+config
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

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(ParseOutcome::Help),
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
            other => return Err(CliError::UnknownFlag(other.to_string())),
        }
    }

    let mode = mode.ok_or(CliError::MissingMode)?;

    Ok(ParseOutcome::Run(Cli {
        mode,
        check_only,
        golden_path: golden_path.unwrap_or_else(default_golden_path),
        out_dir: out_dir.unwrap_or_else(default_out_dir),
        force,
        audit,
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
    fn check_only_combined_with_fake_parses_cleanly() {
        // Not an error at the parse layer: the spec treats this combination
        // as a harmless no-op resolved by main's control flow, not a parse
        // failure.
        let cli = run_cli(&["--fake", "--check-only"]);
        assert_eq!(cli.mode, Mode::Fake);
        assert!(cli.check_only);
    }
}
