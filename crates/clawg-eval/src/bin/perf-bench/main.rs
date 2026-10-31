mod diff;
mod stats;

use std::path::PathBuf;

const HELP_TEXT: &str = "\
perf-bench diff FIRST SECOND [--fail-on-regression]

  diff   compares two result files of the daemon bench
         (crates/clawg-cli/tests/perf_bench.rs), scenario by scenario: the
         p50 and p95 of each, the relative change of both with a 95 %
         bootstrap interval (each run drawn again on its own) and the
         Mann-Whitney rank test. A scenario is slower when its p95 rose over
         20 % and the interval leaves out no change, or when the second run
         has failures the first did not; with fewer than 20 samples a side it
         decides nothing. Runs of another build, embedder, machine, store
         size or seed are compared with a warning.

Exit 0 compared, 1 a file could not be read, 2 bad arguments,
6 a scenario is slower and --fail-on-regression was given.";

#[derive(Debug, PartialEq)]
enum Command {
    Diff { first: PathBuf, second: PathBuf, fail_on_regression: bool },
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Command>, String> {
    let mut args = args.skip(1);
    let kind = args.next().ok_or("name a command")?;
    if kind == "--help" || kind == "-h" {
        return Ok(None);
    }
    if kind != "diff" {
        return Err(format!("unknown command {kind}"));
    }
    let mut files = Vec::new();
    let mut fail_on_regression = false;
    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--fail-on-regression" => fail_on_regression = true,
            other if other.starts_with("--") => return Err(format!("unknown argument {other}")),
            _ => files.push(PathBuf::from(arg)),
        }
    }
    let [first, second]: [PathBuf; 2] = files.try_into().map_err(|_| "diff takes two result files")?;
    Ok(Some(Command::Diff { first, second, fail_on_regression }))
}

fn main() {
    let code = match parse_args(std::env::args()) {
        Ok(None) => {
            println!("{HELP_TEXT}");
            0
        }
        Err(e) => {
            eprintln!("perf-bench: {e}\n\n{HELP_TEXT}");
            2
        }
        Ok(Some(Command::Diff { first, second, fail_on_regression })) => {
            match (diff::read(&first), diff::read(&second)) {
                (Ok(a), Ok(b)) => {
                    let d = diff::diff(&a, &b);
                    print!("{}", diff::render(&a, &b, &d));
                    if fail_on_regression && d.regressed() {
                        6
                    } else {
                        0
                    }
                }
                (Err(e), _) | (_, Err(e)) => {
                    eprintln!("perf-bench: {e}");
                    1
                }
            }
        }
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        let owned: Vec<String> = std::iter::once("perf-bench").chain(list.iter().copied()).map(String::from).collect();
        owned.into_iter()
    }

    #[test]
    fn diff_takes_two_files_and_the_flag() {
        assert_eq!(
            parse_args(args(&["diff", "a.json", "b.json", "--fail-on-regression"])),
            Ok(Some(Command::Diff { first: "a.json".into(), second: "b.json".into(), fail_on_regression: true }))
        );
        assert_eq!(
            parse_args(args(&["diff", "a.json", "b.json"])),
            Ok(Some(Command::Diff { first: "a.json".into(), second: "b.json".into(), fail_on_regression: false }))
        );
        assert_eq!(parse_args(args(&["--help"])), Ok(None));
        assert_eq!(parse_args(args(&["diff", "--help"])), Ok(None));
    }

    #[test]
    fn anything_else_is_refused() {
        assert!(parse_args(args(&[])).is_err());
        assert!(parse_args(args(&["run"])).unwrap_err().contains("unknown command"));
        assert!(parse_args(args(&["diff", "a.json"])).unwrap_err().contains("two result files"));
        assert!(parse_args(args(&["diff", "a", "b", "c"])).unwrap_err().contains("two result files"));
        assert!(parse_args(args(&["diff", "a", "b", "--loud"])).unwrap_err().contains("unknown argument"));
    }
}
