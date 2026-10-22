//! The pure half of `warned`: which pairs of an export the topic-key warning
//! marked, which saves it did not fire on, how the labels of both score against
//! the criterion fixed before any real case was seen.
//!
//! Nothing here touches a file, a store or a model. The values that carry
//! stored text (`Selection::saves`, the label lines) stay between this module and
//! `run.rs`; the result it builds holds counts, rates, intervals and the cuts
//! of distance and store size, and nothing else.

use std::collections::{BTreeMap, HashMap, HashSet};

use clawg_core::eval_api::{is_claim_type, EvalSaved};
use clawg_core::memory::backup::{parse_snapshot_name, parse_snapshot_time, Reason};
use clawg_core::memory::portability::format::{self, ConflictLine, Filter, FormatError, MemoryLine, Parsed};
use serde::{Deserialize, Serialize};

use crate::stats::{Rate, SplitMix64};
use crate::store::{self, Gate, Prevalence, SaveGroup, WarningCost};

/// The seed of the sample when `--seed` is not given.
pub const DEFAULT_SEED: u64 = 0x5EED_0006;

/// The most saves labelled among those the warning did not fire on. When the
/// window holds fewer, all of them are labelled and they are a census, like
/// the marked pairs.
pub const REST_SAMPLE: usize = 100;

/// The number of marked pairs at which the rule stops waiting: a precision
/// that neither clears the minimum nor falls below it by then leaves the
/// warning as it is.
pub const N_MAX: usize = 200;

#[derive(Debug, thiserror::Error)]
pub enum WarnedError {
    #[error("the export cannot be read: {0}")]
    Format(#[from] FormatError),
    #[error("the export was not made with `clawg export --include-deleted`: the pairs of a deleted row are missing from it")]
    WithoutDeleted,
    #[error("the export is limited to {0}: part of the store and of its pairs is missing from it")]
    Filtered(&'static str),
    #[error("{0:?} is not the name of a pre-migration snapshot (memory-<UTC>-pre-migration.db)")]
    Since(String),
    #[error("{0} marked pairs carry no distance: a store that did not keep it cannot be measured")]
    NoDistance(usize),
    #[error("pair {n} has no `{field}`: every pair needs `a` and `b`, and `c` where they differ")]
    EmptyLabel { n: usize, field: &'static str },
    #[error("pair {n} holds {value:?} in `{field}`; a label is `revision` or `other`")]
    BadLabel { n: usize, field: &'static str, value: String },
    #[error("line {line} of the labels file cannot be read: {reason}")]
    Labels { line: usize, reason: String },
    #[error("the labels file does not match the replay: {0}")]
    Mismatch(String),
    #[error("the replay file cannot be read: {0}")]
    Replay(String),
}

impl WarnedError {
    /// Whether the working directory is worth keeping: a labels file the owner
    /// has not finished is fixed by editing it, and the hours spent labelling
    /// are in it.
    pub fn keeps_work(&self) -> bool {
        matches!(self, Self::EmptyLabel { .. } | Self::BadLabel { .. } | Self::Labels { .. } | Self::Mismatch(_))
    }
}

/// The window start, in milliseconds, of the snapshot `since` names: the
/// `pre-migration` one the opening that migrated the store wrote. A path names
/// its file, so the one `clawg backup` printed can be pasted as it is.
pub fn parse_since(since: &str) -> Result<i64, WarnedError> {
    let name = since.rsplit(['/', '\\']).next().unwrap_or(since);
    if parse_snapshot_name(name) != Some(Reason::PreMigration) {
        return Err(WarnedError::Since(since.to_string()));
    }
    parse_snapshot_time(name).ok_or_else(|| WarnedError::Since(since.to_string()))
}

fn check_filter(filter: &Filter) -> Result<(), WarnedError> {
    if !filter.include_deleted {
        return Err(WarnedError::WithoutDeleted);
    }
    if filter.project.is_some() {
        return Err(WarnedError::Filtered("a project"));
    }
    if filter.agent.is_some() {
        return Err(WarnedError::Filtered("an agent"));
    }
    Ok(())
}

/// One save of the export, as the agent made it.
#[derive(Debug, Clone, PartialEq)]
pub struct WSave {
    pub agent: Option<String>,
    pub project: Option<String>,
    pub memory_type: String,
    pub importance: f64,
    pub topic_key: Option<String>,
    pub text: String,
}

/// What the agent did with a warning, read from the export.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actions {
    /// The warned save's first row has a key.
    pub followed: bool,
    /// The old row was replaced by a verdict in favour of a row of the warned
    /// save.
    pub hid: bool,
    /// A later save of the same agent and project carries the warned save's
    /// key. Only a followed warning can have one.
    pub reused: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PickKind {
    /// The warning fired: `earlier` is the save of the pair's old row.
    Marked { earlier: usize, distance: f64, actions: Actions },
    /// The warning did not: the replay finds, among `predecessors`, the save
    /// the closest of its rows lies to.
    Rest { predecessors: Vec<usize> },
}

/// A save picked for the owner to label, by its place in [`Selection::saves`].
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub later: usize,
    /// The saves of the export made before it and live when it came.
    pub store_size: usize,
    pub kind: PickKind,
}

/// What the window holds. Counts only.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowCounts {
    pub rows: usize,
    pub saves: usize,
    pub window_saves: usize,
    pub window_saves_with_agent: usize,
    /// Warned saves with a pair to compare.
    pub marked: usize,
    /// Marks of the same save beyond the first: each piece of a long save
    /// marks its own pair against the row the warning named.
    pub marks_merged: usize,
    /// Marks the export cannot place: a row it lacks, two rows of one save, a
    /// save no agent made.
    pub marked_skipped: usize,
    /// Saves the warning could have fired on and did not.
    pub rest: usize,
    pub rest_chosen: usize,
    /// Chosen saves the replay could not pair with an earlier one.
    pub rest_unpaired: usize,
    pub invalid_lines: usize,
}

/// What the first step reads from an export.
#[derive(Debug)]
pub struct Selection {
    pub format: String,
    pub version: u32,
    pub window_start_ms: i64,
    pub counts: WindowCounts,
    /// The store size at each save of the window that has an agent.
    pub store_sizes: Vec<usize>,
    /// Every save of the export, in the order they were made.
    pub saves: Vec<WSave>,
    /// The places in `saves` of those with an agent: what a replay sends.
    pub flow: Vec<usize>,
    /// The pairs to label, in an order that tells nothing about which is which.
    pub picks: Vec<Pick>,
}

/// Reads `text` as an export and chooses the pairs to label: every marked pair
/// of the window and a seeded sample of the saves it did not fire on.
pub fn select(text: &str, since_ms: i64, seed: u64) -> Result<Selection, WarnedError> {
    let (header, parsed) = format::parse(text)?;
    check_filter(&header.filter)?;
    let mut groups = store::saves_of(&parsed);
    groups.sort_by_key(|g| (g.head.created_at, g.head.id));
    let group_of = group_of_rows(&groups);
    let rows: HashMap<i64, &MemoryLine> = parsed.memories.iter().map(|(_, m)| (m.id, m)).collect();
    let world = World { gone_at: gone_at(&parsed, &rows), groups };

    let window: Vec<usize> = (0..world.groups.len()).filter(|&i| world.groups[i].head.created_at >= since_ms).collect();
    let in_flow: Vec<usize> = window.iter().copied().filter(|&i| world.has_agent(i)).collect();
    let mut counts = WindowCounts {
        rows: parsed.memories.len(),
        saves: world.groups.len(),
        window_saves: window.len(),
        window_saves_with_agent: in_flow.len(),
        invalid_lines: parsed.invalid.len(),
        ..WindowCounts::default()
    };
    let sizes: HashMap<usize, usize> = in_flow.iter().map(|&i| (i, world.store_size(i))).collect();

    // A save with a mark that cannot be placed still fired: it is not part of
    // the rest.
    let mut warned: HashSet<usize> = HashSet::new();
    let mut marks: BTreeMap<usize, Vec<Mark>> = BTreeMap::new();
    for (_, line) in parsed.conflicts.iter().filter(|(_, c)| c.topic_key_warned) {
        let Some(&later) = group_of.get(&line.new_id) else {
            counts.marked_skipped += 1;
            continue;
        };
        warned.insert(later);
        if world.groups[later].head.created_at < since_ms {
            continue;
        }
        let Some(&earlier) = group_of.get(&line.old_id) else {
            counts.marked_skipped += 1;
            continue;
        };
        if earlier == later || !world.has_agent(later) {
            counts.marked_skipped += 1;
            continue;
        }
        marks.entry(later).or_default().push(Mark { old: rows[&line.old_id], earlier, line });
    }

    let mut picks = Vec::new();
    let mut without_distance = 0;
    for (later, found) in marks {
        counts.marks_merged += found.len() - 1;
        let distance_of = |m: &Mark| m.line.distance.unwrap_or(f64::INFINITY);
        let best = found.into_iter().min_by(|a, b| distance_of(a).total_cmp(&distance_of(b))).expect("a save with marks has one");
        let Some(distance) = best.line.distance else {
            without_distance += 1;
            continue;
        };
        counts.marked += 1;
        picks.push(Pick {
            later,
            store_size: sizes[&later],
            kind: PickKind::Marked { earlier: best.earlier, distance, actions: world.actions(later, best.old) },
        });
    }
    if without_distance > 0 {
        return Err(WarnedError::NoDistance(without_distance));
    }

    let mut rest: Vec<(usize, Vec<usize>)> = in_flow
        .iter()
        .filter_map(|&i| {
            let head = world.groups[i].head;
            if !is_claim_type(&head.memory_type) || head.topic_key.is_some() || warned.contains(&i) {
                return None;
            }
            let predecessors = world.predecessors(i);
            (!predecessors.is_empty()).then_some((i, predecessors))
        })
        .collect();
    counts.rest = rest.len();
    let mut rng = SplitMix64::new(seed);
    rng.shuffle(&mut rest);
    rest.truncate(REST_SAMPLE);
    counts.rest_chosen = rest.len();
    picks.extend(
        rest.into_iter().map(|(later, predecessors)| Pick { later, store_size: sizes[&later], kind: PickKind::Rest { predecessors } }),
    );
    rng.shuffle(&mut picks);

    let store_sizes = in_flow.iter().map(|i| sizes[i]).collect();
    let saves: Vec<WSave> = world
        .groups
        .iter()
        .map(|g| WSave {
            agent: g.head.agent_id.clone(),
            project: g.head.project_id.clone(),
            memory_type: g.head.memory_type.clone(),
            importance: g.head.importance,
            topic_key: g.head.topic_key.clone(),
            text: g.text.clone(),
        })
        .collect();
    let flow = (0..saves.len()).filter(|&i| saves[i].agent.is_some()).collect();
    Ok(Selection {
        format: header.format,
        version: header.version,
        window_start_ms: since_ms,
        counts,
        store_sizes,
        saves,
        flow,
        picks,
    })
}

/// A pair the warning marked, with the save of its old row.
struct Mark<'a> {
    old: &'a MemoryLine,
    earlier: usize,
    line: &'a ConflictLine,
}

/// The save each row belongs to: the first one, in the order they were made,
/// that holds it.
fn group_of_rows(groups: &[SaveGroup<'_>]) -> HashMap<i64, usize> {
    let mut of = HashMap::new();
    for (i, group) in groups.iter().enumerate() {
        for row in &group.rows {
            of.entry(row.id).or_insert(i);
        }
    }
    of
}

/// When each row that is not live any more stopped being so: the earlier of
/// its deletion and its replacement. A replacement by a revision happened when
/// the row that replaced it was made; one by a verdict, when the pair between
/// the two was judged, or failing that when the row that won was made. A
/// replaced row whose replacement the export lacks was never live for a later
/// save.
fn gone_at(parsed: &Parsed, rows: &HashMap<i64, &MemoryLine>) -> HashMap<i64, i64> {
    let pair = |a: i64, b: i64| (a.min(b), a.max(b));
    let mut judged: HashMap<(i64, i64), i64> = HashMap::new();
    for (_, c) in &parsed.conflicts {
        if let (true, Some(at)) = (c.state == "judged" && c.verdict.as_deref() == Some("supersedes"), c.judged_at) {
            judged.entry(pair(c.new_id, c.old_id)).or_insert(at);
        }
    }
    let mut gone = HashMap::new();
    for (_, row) in &parsed.memories {
        let replaced = (row.status == "superseded").then(|| {
            let by = row.superseded_by;
            let made = by.and_then(|id| rows.get(&id)).map(|r| r.created_at);
            match row.supersede_cause.as_deref() {
                Some("verdict") => by.and_then(|id| judged.get(&pair(row.id, id)).copied()).or(made),
                _ => made,
            }
            .unwrap_or(row.created_at)
        });
        if let Some(at) = [row.deleted_at, replaced].into_iter().flatten().min() {
            gone.insert(row.id, at);
        }
    }
    gone
}

/// The saves of an export in the order they were made, and when their rows
/// stopped being live.
struct World<'a> {
    groups: Vec<SaveGroup<'a>>,
    gone_at: HashMap<i64, i64>,
}

impl World<'_> {
    fn has_agent(&self, save: usize) -> bool {
        self.groups[save].head.agent_id.is_some()
    }

    /// Whether the save still held a live row at `at`: made before, not deleted
    /// and not replaced by then.
    fn live(&self, save: usize, at: i64) -> bool {
        self.groups[save].rows.iter().any(|r| r.created_at < at && self.gone_at.get(&r.id).map_or(true, |&gone| gone > at))
    }

    fn same_scope(&self, a: usize, b: usize) -> bool {
        let (a, b) = (self.groups[a].head, self.groups[b].head);
        a.agent_id == b.agent_id && a.project_id == b.project_id
    }

    /// The saves made before the save `i` and live when it came, with or
    /// without an agent.
    fn store_size(&self, i: usize) -> usize {
        let at = self.groups[i].head.created_at;
        (0..i).filter(|&j| self.live(j, at)).count()
    }

    /// The earlier saves of the same agent and project live when the save `i`
    /// came: the only ones it could revise.
    fn predecessors(&self, i: usize) -> Vec<usize> {
        let at = self.groups[i].head.created_at;
        (0..i).filter(|&j| self.same_scope(j, i) && self.live(j, at)).collect()
    }

    /// What the agent did about the warning of the save `later`, whose pair
    /// named the row `old`.
    fn actions(&self, later: usize, old: &MemoryLine) -> Actions {
        let group = &self.groups[later];
        let key = group.head.topic_key.as_ref();
        let hid = old.status == "superseded"
            && old.supersede_cause.as_deref() == Some("verdict")
            && old.superseded_by.is_some_and(|by| group.rows.iter().any(|r| r.id == by));
        let reused = key.is_some_and(|key| {
            ((later + 1)..self.groups.len()).any(|k| self.same_scope(k, later) && self.groups[k].head.topic_key.as_ref() == Some(key))
        });
        Actions { followed: key.is_some(), hid, reused }
    }
}

/// The rows a replay stored, by the save that made each.
pub struct ReplayRows {
    reported: HashMap<usize, Vec<i64>>,
    inserted: HashMap<usize, Vec<i64>>,
    owner: HashMap<i64, usize>,
}

impl ReplayRows {
    /// `flow[k]` is the save the `k`th of the `reports` answers.
    pub fn new(flow: &[usize], reports: &[EvalSaved]) -> Self {
        assert_eq!(flow.len(), reports.len(), "a report for every save of the flow");
        let mut rows = Self { reported: HashMap::new(), inserted: HashMap::new(), owner: HashMap::new() };
        for (&save, report) in flow.iter().zip(reports) {
            rows.reported.insert(save, report.rows.iter().map(|&(id, _)| id).collect());
            let own: Vec<i64> = report.rows.iter().filter(|&&(_, inserted)| inserted).map(|&(id, _)| id).collect();
            rows.owner.extend(own.iter().map(|&id| (id, save)));
            rows.inserted.insert(save, own);
        }
        rows
    }

    /// Every row the replay of `save` reported, those it only repeated included.
    pub fn reported(&self, save: usize) -> Vec<i64> {
        self.reported.get(&save).cloned().unwrap_or_default()
    }

    /// The rows the replay of `saves` stored.
    pub fn inserted(&self, saves: &[usize]) -> Vec<i64> {
        saves.iter().flat_map(|s| self.inserted.get(s).into_iter().flatten().copied()).collect()
    }

    /// The save that made the row of `distances` (`(from, to, distance)`) that
    /// lies the least distance away; the earlier of two as close.
    pub fn nearest(&self, distances: &[(i64, i64, f64)]) -> Option<usize> {
        let mut nearest: BTreeMap<usize, f64> = BTreeMap::new();
        for &(_, row, distance) in distances {
            let Some(&save) = self.owner.get(&row) else { continue };
            let best = nearest.entry(save).or_insert(distance);
            *best = best.min(distance);
        }
        store::closest(&nearest.into_iter().collect::<Vec<_>>())
    }
}

/// One line of `labels.jsonl`. Nothing in it says whether the warning fired.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelLine {
    pub n: usize,
    pub earlier: String,
    pub later: String,
    /// The first blind pass: `revision` or `other`.
    pub a: Option<String>,
    /// The second blind pass.
    pub b: Option<String>,
    /// The third, which only settles what `a` and `b` disagree on.
    pub c: Option<String>,
}

/// What `warned.json` keeps of a labelled pair: no text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairRecord {
    pub n: usize,
    /// The warning fired on it.
    pub marked: bool,
    /// The detector's distance, of the marked pairs.
    pub distance: Option<f64>,
    /// The saves of the export made before it and live when it came.
    pub store_size: usize,
    pub actions: Option<Actions>,
}

pub struct Labels {
    pub lines: Vec<LabelLine>,
    pub pairs: Vec<PairRecord>,
    /// Chosen saves of the rest the replay found no earlier save for.
    pub unpaired: usize,
}

/// The pairs to label, numbered in the order they were picked. `found[k]` is
/// the earlier save the replay found for `selection.picks[k]` when it is a save of
/// the rest, `None` when no row of its predecessors could be measured; the
/// entries of the marked pairs, which come with theirs, are not read.
pub fn build_labels(selection: &Selection, found: &[Option<usize>]) -> Labels {
    assert_eq!(selection.picks.len(), found.len(), "an entry for every pick");
    let mut labels = Labels { lines: Vec::new(), pairs: Vec::new(), unpaired: 0 };
    for (pick, found) in selection.picks.iter().zip(found) {
        let (marked, earlier, distance, actions) = match &pick.kind {
            PickKind::Marked { earlier, distance, actions } => (true, Some(*earlier), Some(*distance), Some(*actions)),
            PickKind::Rest { .. } => (false, *found, None, None),
        };
        let Some(earlier) = earlier else {
            labels.unpaired += 1;
            continue;
        };
        let n = labels.lines.len() + 1;
        labels.lines.push(LabelLine {
            n,
            earlier: selection.saves[earlier].text.clone(),
            later: selection.saves[pick.later].text.clone(),
            a: None,
            b: None,
            c: None,
        });
        labels.pairs.push(PairRecord { n, marked, distance, store_size: pick.store_size, actions });
    }
    labels
}

/// Everything the first step leaves for the second, apart from the labels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WarnedFile {
    pub export_format: String,
    pub export_version: u32,
    pub model_contract: String,
    pub seed: u64,
    /// A run on the double: its result is printed, never written.
    pub fake: bool,
    pub window_start_ms: i64,
    pub counts: WindowCounts,
    pub warning_line_tokens: usize,
    /// The store size at each save of the window that has an agent.
    pub store_sizes: Vec<usize>,
    /// One record for each line of the labels file, in its order.
    pub pairs: Vec<PairRecord>,
}

pub fn parse_warned(text: &str) -> Result<WarnedFile, WarnedError> {
    serde_json::from_str(text).map_err(|e| WarnedError::Replay(e.to_string()))
}

pub fn parse_labels(text: &str) -> Result<Vec<LabelLine>, WarnedError> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(i, line)| {
            serde_json::from_str(line).map_err(|e| WarnedError::Labels { line: i + 1, reason: e.to_string() })
        })
        .collect()
}

/// The 95 % normal quantile of the asymptotic interval of kappa.
const Z: f64 = 1.96;

/// Cohen's kappa between the first two passes, over every pair labelled.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Kappa {
    pub n: usize,
    pub disagreements: usize,
    pub observed: f64,
    pub expected: f64,
    /// Null when the passes cannot differ (`expected` is 1) or there is no pair.
    pub kappa: Option<f64>,
    pub lo: Option<f64>,
    pub hi: Option<f64>,
}

/// `passes` holds, for each pair, whether the first and the second pass said
/// `revision`. The interval is `kappa ± z · sqrt(p_o (1 − p_o) / n) / (1 − p_e)`.
fn kappa(passes: &[(bool, bool)]) -> Kappa {
    let n = passes.len();
    let disagreements = passes.iter().filter(|(a, b)| a != b).count();
    if n == 0 {
        return Kappa { n, disagreements, observed: 0.0, expected: 0.0, kappa: None, lo: None, hi: None };
    }
    let total = n as f64;
    let observed = (n - disagreements) as f64 / total;
    let share = |said: fn(&(bool, bool)) -> bool| passes.iter().filter(|p| said(p)).count() as f64 / total;
    let (a, b) = (share(|p| p.0), share(|p| p.1));
    let expected = a * b + (1.0 - a) * (1.0 - b);
    if expected >= 1.0 {
        return Kappa { n, disagreements, observed, expected, kappa: None, lo: None, hi: None };
    }
    let value = (observed - expected) / (1.0 - expected);
    let half = Z * (observed * (1.0 - observed) / total).sqrt() / (1.0 - expected);
    Kappa { n, disagreements, observed, expected, kappa: Some(value), lo: Some(value - half), hi: Some(value + half) }
}

/// What the rule says about the warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// The lower bound of the precision clears the minimum.
    Keeps,
    /// The upper bound falls below it.
    AdjustOrRemove,
    /// Neither, with [`N_MAX`] marked pairs or more: the warning stays.
    KeepsByDefault,
    /// Neither, with fewer: more cases are needed.
    GatherMore,
}

/// What looking at the precision by distance leaves, once the warning is to be
/// adjusted or removed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DistanceOutcome {
    /// The closest third clears the minimum: the rule shrinks to this distance.
    NarrowTo { max_distance: f64 },
    /// It does not.
    Remove,
}

/// The values at which `values` is cut into three bands: the highest value of
/// the first two. The first band ends at the third of them rounded up and the
/// second at two thirds rounded up, and a value equal to the last one of a
/// band goes with that band, so a run of equals is never split. Null where a
/// band holds nothing.
fn tertile_bounds(values: &[f64]) -> [Option<f64>; 2] {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    let extend = |mut at: usize| {
        while at > 0 && at < n && sorted[at] == sorted[at - 1] {
            at += 1;
        }
        at
    };
    let first = extend(n.div_ceil(3));
    let second = extend((2 * n).div_ceil(3).max(first));
    [first.checked_sub(1).map(|i| sorted[i]), second.checked_sub(1).map(|i| sorted[i])]
}

/// The band, from 0 to 2, a value falls in.
fn band_of(bounds: &[Option<f64>; 2], value: f64) -> usize {
    if bounds[0].is_some_and(|b| value <= b) {
        0
    } else if bounds[1].is_some_and(|b| value <= b) {
        1
    } else {
        2
    }
}

/// A third of the marked pairs by the detector's distance.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DistanceBand {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub precision: Rate,
    /// The lower bound of the precision clears the minimum.
    pub hits: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ByDistance {
    pub bands: Vec<DistanceBand>,
    /// Only when the warning is to be adjusted or removed.
    pub outcome: Option<DistanceOutcome>,
}

/// A third of the saves of the window by the size of the store when they came.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SizeBand {
    pub min: Option<usize>,
    pub max: Option<usize>,
    pub window_saves: usize,
    /// Marked saves of the window saves in the band.
    pub fire_rate: Rate,
    pub precision: Rate,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Discrimination {
    pub rest_population: usize,
    pub rest_labelled: usize,
    /// Revisions and non-revisions among the saves the warning did not fire
    /// on: the rest times the share the sample had.
    pub estimated_false_negatives: Option<f64>,
    pub estimated_true_negatives: Option<f64>,
    pub sensitivity: Option<f64>,
    pub false_alarm_rate: Option<f64>,
    /// Sensitivity over false-alarm rate; null where there is no false alarm.
    pub positive_likelihood_ratio: Option<f64>,
}

/// The conditions of the earlier measurement that do not depend on how the
/// rule is read, each with its value.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Gates {
    pub line_tokens: Gate,
    pub mean_tokens_per_save: Gate,
    pub revision_share: Gate,
    pub revision_share_lower_bound: Gate,
    pub met: bool,
}

/// What the agent did about the warnings of one group of marked pairs.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GroupActions {
    pub marked: usize,
    pub followed: Rate,
    pub hid: Rate,
    /// Over the warnings that were followed.
    pub reused: Rate,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentActions {
    pub hits: GroupActions,
    pub false_alarms: GroupActions,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LabelledCounts {
    pub marked: usize,
    pub rest: usize,
    /// Pairs the first two passes disagreed on.
    pub tiebreaks: usize,
}

/// What the second step reports. It holds counts, rates, intervals and cuts:
/// no text, no row id and no key.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Scored {
    pub export_format: String,
    pub export_version: u32,
    pub model_contract: String,
    pub seed: u64,
    #[serde(skip)]
    pub fake: bool,
    pub window_start_ms: i64,
    pub window: WindowCounts,
    pub labelled: LabelledCounts,
    pub agreement: Kappa,
    pub precision: Rate,
    pub prevalence: Prevalence,
    /// `2π / (1 + π)`: the precision at which the warning fires twice as often
    /// on revisions as on the rest.
    pub p_min: f64,
    pub discrimination: Discrimination,
    pub decision: Decision,
    pub n_max: usize,
    pub by_distance: ByDistance,
    pub by_store_size: Vec<SizeBand>,
    pub warning: WarningCost,
    pub gates: Gates,
    pub agent_actions: AgentActions,
}

/// A marked pair with the label it ended with.
struct Judged {
    distance: f64,
    store_size: usize,
    actions: Actions,
    revision: bool,
}

fn revision_of(n: usize, field: &'static str, value: Option<&str>) -> Result<bool, WarnedError> {
    match value {
        None | Some("") => Err(WarnedError::EmptyLabel { n, field }),
        Some(value) if store::valid(value) => Ok(value == store::REVISION),
        Some(value) => Err(WarnedError::BadLabel { n, field, value: value.to_string() }),
    }
}

/// Scores the labels. Every pair needs `a` and `b`, and `c` where they differ;
/// the label is the one the first two share, or the third.
pub fn score(file: &WarnedFile, labels: &[LabelLine]) -> Result<Scored, WarnedError> {
    if labels.len() != file.pairs.len() {
        return Err(WarnedError::Mismatch(format!("{} pairs in the file, {} in the replay", labels.len(), file.pairs.len())));
    }
    let mut passes = Vec::with_capacity(labels.len());
    let mut marked: Vec<Judged> = Vec::new();
    let mut rest: Vec<bool> = Vec::new();
    let mut without_distance = 0;
    for (line, pair) in labels.iter().zip(&file.pairs) {
        if line.n != pair.n {
            return Err(WarnedError::Mismatch(format!("pair {} stands where the replay has {}", line.n, pair.n)));
        }
        let a = revision_of(line.n, "a", line.a.as_deref())?;
        let b = revision_of(line.n, "b", line.b.as_deref())?;
        let c = match line.c.as_deref() {
            None | Some("") => None,
            Some(c) => Some(revision_of(line.n, "c", Some(c))?),
        };
        let revision = if a == b { a } else { c.ok_or(WarnedError::EmptyLabel { n: line.n, field: "c" })? };
        passes.push((a, b));
        match (pair.marked, pair.distance) {
            (false, _) => rest.push(revision),
            (true, Some(distance)) => marked.push(Judged {
                distance,
                store_size: pair.store_size,
                actions: pair.actions.unwrap_or_default(),
                revision,
            }),
            (true, None) => without_distance += 1,
        }
    }
    if without_distance > 0 {
        return Err(WarnedError::NoDistance(without_distance));
    }

    let revisions = marked.iter().filter(|m| m.revision).count();
    let rest_revisions = rest.iter().filter(|&&r| r).count();
    let precision = Rate::of(revisions, marked.len());
    let prevalence = store::prevalence(marked.len() + file.counts.rest, marked.len(), revisions, rest.len(), rest_revisions);
    let p_min = 2.0 * prevalence.estimate / (1.0 + prevalence.estimate);
    let decision = if precision.lo > p_min {
        Decision::Keeps
    } else if precision.hi < p_min {
        Decision::AdjustOrRemove
    } else if marked.len() >= N_MAX {
        Decision::KeepsByDefault
    } else {
        Decision::GatherMore
    };
    let by_distance = by_distance(&marked, p_min, decision);

    let fire_rate = Rate::of(marked.len(), file.counts.window_saves_with_agent);
    let line_tokens = file.warning_line_tokens;
    let warning = WarningCost { line_tokens, mean_tokens_per_save: line_tokens as f64 * fire_rate.rate, fire_rate };
    let mut gates = Gates {
        line_tokens: Gate::at_most(line_tokens as f64, 40.0),
        mean_tokens_per_save: Gate::at_most(warning.mean_tokens_per_save, 15.0),
        revision_share: Gate::at_least(prevalence.estimate, 0.10),
        revision_share_lower_bound: Gate::at_least(prevalence.lower_bound, 0.05),
        met: false,
    };
    gates.met = [&gates.line_tokens, &gates.mean_tokens_per_save, &gates.revision_share, &gates.revision_share_lower_bound]
        .iter()
        .all(|gate| gate.met);

    let (hits, false_alarms): (Vec<&Judged>, Vec<&Judged>) = marked.iter().partition(|m| m.revision);
    Ok(Scored {
        export_format: file.export_format.clone(),
        export_version: file.export_version,
        model_contract: file.model_contract.clone(),
        seed: file.seed,
        fake: file.fake,
        window_start_ms: file.window_start_ms,
        window: file.counts.clone(),
        labelled: LabelledCounts {
            marked: marked.len(),
            rest: rest.len(),
            tiebreaks: passes.iter().filter(|(a, b)| a != b).count(),
        },
        agreement: kappa(&passes),
        discrimination: discrimination(revisions, marked.len() - revisions, file.counts.rest, rest.len(), rest_revisions),
        precision,
        prevalence,
        p_min,
        decision,
        n_max: N_MAX,
        by_distance,
        by_store_size: by_store_size(file, &marked),
        warning,
        gates,
        agent_actions: AgentActions { hits: group_actions(&hits), false_alarms: group_actions(&false_alarms) },
    })
}

/// Sensitivity and the rate of false alarms, with the revisions and
/// non-revisions of the rest estimated from its sample.
fn discrimination(
    hits: usize,
    false_alarms: usize,
    rest_population: usize,
    sample: usize,
    sample_revisions: usize,
) -> Discrimination {
    let estimate = if sample > 0 {
        let share = sample_revisions as f64 / sample as f64;
        Some((rest_population as f64 * share, rest_population as f64 * (1.0 - share)))
    } else if rest_population == 0 {
        Some((0.0, 0.0))
    } else {
        None
    };
    let (missed, quiet) = (estimate.map(|e| e.0), estimate.map(|e| e.1));
    let ratio = |found: usize, other: Option<f64>| {
        let all = found as f64 + other?;
        (all > 0.0).then(|| found as f64 / all)
    };
    let (sensitivity, false_alarm_rate) = (ratio(hits, missed), ratio(false_alarms, quiet));
    let positive_likelihood_ratio = match (sensitivity, false_alarm_rate) {
        (Some(s), Some(f)) if f > 0.0 => Some(s / f),
        _ => None,
    };
    Discrimination {
        rest_population,
        rest_labelled: sample,
        estimated_false_negatives: missed,
        estimated_true_negatives: quiet,
        sensitivity,
        false_alarm_rate,
        positive_likelihood_ratio,
    }
}

/// The precision of each third of the marked pairs by distance. When the
/// warning is to be adjusted or removed, the closest third decides which.
fn by_distance(marked: &[Judged], p_min: f64, decision: Decision) -> ByDistance {
    if marked.is_empty() {
        return ByDistance { bands: Vec::new(), outcome: None };
    }
    let bounds = tertile_bounds(&marked.iter().map(|m| m.distance).collect::<Vec<_>>());
    let mut bands: [(Vec<f64>, usize); 3] = Default::default();
    for m in marked {
        let (distances, revisions) = &mut bands[band_of(&bounds, m.distance)];
        distances.push(m.distance);
        *revisions += usize::from(m.revision);
    }
    let bands: Vec<DistanceBand> = bands
        .into_iter()
        .map(|(distances, revisions)| {
            let precision = Rate::of(revisions, distances.len());
            DistanceBand {
                min: distances.iter().copied().reduce(f64::min),
                max: distances.iter().copied().reduce(f64::max),
                hits: precision.lo > p_min,
                precision,
            }
        })
        .collect();
    let outcome = (decision == Decision::AdjustOrRemove).then(|| match bands.first() {
        Some(DistanceBand { hits: true, max: Some(max_distance), .. }) => DistanceOutcome::NarrowTo { max_distance: *max_distance },
        _ => DistanceOutcome::Remove,
    });
    ByDistance { bands, outcome }
}

/// The fire rate and the precision of each third of the window's saves by the
/// size of the store when they came.
fn by_store_size(file: &WarnedFile, marked: &[Judged]) -> Vec<SizeBand> {
    if file.store_sizes.is_empty() {
        return Vec::new();
    }
    let bounds = tertile_bounds(&file.store_sizes.iter().map(|&s| s as f64).collect::<Vec<_>>());
    let mut sizes: [Vec<usize>; 3] = Default::default();
    for &size in &file.store_sizes {
        sizes[band_of(&bounds, size as f64)].push(size);
    }
    let mut fired: [(usize, usize); 3] = Default::default();
    for m in marked {
        let (n, revisions) = &mut fired[band_of(&bounds, m.store_size as f64)];
        *n += 1;
        *revisions += usize::from(m.revision);
    }
    sizes
        .into_iter()
        .zip(fired)
        .map(|(sizes, (marked, revisions))| SizeBand {
            min: sizes.iter().copied().min(),
            max: sizes.iter().copied().max(),
            window_saves: sizes.len(),
            fire_rate: Rate::of(marked, sizes.len()),
            precision: Rate::of(revisions, marked),
        })
        .collect()
}

fn group_actions(group: &[&Judged]) -> GroupActions {
    let count = |holds: fn(&Actions) -> bool| group.iter().filter(|m| holds(&m.actions)).count();
    let followed = count(|a| a.followed);
    GroupActions {
        marked: group.len(),
        followed: Rate::of(followed, group.len()),
        hid: Rate::of(count(|a| a.hid), group.len()),
        reused: Rate::of(count(|a| a.followed && a.reused), followed),
    }
}

#[cfg(test)]
mod tests {
    use clawg_core::memory::backup::{snapshot_name, Reason};
    use serde_json::json;

    use super::*;
    use crate::store::fixtures::*;

    fn select_of(rows: &[Row], extra: &[String], since: i64) -> Result<Selection, WarnedError> {
        let mut lines: Vec<String> = rows.iter().map(Row::line).collect();
        lines.extend(extra.iter().cloned());
        select(&export(&lines), since, 7)
    }

    fn marked(selection: &Selection) -> Vec<&Pick> {
        selection.picks.iter().filter(|p| matches!(p.kind, PickKind::Marked { .. })).collect()
    }

    #[test]
    fn an_export_made_without_the_deleted_rows_or_limited_to_part_of_the_store_is_refused() {
        let lines = vec![Row::new(1, "a", "p", "one").line()];
        let with = |filter: serde_json::Value| select(&export_with(header_filtered(1, filter), &lines), 0, 1);

        assert!(matches!(with(json!({"include_deleted": false})), Err(WarnedError::WithoutDeleted)));
        assert!(matches!(with(json!({})), Err(WarnedError::WithoutDeleted)), "no filter says nothing of the deleted rows");
        assert!(matches!(with(json!({"include_deleted": true, "project": "repo-1"})), Err(WarnedError::Filtered("a project"))));
        assert!(matches!(with(json!({"include_deleted": true, "agent": "claude-code"})), Err(WarnedError::Filtered("an agent"))));
        assert!(with(json!({"include_deleted": true})).is_ok());
        let unknown = export(&lines).replacen("\"version\":1", "\"version\":2", 1);
        assert!(matches!(select(&unknown, 0, 1), Err(WarnedError::Format(_))));
    }

    #[test]
    fn only_the_name_of_a_pre_migration_snapshot_gives_a_window_start() {
        const MS: i64 = 1_791_827_476_123;
        let name = |reason| snapshot_name(MS, reason);
        assert_eq!(parse_since(&name(Reason::PreMigration)).unwrap(), MS);
        let windows = format!("C:\\Users\\someone\\.clawg\\backups\\{}", name(Reason::PreMigration));
        assert_eq!(parse_since(&windows).unwrap(), MS, "a path names its file");
        let unix = format!("/home/someone/.clawg/backups/{}", name(Reason::PreMigration));
        assert_eq!(parse_since(&unix).unwrap(), MS);
        for other in [
            name(Reason::Manual),
            name(Reason::PreRestore),
            name(Reason::PreReembed),
            name(Reason::PreImport),
            "memory.db".to_string(),
            "2026-10-22".to_string(),
            String::new(),
        ] {
            assert!(matches!(parse_since(&other), Err(WarnedError::Since(_))), "{other}");
        }
    }

    #[test]
    fn the_window_is_the_saves_made_from_the_snapshot_on() {
        let mut shared = Row::new(5, "a", "p", "A note no agent saved.");
        (shared.agent, shared.project) = (None, None);
        let rows = [
            Row::new(1, "a", "p", "before"),
            Row::new(2, "a", "p", "before too"),
            Row::new(3, "a", "p", "the first of the window"),
            Row::new(4, "a", "p", "inside"),
            shared,
        ];

        let selection = select_of(&rows, &[], at(3)).unwrap();

        let c = &selection.counts;
        assert_eq!((c.rows, c.saves, c.window_saves, c.window_saves_with_agent), (5, 5, 3, 2));
        assert_eq!(selection.store_sizes.len(), 2, "one size for each save of the window that has an agent");
        assert_eq!((selection.format.as_str(), selection.version, selection.window_start_ms), ("clawg-memories", 1, at(3)));
    }

    #[test]
    fn a_marked_pair_compares_the_warned_save_with_the_save_of_the_old_row() {
        let rows = [
            Row::new(1, "a", "p", "Cache: Redis."),
            Row::new(5, "a", "p", "Before the window, warned."),
            Row::new(10, "a", "p", "Cache: Memcached."),
            Row::new(11, "a", "p", "Not warned."),
        ];
        // A mark before the window is nobody's case, even when its old row is gone.
        let pairs = [
            conflict(10, 1, Some(0.21), true),
            conflict(11, 1, Some(0.35), false),
            conflict(5, 1, Some(0.3), true),
            conflict(5, 99, Some(0.3), true),
        ];

        let selection = select_of(&rows, &pairs, at(10)).unwrap();

        assert_eq!((selection.counts.marked, selection.counts.marked_skipped, selection.counts.marks_merged), (1, 0, 0));
        let marked = marked(&selection);
        assert_eq!(marked.len(), 1);
        let PickKind::Marked { earlier, distance, .. } = &marked[0].kind else { unreachable!() };
        assert_eq!(selection.saves[*earlier].text, "Cache: Redis.");
        assert_eq!(selection.saves[marked[0].later].text, "Cache: Memcached.");
        assert_eq!(*distance, 0.21);
    }

    #[test]
    fn a_marked_pair_the_export_cannot_place_is_skipped_and_counted() {
        let mut nobody = Row::new(12, "a", "p", "Warned, but no agent saved it.");
        (nobody.agent, nobody.project) = (None, None);
        let rows = [
            Row::new(1, "a", "p", "earlier"),
            Row::new(10, "a", "p", "placed"),
            Row::new(11, "a", "p", "its old row is not in the export"),
            nobody,
            Row::new(13, "a", "p", "first piece"),
            Row::new(14, "a", "p", "second piece"),
        ];
        let pairs = [
            conflict(10, 1, Some(0.2), true),
            conflict(11, 99, Some(0.2), true),
            conflict(98, 1, Some(0.2), true),
            conflict(12, 1, Some(0.2), true),
            conflict(14, 13, Some(0.2), true),
        ];
        let pieces = vec![piece(100, 1, 13), piece(100, 2, 14)];
        let mut lines: Vec<String> = rows.iter().map(Row::line).collect();
        lines.extend(pairs);
        lines.extend(pieces);

        let selection = select(&export(&lines), at(10), 7).unwrap();

        assert_eq!((selection.counts.marked, selection.counts.marked_skipped), (1, 4));
    }

    #[test]
    fn the_marks_of_the_pieces_of_one_save_are_one_marked_pair_at_the_least_distance() {
        let rows = [
            Row::new(1, "a", "p", "earlier"),
            Row::new(10, "a", "p", "first piece"),
            Row::new(11, "a", "p", "second piece"),
            Row::new(12, "a", "p", "third piece"),
        ];
        let mut lines: Vec<String> = rows.iter().map(Row::line).collect();
        lines.extend([
            conflict(11, 1, Some(0.30), true),
            conflict(10, 1, Some(0.25), true),
            conflict(12, 1, Some(0.40), true),
            piece(100, 1, 10),
            piece(100, 2, 11),
            piece(100, 3, 12),
        ]);

        let selection = select(&export(&lines), at(10), 7).unwrap();

        assert_eq!((selection.counts.marked, selection.counts.marks_merged), (1, 2));
        let PickKind::Marked { distance, .. } = &marked(&selection)[0].kind else { unreachable!() };
        assert_eq!(*distance, 0.25);
    }

    fn rest(selection: &Selection) -> Vec<&Pick> {
        selection.picks.iter().filter(|p| matches!(p.kind, PickKind::Rest { .. })).collect()
    }

    fn texts<'a>(selection: &'a Selection, picks: &[&Pick]) -> Vec<&'a str> {
        let mut texts: Vec<&str> = picks.iter().map(|p| selection.saves[p.later].text.as_str()).collect();
        texts.sort();
        texts
    }

    /// `n` saves of one agent and project, a second apart.
    fn one_scope(n: i64) -> Vec<Row> {
        (1..=n).map(|i| Row::new(i, "a", "p", &format!("save number {i}"))).collect()
    }

    #[test]
    fn the_store_size_counts_the_saves_live_when_the_save_came() {
        let mut deleted_before = Row::new(2, "a", "p", "deleted before the save");
        deleted_before.deleted_at = Some(at(5));
        let mut deleted_after = Row::new(3, "a", "p", "deleted after the save");
        deleted_after.deleted_at = Some(at(30));
        let mut revised = Row::new(4, "a", "p", "replaced by a revision");
        (revised.status, revised.cause, revised.superseded_by) = ("superseded", Some("revision"), Some(6));
        let mut judged_later = Row::new(5, "a", "p", "replaced by a verdict, judged after the save");
        (judged_later.status, judged_later.cause, judged_later.superseded_by) = ("superseded", Some("verdict"), Some(7));
        let mut judged_before = Row::new(8, "a", "p", "replaced by a verdict, judged before the save");
        (judged_before.status, judged_before.cause, judged_before.superseded_by) = ("superseded", Some("verdict"), Some(9));
        let mut unjudged = Row::new(11, "a", "p", "replaced by a verdict no pair dates");
        (unjudged.status, unjudged.cause, unjudged.superseded_by) = ("superseded", Some("verdict"), Some(12));
        let mut nobody = Row::new(13, "a", "p", "A note no agent saved.");
        (nobody.agent, nobody.project) = (None, None);
        let rows = [
            Row::new(1, "a", "p", "live"),
            deleted_before,
            deleted_after,
            revised,
            Row::new(6, "a", "p", "replaced the revision"),
            judged_later,
            Row::new(7, "a", "p", "replaced the verdict judged later"),
            judged_before,
            Row::new(9, "a", "p", "replaced the verdict judged before"),
            unjudged,
            Row::new(12, "a", "p", "replaced the unjudged one"),
            nobody,
            Row::new(20, "a", "p", "the save of the window"),
            Row::new(21, "a", "p", "the one after it"),
        ];
        let pairs = [judged(7, 5, at(25)), judged(9, 8, at(10))];

        let selection = select_of(&rows, &pairs, at(20)).unwrap();

        // 1, 3, 6, 5 and 7 (a verdict not yet judged leaves its row live), 9, 12 and the shared note.
        assert_eq!(selection.store_sizes, vec![8, 9]);
        let sizes: Vec<usize> = selection.picks.iter().map(|p| p.store_size).collect();
        assert!(sizes.contains(&8) && sizes.contains(&9), "{sizes:?}");
    }

    #[test]
    fn the_rest_is_the_unkeyed_claims_of_the_window_with_a_live_earlier_save_of_their_scope() {
        let mut keyed = Row::new(11, "a2", "p", "keyed without a mark");
        keyed.topic_key = Some("k/x");
        let mut deleted = Row::new(4, "a4", "p", "predecessor deleted");
        deleted.deleted_at = Some(at(5));
        let mut summary = Row::new(14, "a5", "p", "asserts nothing");
        summary.memory_type = "session_summary";
        let mut replaced = Row::new(7, "a7", "p", "predecessor replaced by a verdict");
        (replaced.status, replaced.cause, replaced.superseded_by) = ("superseded", Some("verdict"), Some(8));
        let mut replaced_later = Row::new(9, "a8", "p", "predecessor replaced after the save");
        (replaced_later.status, replaced_later.cause, replaced_later.superseded_by) = ("superseded", Some("verdict"), Some(91));
        let mut nobody = Row::new(18, "a9", "p", "A note no agent saved.");
        (nobody.agent, nobody.project) = (None, None);
        let rows = [
            Row::new(1, "a1", "p", "s1 earlier"),
            Row::new(10, "a1", "p", "s1 later"),
            Row::new(2, "a2", "p", "s2 earlier"),
            keyed,
            Row::new(3, "a3", "p", "s3 earlier"),
            Row::new(12, "a3", "p", "s3 warned"),
            deleted,
            Row::new(13, "a4", "p", "s4 later"),
            Row::new(5, "a5", "p", "s5 earlier"),
            summary,
            Row::new(6, "a6", "q", "s6 earlier, in another project"),
            Row::new(15, "a6", "p", "s6 later"),
            replaced,
            Row::new(8, "b7", "p", "s7 the replacing row"),
            Row::new(16, "a7", "p", "s7 later"),
            replaced_later,
            Row::new(91, "b8", "p", "s8 the replacing row"),
            Row::new(17, "a8", "p", "s8 later"),
            nobody,
            Row::new(20, "a10", "p", "s10 earlier"),
            Row::new(21, "a10", "p", "s10 later, its mark cannot be placed"),
        ];
        let pairs = [
            conflict(12, 3, Some(0.2), true),
            judged(8, 7, at(9)),
            judged(91, 9, at(95)),
            conflict(21, 99, Some(0.2), true),
        ];

        let selection = select_of(&rows, &pairs, at(10)).unwrap();

        let picks = rest(&selection);
        assert_eq!(texts(&selection, &picks), ["s1 later", "s8 later"]);
        assert_eq!((selection.counts.rest, selection.counts.rest_chosen), (2, 2));
        assert_eq!((selection.counts.marked, selection.counts.marked_skipped), (1, 1));
        for pick in picks {
            let PickKind::Rest { predecessors } = &pick.kind else { unreachable!() };
            let earlier: Vec<&str> = predecessors.iter().map(|&i| selection.saves[i].text.as_str()).collect();
            assert!(earlier == ["s1 earlier"] || earlier == ["predecessor replaced after the save"], "{earlier:?}");
        }
    }

    #[test]
    fn the_sample_of_the_rest_is_a_hundred_saves_or_all_of_them() {
        let many = select_of(&one_scope(151), &[], at(2)).unwrap();
        assert_eq!((many.counts.rest, many.counts.rest_chosen, many.picks.len()), (150, REST_SAMPLE, REST_SAMPLE));

        let few = select_of(&one_scope(61), &[], at(2)).unwrap();
        assert_eq!((few.counts.rest, few.counts.rest_chosen, few.picks.len()), (60, 60, 60));
    }

    #[test]
    fn the_same_seed_chooses_the_same_pairs_in_the_same_order() {
        let rows = one_scope(151);
        let lines: Vec<String> = rows.iter().map(Row::line).collect();
        let text = export(&lines);

        let a = select(&text, at(2), 7).unwrap();

        assert_eq!(a.picks, select(&text, at(2), 7).unwrap().picks);
        assert_ne!(a.picks, select(&text, at(2), 8).unwrap().picks);
    }

    #[test]
    fn every_marked_pair_is_picked_and_mixed_in_with_the_rest() {
        let rows = one_scope(161);
        let pairs: Vec<String> = (2..=31).map(|i| conflict(i, i - 1, Some(0.2), true)).collect();

        let selection = select_of(&rows, &pairs, at(2)).unwrap();

        assert_eq!((marked(&selection).len(), rest(&selection).len()), (30, REST_SAMPLE));
        assert!(selection.picks[..30].iter().any(|p| matches!(p.kind, PickKind::Rest { .. })), "the marked pairs are not listed first");
    }

    /// What the agent did about the only marked pair of `rows`, whose warned
    /// save is row 10 and whose old row is row 1.
    fn actions_in(rows: Vec<Row>) -> Actions {
        let selection = select_of(&rows, &[conflict(10, 1, Some(0.2), true)], at(10)).unwrap();
        let marked = marked(&selection);
        assert_eq!(marked.len(), 1);
        let PickKind::Marked { actions, .. } = &marked[0].kind else { unreachable!() };
        *actions
    }

    #[test]
    fn a_warning_is_followed_when_the_warned_save_took_a_key() {
        let earlier = Row::new(1, "a", "p", "earlier");
        let mut followed = Row::new(10, "a", "p", "warned, then saved again under a key");
        followed.topic_key = Some("cache/layer");

        let actions = actions_in(vec![earlier, followed]);

        assert_eq!(actions, Actions { followed: true, hid: false, reused: false });
        let unfollowed = actions_in(vec![Row::new(1, "a", "p", "earlier"), Row::new(10, "a", "p", "warned, ignored")]);
        assert_eq!(unfollowed, Actions::default());
    }

    #[test]
    fn a_key_is_reused_by_a_later_save_of_the_same_agent_and_project_under_it() {
        let rows = |tweak: &dyn Fn(&mut Row)| {
            let mut followed = Row::new(10, "a", "p", "warned, then saved again under a key");
            followed.topic_key = Some("cache/layer");
            let mut other = Row::new(12, "a", "p", "a later save under the same key");
            other.topic_key = Some("cache/layer");
            tweak(&mut other);
            vec![Row::new(1, "a", "p", "earlier"), followed, other]
        };

        assert!(actions_in(rows(&|_| {})).reused);
        assert!(!actions_in(rows(&|r| r.topic_key = Some("cache/other"))).reused, "another key");
        assert!(!actions_in(rows(&|r| r.project = Some("q"))).reused, "another project");
        assert!(!actions_in(rows(&|r| r.agent = Some("b"))).reused, "another agent");
        assert!(!actions_in(rows(&|r| r.topic_key = None)).reused, "no key");
        let mut before = rows(&|_| {});
        before[2].created_at = at(5);
        assert!(!actions_in(before).reused, "made before the warned save");
        let mut ignored = rows(&|_| {});
        ignored[1].topic_key = None;
        let actions = actions_in(ignored);
        assert!(!actions.followed && !actions.reused, "an ignored warning has no key to reuse");
    }

    #[test]
    fn the_named_row_is_hidden_when_a_verdict_replaced_it_in_favour_of_the_warned_save() {
        let replaced = |cause: &'static str, by: i64| {
            let mut old = Row::new(1, "a", "p", "earlier");
            (old.status, old.cause, old.superseded_by) = ("superseded", Some(cause), Some(by));
            vec![old, Row::new(10, "a", "p", "warned"), Row::new(11, "a", "p", "another save")]
        };

        assert!(actions_in(replaced("verdict", 10)).hid);
        assert!(!actions_in(replaced("verdict", 11)).hid, "replaced in favour of another save");
        assert!(!actions_in(replaced("revision", 10)).hid, "a revision is a key, not a verdict");
        assert!(!actions_in(vec![Row::new(1, "a", "p", "earlier"), Row::new(10, "a", "p", "warned")]).hid, "still live");
    }

    #[test]
    fn a_verdict_in_favour_of_any_piece_of_the_warned_save_hides_the_named_row() {
        let mut old = Row::new(1, "a", "p", "earlier");
        (old.status, old.cause, old.superseded_by) = ("superseded", Some("verdict"), Some(11));
        let mut lines: Vec<String> =
            [old, Row::new(10, "a", "p", "first piece"), Row::new(11, "a", "p", "second piece")].iter().map(Row::line).collect();
        lines.extend([conflict(10, 1, Some(0.2), true), piece(100, 1, 10), piece(100, 2, 11)]);

        let selection = select(&export(&lines), at(10), 7).unwrap();

        let PickKind::Marked { actions, .. } = &marked(&selection)[0].kind else { unreachable!() };
        assert!(actions.hid);
    }

    fn reported(rows: &[(i64, bool)]) -> EvalSaved {
        EvalSaved { rows: rows.to_vec(), related: vec![], replaced: vec![] }
    }

    #[test]
    fn the_closest_predecessor_is_the_save_whose_row_lies_nearest_to_a_row_of_the_save() {
        let reports = [reported(&[(10, true)]), reported(&[(11, true)]), reported(&[(12, true), (13, false)])];
        let rows = ReplayRows::new(&[4, 7, 9], &reports);

        assert_eq!(rows.reported(9), vec![12, 13]);
        assert_eq!(rows.inserted(&[4, 7]), vec![10, 11], "only the rows a save stored are its own");
        assert_eq!(rows.nearest(&[(12, 10, 0.5), (12, 11, 0.2), (13, 10, 0.9)]), Some(7));
        assert_eq!(rows.nearest(&[(12, 10, 0.2), (12, 11, 0.2)]), Some(4), "of two as close, the earlier one");
        assert_eq!(rows.nearest(&[(12, 77, 0.1)]), None, "a row no save of the replay stored is no candidate");
        assert_eq!(rows.nearest(&[]), None);
    }

    /// A single scope of `n` saves in which the saves 2 to 11 are warned.
    fn warned_and_rest(n: i64) -> Selection {
        let pairs: Vec<String> = (2..=11).map(|i| conflict(i, i - 1, Some(0.2 + i as f64 / 100.0), true)).collect();
        select_of(&one_scope(n), &pairs, at(2)).unwrap()
    }

    /// The replay finds the save just before each save of the rest.
    fn found_before(selection: &Selection) -> Vec<Option<usize>> {
        selection.picks.iter().map(|p| Some(p.later - 1)).collect()
    }

    #[test]
    fn the_labels_file_does_not_say_which_pair_fired() {
        let selection = warned_and_rest(61);

        let labels = build_labels(&selection, &found_before(&selection));

        assert_eq!((labels.lines.len(), labels.pairs.len(), labels.unpaired), (60, 60, 0));
        for (i, line) in labels.lines.iter().enumerate() {
            assert_eq!(line.n, i + 1);
            let value: serde_json::Value = serde_json::from_str(&serde_json::to_string(line).unwrap()).unwrap();
            let mut keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
            keys.sort();
            assert_eq!(keys, ["a", "b", "c", "earlier", "later", "n"]);
            assert!(value["a"].is_null() && value["b"].is_null() && value["c"].is_null());
        }
        assert!(labels.pairs[..10].iter().any(|p| !p.marked), "the marked pairs are not listed first");
        assert_eq!(labels.pairs.iter().filter(|p| p.marked).count(), 10);
    }

    #[test]
    fn a_pair_record_keeps_the_distance_and_the_actions_of_the_marked_pairs_only() {
        let selection = warned_and_rest(61);

        let labels = build_labels(&selection, &found_before(&selection));

        for record in &labels.pairs {
            let line = &labels.lines[record.n - 1];
            let number = |text: &str| text.rsplit(' ').next().unwrap().parse::<i64>().unwrap();
            assert_eq!((record.distance.is_some(), record.actions.is_some()), (record.marked, record.marked));
            if record.marked {
                assert_eq!(number(&line.earlier), number(&line.later) - 1, "the pair of the export");
                let expected = 0.2 + number(&line.later) as f64 / 100.0;
                assert!((record.distance.unwrap() - expected).abs() < 1e-9, "{:?} against {expected}", record.distance);
            }
        }
    }

    #[test]
    fn a_save_of_the_rest_the_replay_could_not_pair_is_left_out_and_counted() {
        let selection = warned_and_rest(61);
        let mut found = found_before(&selection);
        let mut left_out = 0;
        for (pick, found) in selection.picks.iter().zip(found.iter_mut()) {
            if matches!(pick.kind, PickKind::Rest { .. }) && left_out < 3 {
                *found = None;
                left_out += 1;
            }
        }

        let labels = build_labels(&selection, &found);

        assert_eq!((labels.lines.len(), labels.unpaired), (57, 3));
        assert_eq!(labels.pairs.iter().filter(|p| p.marked).count(), 10, "a marked pair needs no replay");
        assert_eq!(labels.lines.last().unwrap().n, 57, "the lines are numbered without gaps");
    }

    #[test]
    fn a_marked_pair_is_labelled_whatever_the_replay_found() {
        let selection = warned_and_rest(61);

        let labels = build_labels(&selection, &vec![None; selection.picks.len()]);

        assert_eq!((labels.lines.len(), labels.unpaired), (10, 50));
        assert!(labels.pairs.iter().all(|p| p.marked));
    }

    #[test]
    fn a_marked_pair_without_a_distance_is_refused_without_naming_it() {
        let rows = [Row::new(1, "a", "p", "earlier"), Row::new(10, "a", "p", "later")];

        let error = select_of(&rows, &[conflict(10, 1, None, true)], at(10)).err().expect("refused");

        assert!(matches!(error, WarnedError::NoDistance(1)), "{error}");
        assert_eq!(error.to_string(), "1 marked pairs carry no distance: a store that did not keep it cannot be measured");
    }

    // The second step.

    /// The pairs of a labelled run, built by hand: the marked ones as
    /// `(distance, store size, is a revision)` and then those of the rest as
    /// `(is a revision)`. Every pair is labelled alike by both passes.
    struct Case {
        marked: Vec<(f64, usize, bool)>,
        rest: Vec<bool>,
        rest_population: usize,
        store_sizes: Vec<usize>,
        actions: Vec<Actions>,
    }

    impl Case {
        fn new(marked: &[(f64, usize, bool)], rest: &[bool]) -> Self {
            Self {
                marked: marked.to_vec(),
                rest: rest.to_vec(),
                rest_population: rest.len(),
                store_sizes: (0..1000).collect(),
                actions: vec![Actions::default(); marked.len()],
            }
        }

        fn file(&self) -> WarnedFile {
            let mut pairs = Vec::new();
            for (i, &(distance, store_size, _)) in self.marked.iter().enumerate() {
                pairs.push(PairRecord {
                    n: i + 1,
                    marked: true,
                    distance: Some(distance),
                    store_size,
                    actions: Some(self.actions[i]),
                });
            }
            for j in 0..self.rest.len() {
                pairs.push(PairRecord { n: self.marked.len() + j + 1, marked: false, distance: None, store_size: 10, actions: None });
            }
            WarnedFile {
                export_format: "clawg-memories".into(),
                export_version: 1,
                model_contract: "test".into(),
                seed: 7,
                fake: false,
                window_start_ms: 0,
                counts: WindowCounts {
                    window_saves_with_agent: self.store_sizes.len(),
                    marked: self.marked.len(),
                    rest: self.rest_population,
                    rest_chosen: self.rest.len(),
                    ..WindowCounts::default()
                },
                warning_line_tokens: 30,
                store_sizes: self.store_sizes.clone(),
                pairs,
            }
        }

        fn lines(&self) -> Vec<LabelLine> {
            let revisions = self.marked.iter().map(|m| m.2).chain(self.rest.iter().copied());
            revisions.enumerate().map(|(i, revision)| pass_line(i + 1, label(revision), label(revision), None)).collect()
        }

        fn score(&self) -> Scored {
            score(&self.file(), &self.lines()).expect("the labels score")
        }
    }

    fn label(revision: bool) -> &'static str {
        if revision {
            "revision"
        } else {
            "other"
        }
    }

    fn pass_line(n: usize, a: &str, b: &str, c: Option<&str>) -> LabelLine {
        let text = |v: &str| (!v.is_empty()).then(|| v.to_string());
        LabelLine { n, earlier: "e".into(), later: "l".into(), a: text(a), b: text(b), c: c.map(str::to_string) }
    }

    /// `count` marked pairs a tenth apart from the first, `revisions` of them
    /// revisions (the closest ones first).
    fn marked_spread(count: usize, revisions: usize) -> Vec<(f64, usize, bool)> {
        (0..count).map(|i| (0.10 + i as f64 / 1000.0, 10, i < revisions)).collect()
    }

    #[test]
    fn a_labels_file_is_refused_where_a_pass_is_empty_or_wrong_and_the_directory_stays() {
        let case = Case::new(&marked_spread(2, 1), &[false]);
        let file = case.file();
        let ok = case.lines();
        let with = |edit: &dyn Fn(&mut Vec<LabelLine>)| {
            let mut lines = ok.clone();
            edit(&mut lines);
            score(&file, &lines).err().expect("refused")
        };

        for empty in [None, Some(String::new())] {
            let error = with(&|l| l[1].a = empty.clone());
            assert!(matches!(error, WarnedError::EmptyLabel { n: 2, field: "a" }), "{error}");
            let error = with(&|l| l[2].b = empty.clone());
            assert!(matches!(error, WarnedError::EmptyLabel { n: 3, field: "b" }), "{error}");
            assert!(error.keeps_work(), "the hours of labelling are in that file");
        }
        let error = with(&|l| l[0].b = Some("maybe".into()));
        assert!(matches!(error, WarnedError::BadLabel { n: 1, field: "b", .. }), "{error}");
        let error = with(&|l| {
            l[0].a = Some("revision".into());
            l[0].b = Some("other".into());
            l[0].c = Some("both".into());
        });
        assert!(matches!(error, WarnedError::BadLabel { n: 1, field: "c", .. }), "{error}");
        assert!(matches!(with(&|l| l.truncate(2)), WarnedError::Mismatch(_)));
        assert!(matches!(with(&|l| l[0].n = 9), WarnedError::Mismatch(_)));
        assert!(!WarnedError::Replay("x".into()).keeps_work() && !WarnedError::NoDistance(1).keeps_work());
    }

    #[test]
    fn a_disagreement_needs_the_third_pass_and_it_decides() {
        let case = Case::new(&marked_spread(3, 0), &[]);
        let file = case.file();
        let lines = |c: Option<&str>| {
            vec![pass_line(1, "revision", "revision", None), pass_line(2, "revision", "other", c), pass_line(3, "other", "other", Some("revision"))]
        };

        for empty in [None, Some("")] {
            let error = score(&file, &lines(empty)).err().expect("refused");
            assert!(matches!(error, WarnedError::EmptyLabel { n: 2, field: "c" }), "{error}");
            assert!(error.keeps_work());
        }
        let revision = score(&file, &lines(Some("revision"))).unwrap();
        let other = score(&file, &lines(Some("other"))).unwrap();
        // The pair they share wins; the third pass only settles pair 2, and is ignored on pair 3.
        assert_eq!((revision.precision.k, revision.precision.n), (2, 3));
        assert_eq!((other.precision.k, other.precision.n), (1, 3));
        assert_eq!((revision.labelled.tiebreaks, revision.agreement.disagreements), (1, 1));
    }

    #[test]
    fn cohens_kappa_comes_with_its_asymptotic_interval_and_is_null_when_the_passes_cannot_differ() {
        let passes = |pairs: &[(&str, &str, usize)]| -> Vec<(bool, bool)> {
            pairs.iter().flat_map(|&(a, b, n)| vec![(a == "revision", b == "revision"); n]).collect()
        };
        // 4 revisions both, 4 other both, one each way: p_o = 0.8, p_e = 0.5.
        let k = kappa(&passes(&[("revision", "revision", 4), ("other", "other", 4), ("revision", "other", 1), ("other", "revision", 1)]));

        assert_eq!((k.n, k.disagreements), (10, 2));
        assert!((k.observed - 0.8).abs() < 1e-12 && (k.expected - 0.5).abs() < 1e-12);
        let value = k.kappa.unwrap();
        assert!((value - 0.6).abs() < 1e-12, "{value}");
        let half = 1.96 * (0.8_f64 * 0.2 / 10.0).sqrt() / 0.5;
        assert!((k.lo.unwrap() - (0.6 - half)).abs() < 1e-12 && (k.hi.unwrap() - (0.6 + half)).abs() < 1e-12);

        let same = kappa(&passes(&[("revision", "revision", 5)]));
        assert_eq!((same.kappa, same.lo, same.hi), (None, None, None), "p_e is 1 when both passes say one thing");
        assert!((same.observed - 1.0).abs() < 1e-12 && (same.expected - 1.0).abs() < 1e-12);
        let perfect = kappa(&passes(&[("revision", "revision", 3), ("other", "other", 3)]));
        assert_eq!((perfect.kappa, perfect.lo, perfect.hi), (Some(1.0), Some(1.0), Some(1.0)));
        assert_eq!(kappa(&[]).kappa, None);
    }

    #[test]
    fn the_precision_is_the_revisions_among_the_marked_with_their_wilson_interval() {
        let scored = Case::new(&marked_spread(100, 90), &vec![false; 100]).score();

        assert_eq!((scored.precision.k, scored.precision.n), (90, 100));
        let wilson = crate::stats::Rate::of(90, 100);
        assert_eq!((scored.precision.lo, scored.precision.hi), (wilson.lo, wilson.hi));
        assert_eq!((scored.labelled.marked, scored.labelled.rest), (100, 100));
    }

    #[test]
    fn the_minimum_precision_comes_from_the_prevalence_of_the_marked_pairs_and_the_rest() {
        // 90 revisions among 100 marked, none among 100 of the rest, which is all of it: π = 0.45.
        let scored = Case::new(&marked_spread(100, 90), &vec![false; 100]).score();

        assert!((scored.prevalence.estimate - 0.45).abs() < 1e-12 && scored.prevalence.census);
        assert!((scored.p_min - 0.9 / 1.45).abs() < 1e-12, "{}", scored.p_min);

        // A rest that is a sample of a larger one: the marked pairs are a census, the rest is not.
        let mut sampled = Case::new(&marked_spread(100, 50), &[vec![true; 10], vec![false; 40]].concat());
        sampled.rest_population = 400;
        let scored = sampled.score();
        assert!(!scored.prevalence.census);
        assert!((scored.prevalence.estimate - (50.0 + 400.0 * 0.2) / 500.0).abs() < 1e-12);
    }

    #[test]
    fn sensitivity_and_false_alarms_estimate_what_the_warning_missed_from_the_sample_of_the_rest() {
        // 100 marked, 60 of them revisions; the rest has 400 saves, a sample of 50 holds 10 revisions.
        let mut case = Case::new(&marked_spread(100, 60), &[vec![true; 10], vec![false; 40]].concat());
        case.rest_population = 400;

        let d = case.score().discrimination;

        assert!((d.estimated_false_negatives.unwrap() - 80.0).abs() < 1e-9 && (d.estimated_true_negatives.unwrap() - 320.0).abs() < 1e-9);
        assert!((d.sensitivity.unwrap() - 60.0 / 140.0).abs() < 1e-12);
        assert!((d.false_alarm_rate.unwrap() - 40.0 / 360.0).abs() < 1e-12);
        assert!((d.positive_likelihood_ratio.unwrap() - (60.0 / 140.0) / (40.0 / 360.0)).abs() < 1e-12);

        let mut no_alarms = Case::new(&marked_spread(10, 10), &[false; 20]);
        no_alarms.rest_population = 20;
        let d = no_alarms.score().discrimination;
        assert_eq!(d.false_alarm_rate, Some(0.0));
        assert_eq!(d.positive_likelihood_ratio, None, "a ratio over no false alarm is not a number");

        let mut unsampled = Case::new(&marked_spread(10, 5), &[]);
        unsampled.rest_population = 30;
        let d = unsampled.score().discrimination;
        assert_eq!((d.estimated_false_negatives, d.sensitivity, d.positive_likelihood_ratio), (None, None, None));
    }

    #[test]
    fn the_warning_stays_when_the_lower_bound_of_the_precision_clears_the_minimum() {
        let scored = Case::new(&marked_spread(100, 90), &vec![false; 100]).score();

        assert_eq!(scored.decision, Decision::Keeps);
        assert!(scored.precision.lo > scored.p_min);
        assert_eq!(scored.by_distance.outcome, None, "the distance is only looked at before removing or adjusting");
    }

    #[test]
    fn the_warning_is_adjusted_or_removed_when_the_upper_bound_falls_below_the_minimum() {
        // 10 revisions among 100 marked, 40 among 100 of the rest: π = 0.25, p_min = 0.4.
        let scored = Case::new(&marked_spread(100, 10), &[vec![true; 40], vec![false; 60]].concat()).score();

        assert_eq!(scored.decision, Decision::AdjustOrRemove);
        assert!(scored.precision.hi < scored.p_min);
    }

    #[test]
    fn an_undecided_precision_waits_for_more_cases_until_n_max_and_then_keeps_the_warning() {
        // π = 1/3 and p_min = 1/2, with a precision of 1/2 whose interval holds it.
        let waiting = Case::new(&marked_spread(100, 50), &[false; 50]).score();
        assert!(waiting.precision.lo < waiting.p_min && waiting.p_min < waiting.precision.hi);
        assert_eq!(waiting.decision, Decision::GatherMore);

        let enough = Case::new(&marked_spread(N_MAX, 100), &[false; 100]).score();
        assert!(enough.precision.lo < enough.p_min && enough.p_min < enough.precision.hi);
        assert_eq!(enough.decision, Decision::KeepsByDefault);

        let just_short = Case::new(&marked_spread(N_MAX - 1, 100), &[false; 100]).score();
        assert!(just_short.precision.lo < just_short.p_min && just_short.p_min < just_short.precision.hi);
        assert_eq!(just_short.decision, Decision::GatherMore);

        let none = Case::new(&[], &[false; 10]).score();
        assert_eq!(none.decision, Decision::GatherMore, "nothing marked yet is nothing to decide");
        assert!(none.by_distance.bands.is_empty() && none.by_store_size.len() == 3);
    }

    #[test]
    fn the_decision_serializes_as_its_four_names() {
        let names: Vec<String> = [Decision::Keeps, Decision::AdjustOrRemove, Decision::KeepsByDefault, Decision::GatherMore]
            .iter()
            .map(|d| serde_json::to_value(d).unwrap().as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, ["keeps", "adjust_or_remove", "keeps_by_default", "gather_more"]);
    }

    #[test]
    fn tertiles_cut_at_the_rounded_up_thirds_and_a_tie_goes_with_the_band_before() {
        let bounds = |values: &[f64]| tertile_bounds(values);

        assert_eq!(bounds(&[]), [None, None]);
        assert_eq!(bounds(&[0.5]), [Some(0.5), Some(0.5)]);
        assert_eq!(bounds(&[0.3, 0.1, 0.2]), [Some(0.1), Some(0.2)]);
        assert_eq!(bounds(&[0.1, 0.2, 0.3, 0.4]), [Some(0.2), Some(0.3)], "ceil(4/3) = 2 and ceil(8/3) = 3");
        assert_eq!(bounds(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]), [Some(2.0), Some(4.0)]);
        assert_eq!(bounds(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]), [Some(3.0), Some(5.0)]);
        // The first cut falls inside a run of 0.3: the whole run goes with the first band, and the second band is empty.
        let tied = [0.1, 0.2, 0.3, 0.3, 0.3, 0.4, 0.5];
        assert_eq!(bounds(&tied), [Some(0.3), Some(0.3)]);
        assert_eq!([0.1, 0.3, 0.4].map(|v| band_of(&bounds(&tied), v)), [0, 0, 2]);
        assert_eq!(band_of(&[None, None], 0.2), 2);
    }

    #[test]
    fn each_distance_band_reports_its_range_its_precision_and_whether_it_clears_the_minimum() {
        // Nine marked pairs: the closest third are all revisions, the rest none, over a rest of 91 saves with none
        // (π = 3/100, p_min ≈ 0.058).
        let marked: Vec<(f64, usize, bool)> = (0..9).map(|i| (0.10 + i as f64 / 100.0, 10, i < 3)).collect();
        let scored = Case::new(&marked, &[false; 91]).score();

        let bands = &scored.by_distance.bands;
        assert_eq!(bands.len(), 3);
        assert!((bands[0].min.unwrap() - 0.10).abs() < 1e-9 && (bands[0].max.unwrap() - 0.12).abs() < 1e-9);
        assert!((bands[2].min.unwrap() - 0.16).abs() < 1e-9 && (bands[2].max.unwrap() - 0.18).abs() < 1e-9);
        assert_eq!((bands[0].precision.k, bands[0].precision.n), (3, 3));
        assert_eq!((bands[1].precision.k, bands[1].precision.n), (0, 3));
        let wilson_all = crate::stats::Rate::of(3, 3);
        assert!(wilson_all.lo > scored.p_min && bands[0].hits && !bands[1].hits && !bands[2].hits);
    }

    /// 90 marked pairs, 25 of them revisions, over a rest of 60 saves with 25:
    /// π = 1/3, p_min = 1/2, and the 25 over 90 stays below it.
    fn adjusting(revisions_in: impl Fn(usize) -> bool) -> Scored {
        let marked: Vec<(f64, usize, bool)> = (0..90).map(|i| (0.10 + i as f64 / 1000.0, 10, revisions_in(i))).collect();
        Case::new(&marked, &[vec![true; 25], vec![false; 35]].concat()).score()
    }

    #[test]
    fn a_rule_that_only_works_up_close_is_narrowed_to_the_range_of_the_closest_third() {
        let scored = adjusting(|i| i < 25);

        assert_eq!(scored.decision, Decision::AdjustOrRemove);
        let closest = &scored.by_distance.bands[0];
        assert!(closest.hits);
        match scored.by_distance.outcome {
            Some(DistanceOutcome::NarrowTo { max_distance }) => assert!((max_distance - closest.max.unwrap()).abs() < 1e-12),
            other => panic!("expected a narrowing, got {other:?}"),
        }
    }

    #[test]
    fn a_rule_the_closest_third_does_not_save_is_removed() {
        let scored = adjusting(|i| i >= 65);

        assert_eq!(scored.decision, Decision::AdjustOrRemove);
        assert!(!scored.by_distance.bands[0].hits);
        assert_eq!(scored.by_distance.outcome, Some(DistanceOutcome::Remove));
        let json = serde_json::to_value(scored.by_distance.outcome).unwrap();
        assert_eq!(json, json!({"action": "remove"}));
    }

    #[test]
    fn a_marked_pair_without_a_distance_in_the_replay_file_is_refused() {
        let mut file = Case::new(&marked_spread(3, 1), &[false]).file();
        file.pairs[0].distance = None;
        let lines = Case::new(&marked_spread(3, 1), &[false]).lines();

        let error = score(&file, &lines).err().expect("refused");

        assert!(matches!(error, WarnedError::NoDistance(1)), "{error}");
    }

    #[test]
    fn the_store_size_bands_give_the_fire_rate_and_the_precision_of_each_third() {
        // 30 saves in the window with sizes 1 to 30; the marked ones sit at 2, 3, 12, 25 and 26.
        let mut case = Case::new(&[(0.1, 2, true), (0.2, 3, true), (0.3, 12, false), (0.4, 25, false), (0.5, 26, true)], &[false; 10]);
        case.store_sizes = (1..=30).collect();

        let bands = case.score().by_store_size;

        assert_eq!(bands.len(), 3);
        let shape: Vec<_> = bands.iter().map(|b| (b.min, b.max, b.window_saves, b.fire_rate.k, b.precision.k, b.precision.n)).collect();
        assert_eq!(shape, [(Some(1), Some(10), 10, 2, 2, 2), (Some(11), Some(20), 10, 1, 0, 1), (Some(21), Some(30), 10, 2, 1, 2)]);
        assert!((bands[0].fire_rate.rate - 0.2).abs() < 1e-12);
    }

    #[test]
    fn the_token_and_prevalence_gates_are_reported_apart_from_the_decision() {
        // 100 marked among 1000 saves of the window, a line of 30 tokens: 3 tokens per save.
        let scored = Case::new(&marked_spread(100, 50), &[false; 100]).score();

        let g = &scored.gates;
        assert_eq!((g.line_tokens.value, g.line_tokens.limit, g.line_tokens.met), (30.0, 40.0, true));
        assert!((scored.warning.mean_tokens_per_save - 3.0).abs() < 1e-12 && g.mean_tokens_per_save.met);
        assert_eq!((scored.warning.fire_rate.k, scored.warning.fire_rate.n), (100, 1000));
        assert!((g.revision_share.value - scored.prevalence.estimate).abs() < 1e-12 && g.revision_share.met);
        assert_eq!((g.revision_share.limit, g.revision_share_lower_bound.limit), (0.10, 0.05));
        assert!(g.met);

        // Two saves in three warned: 20 tokens per save, over the 15 allowed.
        let mut costly = Case::new(&marked_spread(100, 50), &[false; 100]);
        costly.store_sizes = (0..150).collect();
        let costly = costly.score();
        assert!((costly.warning.mean_tokens_per_save - 20.0).abs() < 1e-9);
        assert!(!costly.gates.mean_tokens_per_save.met && !costly.gates.met);
        // One revision in 500 is under the 10 % of revisions the warning must find.
        let rare = Case::new(&marked_spread(100, 1), &[false; 400]).score();
        assert!(!rare.gates.revision_share.met && !rare.gates.met, "{:?}", rare.gates);
    }

    #[test]
    fn what_the_agent_did_is_counted_apart_for_the_hits_and_the_false_alarms() {
        let mut case = Case::new(&[(0.1, 10, true), (0.2, 10, true), (0.3, 10, true), (0.4, 10, false), (0.5, 10, false)], &[false]);
        case.actions = vec![
            Actions { followed: true, hid: true, reused: true },
            Actions { followed: true, hid: false, reused: false },
            Actions { followed: false, hid: true, reused: false },
            Actions { followed: true, hid: false, reused: true },
            Actions { followed: false, hid: false, reused: false },
        ];

        let actions = case.score().agent_actions;

        let hits = &actions.hits;
        assert_eq!((hits.marked, hits.followed.k, hits.followed.n, hits.hid.k, hits.hid.n), (3, 2, 3, 2, 3));
        assert_eq!((hits.reused.k, hits.reused.n), (1, 2), "a reused key is counted over the warnings that were followed");
        let alarms = &actions.false_alarms;
        assert_eq!((alarms.marked, alarms.followed.k, alarms.followed.n, alarms.hid.k), (2, 1, 2, 0));
        assert_eq!((alarms.reused.k, alarms.reused.n), (1, 1));
    }
}
