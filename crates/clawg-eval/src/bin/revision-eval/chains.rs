//! What `chains` makes of its three arms, kept apart from the stores and the
//! model so every rule here is testable without either.

use std::collections::{BTreeMap, HashSet};

use clawg_core::eval_api::{EvalConflictPair, CONTRADICTION_THRESHOLD, NEAR_DUPLICATE_FLOOR};
use serde::Serialize;

use crate::set::{Chain, ChainSet, Change};
use crate::stats::Rate;

/// The three stores `chains` saves the set into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arm {
    /// Every version, without a key.
    NoKey,
    /// Every version, under its chain's key.
    Keyed,
    /// Only the last version of each chain.
    Control,
}

impl Arm {
    pub const ALL: [Arm; 3] = [Self::NoKey, Self::Keyed, Self::Control];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoKey => "no_key",
            Self::Keyed => "keyed",
            Self::Control => "control",
        }
    }

    fn key(self, chain: &Chain) -> Option<&str> {
        match self {
            Self::NoKey => None,
            Self::Keyed | Self::Control => Some(&chain.topic_key),
        }
    }
}

/// One save an arm makes: version `version` (from 0) of `chain`, the chain at
/// `index` in its set.
#[derive(Debug, Clone, Copy)]
pub struct PlannedSave<'a> {
    pub index: usize,
    pub chain: &'a Chain,
    pub version: usize,
    pub topic_key: Option<&'a str>,
}

/// The saves of `arm`, in the order it makes them. The no-key and keyed arms
/// save every first version, then every second, then every third, so other
/// memories are saved between two versions of a topic, as in daily use. The
/// control saves only the last version of each chain, under its key as the
/// keyed arm does, so that the older versions are all that tells them apart.
pub fn arm_saves(set: &ChainSet, arm: Arm) -> Vec<PlannedSave<'_>> {
    let planned = |index: usize, version: usize| {
        let chain = &set.chains[index];
        PlannedSave { index, chain, version, topic_key: arm.key(chain) }
    };
    match arm {
        Arm::Control => set.chains.iter().enumerate().map(|(i, c)| planned(i, c.versions.len() - 1)).collect(),
        Arm::NoKey | Arm::Keyed => {
            let rounds = set.chains.iter().map(|c| c.versions.len()).max().unwrap_or(0);
            (0..rounds)
                .flat_map(|v| (0..set.chains.len()).filter(move |&i| v < set.chains[i].versions.len()).map(move |i| (i, v)))
                .map(|(i, v)| planned(i, v))
                .collect()
        }
    }
}

/// The sentences of `spec` that state a requirement (they carry `MUST`), as
/// `conflict-eval` reads them: many distinct facts of one project, which every
/// arm saves before its chains.
pub fn requirement_sentences(spec: &str) -> Vec<String> {
    spec.lines()
        .map(|l| l.trim().trim_start_matches(['-', '*', '#', ' ']).trim())
        .filter(|l| l.contains(" MUST") && l.chars().count() >= 40)
        .map(str::to_string)
        .collect()
}

/// The rows each version holds that no later version of its chain holds;
/// `None` for a version the arm does not save. A long version that repeats
/// most of the one before keeps each unchanged piece as one row both saves
/// hold, and that row is the newer version's: it is what the newer one says.
pub fn own_rows(versions: &[Option<Vec<i64>>]) -> Vec<Option<Vec<i64>>> {
    versions
        .iter()
        .enumerate()
        .map(|(i, rows)| {
            let later: HashSet<i64> = versions[i + 1..].iter().flatten().flatten().copied().collect();
            rows.as_ref().map(|rows| rows.iter().copied().filter(|id| !later.contains(id)).collect())
        })
        .collect()
}

/// What a transition compares: the newer version's rows the older one does
/// not hold, which are the ones the detector probed with, and the older
/// version's rows the newer one does not hold. A row both hold would be
/// compared with itself, which says nothing of the change.
pub fn compared_rows(newer: &[i64], older: &[i64]) -> (Vec<i64>, Vec<i64>) {
    let only = |rows: &[i64], other: &[i64]| rows.iter().copied().filter(|id| !other.contains(id)).collect();
    (only(newer, older), only(older, newer))
}

pub fn is_live(status: &str) -> bool {
    matches!(status, "active" | "disputed")
}

/// The pair the detector wrote between two versions, as the store keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pair {
    Escalated,
    Pending,
}

/// What the no-key arm's store holds for one transition.
#[derive(Debug, Clone, PartialEq)]
pub struct Observed {
    pub pair: Option<Pair>,
    /// The chain's type is one the detector pairs.
    pub claim_bearing: bool,
    /// Between the rows [`compared_rows`] gives; none when the newer version
    /// holds every row of the older one.
    pub distances: Vec<f64>,
}

/// Reads the pair between the rows of the newer version and those of the
/// older one; a pair with any other version belongs to another transition.
pub fn observe(
    pairs: &[EvalConflictPair],
    newer: &[i64],
    older: &[i64],
    claim_bearing: bool,
    distances: Vec<f64>,
) -> Observed {
    let between: Vec<&EvalConflictPair> =
        pairs.iter().filter(|p| newer.contains(&p.new_id) && older.contains(&p.old_id)).collect();
    let pair = if between.iter().any(|p| p.escalated) {
        Some(Pair::Escalated)
    } else if between.is_empty() {
        None
    } else {
        Some(Pair::Pending)
    };
    Observed { pair, claim_bearing, distances }
}

/// The band of distances the detector pairs within.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    pub floor: f64,
    pub threshold: f64,
}

impl Band {
    /// `[NEAR_DUPLICATE_FLOOR, CONTRADICTION_THRESHOLD)`, widened to `f64` as
    /// the detector widens them before comparing.
    pub fn detector() -> Self {
        Self { floor: NEAR_DUPLICATE_FLOOR as f64, threshold: CONTRADICTION_THRESHOLD as f64 }
    }
}

/// Why a transition was left without a pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    BelowFloor,
    AboveThreshold,
    OutOfPool,
    NotClaimBearing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Escalated,
    Pending,
    NoMark(Reason),
}

/// The class of a transition. A pair decides it as the detector wrote it;
/// without one the reason is deduced rather than recomputed: a distance inside
/// the band with no pair can only mean that the older version was outside the
/// detector's candidate pool, which its KNN cuts before it filters. An older
/// version with no row of its own left is all inside the newer one, as a
/// distance of zero would say.
pub fn classify(observed: &Observed, band: Band) -> Outcome {
    match observed.pair {
        Some(Pair::Escalated) => return Outcome::Escalated,
        Some(Pair::Pending) => return Outcome::Pending,
        None => {}
    }
    let any = |keep: &dyn Fn(f64) -> bool| observed.distances.iter().any(|&d| keep(d));
    let reason = if !observed.claim_bearing {
        Reason::NotClaimBearing
    } else if observed.distances.is_empty() {
        Reason::BelowFloor
    } else if any(&|d| d >= band.floor && d < band.threshold) {
        Reason::OutOfPool
    } else if any(&|d| d < band.floor) {
        Reason::BelowFloor
    } else {
        Reason::AboveThreshold
    };
    Outcome::NoMark(reason)
}

/// One step of a chain in the no-key arm and what the detector left of it.
#[derive(Debug, Clone, Copy)]
pub struct Transition<'a> {
    pub chain: &'a Chain,
    /// The newer version, from 1.
    pub to: usize,
    pub outcome: Outcome,
}

impl Transition<'_> {
    fn change(&self) -> Change {
        self.chain.versions[self.to].change.expect("the set refuses a version after the first without a change")
    }
}

/// How many transitions fell in each class, out of all of them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Classes {
    pub transitions: usize,
    pub escalated: Rate,
    pub pending: Rate,
    pub no_mark: Rate,
    pub no_mark_reasons: Reasons,
}

/// Why each transition without a mark got none, also out of every transition,
/// so the four add up to `no_mark`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reasons {
    pub below_floor: Rate,
    pub above_threshold: Rate,
    pub out_of_pool: Rate,
    pub not_claim_bearing: Rate,
}

fn classes(outcomes: &[Outcome]) -> Classes {
    let n = outcomes.len();
    let count = |keep: &dyn Fn(Outcome) -> bool| Rate::of(outcomes.iter().filter(|&&o| keep(o)).count(), n);
    let reason = |r: Reason| count(&|o| o == Outcome::NoMark(r));
    Classes {
        transitions: n,
        escalated: count(&|o| o == Outcome::Escalated),
        pending: count(&|o| o == Outcome::Pending),
        no_mark: count(&|o| matches!(o, Outcome::NoMark(_))),
        no_mark_reasons: Reasons {
            below_floor: reason(Reason::BelowFloor),
            above_threshold: reason(Reason::AboveThreshold),
            out_of_pool: reason(Reason::OutOfPool),
            not_claim_bearing: reason(Reason::NotClaimBearing),
        },
    }
}

/// M1: how often a version saved without a key leaves the one before it
/// marked, and how often unmarked and why, over the whole set and cut by kind
/// of change, language and type.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct M1 {
    #[serde(flatten)]
    pub all: Classes,
    pub by_change: BTreeMap<Change, Classes>,
    pub by_lang: BTreeMap<String, Classes>,
    pub by_type: BTreeMap<String, Classes>,
}

pub fn m1(transitions: &[Transition<'_>]) -> M1 {
    fn by<K: Ord>(transitions: &[Transition<'_>], key: impl Fn(&Transition<'_>) -> K) -> BTreeMap<K, Classes> {
        let mut groups: BTreeMap<K, Vec<Outcome>> = BTreeMap::new();
        for t in transitions {
            groups.entry(key(t)).or_default().push(t.outcome);
        }
        groups.into_iter().map(|(k, outcomes)| (k, classes(&outcomes))).collect()
    }
    M1 {
        all: classes(&transitions.iter().map(|t| t.outcome).collect::<Vec<_>>()),
        by_change: by(transitions, |t| t.change()),
        by_lang: by(transitions, |t| t.chain.lang.clone()),
        by_type: by(transitions, |t| t.chain.memory_type.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::set::Version;

    fn chain(id: &str, lang: &str, memory_type: &str, changes: &[Change]) -> Chain {
        let mut versions = vec![Version { content: format!("{id} first"), value: "first".into(), change: None }];
        for (i, change) in changes.iter().enumerate() {
            let value = format!("value{}", i + 2);
            versions.push(Version { content: format!("{id} {value}"), value, change: Some(*change) });
        }
        Chain {
            id: id.into(),
            lang: lang.into(),
            memory_type: memory_type.into(),
            topic_key: format!("topic/{id}"),
            query: format!("what about {id}"),
            versions,
        }
    }

    fn set() -> ChainSet {
        ChainSet {
            version: 1,
            sha256: String::new(),
            chains: vec![
                chain("a", "es", "architectural_decision", &[Change::Value, Change::Extend]),
                chain("b", "en", "user_preference", &[Change::Rewrite]),
            ],
        }
    }

    fn observed(pair: Option<Pair>, claim_bearing: bool, distances: &[f64]) -> Observed {
        Observed { pair, claim_bearing, distances: distances.to_vec() }
    }

    fn pair(new_id: i64, old_id: i64, escalated: bool) -> EvalConflictPair {
        EvalConflictPair { new_id, old_id, distance: Some(0.3), escalated }
    }

    #[test]
    fn a_far_pair_is_no_mark_above_the_threshold() {
        let outcome = classify(&observed(None, true, &[0.55, 0.61]), Band::detector());
        assert_eq!(outcome, Outcome::NoMark(Reason::AboveThreshold));
    }

    #[test]
    fn a_close_pair_of_decisions_is_escalated() {
        assert_eq!(classify(&observed(Some(Pair::Escalated), true, &[0.3]), Band::detector()), Outcome::Escalated);
        assert_eq!(classify(&observed(Some(Pair::Pending), true, &[0.3]), Band::detector()), Outcome::Pending);
    }

    /// No pair, yet one piece sits inside the band: the detector's KNN cut the
    /// older version off before the band was looked at.
    #[test]
    fn a_row_beyond_the_candidate_pool_is_out_of_pool() {
        let outcome = classify(&observed(None, true, &[0.7, 0.35]), Band::detector());
        assert_eq!(outcome, Outcome::NoMark(Reason::OutOfPool));
        let outcome = classify(&observed(None, true, &[0.05, 0.7]), Band::detector());
        assert_eq!(outcome, Outcome::NoMark(Reason::BelowFloor));
    }

    #[test]
    fn a_type_that_asserts_nothing_is_not_claim_bearing() {
        let outcome = classify(&observed(None, false, &[0.3]), Band::detector());
        assert_eq!(outcome, Outcome::NoMark(Reason::NotClaimBearing));
    }

    #[test]
    fn the_band_is_the_detectors() {
        let band = Band::detector();
        assert_eq!(band.floor, clawg_core::eval_api::NEAR_DUPLICATE_FLOOR as f64);
        assert_eq!(band.threshold, clawg_core::eval_api::CONTRADICTION_THRESHOLD as f64);
        assert_eq!(classify(&observed(None, true, &[band.floor]), band), Outcome::NoMark(Reason::OutOfPool));
        assert_eq!(classify(&observed(None, true, &[band.threshold]), band), Outcome::NoMark(Reason::AboveThreshold));
    }

    /// A third version that pairs with the first says nothing of the step
    /// from the second.
    #[test]
    fn the_pair_is_read_between_the_two_versions_only() {
        let pairs = [pair(30, 10, true), pair(31, 21, false)];
        assert_eq!(observe(&pairs, &[30, 31], &[20, 21], true, vec![0.3]).pair, Some(Pair::Pending));
        assert_eq!(observe(&pairs, &[30, 31], &[10], true, vec![0.3]).pair, Some(Pair::Escalated));
        assert_eq!(observe(&pairs, &[20], &[10], true, vec![0.3]).pair, None);
        let both = [pair(30, 20, false), pair(31, 21, true)];
        assert_eq!(observe(&both, &[30, 31], &[20, 21], true, vec![0.3]).pair, Some(Pair::Escalated));
    }

    /// A long version that repeats most of the one before keeps each unchanged
    /// piece as one row both saves hold.
    #[test]
    fn a_row_two_versions_share_is_the_newer_ones() {
        let own = own_rows(&[Some(vec![1, 2, 3]), Some(vec![1, 2, 4]), Some(vec![1, 5])]);
        assert_eq!(own, [Some(vec![3]), Some(vec![2, 4]), Some(vec![1, 5])]);
        assert_eq!(own_rows(&[None, Some(vec![7, 8])]), [None, Some(vec![7, 8])]);
        assert_eq!(own_rows(&[Some(vec![1]), Some(vec![1, 2])]), [Some(vec![]), Some(vec![1, 2])]);
    }

    #[test]
    fn the_rows_a_transition_compares_leave_out_the_ones_both_hold() {
        assert_eq!(compared_rows(&[1, 2, 4], &[1, 2, 3]), (vec![4], vec![3]));
        assert_eq!(compared_rows(&[9], &[8]), (vec![9], vec![8]));
        assert_eq!(compared_rows(&[1, 2], &[1]), (vec![2], vec![]));
    }

    /// Every piece of the older version is repeated in the newer one: nothing
    /// of it differs, as a distance of zero would say.
    #[test]
    fn an_older_version_the_newer_holds_whole_is_below_the_floor() {
        let outcome = classify(&observed(None, true, &[]), Band::detector());
        assert_eq!(outcome, Outcome::NoMark(Reason::BelowFloor));
        let not_claim = classify(&observed(None, false, &[]), Band::detector());
        assert_eq!(not_claim, Outcome::NoMark(Reason::NotClaimBearing));
    }

    #[test]
    fn a_row_is_live_while_it_is_active_or_disputed() {
        assert!(is_live("active") && is_live("disputed"));
        assert!(!is_live("superseded"));
    }

    #[test]
    fn every_transition_falls_in_exactly_one_class() {
        let set = set();
        let (a, b) = (&set.chains[0], &set.chains[1]);
        let transitions = [
            Transition { chain: a, to: 1, outcome: Outcome::Escalated },
            Transition { chain: a, to: 2, outcome: Outcome::NoMark(Reason::OutOfPool) },
            Transition { chain: b, to: 1, outcome: Outcome::Pending },
            Transition { chain: b, to: 1, outcome: Outcome::NoMark(Reason::AboveThreshold) },
            Transition { chain: b, to: 1, outcome: Outcome::NoMark(Reason::AboveThreshold) },
            Transition { chain: b, to: 1, outcome: Outcome::NoMark(Reason::BelowFloor) },
            Transition { chain: b, to: 1, outcome: Outcome::NoMark(Reason::NotClaimBearing) },
        ];

        let m1 = m1(&transitions);

        let all = &m1.all;
        assert_eq!(all.transitions, 7);
        assert_eq!(all.escalated.k + all.pending.k + all.no_mark.k, 7);
        assert_eq!((all.escalated.k, all.pending.k, all.no_mark.k), (1, 1, 5));
        let reasons = &all.no_mark_reasons;
        assert_eq!(
            reasons.below_floor.k + reasons.above_threshold.k + reasons.out_of_pool.k + reasons.not_claim_bearing.k,
            all.no_mark.k
        );
        assert_eq!(reasons.above_threshold.k, 2);
        assert!([&all.escalated, &all.no_mark, &reasons.out_of_pool].iter().all(|r| r.n == 7));

        assert_eq!(m1.by_change[&Change::Value].transitions, 1);
        assert_eq!(m1.by_change[&Change::Extend].no_mark.k, 1);
        assert_eq!(m1.by_change[&Change::Rewrite].transitions, 5);
        assert_eq!(m1.by_lang["es"].transitions, 2);
        assert_eq!(m1.by_lang["en"].pending.k, 1);
        assert_eq!(m1.by_type["user_preference"].no_mark.k, 4);
        assert_eq!(m1.by_type.values().map(|c| c.transitions).sum::<usize>(), 7);
    }

    #[test]
    fn the_control_arm_stores_only_the_last_version() {
        let set = set();
        let control = arm_saves(&set, Arm::Control);
        let saved: Vec<(&str, usize)> = control.iter().map(|s| (s.chain.id.as_str(), s.version)).collect();
        assert_eq!(saved, [("a", 2), ("b", 1)]);
        assert!(control.iter().all(|s| s.topic_key == Some(s.chain.topic_key.as_str())));
    }

    /// Every first version before any second one: other memories are saved
    /// between two versions of a topic, as in daily use.
    #[test]
    fn the_other_arms_save_every_version_one_round_at_a_time() {
        let set = set();
        let no_key = arm_saves(&set, Arm::NoKey);
        let order: Vec<(&str, usize)> = no_key.iter().map(|s| (s.chain.id.as_str(), s.version)).collect();
        assert_eq!(order, [("a", 0), ("b", 0), ("a", 1), ("b", 1), ("a", 2)]);
        assert!(no_key.iter().all(|s| s.topic_key.is_none()));

        let keyed = arm_saves(&set, Arm::Keyed);
        let keyed_order: Vec<(&str, usize)> = keyed.iter().map(|s| (s.chain.id.as_str(), s.version)).collect();
        assert_eq!(keyed_order, order);
        assert!(keyed.iter().all(|s| s.topic_key == Some(s.chain.topic_key.as_str())));
    }

    #[test]
    fn the_distractors_are_the_requirement_sentences_of_a_spec() {
        let spec = "# Title\n\n- The row MUST stay `active` while no dispute is left.\n- A short one MUST go.\nplain text\n";
        assert_eq!(requirement_sentences(spec), ["The row MUST stay `active` while no dispute is left."]);
    }
}
