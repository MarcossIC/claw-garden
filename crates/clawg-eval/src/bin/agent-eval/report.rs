//! A directory of runs reported by task, arm and shape. [`aggregate`],
//! [`pairs`], [`costs`], [`calls`] and their renderings are pure over what they
//! are given; [`load`] and [`load_streams`] are the file reading around them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::arm::{Arm, Shape};
use crate::run::{FactClass, RunResult, Session, RESULT_FILE};
use crate::score::TracerFate;
use crate::stream::{self, ModelUsage, Outcome, Transcript};

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
    #[error("no stream of a session below {0}")]
    NoStreams(PathBuf),
}

/// What the runs of one task under one arm and one shape add up to.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub task: String,
    pub category: String,
    pub example: bool,
    pub arm: String,
    pub shape: Shape,
    pub runs: usize,
    pub successes: usize,
    /// Runs that did not end: the client reported an error or printed no result.
    pub errors: usize,
    /// Means over the runs that report the figure, each the sum of the sessions
    /// of its run; `None` when none does.
    pub cost_usd: Option<f64>,
    pub turns: Option<f64>,
    pub seconds: Option<f64>,
    pub clawg_calls: f64,
    pub used: usize,
    pub retrieved_but_ignored: usize,
    pub never_reached: usize,
    /// The median, over the runs that report it, of the context of the first
    /// request of their first session; `None` when none does.
    pub first_request_tokens: Option<f64>,
    /// The mean of the instruction files each run loaded for a path the
    /// agent worked on.
    pub rules_by_path: f64,
    /// The mean number of sessions a run played.
    pub sessions: f64,
    /// The median of the context of every request of every run of the row.
    pub context_tokens: Option<f64>,
    /// The median of the relay overhead of the runs that carry it.
    pub overhead: Option<f64>,
    /// The facts born in a session, summed over the runs, by what became of them.
    pub not_saved: usize,
    pub saved_not_reached: usize,
    pub reached_not_used: usize,
    pub facts_used: usize,
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

fn median(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut sorted: Vec<f64> = values.collect();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(sorted[n / 2]),
        _ => Some((sorted[n / 2 - 1] + sorted[n / 2]) / 2.0),
    }
}

/// The `load_reason` of an instruction file loaded for a path the agent worked on.
const PATH_GLOB_MATCH: &str = "path_glob_match";

/// Tasks by id, then arms in the order of [`crate::arm::ARMS`], an unknown one last.
fn arm_rank(arm: &str) -> usize {
    Arm::parse(arm).and_then(|a| crate::arm::ARMS.iter().position(|x| *x == a)).unwrap_or(crate::arm::ARMS.len())
}

fn shape_rank(shape: Shape) -> usize {
    [Shape::Split, Shape::Joined, Shape::Both].iter().position(|s| *s == shape).unwrap_or(0)
}

/// What a run adds up to for a figure its sessions report one each: the sum of
/// those that report it, `None` when none does.
fn of_the_run(r: &RunResult, figure: impl Fn(&Outcome) -> Option<f64>) -> Option<f64> {
    let reported: Vec<f64> = r.records().iter().filter_map(|s| s.outcome.as_ref().and_then(&figure)).collect();
    (!reported.is_empty()).then(|| reported.iter().sum())
}

/// The sessions of a run that were its own: a new branch of a fork shares its
/// first session with the continued one, which counts it.
fn own_sessions(r: &RunResult) -> Vec<&Session> {
    let mut sessions = r.records();
    if r.paired.is_some() && r.shape == Shape::Split && sessions.len() > 1 {
        sessions.remove(0);
    }
    sessions
}

pub fn aggregate(results: &[RunResult]) -> Report {
    let mut keys: Vec<(&str, &str, Shape)> = results.iter().map(|r| (r.task.as_str(), r.arm.as_str(), r.shape)).collect();
    keys.sort_by(|a, b| {
        a.0.cmp(b.0).then_with(|| arm_rank(a.1).cmp(&arm_rank(b.1))).then_with(|| a.1.cmp(b.1)).then_with(|| shape_rank(a.2).cmp(&shape_rank(b.2)))
    });
    keys.dedup();
    let rows = keys
        .into_iter()
        .map(|(task, arm, shape)| {
            let group: Vec<&RunResult> = results.iter().filter(|r| r.task == task && r.arm == arm && r.shape == shape).collect();
            let fates = |f: TracerFate| group.iter().map(|r| r.score.tracers.iter().filter(|(_, x)| *x == f).count()).sum();
            let facts = |c: FactClass| group.iter().map(|r| r.facts.iter().filter(|f| f.class == c).count()).sum();
            Row {
                task: task.to_owned(),
                category: group[0].category.clone(),
                example: group[0].example,
                arm: arm.to_owned(),
                shape,
                runs: group.len(),
                successes: group.iter().filter(|r| r.score.success).count(),
                errors: group.iter().filter(|r| !r.score.ended).count(),
                cost_usd: mean(group.iter().filter_map(|r| of_the_run(r, |o| o.total_cost_usd))),
                turns: mean(group.iter().filter_map(|r| of_the_run(r, |o| o.num_turns.map(|n| n as f64)))),
                seconds: mean(group.iter().filter_map(|r| of_the_run(r, |o| o.duration_ms.map(|ms| ms as f64 / 1000.0)))),
                clawg_calls: group.iter().map(|r| r.records().iter().map(|s| s.clawg_calls.values().sum::<usize>()).sum::<usize>()).sum::<usize>() as f64
                    / group.len() as f64,
                used: fates(TracerFate::Used),
                retrieved_but_ignored: fates(TracerFate::RetrievedButIgnored),
                never_reached: fates(TracerFate::NeverReached),
                first_request_tokens: median(group.iter().filter_map(|r| r.records()[0].first_request_tokens).map(|t| t as f64)),
                rules_by_path: group
                    .iter()
                    .flat_map(|r| r.records())
                    .map(|s| s.instructions_loaded.iter().filter(|l| l.load_reason == PATH_GLOB_MATCH).count())
                    .sum::<usize>() as f64
                    / group.len() as f64,
                sessions: group.iter().map(|r| r.records().len()).sum::<usize>() as f64 / group.len() as f64,
                context_tokens: median(group.iter().flat_map(|r| r.records()).flat_map(|s| &s.request_tokens).map(|t| *t as f64)),
                overhead: median(group.iter().filter_map(|r| r.overhead).map(|t| t as f64)),
                not_saved: facts(FactClass::NotSaved),
                saved_not_reached: facts(FactClass::SavedNotReached),
                reached_not_used: facts(FactClass::ReachedNotUsed),
                facts_used: facts(FactClass::Used),
            }
        })
        .collect();
    let mut tokens = Tokens::default();
    results.iter().flat_map(own_sessions).filter_map(|s| s.outcome.as_ref()).for_each(|o| tokens.add(o));
    let cost = |r: &RunResult| own_sessions(r).iter().filter_map(|s| s.outcome.as_ref()?.total_cost_usd).sum::<f64>();
    Report {
        rows,
        runs: results.len(),
        successes: results.iter().filter(|r| r.score.success).count(),
        cost_usd: results.iter().map(cost).sum(),
        runs_without_cost: results.iter().filter(|r| of_the_run(r, |o| o.total_cost_usd).is_none()).count(),
        tokens,
    }
}

pub fn render(report: &Report) -> String {
    let figure = |v: Option<f64>, places: usize| v.map_or_else(|| "-".to_owned(), |v| format!("{v:.places$}"));
    let mut out = String::from(
        "| task | category | arm | shape | runs | successes | errors | cost USD | turns | seconds | clawg calls | used | retrieved_but_ignored | never_reached | first request tokens | rules by path | sessions | context tokens | relay overhead | not_saved | saved_not_reached | reached_not_used | facts used |\n\
         | --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n",
    );
    for r in &report.rows {
        out.push_str(&format!(
            "| {}{} | {} | {} | {} | {} | {}/{} | {} | {} | {} | {} | {:.1} | {} | {} | {} | {} | {:.1} | {:.1} | {} | {} | {} | {} | {} | {} |\n",
            r.task,
            if r.example { " (example)" } else { "" },
            r.category,
            r.arm,
            r.shape.as_str(),
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
            r.sessions,
            figure(r.context_tokens, 0),
            figure(r.overhead, 0),
            r.not_saved,
            r.saved_not_reached,
            r.reached_not_used,
            r.facts_used,
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

/// What the forks of one task under one arm add up to: for each, the two
/// branches that played its second session, continued and new.
#[derive(Debug, Clone, PartialEq)]
pub struct PairRow {
    pub task: String,
    pub arm: String,
    pub pairs: usize,
    pub joined_passed: usize,
    pub split_passed: usize,
    /// Pairs where the continued branch passed and the new one failed.
    pub continued_only: usize,
    /// The median of the context of the last request of the first session.
    pub fork_context: Option<f64>,
    /// The median of what the new session saved over the continued one: 1 minus
    /// the cost of its second session over the cost of the continued one's.
    pub saving: Option<f64>,
    /// The pairs the saving is of: those whose continuation read at least 80 % of
    /// its first request from the cache, since a cold one prices the cache
    /// and not the relay.
    pub priced: usize,
}

/// The continued branch and the new one of a fork.
type Pair<'a> = (&'a RunResult, &'a RunResult);

/// The share of its first request a continuation reads from the cache for its
/// cost to be compared with the new session's.
const WARM_CACHE: f64 = 0.8;

/// The forks among `runs`, named by where they were run: two results of one
/// directory, one of each shape, that name each other as their pair. A
/// branch without its partner is no pair.
pub fn pairs(runs: &[(String, RunResult)]) -> Vec<PairRow> {
    let parent = |name: &str| name.rsplit_once('/').map_or("", |(dir, _)| dir).to_owned();
    let mut found: BTreeMap<(String, usize, String), Vec<Pair>> = BTreeMap::new();
    for (name, joined) in runs.iter().filter(|(_, r)| r.paired.is_some() && r.shape == Shape::Joined) {
        let dir = parent(name);
        let partner = runs.iter().find(|(n, r)| r.paired.is_some() && r.shape == Shape::Split && parent(n) == dir && r.task == joined.task && r.arm == joined.arm);
        if let Some((_, split)) = partner {
            found.entry((joined.task.clone(), arm_rank(&joined.arm), joined.arm.clone())).or_default().push((joined, split));
        }
    }
    found
        .into_iter()
        .map(|((task, _, arm), group)| {
            let second_cost = |r: &RunResult| r.sessions.get(1).and_then(|s| s.session.outcome.as_ref()?.total_cost_usd);
            let savings: Vec<f64> = group
                .iter()
                .filter(|(joined, _)| {
                    let second = joined.sessions.get(1).map(|s| &s.session);
                    let read = second.and_then(|s| s.first_request_cache_read);
                    let first = second.and_then(|s| s.first_request_tokens);
                    matches!((read, first), (Some(read), Some(first)) if first > 0 && read as f64 / first as f64 >= WARM_CACHE)
                })
                .filter_map(|(joined, split)| match (second_cost(joined), second_cost(split)) {
                    (Some(continued), Some(new)) if continued > 0.0 => Some(1.0 - new / continued),
                    _ => None,
                })
                .collect();
            PairRow {
                task,
                arm,
                pairs: group.len(),
                joined_passed: group.iter().filter(|(j, _)| j.score.success).count(),
                split_passed: group.iter().filter(|(_, s)| s.score.success).count(),
                continued_only: group.iter().filter(|(j, s)| j.score.success && !s.score.success).count(),
                fork_context: median(group.iter().filter_map(|(j, _)| j.sessions.first()?.session.request_tokens.last()).map(|t| *t as f64)),
                saving: median(savings.iter().copied()),
                priced: savings.len(),
            }
        })
        .collect()
}

pub fn render_pairs(rows: &[PairRow]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "| task | arm | pairs | joined passed | split passed | joined passed and split failed | context at the fork | cost saving | pairs priced |\n\
         | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n",
    );
    for r in rows {
        out.push_str(&format!(
            "| {} | {} | {} | {}/{} | {}/{} | {} | {} | {} | {} |\n",
            r.task,
            r.arm,
            r.pairs,
            r.joined_passed,
            r.pairs,
            r.split_passed,
            r.pairs,
            r.continued_only,
            r.fork_context.map_or_else(|| "-".to_owned(), |c| format!("{c:.0}")),
            r.saving.map_or_else(|| "-".to_owned(), |s| format!("{:.1}%", s * 100.0)),
            r.priced,
        ));
    }
    out
}

const COSTS_HEADER: &str = "run,task,arm,started_at,ended_at,api_key_source,model,input_tokens,output_tokens,cache_read_input_tokens,cache_creation_input_tokens,cache_creation_5m_input_tokens,cache_creation_1h_input_tokens,cost_usd,cost_basis,session,shape";

/// A CSV with a line per session and model, to price the sessions by their
/// tokens and to find each one in the console's usage by its times, each with
/// its number in its run and the shape the run was played in. The cache writes
/// by duration are the main loop's, so they go on the line of the session's
/// model only. A session that printed no result has a line without figures.
/// The first session of the new branch of a fork is not listed: it is the one
/// the continued branch lists.
pub fn costs(runs: &[(String, RunResult)]) -> String {
    let field = |v: Option<&str>| match v {
        Some(s) if s.contains([',', '"', '\n']) => format!("\"{}\"", s.replace('"', "\"\"")),
        Some(s) => s.to_owned(),
        None => String::new(),
    };
    let mut out = String::from(COSTS_HEADER);
    out.push('\n');
    for (name, r) in runs {
        let skipped = r.records().len() - own_sessions(r).len();
        for (i, s) in r.records().into_iter().enumerate().skip(skipped) {
            let head = [Some(name.as_str()), Some(r.task.as_str()), Some(r.arm.as_str()), s.started_at.as_deref(), s.ended_at.as_deref(), s.api_key_source.as_deref()]
                .map(field)
                .join(",");
            let tail = format!("{},{}", i + 1, r.shape.as_str());
            let Some(outcome) = &s.outcome else {
                out.push_str(&format!("{head}{},{tail}\n", ",".repeat(9)));
                continue;
            };
            let u = &outcome.usage;
            let durations = format!("{},{}", u.cache_creation_5m_input_tokens, u.cache_creation_1h_input_tokens);
            let mut line = |model: Option<&str>, m: &ModelUsage, durations: &str| {
                out.push_str(&format!(
                    "{head},{},{},{},{},{},{durations},{},{},{tail}\n",
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
    }
    out
}

/// What the calls to one tool add up to.
#[derive(Default)]
struct ToolStats {
    calls: usize,
    /// The tokens each call that could be measured alone added to the context.
    added: Vec<u64>,
    /// It is a tool of an MCP server, not one of the client's.
    server: bool,
}

/// The median of the tokens a tool adds over which it is noted as a candidate
/// for a diet: twice what `search_memory` measures in agent-view.
const DIET_MEDIAN: f64 = 1500.0;

/// A table by tool, over every session of `transcripts`: its calls, those that
/// carry tokens of their own and the median and p90 of those tokens.
pub fn calls(transcripts: &[Transcript]) -> String {
    let mut tools: BTreeMap<String, ToolStats> = BTreeMap::new();
    for call in transcripts.iter().flat_map(|t| &t.calls) {
        let stats = tools.entry(stream::tool_name(&call.name).to_owned()).or_default();
        stats.calls += 1;
        stats.server |= call.name.starts_with("mcp__");
        stats.added.extend(call.added_tokens);
    }
    let mut rows: Vec<(String, ToolStats)> = tools.into_iter().collect();
    rows.sort_by(|a, b| b.1.calls.cmp(&a.1.calls).then_with(|| a.0.cmp(&b.0)));
    let mut out = String::from("| tool | calls | with tokens | median tokens | p90 tokens | note |\n| --- | ---: | ---: | ---: | ---: | --- |\n");
    for (name, mut stats) in rows {
        stats.added.sort_unstable();
        let median = median(stats.added.iter().map(|t| *t as f64));
        let p90 = (!stats.added.is_empty()).then(|| stats.added[(stats.added.len() * 9).div_ceil(10) - 1]);
        let note = if stats.server && median.is_some_and(|m| m > DIET_MEDIAN) { "candidate for a diet" } else { "" };
        out.push_str(&format!(
            "| {name} | {} | {} | {} | {} | {note} |\n",
            stats.calls,
            stats.added.len(),
            median.map_or_else(|| "-".to_owned(), |m| format!("{m:.0}")),
            p90.map_or_else(|| "-".to_owned(), |p| p.to_string()),
        ));
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

/// Whether `name` is the stream of a session: `stream.jsonl`, or `stream-K.jsonl`.
fn is_stream_file(name: &str) -> bool {
    name == crate::run::STREAM_FILE || name.strip_prefix("stream-").and_then(|rest| rest.strip_suffix(".jsonl")).is_some_and(|k| k.parse::<usize>().is_ok())
}

/// The stream of every session below `dir`, down to a branch of a fork of a run
/// of a phase: one more level than the results [`find`] looks for.
fn find_streams(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<(), ReportError> {
    let mut entries: Vec<_> = std::fs::read_dir(dir).map_err(|source| ReportError::Io { path: dir.to_owned(), source })?.collect::<Result<_, _>>().map_err(|source| ReportError::Io { path: dir.to_owned(), source })?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let kind = entry.file_type().map_err(|source| ReportError::Io { path: path.clone(), source })?;
        if kind.is_dir() && depth <= MAX_DEPTH {
            find_streams(&path, depth + 1, out)?;
        } else if kind.is_file() && is_stream_file(&entry.file_name().to_string_lossy()) {
            out.push(path);
        }
    }
    Ok(())
}

/// Every session stream below `dir`, read.
pub fn load_streams(dir: &Path) -> Result<Vec<Transcript>, ReportError> {
    let mut paths = Vec::new();
    find_streams(dir, 1, &mut paths)?;
    if paths.is_empty() {
        return Err(ReportError::NoStreams(dir.to_owned()));
    }
    paths
        .into_iter()
        .map(|path| std::fs::read_to_string(&path).map(|text| stream::parse(&text)).map_err(|source| ReportError::Io { path, source }))
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
                request_tokens: vec![],
                first_request_cache_read: None,
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
            rereads: None,
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
        assert!(lines[0].starts_with("| task | category | arm | shape | runs | successes | errors | cost USD |"));
        assert!(lines[1].starts_with("| --- |"));
        assert_eq!(lines[2], "| a-count | K1 | a0 | split | 1 | 1/1 | 0 | 0.2500 | 4.0 | 10.0 | 2.0 | 1 | 0 | 1 | - | 0.0 | 1.0 | - | - | 0 | 0 | 0 | 0 |");
        assert_eq!(lines[3], "| b-port | K1 | a3 | split | 3 | 1/3 | 1 | 0.2500 | 5.3 | 13.3 | 1.3 | 2 | 2 | 2 | - | 0.0 | 1.0 | - | - | 0 | 0 | 0 | 0 |");
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
        assert!(text.lines().next().unwrap().ends_with("| never_reached | first request tokens | rules by path | sessions | context tokens | relay overhead | not_saved | saved_not_reached | reached_not_used | facts used |"), "{text}");
        assert!(text.lines().nth(5).unwrap().ends_with("| 200 | 1.0 | 1.0 | - | - | 0 | 0 | 0 | 0 |"), "{text}");
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
            "run,task,arm,started_at,ended_at,api_key_source,model,input_tokens,output_tokens,cache_read_input_tokens,cache_creation_input_tokens,cache_creation_5m_input_tokens,cache_creation_1h_input_tokens,cost_usd,cost_basis,session,shape"
        );
        assert_eq!(
            lines[1],
            "humo/b-port.a3.1,b-port,a3,2026-11-11 15:00:00 UTC,2026-11-11 15:04:30 UTC,ANTHROPIC_API_KEY,claude-haiku-5-5,400,20,0,0,,,0.0012,list,1,split"
        );
        assert_eq!(
            lines[2],
            "humo/b-port.a3.1,b-port,a3,2026-11-11 15:00:00 UTC,2026-11-11 15:04:30 UTC,ANTHROPIC_API_KEY,claude-sonnet-5-5,30,800,50000,12000,11000,1000,0.09,list,1,split"
        );
        assert_eq!(lines[3], "humo/b-port.a4.1,b-port,a4,,,,,,,,,,,,,1,split", "a session cut short is listed without figures");
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn a_session_without_a_breakdown_by_model_is_costed_from_its_main_loop() {
        let mut r = result(&spec("a-count", "a0"));
        r.session.model = Some("m".to_owned());
        r.session.outcome.as_mut().unwrap().usage = Usage { input_tokens: 5, output_tokens: 7, ..Usage::default() };
        let csv = costs(&[("r".to_owned(), r)]);
        assert_eq!(csv.lines().nth(1), Some("r,a-count,a0,,,,m,5,7,0,0,0,0,0.25,,1,split"));
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
        assert!(matches!(load_named(dir.path()), Err(ReportError::Empty(_))));

        let original = result(&spec("t", "a0"));
        let run = dir.path().join("runs").join("r1");
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(run.join(RESULT_FILE), serde_json::to_string_pretty(&original).unwrap()).unwrap();
        assert_eq!(load_named(dir.path()).unwrap(), [("runs/r1".to_owned(), original)], "a run is named by its path below the directory");

        let broken = dir.path().join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join(RESULT_FILE), "{}").unwrap();
        assert!(matches!(load_named(dir.path()), Err(ReportError::Invalid(..))));
    }

    /// One session of a staged run: what it cost, how many turns, the context
    /// of each of its requests and how much of its first one the cache gave.
    struct Part {
        cost: f64,
        turns: u64,
        requests: &'static [u64],
        cached: Option<u64>,
    }

    fn part(cost: f64, turns: u64, requests: &'static [u64]) -> Part {
        Part { cost, turns, requests, cached: None }
    }

    /// A run of `shape` made of `parts`, which `paired` names the other branch of.
    fn staged(task: &'static str, arm: &'static str, shape: Shape, paired: Option<&str>, success: bool, parts: &[Part]) -> RunResult {
        let mut r = result(&Spec { success, ..spec(task, arm) });
        let sessions: Vec<crate::run::SessionResult> = parts
            .iter()
            .map(|p| {
                let mut session = r.session.clone();
                let outcome = session.outcome.as_mut().unwrap();
                outcome.total_cost_usd = Some(p.cost);
                outcome.num_turns = Some(p.turns);
                outcome.duration_ms = Some(p.turns * 1000);
                session.request_tokens = p.requests.to_vec();
                session.first_request_tokens = p.requests.first().copied();
                session.first_request_cache_read = p.cached;
                crate::run::SessionResult { session, score: r.score.clone(), followed: vec![], saved: vec![], native_memory_writes: 0, saves: vec![] }
            })
            .collect();
        r.session = sessions.last().unwrap().session.clone();
        r.sessions = sessions;
        r.shape = shape;
        r.paired = paired.map(str::to_owned);
        r
    }

    fn fact(tracer: &str, class: crate::run::FactClass) -> crate::run::Fact {
        crate::run::Fact { tracer: tracer.to_owned(), born: 1, class }
    }

    #[test]
    fn rows_are_split_by_the_shape_of_the_run_and_add_up_its_sessions() {
        use crate::run::FactClass;
        let mut first = staged("relay", "a3", Shape::Split, None, true, &[part(0.10, 3, &[1000, 2000]), part(0.20, 4, &[1500, 2500])]);
        first.overhead = Some(300);
        first.facts = vec![fact("a", FactClass::Used)];
        let mut second = staged("relay", "a3", Shape::Split, None, false, &[part(0.30, 5, &[1000]), part(0.40, 6, &[3000])]);
        second.overhead = Some(100);
        second.facts = vec![fact("a", FactClass::NotSaved)];
        let mut joined = staged("relay", "a3", Shape::Joined, None, true, &[part(0.10, 3, &[100, 200]), part(0.10, 3, &[300])]);
        joined.facts = vec![fact("a", FactClass::SavedNotReached)];
        let single = result(&spec("short", "a3"));

        let report = aggregate(&[joined, first, single, second]);

        let keys: Vec<(&str, &str)> = report.rows.iter().map(|r| (r.task.as_str(), r.shape.as_str())).collect();
        assert_eq!(keys, [("relay", "split"), ("relay", "joined"), ("short", "split")], "a row for each task, arm and shape");
        let split = &report.rows[0];
        assert_eq!((split.runs, split.successes, split.sessions), (2, 1, 2.0));
        assert!((split.cost_usd.unwrap() - 0.5).abs() < 1e-9, "the mean of the cost of the whole run, the sum of its sessions: {:?}", split.cost_usd);
        assert_eq!((split.turns, split.seconds), (Some(9.0), Some(9.0)));
        assert_eq!(split.context_tokens, Some(1750.0), "the median of the context of every request of the row");
        assert_eq!(split.overhead, Some(200.0));
        assert_eq!((split.facts_used, split.not_saved, split.saved_not_reached, split.reached_not_used), (1, 1, 0, 0));
        let joined = &report.rows[1];
        assert_eq!((joined.context_tokens, joined.overhead, joined.saved_not_reached), (Some(200.0), None, 1));
        let short = &report.rows[2];
        assert_eq!((short.sessions, short.cost_usd, short.turns), (1.0, Some(0.25), Some(4.0)), "a result of one session reports as before");

        let text = render(&report);
        let header = text.lines().next().unwrap();
        assert!(header.starts_with("| task | category | arm | shape | runs |"), "{header}");
        assert!(header.ends_with("| sessions | context tokens | relay overhead | not_saved | saved_not_reached | reached_not_used | facts used |"), "{header}");
        assert!(text.lines().nth(2).unwrap().starts_with("| relay | K1 | a3 | split | 2 | 1/2 |"), "{text}");
        assert!(text.lines().nth(2).unwrap().ends_with("| 2.0 | 1750 | 200 | 1 | 0 | 0 | 1 |"), "{text}");
    }

    /// The two branches of one fork, named by where they were run.
    fn pair(run: &str, passes: (bool, bool), fork: &'static [u64], joined: (f64, Option<u64>), split_cost: f64) -> [(String, RunResult); 2] {
        let first = || Part { cost: 0.65, turns: 10, requests: fork, cached: None };
        let continued = Part { cost: joined.0, turns: 6, requests: &[150_000], cached: joined.1 };
        [
            (format!("{run}/joined"), staged("relay", "a3", Shape::Joined, Some("split"), passes.0, &[first(), continued])),
            (format!("{run}/split"), staged("relay", "a3", Shape::Split, Some("joined"), passes.1, &[first(), part(split_cost, 5, &[20_000])])),
        ]
    }

    #[test]
    fn a_forked_run_reports_its_pairs_and_what_the_new_session_saved() {
        let runs: Vec<(String, RunResult)> = [
            pair("r1", (true, true), &[100_000, 150_000], (0.35, Some(148_000)), 0.25),
            pair("r2", (true, false), &[100_000, 160_000], (0.40, Some(150_000)), 0.20),
            pair("r3", (false, false), &[100_000, 140_000], (0.50, Some(149_000)), 0.30),
        ]
        .concat();
        let mut with_noise = runs.clone();
        with_noise.push(("alone".to_owned(), result(&spec("relay", "a3"))));
        with_noise.push(("lone/joined".to_owned(), staged("relay", "a3", Shape::Joined, Some("split"), true, &[part(0.1, 1, &[10]), part(0.1, 1, &[10])])));

        let found = pairs(&with_noise);

        assert_eq!(found.len(), 1);
        let row = &found[0];
        assert_eq!((row.task.as_str(), row.arm.as_str(), row.pairs), ("relay", "a3", 3), "a run without a partner and a run that did not fork are no pair");
        assert_eq!((row.joined_passed, row.split_passed, row.continued_only), (2, 1, 1));
        assert_eq!(row.fork_context, Some(150_000.0), "the median of the last request of the first session");
        let savings = [1.0 - 0.25 / 0.35, 1.0 - 0.20 / 0.40, 1.0 - 0.30 / 0.50];
        assert!((row.saving.unwrap() - savings[2]).abs() < 1e-9, "the median saving of the new session over the continued one: {:?}", row.saving);
        assert_eq!(row.priced, 3);

        let text = render_pairs(&found);
        assert!(
            text.lines().next().unwrap().starts_with("| task | arm | pairs | joined passed | split passed | joined passed and split failed | context at the fork | cost saving | pairs priced |"),
            "{text}"
        );
        assert!(text.contains("| relay | a3 | 3 | 2/3 | 1/3 | 1 | 150000 | 40.0% | 3 |"), "{text}");

        let report = aggregate(&runs.iter().map(|(_, r)| r.clone()).collect::<Vec<_>>());
        assert!((report.cost_usd - (3.0 * 0.65 + 0.35 + 0.25 + 0.40 + 0.20 + 0.50 + 0.30)).abs() < 1e-9, "the first session of a fork is paid once: {}", report.cost_usd);
    }

    #[test]
    fn a_pair_that_continued_cold_does_not_count_for_the_cost() {
        let runs: Vec<(String, RunResult)> = [
            pair("warm", (true, true), &[1, 2], (0.40, Some(120_000)), 0.20),
            pair("cold", (true, true), &[1, 2], (0.30, Some(10_000)), 0.30),
            pair("none", (true, true), &[1, 2], (0.30, None), 0.30),
        ]
        .concat();
        let row = &pairs(&runs)[0];
        assert_eq!(row.pairs, 3);
        assert_eq!(row.priced, 1, "only the pair that read 80 % of its first request from the cache is priced");
        assert!((row.saving.unwrap() - 0.5).abs() < 1e-9, "{:?}", row.saving);
        assert_eq!((row.joined_passed, row.split_passed), (3, 3), "a cold continuation still counts for the success");

        let cold: Vec<(String, RunResult)> = pair("cold", (true, true), &[1, 2], (0.30, Some(10_000)), 0.30).into();
        let row = &pairs(&cold)[0];
        assert_eq!((row.priced, row.saving), (0, None));
        assert!(render_pairs(&pairs(&cold)).contains("| - | 0 |"), "{}", render_pairs(&pairs(&cold)));
    }

    #[test]
    fn the_costs_have_a_line_per_session_with_its_number_and_shape() {
        let fork = pair("r1", (true, true), &[1000], (0.35, Some(900)), 0.25);
        let mut once = result(&spec("short", "a0"));
        once.session.model = Some("m".to_owned());
        let named: Vec<(String, RunResult)> = fork.into_iter().chain([("short".to_owned(), once)]).collect();

        let csv = costs(&named);
        let lines: Vec<&str> = csv.lines().collect();

        assert!(lines[0].ends_with(",cost_usd,cost_basis,session,shape"), "{}", lines[0]);
        let keyed: Vec<(&str, &str, &str)> = lines[1..]
            .iter()
            .map(|l| {
                let f: Vec<&str> = l.split(',').collect();
                (f[0], f[f.len() - 2], f[f.len() - 1])
            })
            .collect();
        assert_eq!(
            keyed,
            [("r1/joined", "1", "joined"), ("r1/joined", "2", "joined"), ("r1/split", "2", "split"), ("short", "1", "split")],
            "the first session of a fork is listed once, with the branch that ran it first"
        );
    }

    #[test]
    fn the_calls_table_tells_what_each_tool_adds() {
        let message = |id: &str, context: u64, call: &str, tool: &str| {
            format!(
                "{}\n{}\n",
                serde_json::json!({"type": "assistant", "message": {"id": id, "content": [{"type": "tool_use", "id": call, "name": tool, "input": {}}], "usage": {"input_tokens": context}}}),
                serde_json::json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": call, "content": "x"}]}})
            )
        };
        let end = |id: &str, context: u64| format!("{}\n", serde_json::json!({"type": "assistant", "message": {"id": id, "content": [], "usage": {"input_tokens": context}}}));
        let first = message("m1", 1000, "a", "mcp__clawg__search_memory")
            + &message("m2", 3000, "b", "Read")
            + &message("m3", 3100, "c", "mcp__clawg__search_memory")
            + &message("m4", 3300, "d", "mcp__clawg__recall_context")
            + &end("m5", 6000);
        let second = message("n1", 500, "e", "mcp__clawg__search_memory") + &end("n2", 800);
        let transcripts = [crate::stream::parse(&first), crate::stream::parse(&second)];

        let text = calls(&transcripts);

        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "| tool | calls | with tokens | median tokens | p90 tokens | note |");
        assert_eq!(lines[2], "| search_memory | 3 | 3 | 300 | 2000 |  |", "its calls added 2000, 200 and 300 tokens: {text}");
        assert!(text.contains("| recall_context | 1 | 1 | 2700 | 2700 | candidate for a diet |"), "{text}");
        assert!(text.contains("| Read | 1 | 1 | 100 | 100 |  |"), "a tool that is not a server one is never a candidate: {text}");
    }
}
