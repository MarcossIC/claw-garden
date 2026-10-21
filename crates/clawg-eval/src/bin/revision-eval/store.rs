//! The pure half of `store`: the saves of an export, what a replay of them
//! through the detector means, which pairs the owner labels and how the
//! labels score.
//!
//! Nothing here touches a file, a store or a model. A save is the text a
//! `save_memory` call carried, rebuilt from the rows and pieces of an export,
//! so everything that carries stored text stays inside the values this module
//! hands to `run.rs`; the result it builds holds counts, rates and bounds only.

use std::collections::{BTreeMap, HashMap, HashSet};

use clawg_core::eval_api::{is_claim_type, join_pieces, EvalSaved};
use clawg_core::memory::portability::format::{self, FormatError, MemoryLine, Parsed};
use serde::{Deserialize, Serialize};

use crate::stats::{Rate, SplitMix64};

/// The seed of the sample when `--seed` is not given.
pub const DEFAULT_SEED: u64 = 0x5EED_0005;

/// The most saves labelled among those the warning would not fire on.
pub const SAMPLE_OF_REST: usize = 50;

/// The two values a label takes.
const REVISION: &str = "revision";
const OTHER: &str = "other";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("the export cannot be read: {0}")]
    Format(#[from] FormatError),
    #[error("label {0} is empty: only the owner's label scores")]
    EmptyLabel(usize),
    #[error("pair {n} holds {value:?}; a label is `revision` or `other`")]
    BadLabel { n: usize, value: String },
    #[error("line {line} of the labels file cannot be read: {reason}")]
    Labels { line: usize, reason: String },
    #[error("the labels file does not match the replay: {0}")]
    Mismatch(String),
    #[error("the replay file cannot be read: {0}")]
    Replay(String),
}

impl StoreError {
    /// Whether the working directory is worth keeping: a labels file the owner
    /// has not finished is fixed by editing it, and the hours spent labelling
    /// are in it.
    pub fn keeps_work(&self) -> bool {
        matches!(self, Self::EmptyLabel(_) | Self::BadLabel { .. } | Self::Labels { .. } | Self::Mismatch(_))
    }
}

/// One save of the export, as the agent made it.
#[derive(Debug, Clone, PartialEq)]
pub struct Save {
    pub agent: String,
    pub project: Option<String>,
    pub memory_type: String,
    pub importance: f64,
    pub topic_key: Option<String>,
    pub text: String,
    /// Deleted in the export, or replaced by a verdict: a replay cannot do
    /// either, so a candidate that is one is counted apart.
    pub gone: bool,
}

/// What an export says without a replay.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExportCounts {
    pub rows: usize,
    pub saves: usize,
    /// Saves of rows no agent saved (`global` and `project` rows): not replayed.
    pub without_agent: usize,
    pub keyed_saves: usize,
    /// Saves of a type that asserts something, from the first keyed save on.
    pub claim_saves_since_first_key: usize,
    pub superseded_by_revision: usize,
    pub superseded_by_verdict: usize,
    pub judged_supersedes_pairs: usize,
    pub invalid_lines: usize,
}

pub struct Export {
    pub format: String,
    pub version: u32,
    /// The saves with an agent, in the order they were made.
    pub flow: Vec<Save>,
    pub counts: ExportCounts,
}

/// Reads `text` as an export. A format or version this binary does not know
/// fails before anything else is looked at.
pub fn read_export(text: &str) -> Result<Export, StoreError> {
    let (header, parsed) = format::parse(text)?;
    let mut heads = saves_of(&parsed);
    heads.sort_by_key(|(head, _)| (head.created_at, head.id));

    let first_key_at = heads.iter().filter(|(head, _)| head.topic_key.is_some()).map(|(head, _)| head.created_at).min();
    let mut counts = ExportCounts {
        rows: parsed.memories.len(),
        saves: heads.len(),
        invalid_lines: parsed.invalid.len(),
        judged_supersedes_pairs: parsed
            .conflicts
            .iter()
            .filter(|(_, c)| c.state == "judged" && c.verdict.as_deref() == Some("supersedes"))
            .count(),
        ..ExportCounts::default()
    };
    for (_, row) in &parsed.memories {
        if row.status == "superseded" {
            match row.supersede_cause.as_deref() {
                Some("revision") => counts.superseded_by_revision += 1,
                Some("verdict") => counts.superseded_by_verdict += 1,
                _ => {}
            }
        }
    }
    let mut flow = Vec::new();
    for (head, text) in heads {
        if head.topic_key.is_some() {
            counts.keyed_saves += 1;
        }
        if is_claim_type(&head.memory_type) && first_key_at.is_some_and(|at| head.created_at >= at) {
            counts.claim_saves_since_first_key += 1;
        }
        let Some(agent) = head.agent_id.clone() else {
            counts.without_agent += 1;
            continue;
        };
        flow.push(Save {
            agent,
            project: head.project_id.clone(),
            memory_type: head.memory_type.clone(),
            importance: head.importance,
            topic_key: head.topic_key.clone(),
            text,
            gone: head.deleted_at.is_some()
                || (head.status == "superseded" && head.supersede_cause.as_deref() == Some("verdict")),
        });
    }
    Ok(Export { format: header.format, version: header.version, flow, counts })
}

/// Each save's first row and its text. A save of several pieces is joined as
/// `read_memory` joins it; a piece the export does not hold cuts the text in
/// two runs, joined by a line break, as `read_save_core` does for a piece that
/// is gone.
fn saves_of(parsed: &Parsed) -> Vec<(&MemoryLine, String)> {
    let rows: HashMap<i64, &MemoryLine> = parsed.memories.iter().map(|(_, m)| (m.id, m)).collect();
    let mut groups: BTreeMap<i64, BTreeMap<i64, i64>> = BTreeMap::new();
    for (_, piece) in &parsed.pieces {
        groups.entry(piece.save).or_default().insert(piece.piece, piece.memory);
    }
    let in_a_save: HashSet<i64> = groups.values().flat_map(|g| g.values().copied()).collect();

    let mut saves = Vec::new();
    for pieces in groups.values() {
        let mut head: Option<&MemoryLine> = None;
        let mut previous: Option<i64> = None;
        let mut runs: Vec<Vec<&str>> = vec![Vec::new()];
        for (&position, memory) in pieces {
            let Some(row) = rows.get(memory) else { continue };
            if previous.is_some_and(|p| position != p + 1) {
                runs.push(Vec::new());
            }
            previous = Some(position);
            head.get_or_insert(row);
            runs.last_mut().expect("runs starts non-empty").push(&row.content);
        }
        if let Some(head) = head {
            let text = runs.iter().filter(|r| !r.is_empty()).map(|r| join_pieces(r)).collect::<Vec<_>>().join("\n");
            saves.push((head, text));
        }
    }
    for (_, row) in &parsed.memories {
        if !in_a_save.contains(&row.id) {
            saves.push((row, row.content.clone()));
        }
    }
    saves
}

/// What a save of the flow turned out to be once replayed.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Status {
    Keyed,
    /// Every row was already stored: it said again what a row said.
    Reinforced,
    /// The warning fires; `earlier` is the save of its closest candidate.
    Fires { earlier: usize },
    /// No candidate, but an earlier live save of the same agent and project.
    Pairable,
    Alone,
}

/// What replaying the flow counted.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReplayCounts {
    pub sent: usize,
    pub keyed: usize,
    pub unkeyed: usize,
    pub unkeyed_claim: usize,
    pub reinforced: usize,
    /// Unkeyed saves that can be labelled: those that fire and those with an
    /// earlier live save of their agent and project.
    pub population: usize,
    pub fires: usize,
    /// Firings whose candidate was deleted, or replaced by a verdict, in the
    /// real store: a bound on the bias of not replaying either.
    pub fires_with_gone_candidate: usize,
    /// Claim saves that fire on no row of their own project but have a
    /// candidate of another.
    pub other_project_candidate: usize,
    pub rest: usize,
}

pub struct Analysis {
    pub counts: ReplayCounts,
    status: Vec<Status>,
    /// The rows each save inserted, by its place in the flow.
    owned: Vec<Vec<i64>>,
    owner: HashMap<i64, usize>,
    /// The save that replaced a row by its key.
    replaced_at: HashMap<i64, usize>,
}

fn same_scope(a: &Save, b: &Save) -> bool {
    a.agent == b.agent && a.project == b.project
}

/// Reads what each save of `flow` left (`reports`, aligned with it): which
/// fire, which only repeat a row, which can be paired with an earlier save.
pub fn analyse(flow: &[Save], reports: &[EvalSaved]) -> Analysis {
    assert_eq!(flow.len(), reports.len(), "a report for every save");
    let mut owned = vec![Vec::new(); flow.len()];
    let mut owner = HashMap::new();
    let mut replaced_at = HashMap::new();
    for (i, report) in reports.iter().enumerate() {
        for &(id, inserted) in &report.rows {
            if inserted {
                owner.insert(id, i);
                owned[i].push(id);
            }
        }
        for &id in &report.replaced {
            replaced_at.insert(id, i);
        }
    }
    let mut analysis = Analysis { counts: ReplayCounts::default(), status: Vec::new(), owned, owner, replaced_at };
    let mut counts = ReplayCounts { sent: flow.len(), ..Default::default() };
    for (i, (save, report)) in flow.iter().zip(reports).enumerate() {
        let status = if save.topic_key.is_some() {
            counts.keyed += 1;
            Status::Keyed
        } else {
            counts.unkeyed += 1;
            let claim = is_claim_type(&save.memory_type);
            if claim {
                counts.unkeyed_claim += 1;
            }
            if report.rows.iter().all(|&(_, inserted)| !inserted) {
                counts.reinforced += 1;
                Status::Reinforced
            } else {
                let mut closest: Option<(usize, f64)> = None;
                let mut elsewhere = false;
                for candidate in &report.related {
                    let Some(&j) = analysis.owner.get(&candidate.id).filter(|&&j| j < i) else { continue };
                    if !same_scope(&flow[j], save) {
                        elsewhere = true;
                    } else if closest.map_or(true, |(_, d)| candidate.distance < d) {
                        closest = Some((j, candidate.distance));
                    }
                }
                match closest.filter(|_| claim) {
                    Some((earlier, _)) => {
                        counts.fires += 1;
                        if flow[earlier].gone {
                            counts.fires_with_gone_candidate += 1;
                        }
                        Status::Fires { earlier }
                    }
                    None => {
                        if claim && elsewhere {
                            counts.other_project_candidate += 1;
                        }
                        if analysis.has_live_predecessor(flow, i) {
                            Status::Pairable
                        } else {
                            Status::Alone
                        }
                    }
                }
            }
        };
        analysis.status.push(status);
        if matches!(status, Status::Pairable) {
            counts.rest += 1;
        }
    }
    counts.population = counts.fires + counts.rest;
    analysis.counts = counts;
    analysis
}

impl Analysis {
    /// Whether the save `j` still held a live row when the save `i` came: a
    /// row is gone from the moment a keyed save replaces it.
    fn live_before(&self, j: usize, i: usize) -> bool {
        self.owned[j].iter().any(|row| self.replaced_at.get(row).map_or(true, |&by| by >= i))
    }

    fn has_live_predecessor(&self, flow: &[Save], i: usize) -> bool {
        (0..i).rev().any(|j| same_scope(&flow[j], &flow[i]) && self.live_before(j, i))
    }

    /// The earlier saves of the same agent and project that held a live row
    /// when the save `i` came: the only ones it could revise.
    pub fn predecessors(&self, flow: &[Save], i: usize) -> Vec<usize> {
        (0..i).filter(|&j| same_scope(&flow[j], &flow[i]) && self.live_before(j, i)).collect()
    }

    /// The save that inserted `row`.
    pub fn owner_of(&self, row: i64) -> Option<usize> {
        self.owner.get(&row).copied()
    }

    /// The rows the save `i` inserted.
    pub fn rows_of(&self, i: usize) -> &[i64] {
        &self.owned[i]
    }
}

/// The earlier save at the least distance; the first of equals.
pub fn closest(candidates: &[(usize, f64)]) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for &(save, distance) in candidates {
        if best.map_or(true, |(_, d)| distance < d) {
            best = Some((save, distance));
        }
    }
    best.map(|(save, _)| save)
}

/// A save picked for the owner to label.
#[derive(Debug, Clone, PartialEq)]
pub struct Selected {
    pub save: usize,
    pub fired: bool,
    /// The save it is compared with: the closest candidate of a firing, and,
    /// for the others, what `run.rs` finds by distance.
    pub earlier: Option<usize>,
}

/// Every save the warning fires on, and a sample of at most [`SAMPLE_OF_REST`]
/// of the others, in an order that tells nothing about which is which. The
/// same seed gives the same choice.
pub fn select(analysis: &Analysis, seed: u64) -> Vec<Selected> {
    let mut rng = SplitMix64::new(seed);
    let mut rest: Vec<usize> =
        analysis.status.iter().enumerate().filter(|(_, s)| **s == Status::Pairable).map(|(i, _)| i).collect();
    rng.shuffle(&mut rest);
    rest.truncate(SAMPLE_OF_REST);
    let mut chosen: Vec<Selected> = analysis
        .status
        .iter()
        .enumerate()
        .filter_map(|(save, status)| match status {
            Status::Fires { earlier } => Some(Selected { save, fired: true, earlier: Some(*earlier) }),
            _ => None,
        })
        .chain(rest.into_iter().map(|save| Selected { save, fired: false, earlier: None }))
        .collect();
    rng.shuffle(&mut chosen);
    chosen
}

/// One line of `labels.jsonl`. Nothing in it says whether the warning fired.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelLine {
    pub n: usize,
    pub earlier: String,
    pub later: String,
    /// What an agent proposes: `revision` or `other`.
    pub proposed: Option<String>,
    /// What the owner decides, and the only value that scores.
    pub label: Option<String>,
}

/// What `replay.json` keeps of a labelled pair: no text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Labelled {
    pub n: usize,
    pub fired: bool,
    pub memory_type: String,
}

pub struct Labels {
    pub lines: Vec<LabelLine>,
    pub meta: Vec<Labelled>,
}

/// The pairs to label, numbered in the order given. A pick with no earlier
/// save to compare with is left out.
pub fn build_labels(flow: &[Save], selected: &[Selected]) -> Labels {
    let mut lines = Vec::new();
    let mut meta = Vec::new();
    for pick in selected {
        let Some(earlier) = pick.earlier else { continue };
        let n = lines.len() + 1;
        lines.push(LabelLine {
            n,
            earlier: flow[earlier].text.clone(),
            later: flow[pick.save].text.clone(),
            proposed: None,
            label: None,
        });
        meta.push(Labelled { n, fired: pick.fired, memory_type: flow[pick.save].memory_type.clone() });
    }
    Labels { lines, meta }
}

/// Everything the first step leaves for the second, apart from the labels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayFile {
    pub export_format: String,
    pub export_version: u32,
    pub model_contract: String,
    pub seed: u64,
    /// A run on the double: its result is printed, never written.
    pub fake: bool,
    pub export: ExportCounts,
    pub replay: ReplayCounts,
    pub warning_line_tokens: usize,
    pub labelled: Vec<Labelled>,
}

pub fn parse_replay(text: &str) -> Result<ReplayFile, StoreError> {
    serde_json::from_str(text).map_err(|e| StoreError::Replay(e.to_string()))
}

pub fn parse_labels(text: &str) -> Result<Vec<LabelLine>, StoreError> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(i, line)| {
            serde_json::from_str(line).map_err(|e| StoreError::Labels { line: i + 1, reason: e.to_string() })
        })
        .collect()
}

/// How many of the unkeyed saves are revisions.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Prevalence {
    pub population: usize,
    pub revisions_among_firing: usize,
    pub rest: usize,
    pub revisions_in_sample: usize,
    pub sample: usize,
    /// Every one of the rest was labelled, so the bound is Wilson of `k/N`.
    pub census: bool,
    pub estimate: f64,
    pub lower_bound: f64,
}

fn prevalence(
    population: usize,
    firing: usize,
    revisions_among_firing: usize,
    sample: usize,
    revisions_in_sample: usize,
) -> Prevalence {
    let rest = population.saturating_sub(firing);
    let census = sample >= rest;
    let (estimate, lower_bound) = if population == 0 {
        (0.0, 0.0)
    } else if census {
        let all = Rate::of(revisions_among_firing + revisions_in_sample, population);
        (all.rate, all.lo)
    } else {
        // The ones that fire are a census; only the rest is a sample.
        let sampled = Rate::of(revisions_in_sample, sample);
        let n = population as f64;
        (
            (revisions_among_firing as f64 + rest as f64 * sampled.rate) / n,
            (revisions_among_firing as f64 + rest as f64 * sampled.lo) / n,
        )
    };
    Prevalence { population, revisions_among_firing, rest, revisions_in_sample, sample, census, estimate, lower_bound }
}

/// A measured value against the limit it must keep.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Gate {
    pub value: f64,
    pub limit: f64,
    pub met: bool,
}

impl Gate {
    fn at_least(value: f64, limit: f64) -> Self {
        Self { value, limit, met: value >= limit }
    }

    fn at_most(value: f64, limit: f64) -> Self {
        Self { value, limit, met: value <= limit }
    }
}

/// The row of the threshold table for the warning, each condition with its
/// value.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Thresholds {
    pub revision_share: Gate,
    pub revision_share_lower_bound: Gate,
    pub firing_precision: Gate,
    pub max_tokens_where_it_appears: Gate,
    pub mean_tokens_per_save: Gate,
    pub met: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WarningCost {
    pub line_tokens: usize,
    /// Saves of the flow it fires on.
    pub fire_rate: Rate,
    pub mean_tokens_per_save: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LabelledCounts {
    pub firing: usize,
    pub sample: usize,
}

/// What the second step reports. It holds counts, rates and bounds: no text,
/// no row id and no key.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Scored {
    pub export_format: String,
    pub export_version: u32,
    pub model_contract: String,
    pub seed: u64,
    #[serde(skip)]
    pub fake: bool,
    pub export: ExportCounts,
    pub replay: ReplayCounts,
    pub labelled: LabelledCounts,
    pub revisions_among_firing: Rate,
    pub prevalence: Prevalence,
    /// How often the agent's `proposed` equals the owner's `label`, over the
    /// pairs that carry a proposal.
    pub agreement: Rate,
    pub warning: WarningCost,
    pub thresholds: Thresholds,
}

fn valid(value: &str) -> bool {
    value == REVISION || value == OTHER
}

/// Scores the owner's labels. Only `label` counts; an empty one refuses the
/// whole file.
pub fn score(replay: &ReplayFile, labels: &[LabelLine]) -> Result<Scored, StoreError> {
    if labels.len() != replay.labelled.len() {
        return Err(StoreError::Mismatch(format!(
            "{} pairs in the file, {} in the replay",
            labels.len(),
            replay.labelled.len()
        )));
    }
    let (mut firing, mut firing_revisions, mut sample, mut sample_revisions) = (0, 0, 0, 0);
    let (mut proposed_filled, mut agreed) = (0, 0);
    for (line, meta) in labels.iter().zip(&replay.labelled) {
        if line.n != meta.n {
            return Err(StoreError::Mismatch(format!("pair {} stands where the replay has {}", line.n, meta.n)));
        }
        let label = match line.label.as_deref() {
            None | Some("") => return Err(StoreError::EmptyLabel(line.n)),
            Some(label) if valid(label) => label,
            Some(other) => return Err(StoreError::BadLabel { n: line.n, value: other.to_string() }),
        };
        match line.proposed.as_deref() {
            None | Some("") => {}
            Some(proposed) if valid(proposed) => {
                proposed_filled += 1;
                agreed += usize::from(proposed == label);
            }
            Some(other) => return Err(StoreError::BadLabel { n: line.n, value: other.to_string() }),
        }
        let revision = label == REVISION;
        if meta.fired {
            firing += 1;
            firing_revisions += usize::from(revision);
        } else {
            sample += 1;
            sample_revisions += usize::from(revision);
        }
    }

    let counts = &replay.replay;
    let revisions_among_firing = Rate::of(firing_revisions, firing);
    let prevalence = prevalence(counts.population, firing, firing_revisions, sample, sample_revisions);
    let fire_rate = Rate::of(counts.fires, counts.sent);
    let line_tokens = replay.warning_line_tokens;
    let warning = WarningCost { line_tokens, mean_tokens_per_save: line_tokens as f64 * fire_rate.rate, fire_rate };
    let mut thresholds = Thresholds {
        revision_share: Gate::at_least(prevalence.estimate, 0.10),
        revision_share_lower_bound: Gate::at_least(prevalence.lower_bound, 0.05),
        firing_precision: Gate::at_least(revisions_among_firing.rate, 0.40),
        max_tokens_where_it_appears: Gate::at_most(line_tokens as f64, 40.0),
        mean_tokens_per_save: Gate::at_most(warning.mean_tokens_per_save, 15.0),
        met: false,
    };
    thresholds.met = [
        &thresholds.revision_share,
        &thresholds.revision_share_lower_bound,
        &thresholds.firing_precision,
        &thresholds.max_tokens_where_it_appears,
        &thresholds.mean_tokens_per_save,
    ]
    .iter()
    .all(|gate| gate.met);
    Ok(Scored {
        export_format: replay.export_format.clone(),
        export_version: replay.export_version,
        model_contract: replay.model_contract.clone(),
        seed: replay.seed,
        fake: replay.fake,
        export: replay.export.clone(),
        replay: replay.replay.clone(),
        labelled: LabelledCounts { firing, sample },
        revisions_among_firing,
        prevalence,
        agreement: Rate::of(agreed, proposed_filled),
        warning,
        thresholds,
    })
}

#[cfg(test)]
pub(crate) mod fixtures {
    use std::path::PathBuf;

    use serde_json::json;

    /// A row of an export; the rest of its columns are the ones a plain save has.
    pub struct Row {
        pub id: i64,
        pub agent: Option<&'static str>,
        pub project: Option<&'static str>,
        pub memory_type: &'static str,
        pub content: String,
        pub topic_key: Option<&'static str>,
        pub status: &'static str,
        pub cause: Option<&'static str>,
        pub deleted: bool,
    }

    impl Row {
        pub fn new(id: i64, agent: &'static str, project: &'static str, content: &str) -> Self {
            Self {
                id,
                agent: Some(agent),
                project: Some(project),
                memory_type: "semantic_fact",
                content: content.to_string(),
                topic_key: None,
                status: "active",
                cause: None,
                deleted: false,
            }
        }

        pub fn line(&self) -> String {
            json!({
                "kind": "memory", "id": self.id,
                "scope": if self.agent.is_some() { "agent" } else { "global" },
                "agent_id": self.agent, "project_id": self.project,
                "content": self.content, "memory_type": self.memory_type, "source": "explicit",
                "created_at": 1_750_000_000_000_i64 + self.id * 1000, "created_by_type": "agent",
                "created_by_ref": "test", "last_modified_at": 0, "last_accessed_at": 0,
                "importance": 5.0, "tier": 1, "status": self.status, "supersede_cause": self.cause,
                "deleted_at": if self.deleted { json!(1_760_000_000_000_i64) } else { json!(null) },
                "topic_key": self.topic_key,
            })
            .to_string()
        }
    }

    pub fn piece(save: i64, piece: i64, memory: i64) -> String {
        json!({"kind": "piece", "save": save, "piece": piece, "memory": memory, "context": ""}).to_string()
    }

    pub fn header(version: u32) -> String {
        json!({"kind": "header", "format": "clawg-memories", "version": version, "exported_at": 0,
               "clawg": "0.0.0", "embedding_model": "test", "filter": {"include_deleted": true}})
        .to_string()
    }

    /// A fresh working directory, as `clawg export --out` would be told to
    /// make one, holding an export of these rows.
    pub fn work_with(rows: &[Row]) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("revision-eval-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("export.jsonl");
        std::fs::write(&path, export(&rows.iter().map(Row::line).collect::<Vec<_>>())).unwrap();
        (dir, path)
    }

    pub fn small_export() -> Vec<Row> {
        let mut keyed = Row::new(5, "claude-code", "repo", "Cache layer: a Redis cluster for sessions.");
        keyed.topic_key = Some("cache/layer");
        let mut nobody = Row::new(6, "claude-code", "repo", "A shared note no agent saved.");
        (nobody.agent, nobody.project) = (None, None);
        vec![
            Row::new(1, "claude-code", "repo", "Deploys go out on Tuesdays after the freeze."),
            Row::new(2, "claude-code", "repo", "The nightly backup keeps the last seven snapshots."),
            Row::new(3, "claude-code", "repo", "Logs rotate daily and are shipped to cold storage."),
            Row::new(4, "claude-code", "repo", "The nightly backup keeps the last seven snapshots."),
            keyed,
            nobody,
        ]
    }

    /// The file an export writes: its header and then `lines`.
    pub fn export(lines: &[String]) -> String {
        let mut text = header(1);
        for line in lines {
            text.push('\n');
            text.push_str(line);
        }
        text.push('\n');
        text
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use clawg_core::eval_api::EvalCandidate;

    fn save(agent: &str, project: &str, memory_type: &str, text: &str) -> Save {
        Save {
            agent: agent.into(),
            project: Some(project.into()),
            memory_type: memory_type.into(),
            importance: 5.0,
            topic_key: None,
            text: text.into(),
            gone: false,
        }
    }

    /// What a save that inserted `rows` and was paired with `related` reports.
    fn report(rows: &[i64], related: &[i64], replaced: &[i64]) -> EvalSaved {
        EvalSaved {
            rows: rows.iter().map(|&id| (id, true)).collect(),
            related: related.iter().map(|&id| EvalCandidate { id, distance: 0.3, escalated: false }).collect(),
            replaced: replaced.to_vec(),
        }
    }

    #[test]
    fn the_counts_that_need_no_replay_come_from_the_parsed_export() {
        let keyed = |id, text: &str| {
            let mut row = Row::new(id, "claude-code", "repo", text);
            row.memory_type = "architectural_decision";
            row.topic_key = Some("storage/engine");
            row
        };
        let mut rows = vec![Row::new(1, "claude-code", "repo", "Before any key.")];
        let (mut a, mut b, c) = (keyed(2, "Engine: A."), keyed(3, "Engine: B."), keyed(4, "Engine: C."));
        (a.status, a.cause) = ("superseded", Some("revision"));
        (b.status, b.cause) = ("superseded", Some("revision"));
        rows.extend([a, b, c]);
        let mut by_verdict = Row::new(5, "claude-code", "repo", "Replaced by a verdict.");
        (by_verdict.status, by_verdict.cause) = ("superseded", Some("verdict"));
        rows.push(by_verdict);
        let mut shared = Row::new(6, "claude-code", "repo", "Said by nobody in particular.");
        (shared.agent, shared.project) = (None, None);
        rows.push(shared);
        let mut lines: Vec<String> = rows.iter().map(Row::line).collect();
        lines.push(r#"{"kind":"conflict","new_id":4,"old_id":3,"distance":0.2,"state":"judged","verdict":"supersedes","created_at":1}"#.into());
        lines.push(r#"{"kind":"conflict","new_id":5,"old_id":1,"distance":0.2,"state":"judged","verdict":"compatible","created_at":1}"#.into());

        let export = read_export(&export(&lines)).unwrap();

        let c = &export.counts;
        assert_eq!((c.rows, c.saves, c.without_agent), (6, 6, 1));
        assert_eq!(c.keyed_saves, 3);
        assert_eq!(c.claim_saves_since_first_key, 5, "from row 2 on: its three saves and the facts 5 and 6");
        assert_eq!((c.superseded_by_revision, c.superseded_by_verdict, c.judged_supersedes_pairs), (2, 1, 1));
        assert_eq!(export.flow.len(), 5, "the row no agent saved is not replayed");
        assert_eq!((export.format.as_str(), export.version), ("clawg-memories", 1));
    }

    #[test]
    fn the_pieces_of_a_save_are_joined_and_a_missing_one_cuts_the_text() {
        let lines = [
            Row::new(1, "a", "p", "first piece").line(),
            Row::new(2, "a", "p", "second piece").line(),
            Row::new(4, "a", "p", "fourth piece").line(),
            Row::new(9, "a", "p", "alone").line(),
            piece(100, 1, 1),
            piece(100, 2, 2),
            piece(100, 4, 4),
            piece(100, 3, 3),
        ];

        let export = read_export(&export(&lines)).unwrap();

        let texts: Vec<&str> = export.flow.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts.len(), 2, "{texts:?}");
        assert!(texts[0].starts_with("first piece") && texts[0].contains("second piece"), "{texts:?}");
        assert!(texts[0].ends_with("\nfourth piece"), "the missing third piece cuts the text: {texts:?}");
        assert_eq!(texts[1], "alone");
    }

    #[test]
    fn an_unknown_export_format_writes_nothing() {
        let text = export(&[Row::new(1, "a", "p", "text").line()]).replacen("\"version\":1", "\"version\":2", 1);
        assert!(matches!(read_export(&text), Err(StoreError::Format(FormatError::UnknownVersion(2)))));
        let foreign = export(&[]).replacen("clawg-memories", "engram", 1);
        assert!(matches!(read_export(&foreign), Err(StoreError::Format(FormatError::OtherFormat(_)))));
    }

    #[test]
    fn the_nearest_earlier_save_is_looked_up_within_agent_and_project() {
        let flow = [
            save("a", "x", "semantic_fact", "0: a in x"),
            save("b", "x", "semantic_fact", "1: b in x"),
            save("a", "y", "semantic_fact", "2: a in y"),
            save("a", "x", "semantic_fact", "3: a in x"),
        ];
        let reports: Vec<EvalSaved> = (0..4).map(|i| report(&[i + 10], &[], &[])).collect();

        let analysis = analyse(&flow, &reports);

        assert_eq!(analysis.predecessors(&flow, 3), vec![0]);
        assert_eq!(analysis.predecessors(&flow, 1), Vec::<usize>::new());
        assert_eq!(closest(&[(5, 0.4), (2, 0.1), (7, 0.1)]), Some(2));
        assert_eq!(closest(&[]), None);
    }

    #[test]
    fn a_save_a_key_replaced_is_no_longer_an_earlier_save() {
        let mut keyed = save("a", "x", "semantic_fact", "1: keyed");
        keyed.topic_key = Some("k".into());
        let flow = [save("a", "x", "semantic_fact", "0: old"), keyed, save("a", "x", "semantic_fact", "2: later")];
        let reports = [report(&[10], &[], &[]), report(&[11], &[], &[10]), report(&[12], &[], &[])];

        let analysis = analyse(&flow, &reports);

        assert_eq!(analysis.predecessors(&flow, 2), vec![1]);
        assert_eq!(analysis.owner_of(11), Some(1));
        assert_eq!(analysis.rows_of(1), &[11]);
    }

    #[test]
    fn the_warning_fires_on_an_unkeyed_claim_with_a_candidate_of_its_own_scope() {
        let mut gone = save("a", "x", "architectural_decision", "0: gone");
        gone.gone = true;
        let flow = [
            gone,
            save("a", "x", "architectural_decision", "1: revises 0"),
            save("a", "x", "session_summary", "2: asserts nothing"),
            save("a", "x", "architectural_decision", "3: same words"),
        ];
        let mut again = report(&[], &[], &[]);
        again.rows = vec![(11, false)];
        let reports = [report(&[10], &[], &[]), report(&[11], &[10], &[]), report(&[12], &[10], &[]), again];

        let analysis = analyse(&flow, &reports);

        let c = &analysis.counts;
        assert_eq!((c.sent, c.unkeyed, c.unkeyed_claim, c.reinforced), (4, 4, 3, 1));
        assert_eq!((c.fires, c.fires_with_gone_candidate), (1, 1));
        assert_eq!((c.population, c.rest), (2, 1), "the session summary has a candidate but asserts nothing");
    }

    #[test]
    fn a_candidate_of_another_project_does_not_fire() {
        let mut preference = save("a", "x", "user_preference", "0: a preference of x");
        preference.memory_type = "user_preference".into();
        let flow = [preference, save("a", "y", "architectural_decision", "1: a decision of y")];
        let reports = [report(&[10], &[], &[]), report(&[11], &[10], &[])];

        let analysis = analyse(&flow, &reports);

        let c = &analysis.counts;
        assert_eq!((c.fires, c.other_project_candidate), (0, 1));
        assert_eq!(c.population, 0, "no earlier save of its own project to compare with");
    }

    /// 200 unkeyed claim saves, one scope, the first twelve after the first
    /// pair firing on the save just before them.
    fn two_hundred() -> (Vec<Save>, Vec<EvalSaved>) {
        let flow: Vec<Save> =
            (0..200).map(|i| save("a", "x", "semantic_fact", &format!("save number {i} says something"))).collect();
        let reports = (0..200)
            .map(|i| {
                let related: Vec<i64> = if (1..=12).contains(&i) { vec![1000 + i - 1] } else { vec![] };
                report(&[1000 + i], &related, &[])
            })
            .collect();
        (flow, reports)
    }

    #[test]
    fn every_firing_save_is_labelled_plus_a_seeded_sample() {
        let (flow, reports) = two_hundred();
        let analysis = analyse(&flow, &reports);

        let selected = select(&analysis, DEFAULT_SEED);

        assert_eq!(analysis.counts.fires, 12);
        assert_eq!(selected.iter().filter(|s| s.fired).count(), 12);
        assert_eq!(selected.iter().filter(|s| !s.fired).count(), SAMPLE_OF_REST);
        let fired: HashSet<usize> = selected.iter().filter(|s| s.fired).map(|s| s.save).collect();
        assert_eq!(fired, (1..=12).collect::<HashSet<_>>());
        for pick in selected.iter().filter(|s| s.fired) {
            assert_eq!(pick.earlier, Some(pick.save - 1), "a firing is paired with its candidate");
        }
        let few = analyse(&flow[..30], &reports[..30]);
        assert_eq!(select(&few, DEFAULT_SEED).len(), 12 + 17, "with 50 or fewer in the rest, all are labelled");
    }

    #[test]
    fn the_same_seed_picks_the_same_sample() {
        let (flow, reports) = two_hundred();
        let analysis = analyse(&flow, &reports);

        let a = select(&analysis, 7);
        assert_eq!(a, select(&analysis, 7));
        assert_ne!(a, select(&analysis, 8));
    }

    #[test]
    fn the_labels_file_does_not_say_which_pair_fired() {
        let (flow, reports) = two_hundred();
        let analysis = analyse(&flow, &reports);
        let mut selected = select(&analysis, DEFAULT_SEED);
        for pick in selected.iter_mut().filter(|s| !s.fired) {
            pick.earlier = Some(pick.save.saturating_sub(1));
        }

        let labels = build_labels(&flow, &selected);

        assert_eq!(labels.lines.len(), 62);
        for (i, line) in labels.lines.iter().enumerate() {
            assert_eq!(line.n, i + 1);
            let value: serde_json::Value = serde_json::from_str(&serde_json::to_string(line).unwrap()).unwrap();
            let mut keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
            keys.sort();
            assert_eq!(keys, ["earlier", "label", "later", "n", "proposed"]);
            assert!(value["proposed"].is_null() && value["label"].is_null());
        }
        let first_twelve = &labels.meta[..12];
        assert!(first_twelve.iter().any(|m| !m.fired), "the firing pairs are not listed first");
        assert_eq!(labels.meta.iter().filter(|m| m.fired).count(), 12);
    }

    fn replay_file(fires: usize, population: usize, labelled: Vec<Labelled>) -> ReplayFile {
        ReplayFile {
            export_format: "clawg-memories".into(),
            export_version: 1,
            model_contract: "test".into(),
            seed: DEFAULT_SEED,
            fake: false,
            export: ExportCounts::default(),
            replay: ReplayCounts { sent: 1000, fires, population, rest: population - fires, ..Default::default() },
            warning_line_tokens: 30,
            labelled,
        }
    }

    fn labelled(fired: &[bool]) -> Vec<Labelled> {
        fired.iter().enumerate().map(|(i, &fired)| Labelled { n: i + 1, fired, memory_type: "semantic_fact".into() }).collect()
    }

    fn line(n: usize, proposed: Option<&str>, label: Option<&str>) -> LabelLine {
        LabelLine { n, earlier: "e".into(), later: "l".into(), proposed: proposed.map(str::to_string), label: label.map(str::to_string) }
    }

    #[test]
    fn a_labels_file_with_an_empty_label_is_refused() {
        let replay = replay_file(1, 3, labelled(&[true, false]));
        for empty in [None, Some("")] {
            let labels = [line(1, Some("revision"), Some("revision")), line(2, Some("other"), empty)];
            let error = score(&replay, &labels).unwrap_err();
            assert!(matches!(error, StoreError::EmptyLabel(2)), "{error}");
            assert!(error.keeps_work(), "the hours of labelling are in that file");
        }
        assert!(matches!(
            score(&replay, &[line(1, None, Some("revision")), line(2, None, Some("maybe"))]),
            Err(StoreError::BadLabel { n: 2, .. })
        ));
        assert!(matches!(score(&replay, &[line(1, None, Some("revision"))]), Err(StoreError::Mismatch(_))));
        assert!(!StoreError::Replay("x".into()).keeps_work());
    }

    #[test]
    fn the_score_reads_only_the_owners_label_and_reports_the_agreement() {
        let replay = replay_file(2, 5, labelled(&[true, true, false, false, false]));
        let labels = [
            line(1, Some("other"), Some("revision")),
            line(2, Some("revision"), Some("revision")),
            line(3, Some("other"), Some("other")),
            line(4, None, Some("revision")),
            line(5, Some("revision"), Some("other")),
        ];

        let scored = score(&replay, &labels).unwrap();

        assert_eq!((scored.labelled.firing, scored.labelled.sample), (2, 3));
        assert_eq!((scored.revisions_among_firing.k, scored.revisions_among_firing.n), (2, 2));
        assert_eq!((scored.agreement.k, scored.agreement.n), (2, 4), "pair 4 carries no proposal");
        assert_eq!((scored.prevalence.revisions_among_firing, scored.prevalence.revisions_in_sample), (2, 1));
        assert!(scored.prevalence.census, "the 3 that do not fire were all labelled");
        assert_eq!((scored.prevalence.estimate * 5.0).round(), 3.0);
        assert!((scored.prevalence.lower_bound - Rate::of(3, 5).lo).abs() < 1e-12);
        let json = serde_json::to_string(&scored).unwrap();
        assert!(!json.contains("fake"), "{json}");
    }

    #[test]
    fn the_prevalence_bound_counts_the_firing_as_a_census_and_the_rest_as_a_sample() {
        let p = prevalence(1000, 100, 60, 50, 5);
        assert!(!p.census);
        assert!((p.estimate - (60.0 + 900.0 * 0.1) / 1000.0).abs() < 1e-12);
        let expected = (60.0 + 900.0 * Rate::of(5, 50).lo) / 1000.0;
        assert!((p.lower_bound - expected).abs() < 1e-12);
        assert!(p.lower_bound < p.estimate);
        assert_eq!(prevalence(0, 0, 0, 0, 0).lower_bound, 0.0);
    }

    #[test]
    fn the_thresholds_read_the_proposals_table() {
        let replay = replay_file(20, 100, labelled(&[true; 10]));
        let all_revisions: Vec<LabelLine> = (1..=10).map(|n| line(n, None, Some("revision"))).collect();
        let met = score(&replay, &all_revisions).unwrap();
        assert!(met.thresholds.firing_precision.met && met.thresholds.max_tokens_where_it_appears.met);
        assert!((met.warning.mean_tokens_per_save - 30.0 * 0.02).abs() < 1e-12);
        assert!(met.thresholds.mean_tokens_per_save.met);

        let none: Vec<LabelLine> = (1..=10).map(|n| line(n, None, Some("other"))).collect();
        let missed = score(&replay, &none).unwrap();
        assert!(!missed.thresholds.firing_precision.met && !missed.thresholds.met);
    }
}
