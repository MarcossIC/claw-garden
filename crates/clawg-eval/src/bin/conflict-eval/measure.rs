//! What `conflict-eval` reads and reports, kept apart from the stores and the
//! model so every rule here is testable without either.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// What the newer memory of a labelled pair is to the older one. The detector
/// sees the same band of distances for all five; only the first two are worth
/// the user's time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// The two cannot both hold.
    Contradiction,
    /// A newer version of the same fact.
    Update,
    /// Another fact on the same topic.
    Complement,
    /// The same fact in other words.
    Rephrase,
    /// Another fact of the same project, on another topic.
    Distinct,
}

impl Class {
    pub const ALL: [Class; 5] = [Self::Contradiction, Self::Update, Self::Complement, Self::Rephrase, Self::Distinct];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Contradiction => "contradiction",
            Self::Update => "update",
            Self::Complement => "complement",
            Self::Rephrase => "rephrase",
            Self::Distinct => "distinct",
        }
    }

    /// The fewest pairs a class needs for its rate to mean something: with
    /// 30, one pair moves the rate by about three points.
    pub fn minimum(self) -> usize {
        match self {
            Self::Contradiction | Self::Distinct => 30,
            Self::Update | Self::Complement | Self::Rephrase => 15,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LabelledPair {
    pub class: Class,
    pub memory_type: String,
    pub lang: String,
    pub old: String,
    pub new: String,
}

#[derive(Debug, Deserialize)]
struct PairsFile {
    version: u32,
    pairs: Vec<LabelledPair>,
}

/// Parses a labelled pair file and checks that each class has its minimum.
pub fn parse_pairs(text: &str) -> Result<Vec<LabelledPair>, String> {
    let file: PairsFile = serde_json::from_str(text).map_err(|e| format!("not a pair file: {e}"))?;
    if file.version != 1 {
        return Err(format!("pair file version {} is not 1", file.version));
    }
    for class in Class::ALL {
        let n = file.pairs.iter().filter(|p| p.class == class).count();
        if n < class.minimum() {
            return Err(format!("class {} has {n} pairs, fewer than {}", class.as_str(), class.minimum()));
        }
    }
    Ok(file.pairs)
}

/// The sentences of `spec` that state a requirement (they carry `MUST`), one
/// fact each for the flow: many distinct facts of one project saved one at a
/// time, which is what the detector meets in daily use.
pub fn requirement_sentences(spec: &str) -> Vec<String> {
    spec.lines()
        .map(|l| l.trim().trim_start_matches(['-', '*', '#', ' ']).trim())
        .filter(|l| l.contains(" MUST") && l.chars().count() >= 40)
        .map(str::to_string)
        .collect()
}

/// The 95 % Wilson score interval of `k` successes in `n` trials.
pub fn wilson(k: usize, n: usize) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let z = 1.96_f64;
    let (k, n) = (k as f64, n as f64);
    let p = k / n;
    let denom = 1.0 + z * z / n;
    let center = (p + z * z / (2.0 * n)) / denom;
    let half = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt() / denom;
    ((center - half).max(0.0), (center + half).min(1.0))
}

/// A count out of `n`, with its rate and Wilson interval.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Rate {
    pub k: usize,
    pub n: usize,
    pub rate: f64,
    pub lo: f64,
    pub hi: f64,
}

impl Rate {
    pub fn of(k: usize, n: usize) -> Self {
        let (lo, hi) = wilson(k, n);
        Self { k, n, rate: if n == 0 { 0.0 } else { k as f64 / n as f64 }, lo, hi }
    }
}

/// What the detector did with one labelled pair.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PairOutcome {
    pub candidate: bool,
    pub escalated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ClassReport {
    /// Pairs the detector noted at all.
    pub candidate: Rate,
    /// Pairs it put to the user.
    pub escalated: Rate,
}

/// Groups outcomes by `key` and reports each group.
pub fn by<K: Ord>(outcomes: &[(K, PairOutcome)]) -> BTreeMap<&K, ClassReport> {
    let mut groups: BTreeMap<&K, Vec<PairOutcome>> = BTreeMap::new();
    for (key, outcome) in outcomes {
        groups.entry(key).or_default().push(*outcome);
    }
    groups
        .into_iter()
        .map(|(key, outcomes)| {
            let n = outcomes.len();
            let report = ClassReport {
                candidate: Rate::of(outcomes.iter().filter(|o| o.candidate).count(), n),
                escalated: Rate::of(outcomes.iter().filter(|o| o.escalated).count(), n),
            };
            (key, report)
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FlowReport {
    pub saves: usize,
    pub candidates: usize,
    pub escalated: usize,
    /// Saves that put at least one pair to the user.
    pub saves_escalated: Rate,
    pub candidates_per_save: f64,
    pub escalated_per_save: f64,
}

/// `pairs` is `(new_id, escalated)` for each pair the flow left.
pub fn flow_report(saves: usize, pairs: &[(i64, bool)]) -> FlowReport {
    let escalated = pairs.iter().filter(|(_, e)| *e).count();
    let mut escalating: Vec<i64> = pairs.iter().filter(|(_, e)| *e).map(|(id, _)| *id).collect();
    escalating.sort_unstable();
    escalating.dedup();
    let per = |k: usize| if saves == 0 { 0.0 } else { k as f64 / saves as f64 };
    FlowReport {
        saves,
        candidates: pairs.len(),
        escalated,
        saves_escalated: Rate::of(escalating.len(), saves),
        candidates_per_save: per(pairs.len()),
        escalated_per_save: per(escalated),
    }
}

/// The thresholds fixed before the measurement: fewer than 5 % of the flow's
/// saves put a pair to the user, and at least 0.8 of the contradictions are
/// noted, as the threshold's calibration measured.
pub const GATE_ESCALATED_PER_SAVE: f64 = 0.05;
pub const GATE_CONTRADICTION_CANDIDATES: f64 = 0.8;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Gate {
    pub escalated_per_save_below: f64,
    pub contradiction_candidates_at_least: f64,
    pub escalated_per_save_ok: bool,
    pub contradiction_candidates_ok: bool,
}

pub fn gate(flow: &FlowReport, contradiction: &ClassReport) -> Gate {
    Gate {
        escalated_per_save_below: GATE_ESCALATED_PER_SAVE,
        contradiction_candidates_at_least: GATE_CONTRADICTION_CANDIDATES,
        escalated_per_save_ok: flow.saves_escalated.rate < GATE_ESCALATED_PER_SAVE,
        contradiction_candidates_ok: contradiction.candidate.rate >= GATE_CONTRADICTION_CANDIDATES,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wilson_interval_cases() {
        let (lo, hi) = wilson(36, 46);
        assert!((lo - 0.644).abs() < 1e-3 && (hi - 0.877).abs() < 1e-3, "{lo} {hi}");
        let (lo, hi) = wilson(0, 10);
        assert_eq!(lo, 0.0);
        assert!((hi - 0.2775).abs() < 1e-3, "{hi}");
        let (lo, hi) = wilson(10, 10);
        assert!((lo - 0.7225).abs() < 1e-3, "{lo}");
        assert_eq!(hi, 1.0);
        assert_eq!(wilson(0, 0), (0.0, 1.0));
    }

    #[test]
    fn labelled_pairs_parse_and_cover_every_class() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/data/conflict-pairs.json")).unwrap();
        let pairs = parse_pairs(&text).unwrap();
        for p in &pairs {
            assert!(
                ["semantic_fact", "user_preference", "architectural_decision"].contains(&p.memory_type.as_str()),
                "{p:?}"
            );
            assert!(["en", "es"].contains(&p.lang.as_str()), "{p:?}");
            assert_ne!(p.old.trim(), p.new.trim(), "an identical pair never reaches the detector: {p:?}");
        }
        for lang in ["en", "es"] {
            assert!(pairs.iter().any(|p| p.lang == lang && p.class == Class::Contradiction), "{lang}");
        }
    }

    #[test]
    fn a_class_under_its_minimum_is_refused() {
        let text = r#"{"version":1,"pairs":[{"class":"distinct","memory_type":"semantic_fact","lang":"en","old":"a","new":"b"}]}"#;
        let err = parse_pairs(text).unwrap_err();
        assert!(err.contains("contradiction has 0 pairs"), "{err}");
        assert!(parse_pairs(r#"{"version":2,"pairs":[]}"#).unwrap_err().contains("version 2"));
    }

    #[test]
    fn requirement_sentences_take_the_lines_with_must() {
        let spec = "# Title\n\n- La fila MUST quedar `active` cuando no queda ninguna disputa abierta.\nShort MUST.\nNo requirement in this long enough line at all, really.\n";
        assert_eq!(
            requirement_sentences(spec),
            vec!["La fila MUST quedar `active` cuando no queda ninguna disputa abierta.".to_string()]
        );
    }

    #[test]
    fn the_flow_counts_saves_that_escalate_once() {
        let report = flow_report(10, &[(3, true), (3, true), (5, false), (7, true)]);
        assert_eq!((report.candidates, report.escalated), (4, 3));
        assert_eq!(report.saves_escalated.k, 2);
        assert!((report.escalated_per_save - 0.3).abs() < 1e-9);
        let g = gate(&report, &ClassReport { candidate: Rate::of(8, 10), escalated: Rate::of(1, 10) });
        assert!(!g.escalated_per_save_ok && g.contradiction_candidates_ok);
    }

    #[test]
    fn outcomes_group_by_class() {
        let yes = PairOutcome { candidate: true, escalated: false };
        let no = PairOutcome { candidate: false, escalated: false };
        let outcomes = [(Class::Distinct, no), (Class::Contradiction, yes), (Class::Distinct, yes)];
        let grouped = by(&outcomes);
        assert_eq!(grouped[&Class::Distinct].candidate.k, 1);
        assert_eq!(grouped[&Class::Distinct].candidate.n, 2);
        assert_eq!(grouped[&Class::Contradiction].escalated.k, 0);
    }
}
