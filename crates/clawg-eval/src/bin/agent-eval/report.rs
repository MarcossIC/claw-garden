//! A directory of runs reported by task and arm. [`aggregate`] and [`render`]
//! are pure over the results; [`load`] is the file reading around them.

use std::path::{Path, PathBuf};

use crate::arm::Arm;
use crate::run::{RunResult, RESULT_FILE};
use crate::score::TracerFate;
use crate::stream::{ModelUsage, Outcome};

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
    /// The median over the runs that report it; `None` when none does.
    pub first_request_tokens: Option<f64>,
    /// The mean of the instruction files each run loaded for a path the
    /// agent worked on.
    pub rules_by_path: f64,
}

/// Tokens summed over runs: of every model a run reports, or of its main loop
/// when it reports none by model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// The main loop's cache writes by how long they last; the client splits
    /// no other model's.
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
}

impl Tokens {
    fn add(&mut self, outcome: &Outcome) {
        let u = &outcome.usage;
        if outcome.model_usage.is_empty() {
            self.input += u.input_tokens;
            self.output += u.output_tokens;
            self.cache_read += u.cache_read_input_tokens;
            self.cache_write += u.cache_creation_input_tokens;
        }
        for m in outcome.model_usage.values() {
            self.input += m.input_tokens;
            self.output += m.output_tokens;
            self.cache_read += m.cache_read_input_tokens;
            self.cache_write += m.cache_creation_input_tokens;
        }
        self.cache_write_5m += u.cache_creation_5m_input_tokens;
        self.cache_write_1h += u.cache_creation_1h_input_tokens;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub rows: Vec<Row>,
    pub runs: usize,
    pub successes: usize,
    /// The sum of the costs reported; a run that reports none adds 0.
    pub cost_usd: f64,
    pub runs_without_cost: usize,
    pub tokens: Tokens,
}

fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, n) = values.fold((0.0, 0_u32), |(sum, n), v| (sum + v, n + 1));
    (n > 0).then(|| sum / f64::from(n))
}

fn median(values: impl Iterator<Item = u64>) -> Option<f64> {
    let mut sorted: Vec<u64> = values.collect();
    sorted.sort_unstable();
    let n = sorted.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(sorted[n / 2] as f64),
        _ => Some((sorted[n / 2 - 1] + sorted[n / 2]) as f64 / 2.0),
    }
}

/// The `load_reason` of an instruction file loaded for a path the agent worked on.
const PATH_GLOB_MATCH: &str = "path_glob_match";

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
                first_request_tokens: median(group.iter().filter_map(|r| r.session.first_request_tokens)),
                rules_by_path: group
                    .iter()
                    .map(|r| r.session.instructions_loaded.iter().filter(|l| l.load_reason == PATH_GLOB_MATCH).count())
                    .sum::<usize>() as f64
                    / group.len() as f64,
            }
        })
        .collect();
    let costs = |r: &RunResult| r.session.outcome.as_ref().and_then(|o| o.total_cost_usd);
    let mut tokens = Tokens::default();
    results.iter().filter_map(|r| r.session.outcome.as_ref()).for_each(|o| tokens.add(o));
    Report {
        rows,
        runs: results.len(),
        successes: results.iter().filter(|r| r.score.success).count(),
        cost_usd: results.iter().filter_map(costs).sum(),
        runs_without_cost: results.iter().filter(|r| costs(r).is_none()).count(),
        tokens,
    }
}

pub fn render(report: &Report) -> String {
    let figure = |v: Option<f64>, places: usize| v.map_or_else(|| "-".to_owned(), |v| format!("{v:.places$}"));
    let mut out = String::from(
        "| task | category | arm | runs | successes | errors | cost USD | turns | seconds | clawg calls | used | retrieved_but_ignored | never_reached | first request tokens | rules by path |\n\
         | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n",
    );
    for r in &report.rows {
        out.push_str(&format!(
            "| {}{} | {} | {} | {} | {}/{} | {} | {} | {} | {} | {:.1} | {} | {} | {} | {} | {:.1} |\n",
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
            figure(r.first_request_tokens, 0),
            r.rules_by_path,
        ));
    }
    out.push_str(&format!(
        "\nTotals: {} runs, {} successes, cost {:.4} USD ({} runs without a cost)\n",
        report.runs, report.successes, report.cost_usd, report.runs_without_cost
    ));
    let t = &report.tokens;
    out.push_str(&format!(
        "Tokens of every model: input {}, output {}, cache read {}, cache write {} (in the main loop, {} for 5 minutes and {} for 1 hour)\n",
        t.input, t.output, t.cache_read, t.cache_write, t.cache_write_5m, t.cache_write_1h
    ));
    out
}

const COSTS_HEADER: &str = "run,task,arm,started_at,ended_at,api_key_source,model,input_tokens,output_tokens,cache_read_input_tokens,cache_creation_input_tokens,cache_creation_5m_input_tokens,cache_creation_1h_input_tokens,cost_usd,cost_basis";

/// A CSV with a line per session and model, to price the sessions by their
/// tokens and to find each one in the console's usage by its times. The cache
/// writes by duration are the main loop's, so they go on the line of the
/// session's model only. A session that printed no result has a line without
/// figures.
pub fn costs(runs: &[(String, RunResult)]) -> String {
    let field = |v: Option<&str>| match v {
        Some(s) if s.contains([',', '"', '\n']) => format!("\"{}\"", s.replace('"', "\"\"")),
        Some(s) => s.to_owned(),
        None => String::new(),
    };
    let mut out = String::from(COSTS_HEADER);
    out.push('\n');
    for (name, r) in runs {
        let s = &r.session;
        let head = [Some(name.as_str()), Some(r.task.as_str()), Some(r.arm.as_str()), s.started_at.as_deref(), s.ended_at.as_deref(), s.api_key_source.as_deref()]
            .map(field)
            .join(",");
        let Some(outcome) = &s.outcome else {
            out.push_str(&format!("{head}{}\n", ",".repeat(9)));
            continue;
        };
        let u = &outcome.usage;
        let durations = format!("{},{}", u.cache_creation_5m_input_tokens, u.cache_creation_1h_input_tokens);
        let mut line = |model: Option<&str>, m: &ModelUsage, durations: &str| {
            out.push_str(&format!(
                "{head},{},{},{},{},{},{durations},{},{}\n",
                field(model),
                m.input_tokens,
                m.output_tokens,
                m.cache_read_input_tokens,
                m.cache_creation_input_tokens,
                m.cost_usd.map(|c| c.to_string()).unwrap_or_default(),
                field(m.cost_basis.as_deref()),
            ));
        };
        if outcome.model_usage.is_empty() {
            let main = ModelUsage {
                input_tokens: u.input_tokens,
                output_tokens: u.output_tokens,
                cache_read_input_tokens: u.cache_read_input_tokens,
                cache_creation_input_tokens: u.cache_creation_input_tokens,
                cost_usd: outcome.total_cost_usd,
                cost_basis: None,
            };
            line(s.model.as_deref(), &main, &durations);
        }
        for (model, m) in &outcome.model_usage {
            let own = s.model.as_deref() == Some(model.as_str());
            line(Some(model), m, if own { &durations } else { "," });
        }
    }
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
    Ok(load_named(dir)?.into_iter().map(|(_, r)| r).collect())
}

/// Every result below `dir`, each named by the path of its run below `dir`,
/// with `/` between directories.
pub fn load_named(dir: &Path) -> Result<Vec<(String, RunResult)>, ReportError> {
    let mut paths = Vec::new();
    find(dir, 1, &mut paths)?;
    if paths.is_empty() {
        return Err(ReportError::Empty(dir.to_owned()));
    }
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).map_err(|source| ReportError::Io { path: path.clone(), source })?;
            let result = serde_json::from_str(&text).map_err(|e| ReportError::Invalid(path.clone(), e.to_string()))?;
            let run = path.parent().and_then(|p| p.strip_prefix(dir).ok()).unwrap_or(Path::new(""));
            let name = run.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            Ok((name, result))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arm::Shape;
    use crate::hooks::Loaded;
    use crate::run::Session;
    use crate::score::{CheckResult, Score};
    use crate::stream::{ModelUsage, Outcome, Usage};

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
                api_key_source: None,
                started_at: None,
                ended_at: None,
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
                first_request_tokens: None,
                instructions_loaded: vec![],
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
            sessions: vec![],
            shape: Shape::Split,
            noise_rules: 0,
            facts: vec![],
            overhead: None,
            paired: None,
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
        assert_eq!(lines[2], "| a-count | K1 | a0 | 1 | 1/1 | 0 | 0.2500 | 4.0 | 10.0 | 2.0 | 1 | 0 | 1 | - | 0.0 |");
        assert_eq!(lines[3], "| b-port | K1 | a3 | 3 | 1/3 | 1 | 0.2500 | 5.3 | 13.3 | 1.3 | 2 | 2 | 2 | - | 0.0 |");
        assert!(lines[5].starts_with("| z-example (example) | K1 | a0 |"));
        assert_eq!(lines[7], "Totals: 6 runs, 4 successes, cost 1.2500 USD (1 runs without a cost)");
    }

    #[test]
    fn rows_carry_the_median_first_request_and_the_rules_loaded_by_path() {
        let run = |arm: &'static str, tokens: Option<u64>, by_path: usize| {
            let mut r = result(&spec("k", arm));
            r.session.first_request_tokens = tokens;
            r.session.instructions_loaded = (0..by_path)
                .map(|i| Loaded { file_path: format!("/w/.claude/rules/clawg/rules-{i}.md"), load_reason: "path_glob_match".into() })
                .chain([Loaded { file_path: "/c/CLAUDE.md".into(), load_reason: "session_start".into() }])
                .collect();
            r
        };
        let report = aggregate(&[
            run("l2", Some(300), 3),
            run("l2", Some(100), 0),
            run("l2", None, 0),
            run("l2", Some(200), 1),
            run("a4", Some(100), 0),
            run("a4", Some(300), 0),
            run("l0", None, 0),
            run("a3", None, 0),
        ]);
        let order: Vec<&str> = report.rows.iter().map(|r| r.arm.as_str()).collect();
        assert_eq!(order, ["a3", "a4", "l0", "l2"]);
        let l2 = &report.rows[3];
        assert_eq!(l2.first_request_tokens, Some(200.0), "the median of the runs that give it");
        assert!((l2.rules_by_path - 1.0).abs() < 1e-9);
        assert_eq!(report.rows[1].first_request_tokens, Some(200.0), "an even count takes the mean of the middle two");
        assert_eq!(report.rows[0].first_request_tokens, None);
        let text = render(&report);
        assert!(text.lines().next().unwrap().ends_with("| never_reached | first request tokens | rules by path |"), "{text}");
        assert!(text.lines().nth(5).unwrap().ends_with("| 200 | 1.0 |"), "{text}");
    }

    fn model(input: u64, output: u64, read: u64, write: u64, cost: f64) -> ModelUsage {
        ModelUsage {
            input_tokens: input,
            output_tokens: output,
            cache_read_input_tokens: read,
            cache_creation_input_tokens: write,
            cost_usd: Some(cost),
            cost_basis: Some("list".to_owned()),
        }
    }

    /// A paid session on two models, the way Claude Code reports one.
    fn paid() -> RunResult {
        let mut r = result(&spec("b-port", "a3"));
        r.session.model = Some("claude-sonnet-5-5".to_owned());
        r.session.api_key_source = Some("ANTHROPIC_API_KEY".to_owned());
        r.session.started_at = Some("2026-11-11 15:00:00 UTC".to_owned());
        r.session.ended_at = Some("2026-11-11 15:04:30 UTC".to_owned());
        let outcome = r.session.outcome.as_mut().unwrap();
        outcome.total_cost_usd = Some(0.0912);
        outcome.usage = Usage {
            input_tokens: 30,
            output_tokens: 800,
            cache_creation_input_tokens: 12000,
            cache_read_input_tokens: 50000,
            cache_creation_5m_input_tokens: 11000,
            cache_creation_1h_input_tokens: 1000,
        };
        outcome.model_usage = [
            ("claude-sonnet-5-5".to_owned(), model(30, 800, 50000, 12000, 0.09)),
            ("claude-haiku-5-5".to_owned(), model(400, 20, 0, 0, 0.0012)),
        ]
        .into();
        r
    }

    #[test]
    fn the_totals_add_the_tokens_of_every_model() {
        let mut no_models = result(&spec("a-count", "a0"));
        no_models.session.outcome.as_mut().unwrap().usage =
            Usage { input_tokens: 5, output_tokens: 7, cache_read_input_tokens: 100, cache_creation_input_tokens: 9, ..Usage::default() };
        let report = aggregate(&[paid(), no_models]);
        assert_eq!(
            report.tokens,
            Tokens { input: 435, output: 827, cache_read: 50100, cache_write: 12009, cache_write_5m: 11000, cache_write_1h: 1000 },
            "a run without a breakdown by model adds its main loop"
        );
        let text = render(&report);
        assert!(
            text.ends_with("Tokens of every model: input 435, output 827, cache read 50100, cache write 12009 (in the main loop, 11000 for 5 minutes and 1000 for 1 hour)\n"),
            "{text}"
        );
    }

    #[test]
    fn the_costs_have_a_line_per_session_and_model_with_the_cache_durations_on_the_session_model() {
        let cut = {
            let mut r = result(&spec("b-port", "a4"));
            r.session.outcome = None;
            r
        };
        let csv = costs(&[("humo/b-port.a3.1".to_owned(), paid()), ("humo/b-port.a4.1".to_owned(), cut)]);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(
            lines[0],
            "run,task,arm,started_at,ended_at,api_key_source,model,input_tokens,output_tokens,cache_read_input_tokens,cache_creation_input_tokens,cache_creation_5m_input_tokens,cache_creation_1h_input_tokens,cost_usd,cost_basis"
        );
        assert_eq!(
            lines[1],
            "humo/b-port.a3.1,b-port,a3,2026-11-11 15:00:00 UTC,2026-11-11 15:04:30 UTC,ANTHROPIC_API_KEY,claude-haiku-5-5,400,20,0,0,,,0.0012,list"
        );
        assert_eq!(
            lines[2],
            "humo/b-port.a3.1,b-port,a3,2026-11-11 15:00:00 UTC,2026-11-11 15:04:30 UTC,ANTHROPIC_API_KEY,claude-sonnet-5-5,30,800,50000,12000,11000,1000,0.09,list"
        );
        assert_eq!(lines[3], "humo/b-port.a4.1,b-port,a4,,,,,,,,,,,,", "a session cut short is listed without figures");
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn a_session_without_a_breakdown_by_model_is_costed_from_its_main_loop() {
        let mut r = result(&spec("a-count", "a0"));
        r.session.model = Some("m".to_owned());
        r.session.outcome.as_mut().unwrap().usage = Usage { input_tokens: 5, output_tokens: 7, ..Usage::default() };
        let csv = costs(&[("r".to_owned(), r)]);
        assert_eq!(csv.lines().nth(1), Some("r,a-count,a0,,,,m,5,7,0,0,0,0,0.25,"));
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
        assert_eq!(load(dir.path()).unwrap(), std::slice::from_ref(&original));
        assert_eq!(load_named(dir.path()).unwrap(), [("runs/r1".to_owned(), original)], "a run is named by its path below the directory");

        let broken = dir.path().join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join(RESULT_FILE), "{}").unwrap();
        assert!(matches!(load(dir.path()), Err(ReportError::Invalid(..))));
    }
}
