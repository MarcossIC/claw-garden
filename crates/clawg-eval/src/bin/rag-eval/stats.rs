//! Pure statistics for reading two runs side by side: whether a difference
//! between them is distinguishable from chance, given how few cases a golden
//! set has. Written out rather than pulled from a crate: each one is a closed
//! formula or a short loop, and a pinned value in a test is easier to audit
//! than a dependency's defaults.

/// Two-sided p-value of the exact binomial test with `p = 0.5` over the
/// `better + worse` cases that moved. On paired data this is the exact
/// McNemar test, and it is also the sign test: it reads only the direction of
/// each change, so it applies to any per-case metric, binary or continuous.
/// Summed in log space so a set of thousands of cases cannot underflow
/// `0.5^n` to zero.
pub fn mcnemar_exact(better: usize, worse: usize) -> f64 {
    let n = better + worse;
    if n == 0 {
        return 1.0;
    }
    let tail = better.min(worse);
    let mut ln_pmf = n as f64 * 0.5f64.ln();
    let mut sum = ln_pmf.exp();
    for i in 0..tail {
        ln_pmf += ((n - i) as f64).ln() - ((i + 1) as f64).ln();
        sum += ln_pmf.exp();
    }
    (2.0 * sum).min(1.0)
}

/// The significance level every verdict in this harness is read at.
pub const ALPHA: f64 = 0.05;

/// The fewest cases that, all moving the same way and none the other, reach
/// `ALPHA`. Independent of the set's size: only the cases that moved count.
pub fn min_one_way_significant() -> usize {
    (1..).find(|&m| mcnemar_exact(m, 0) < ALPHA).expect("the p-value halves with every case")
}

/// A mean difference and its interval.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Interval {
    pub mean: f64,
    pub lo: f64,
    pub hi: f64,
}

pub const BOOTSTRAP_RESAMPLES: usize = 10_000;

/// Fixed so the same two files always print the same interval.
pub const BOOTSTRAP_SEED: u64 = 0x5EED_0F_E7A1;

/// Mean of the per-case differences and its 95 % percentile bootstrap
/// interval: `resamples` draws of the cases with replacement, the mean of
/// each, and the 2.5th and 97.5th of those means. Makes no assumption about
/// the shape of the differences, which for MRR and nDCG pile up at 0 and ±1.
pub fn paired_bootstrap_ci(deltas: &[f64], resamples: usize, seed: u64) -> Interval {
    let n = deltas.len();
    if n == 0 || resamples == 0 {
        return Interval::default();
    }
    let mean = deltas.iter().sum::<f64>() / n as f64;
    let mut rng = SplitMix64::new(seed);
    let mut means: Vec<f64> = (0..resamples)
        .map(|_| (0..n).map(|_| deltas[(rng.next_u64() % n as u64) as usize]).sum::<f64>() / n as f64)
        .collect();
    means.sort_by(f64::total_cmp);
    let lo = means[((ALPHA / 2.0) * resamples as f64).floor() as usize];
    let hi = means[((1.0 - ALPHA / 2.0) * resamples as f64).ceil() as usize - 1];
    Interval { mean, lo, hi }
}

/// SplitMix64, the generator `golden-mine` uses (`synth/rng.rs`), copied
/// rather than shared: the two binaries have no library in common, and this
/// one needs only these two functions. `rand` is avoided for the same reason
/// there: its sequences may change between versions.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix_matches_the_golden_mine_sequence() {
        let mut rng = SplitMix64::new(0);
        assert_eq!(rng.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(rng.next_u64(), 0x6E78_9E6A_A1B9_65F4);
        assert_eq!(rng.next_u64(), 0x06C4_5D18_8009_454F);
    }

    #[test]
    fn bootstrap_is_deterministic_for_a_seed() {
        let deltas = [0.5, -0.25, 0.0, 1.0, 0.0, -1.0, 0.75];
        let a = paired_bootstrap_ci(&deltas, 2_000, 7);
        assert_eq!(a, paired_bootstrap_ci(&deltas, 2_000, 7));
        assert_ne!(a, paired_bootstrap_ci(&deltas, 2_000, 8));
        assert!(a.lo <= a.mean && a.mean <= a.hi, "{a:?}");
    }

    #[test]
    fn bootstrap_of_zero_differences_is_zero() {
        assert_eq!(paired_bootstrap_ci(&[0.0; 12], 1_000, BOOTSTRAP_SEED), Interval::default());
        assert_eq!(paired_bootstrap_ci(&[], 1_000, BOOTSTRAP_SEED), Interval::default());
    }

    #[test]
    fn bootstrap_of_positive_differences_excludes_zero() {
        let deltas = [0.1, 0.5, 0.2, 1.0, 0.3, 0.4];
        let ci = paired_bootstrap_ci(&deltas, BOOTSTRAP_RESAMPLES, BOOTSTRAP_SEED);
        assert!(ci.lo > 0.0, "{ci:?}");
    }

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn mcnemar_exact_matches_known_p_values() {
        for (b, c, p) in [(0, 6, 0.03125), (0, 5, 0.0625), (1, 8, 0.0390625), (1, 7, 0.0703125), (1, 9, 0.021484375)] {
            let got = mcnemar_exact(b, c);
            assert!(close(got, p, 1e-9), "mcnemar_exact({b}, {c}) = {got}, want {p}");
        }
        assert_eq!(mcnemar_exact(0, 0), 1.0);
        assert_eq!(mcnemar_exact(1, 0), 1.0);
    }

    #[test]
    fn mcnemar_exact_is_symmetric_and_never_above_one() {
        for b in 0..12 {
            for c in 0..12 {
                let p = mcnemar_exact(b, c);
                assert_eq!(p, mcnemar_exact(c, b));
                assert!((0.0..=1.0).contains(&p), "p({b}, {c}) = {p}");
            }
        }
        assert_eq!(mcnemar_exact(5, 5), 1.0);
    }

    #[test]
    fn mcnemar_exact_stays_finite_for_thousands_of_cases() {
        let p = mcnemar_exact(1000, 1100);
        assert!(p.is_finite() && p > 0.0 && p < 0.05, "p = {p}");
        assert_eq!(mcnemar_exact(1500, 1500), 1.0);
    }

    #[test]
    fn six_cases_one_way_is_the_smallest_significant_count() {
        assert_eq!(min_one_way_significant(), 6);
    }
}
