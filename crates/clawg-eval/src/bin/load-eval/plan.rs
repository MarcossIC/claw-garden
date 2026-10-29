//! What the gate runs and how it decides: pure, so the rules are fixed by
//! tests before any load is measured.

use serde::Serialize;

/// How the model reaches the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Arm {
    /// The file read into a `Vec`, hashed, handed to `commit_from_memory`:
    /// what clawg did before the read-only view.
    Vec,
    /// The file hashed in blocks and loaded by path with the handle open.
    File,
    /// The read-only view production loads from.
    View,
}

impl Arm {
    pub fn name(self) -> &'static str {
        match self {
            Arm::Vec => "vec",
            Arm::File => "file",
            Arm::View => "view",
        }
    }

    pub fn parse(text: &str) -> Option<Arm> {
        ARMS.into_iter().find(|arm| arm.name() == text)
    }
}

pub const ARMS: [Arm; 3] = [Arm::Vec, Arm::File, Arm::View];

/// One load of the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub round: usize,
    pub arm: Arm,
    /// False for the warm-up round, which only fills the file cache.
    pub counted: bool,
}

/// `loads` rounds that count after a warm-up round, each with the three arms
/// in an order that starts one arm later than the round before, so no arm
/// always loads first.
pub fn schedule(loads: usize) -> Vec<Slot> {
    (0..=loads)
        .flat_map(|round| {
            (0..ARMS.len()).map(move |i| Slot { round, arm: ARMS[(round + i) % ARMS.len()], counted: round > 0 })
        })
        .collect()
}

/// What one counted load left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    pub arm: Arm,
    /// The most memory the load committed, in bytes.
    pub peak_bytes: u64,
    /// Verifying the file plus building the session, in milliseconds.
    pub total_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Decision {
    View,
    File,
    Revert,
}

/// The three rules an arm has to meet against the `Vec` arm of the same run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Rules {
    pub drop_ok: bool,
    pub worst_ok: bool,
    pub time_ok: bool,
}

impl Rules {
    pub fn passes(self) -> bool {
        self.drop_ok && self.worst_ok && self.time_ok
    }
}

const MIN_DROP_BYTES: u64 = 300 * 1024 * 1024;
const TIME_SLACK_MS: u64 = 150;
const TIME_SLACK_PERCENT: u64 = 15;

fn of(samples: &[Sample], arm: Arm) -> Vec<Sample> {
    samples.iter().copied().filter(|s| s.arm == arm).collect()
}

/// The lower middle value, so a median is always a load that happened.
pub fn median(mut values: Vec<u64>) -> Option<u64> {
    values.sort_unstable();
    values.get(values.len().checked_sub(1)? / 2).copied()
}

/// `arm` against the `Vec` arm, or `None` when either has no load.
pub fn rules(samples: &[Sample], arm: Arm) -> Option<Rules> {
    let base = of(samples, Arm::Vec);
    let own = of(samples, arm);
    if base.is_empty() || own.is_empty() {
        return None;
    }
    let peaks = |set: &[Sample]| set.iter().map(|s| s.peak_bytes).collect::<Vec<_>>();
    let base_median = median(peaks(&base))?;
    let own_median = median(peaks(&own))?;
    let best_today = base.iter().map(|s| s.peak_bytes).min()?;
    let worst_own = own.iter().map(|s| s.peak_bytes).max()?;
    let base_ms = median(base.iter().map(|s| s.total_ms).collect())?;
    let own_ms = median(own.iter().map(|s| s.total_ms).collect())?;
    let allowed = TIME_SLACK_MS.max(base_ms * TIME_SLACK_PERCENT / 100);
    Some(Rules {
        drop_ok: base_median.saturating_sub(own_median) >= MIN_DROP_BYTES,
        worst_ok: worst_own < best_today,
        time_ok: own_ms <= base_ms + allowed,
    })
}

/// The gate's decision, or none where the system gives no commit peak to
/// decide on.
pub fn verdict(samples: &[Sample], decides: bool) -> Option<Decision> {
    if !decides {
        return None;
    }
    let passes = |arm| rules(samples, arm).is_some_and(Rules::passes);
    Some(if passes(Arm::View) {
        Decision::View
    } else if passes(Arm::File) {
        Decision::File
    } else {
        Decision::Revert
    })
}

/// What a run found for one arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ArmSummary {
    pub arm: Arm,
    pub loads: usize,
    pub median_peak_bytes: u64,
    pub best_peak_bytes: u64,
    pub worst_peak_bytes: u64,
    pub median_total_ms: u64,
    /// Against the `Vec` arm; none for it.
    pub rules: Option<Rules>,
}

/// The arms that have a load, in the order of [`ARMS`].
pub fn summarize(samples: &[Sample]) -> Vec<ArmSummary> {
    ARMS.into_iter()
        .filter_map(|arm| {
            let own = of(samples, arm);
            let peaks: Vec<u64> = own.iter().map(|s| s.peak_bytes).collect();
            Some(ArmSummary {
                arm,
                loads: own.len(),
                median_peak_bytes: median(peaks.clone())?,
                best_peak_bytes: *peaks.iter().min()?,
                worst_peak_bytes: *peaks.iter().max()?,
                median_total_ms: median(own.iter().map(|s| s.total_ms).collect())?,
                rules: if arm == Arm::Vec { None } else { rules(samples, arm) },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    fn sample(arm: Arm, peak_mib: u64, total_ms: u64) -> Sample {
        Sample { arm, peak_bytes: peak_mib * MIB, total_ms }
    }

    fn arms(vec: &[(u64, u64)], file: &[(u64, u64)], view: &[(u64, u64)]) -> Vec<Sample> {
        let mut out = Vec::new();
        for (arm, set) in [(Arm::Vec, vec), (Arm::File, file), (Arm::View, view)] {
            out.extend(set.iter().map(|&(peak, ms)| sample(arm, peak, ms)));
        }
        out
    }

    #[test]
    fn eleven_rounds_with_the_first_one_discarded() {
        let slots = schedule(10);
        assert_eq!(slots.len(), 33);
        assert!(slots.iter().filter(|s| s.round == 0).all(|s| !s.counted));
        assert!(slots.iter().filter(|s| s.round > 0).all(|s| s.counted));
        assert_eq!(slots.iter().map(|s| s.round).max(), Some(10));
        assert_eq!(slots.iter().filter(|s| s.counted).count(), 30);
    }

    #[test]
    fn every_round_has_the_three_arms_in_rotated_order() {
        let slots = schedule(10);
        let mut firsts = Vec::new();
        for round in 0..=10 {
            let order: Vec<Arm> = slots.iter().filter(|s| s.round == round).map(|s| s.arm).collect();
            let mut sorted = order.clone();
            sorted.sort_by_key(|a| *a as u8);
            assert_eq!(sorted, vec![Arm::Vec, Arm::File, Arm::View], "round {round} has each arm once");
            firsts.push(order[0]);
        }
        assert_eq!(&firsts[..4], &[Arm::Vec, Arm::File, Arm::View, Arm::Vec]);
        let r1: Vec<Arm> = slots.iter().filter(|s| s.round == 1).map(|s| s.arm).collect();
        assert_eq!(r1, vec![Arm::File, Arm::View, Arm::Vec]);
    }

    #[test]
    fn no_loads_means_only_the_warm_up_round() {
        let slots = schedule(0);
        assert_eq!(slots.len(), 3);
        assert!(slots.iter().all(|s| !s.counted));
    }

    #[test]
    fn both_arms_passing_give_view() {
        let s = arms(&[(900, 1000), (950, 1000), (1000, 1000)], &[(500, 1000), (520, 1000), (540, 1000)], &[(400, 1000), (420, 1000), (440, 1000)]);
        assert_eq!(verdict(&s, true), Some(Decision::View));
    }

    #[test]
    fn only_view_passing_gives_view() {
        let s = arms(&[(900, 1000), (950, 1000), (1000, 1000)], &[(900, 1000), (950, 1000), (1000, 1000)], &[(400, 1000), (420, 1000), (440, 1000)]);
        assert_eq!(verdict(&s, true), Some(Decision::View));
    }

    #[test]
    fn only_file_passing_gives_file() {
        let s = arms(&[(900, 1000), (950, 1000), (1000, 1000)], &[(400, 1000), (420, 1000), (440, 1000)], &[(900, 1000), (950, 1000), (1000, 1000)]);
        assert_eq!(verdict(&s, true), Some(Decision::File));
    }

    #[test]
    fn neither_arm_passing_gives_revert() {
        let s = arms(&[(900, 1000), (950, 1000), (1000, 1000)], &[(900, 1000), (950, 1000), (1000, 1000)], &[(900, 1000), (950, 1000), (1000, 1000)]);
        assert_eq!(verdict(&s, true), Some(Decision::Revert));
    }

    #[test]
    fn a_system_that_does_not_decide_reports_without_a_decision() {
        let s = arms(&[(900, 1000)], &[(400, 1000)], &[(400, 1000)]);
        assert_eq!(verdict(&s, false), None);
    }

    #[test]
    fn the_median_must_drop_at_least_300_mib() {
        let base = [(1000, 1000), (1000, 1000), (1000, 1000)];
        let exact = arms(&base, &[(1000, 1000); 3], &[(700, 1000), (700, 1000), (700, 1000)]);
        assert!(rules(&exact, Arm::View).unwrap().drop_ok, "300 MiB exactly is enough");
        let short = arms(&base, &[(1000, 1000); 3], &[(701, 1000), (701, 1000), (701, 1000)]);
        assert!(!rules(&short, Arm::View).unwrap().drop_ok, "299 MiB is not");
    }

    #[test]
    fn the_drop_is_read_on_the_median_not_on_the_mean_or_the_best() {
        let base = [(1000, 1000), (1000, 1000), (1000, 1000)];
        let one_good_load = arms(&base, &[(1000, 1000); 3], &[(100, 1000), (900, 1000), (900, 1000)]);
        assert!(!rules(&one_good_load, Arm::View).unwrap().drop_ok);
    }

    #[test]
    fn the_worst_load_must_stay_under_the_best_baseline_load() {
        let base = [(1000, 1000), (1100, 1000), (1200, 1000)];
        let under = arms(&base, &[(1000, 1000); 3], &[(300, 1000), (400, 1000), (999, 1000)]);
        assert!(rules(&under, Arm::View).unwrap().worst_ok);
        let equal = arms(&base, &[(1000, 1000); 3], &[(300, 1000), (400, 1000), (1000, 1000)]);
        assert!(!rules(&equal, Arm::View).unwrap().worst_ok, "equal to the best of today is not under it");
    }

    #[test]
    fn the_time_may_rise_150_ms_or_15_percent_whichever_is_larger() {
        let slow = |ms: u64| arms(&[(1000, 1000), (1000, 1000), (1000, 1000)], &[(1000, 1000); 3], &[(100, ms), (100, ms), (100, ms)]);
        assert!(rules(&slow(1150), Arm::View).unwrap().time_ok, "+150 ms on a 1 s baseline");
        assert!(!rules(&slow(1151), Arm::View).unwrap().time_ok);
        let long = |ms: u64| arms(&[(1000, 4000), (1000, 4000), (1000, 4000)], &[(1000, 4000); 3], &[(100, ms), (100, ms), (100, ms)]);
        assert!(rules(&long(4600), Arm::View).unwrap().time_ok, "+15 % on a 4 s baseline is 600 ms");
        assert!(!rules(&long(4601), Arm::View).unwrap().time_ok);
    }

    #[test]
    fn a_short_baseline_gets_the_flat_150_ms_not_15_percent_of_itself() {
        let slow = |ms: u64| arms(&[(1000, 500), (1000, 500), (1000, 500)], &[(1000, 500); 3], &[(100, ms), (100, ms), (100, ms)]);
        assert!(rules(&slow(650), Arm::View).unwrap().time_ok);
        assert!(!rules(&slow(651), Arm::View).unwrap().time_ok);
    }

    #[test]
    fn a_faster_load_never_fails_the_time_rule() {
        let s = arms(&[(1000, 1000); 3], &[(1000, 1000); 3], &[(100, 10), (100, 10), (100, 10)]);
        assert!(rules(&s, Arm::View).unwrap().time_ok);
    }

    #[test]
    fn the_median_is_the_lower_middle_load() {
        assert_eq!(median(vec![40, 10, 30, 20]), Some(20));
        assert_eq!(median(vec![30, 10, 20]), Some(20));
        assert_eq!(median(vec![]), None);
    }

    #[test]
    fn the_summary_names_the_median_best_and_worst_of_each_arm() {
        let s = arms(&[(1000, 1000), (1200, 1100), (1100, 900)], &[], &[(400, 1000), (500, 1100), (300, 1300)]);
        let summary = summarize(&s);
        assert_eq!(summary.len(), 2, "an arm with no load is not listed");
        assert_eq!((summary[0].arm, summary[0].loads), (Arm::Vec, 3));
        assert_eq!(summary[0].median_peak_bytes, 1100 * MIB);
        assert_eq!((summary[0].best_peak_bytes, summary[0].worst_peak_bytes), (1000 * MIB, 1200 * MIB));
        assert_eq!(summary[0].median_total_ms, 1000);
        assert!(summary[0].rules.is_none(), "the baseline has no rules against itself");
        assert_eq!(summary[1].arm, Arm::View);
        assert_eq!(summary[1].median_peak_bytes, 400 * MIB);
        assert_eq!(summary[1].rules, rules(&s, Arm::View));
    }

    #[test]
    fn an_arm_with_no_samples_has_no_rules() {
        let s = arms(&[(1000, 1000)], &[(1000, 1000)], &[]);
        assert!(rules(&s, Arm::View).is_none());
        assert_eq!(verdict(&s, true), Some(Decision::Revert));
    }
}
