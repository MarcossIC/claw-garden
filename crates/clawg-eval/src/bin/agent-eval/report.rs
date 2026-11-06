//! A directory of runs reported by task and arm. [`aggregate`] and [`render`]
//! are pure over the results; [`load`] is the file reading around them.

use std::path::{Path, PathBuf};

use crate::arm::Arm;
use crate::run::{RunResult, RESULT_FILE};
use crate::score::TracerFate;

/// How many directories below the one given a `result.json` may be.
const MAX_DEPTH: usize = 3;

#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{0} is not a result of a run: {1}")]
    Invalid(PathBuf, String),
    #[error("no {RESULT_FILE} below {0}")]
    Empty(PathBuf),
}

/// What the runs of one task under one arm add up to.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub task: String,
    pub category: String,
    pub example: bool,
    pub arm: String,
    pub runs: usize,
    pub successes: usize,
    /// Runs that did not end: the client reported an error or printed no result.
    pub errors: usize,
    /// Means over the runs that report the figure; `None` when none does.
    pub cost_usd: Option<f64>,
    pub turns: Option<f64>,
    pub seconds: Option<f64>,
    pub clawg_calls: f64,
    pub used: usize,
    pub retrieved_but_ignored: usize,
    pub never_reached: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub rows: Vec<Row>,
    pub runs: usize,
    pub successes: usize,
    /// The sum of the costs reported; a run that reports none adds 0.
    pub cost_usd: f64,
    pub runs_without_cost: usize,
}

fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, n) = values.fold((0.0, 0_u32), |(sum, n), v| (sum + v, n + 1));
    (n > 0).then(|| sum / f64::from(n))
}

/// Tasks by id, then arms in the order of [`crate::arm::ARMS`], an unknown one last.
fn arm_rank(arm: &str) -> usize {
    Arm::parse(arm).and_then(|a| crate::arm::ARMS.iter().position(|x| *x == a)).unwrap_or(crate::arm::ARMS.len())
}

pub fn aggregate(results: &[RunResult]) -> Report {
    let mut keys: Vec<(&str, &str)> = results.iter().map(|r| (r.task.as_str(), r.arm.as_str())).collect();
    keys.sort_by(|a, b| a.0.cmp(b.0).then_with(|| arm_rank(a.1).cmp(&arm_rank(b.1))).then_with(|| a.1.cmp(b.1)));
    keys.dedup();
    let rows = keys
        .into_iter()
        .map(|(task, arm)| {
            let group: Vec<&RunResult> = results.iter().filter(|r| r.task == task && r.arm == arm).collect();
            let outcome = |r: &&RunResult| r.session.outcome.clone();
            let fates = |f: TracerFate| group.iter().map(|r| r.score.tracers.iter().filter(|(_, x)| *x == f).count()).sum();
            Row {
                task: task.to_owned(),
                category: group[0].category.clone(),
                example: group[0].example,
                arm: arm.to_owned(),
                runs: group.len(),
                successes: group.iter().filter(|r| r.score.success).count(),
                errors: group.iter().filter(|r| !r.score.ended).count(),
                cost_usd: mean(group.iter().filter_map(|r| outcome(r)?.total_cost_usd)),
                turns: mean(group.iter().filter_map(|r| outcome(r)?.num_turns).map(|n| n as f64)),
                seconds: mean(group.iter().filter_map(|r| outcome(r)?.duration_ms).map(|ms| ms as f64 / 1000.0)),
                clawg_calls: group.iter().map(|r| r.session.clawg_calls.values().sum::<usize>()).sum::<usize>() as f64 / group.len() as f64,
                used: fates(TracerFate::Used),
                retrieved_but_ignored: fates(TracerFate::RetrievedButIgnored),
                never_reached: fates(TracerFate::NeverReached),
            }
        })
        .collect();
    let costs = |r: &RunResult| r.session.outcome.as_ref().and_then(|o| o.total_cost_usd);
    Report {
        rows,
        runs: results.len(),
        successes: results.iter().filter(|r| r.score.success).count(),
        cost_usd: results.iter().filter_map(costs).sum(),
        runs_without_cost: results.iter().filter(|r| costs(r).is_none()).count(),
    }
}

pub fn render(report: &Report) -> String {
    let figure = |v: Option<f64>, places: usize| v.map_or_else(|| "-".to_owned(), |v| format!("{v:.places$}"));
    let mut out = String::from(
        "| task | category | arm | runs | successes | errors | cost USD | turns | seconds | clawg calls | used | retrieved_but_ignored | never_reached |\n\
         | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n",
    );
    for r in &report.rows {
        out.push_str(&format!(
            "| {}{} | {} | {} | {} | {}/{} | {} | {} | {} | {} | {:.1} | {} | {} | {} |\n",
            r.task,
            if r.example { " (example)" } else { "" },
            r.category,
            r.arm,
            r.runs,
            r.successes,
            r.runs,
            r.errors,
            figure(r.cost_usd, 4),
            figure(r.turns, 1),
            figure(r.seconds, 1),
            r.clawg_calls,
            r.used,
            r.retrieved_but_ignored,
            r.never_reached,
        ));
    }
    out.push_str(&format!(
        "\nTotals: {} runs, {} successes, cost {:.4} USD ({} runs without a cost)\n",
        report.runs, report.successes, report.cost_usd, report.runs_without_cost
    ));
    out
}

/// The `result.json` of every directory below `dir`, down to [`MAX_DEPTH`]
/// levels, in path order; a directory that holds one is not searched further.
fn find(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<(), ReportError> {
    let io = |path: &Path| {
        let path = path.to_owned();
        move |source| ReportError::Io { path, source }
    };
    let mut subdirs = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(io(dir))? {
        let entry = entry.map_err(io(dir))?;
        if entry.file_type().map_err(io(&entry.path()))?.is_dir() {
            subdirs.push(entry.path());
        }
    }
    subdirs.sort();
    for sub in subdirs {
        let result = sub.join(RESULT_FILE);
        if result.is_file() {
            out.push(result);
        } else if depth < MAX_DEPTH {
            find(&sub, depth + 1, out)?;
        }
    }
    Ok(())
}

/// Every result below `dir`.
pub fn load(dir: &Path) -> Result<Vec<RunResult>, ReportError> {
    let mut paths = Vec::new();
    find(dir, 1, &mut paths)?;
    if paths.is_empty() {
        return Err(ReportError::Empty(dir.to_owned()));
    }
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).map_err(|source| ReportError::Io { path: path.clone(), source })?;
            serde_json::from_str(&text).map_err(|e| ReportError::Invalid(path, e.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::Session;
    use crate::score::{CheckResult, Score};
    use crate::stream::Outcome;

    struct Spec {
        task: &'static str,
        arm: &'static str,
        success: bool,
        cost: Option<f64>,
        turns: u64,
        seconds: u64,
        calls: usize,
        fates: [TracerFate; 2],
        example: bool,
    }

    fn spec(task: &'static str, arm: &'static str) -> Spec {
        Spec {
            task,
            arm,
            success: true,
            cost: Some(0.25),
            turns: 4,
            seconds: 10,
            calls: 2,
            fates: [TracerFate::Used, TracerFate::NeverReached],
            example: false,
        }
    }

    fn result(s: &Spec) -> RunResult {
        RunResult {
            task: s.task.to_owned(),
            category: "K1".to_owned(),
            example: s.example,
            arm: s.arm.to_owned(),
            client: "claude".to_owned(),
            lang: None,
            session: Session {
                session_id: None,
                model: None,
                client_version: None,
                mcp_servers: vec![],
                tool_calls: 0,
                clawg_calls: [("search_memory".to_owned(), s.calls)].into(),
                clawg_calls_after_tool_search: 0,
                clawg_calls_in_subagents: 0,
                tool_search_calls: 0,
                hook_events: 0,
                compactions: 0,
                unknown_lines: 0,
                invalid_lines: vec![],
                outcome: Some(Outcome {
                    subtype: "success".to_owned(),
                    is_error: false,
                    total_cost_usd: s.cost,
                    num_turns: Some(s.turns),
                    duration_ms: Some(s.seconds * 1000),
                    ..Outcome::default()
                }),
            },
            score: Score {
                success: s.success,
                ended: true,
                checks: vec![CheckResult { check: "c".to_owned(), passed: s.success }],
                tracers: vec![("t1".to_owned(), s.fates[0]), ("t2".to_owned(), s.fates[1])],
            },
            saves: vec![],
        }
    }

    fn sample() -> Vec<RunResult> {
        let failed = Spec { success: false, cost: None, turns: 8, seconds: 20, calls: 0, fates: [TracerFate::RetrievedButIgnored; 2], ..spec("b-port", "a3") };
        let mut errored = result(&spec("b-port", "a3"));
        errored.score.ended = false;
        errored.score.success = false;
        vec![
            result(&spec("b-port", "b")),
            result(&spec("b-port", "a3")),
            result(&failed),
            errored,
            result(&spec("a-count", "a0")),
            result(&Spec { example: true, ..spec("z-example", "a0") }),
        ]
    }

    #[test]
    fn rows_come_in_task_then_arm_order_with_their_means_and_sums() {
        let report = aggregate(&sample());
        let order: Vec<(&str, &str)> = report.rows.iter().map(|r| (r.task.as_str(), r.arm.as_str())).collect();
        assert_eq!(order, [("a-count", "a0"), ("b-port", "a3"), ("b-port", "b"), ("z-example", "a0")]);

        let a3 = &report.rows[1];
        assert_eq!((a3.runs, a3.successes, a3.errors), (3, 1, 1));
        assert_eq!(a3.cost_usd, Some(0.25), "the run without a cost is left out of the mean");
        assert_eq!((a3.turns, a3.seconds), (Some((4.0 + 8.0 + 4.0) / 3.0), Some((10.0 + 20.0 + 10.0) / 3.0)));
        assert!((a3.clawg_calls - 4.0 / 3.0).abs() < 1e-9);
        assert_eq!((a3.used, a3.retrieved_but_ignored, a3.never_reached), (2, 2, 2));
        assert!(report.rows[3].example);
    }

    #[test]
    fn the_totals_count_runs_successes_and_cost_with_the_runs_that_report_none() {
        let report = aggregate(&sample());
        assert_eq!((report.runs, report.successes, report.runs_without_cost), (6, 4, 1));
        assert!((report.cost_usd - 1.25).abs() < 1e-9);
    }

    #[test]
    fn a_group_without_any_cost_has_none() {
        let report = aggregate(&[result(&Spec { cost: None, ..spec("t", "a0") })]);
        assert_eq!(report.rows[0].cost_usd, None);
        assert!(render(&report).contains("| - |"));
    }

    #[test]
    fn the_table_is_markdown_with_the_example_marked() {
        let text = render(&aggregate(&sample()));
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("| task | category | arm | runs | successes | errors | cost USD |"));
        assert!(lines[1].starts_with("| --- |"));
        assert_eq!(lines[2], "| a-count | K1 | a0 | 1 | 1/1 | 0 | 0.2500 | 4.0 | 10.0 | 2.0 | 1 | 0 | 1 |");
        assert_eq!(lines[3], "| b-port | K1 | a3 | 3 | 1/3 | 1 | 0.2500 | 5.3 | 13.3 | 1.3 | 2 | 2 | 2 |");
        assert!(lines[5].starts_with("| z-example (example) | K1 | a0 |"));
        assert_eq!(lines[7], "Totals: 6 runs, 4 successes, cost 1.2500 USD (1 runs without a cost)");
    }

    #[test]
    fn load_finds_results_up_to_three_levels_down_and_skips_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        for rel in ["one", "group/two", "a/b/c", "a/b/c/d/four"] {
            let run = dir.path().join(rel);
            std::fs::create_dir_all(&run).unwrap();
            std::fs::write(run.join(RESULT_FILE), "{}").unwrap();
        }
        std::fs::create_dir_all(dir.path().join("empty")).unwrap();
        std::fs::create_dir_all(dir.path().join("w/x/y/z")).unwrap();
        std::fs::write(dir.path().join("w/x/y/z").join(RESULT_FILE), "{}").unwrap();
        let mut found = Vec::new();
        find(dir.path(), 1, &mut found).unwrap();
        let rel: Vec<String> = found.iter().map(|p| p.strip_prefix(dir.path()).unwrap().to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(rel, ["a/b/c/result.json", "group/two/result.json", "one/result.json"]);
    }

    #[test]
    fn load_reads_a_written_result_back_and_names_a_directory_without_any() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(load(dir.path()), Err(ReportError::Empty(_))));

        let original = result(&spec("t", "a0"));
        let run = dir.path().join("runs").join("r1");
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(run.join(RESULT_FILE), serde_json::to_string_pretty(&original).unwrap()).unwrap();
        assert_eq!(load(dir.path()).unwrap(), [original]);

        let broken = dir.path().join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join(RESULT_FILE), "{}").unwrap();
        assert!(matches!(load(dir.path()), Err(ReportError::Invalid(..))));
    }
}
