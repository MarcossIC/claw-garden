//! Rates and their intervals, copied from `conflict-eval` (`measure.rs`) with
//! the cases it fixes: the crate has no library to share them through, and
//! `rag-eval`'s copy takes another z, which would move published intervals.

use serde::Serialize;

/// The 95 % Wilson score interval of `k` successes in `n` trials.
fn wilson(k: usize, n: usize) -> (f64, f64) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rate_carries_its_wilson_interval() {
        let (lo, hi) = wilson(36, 46);
        assert!((lo - 0.644).abs() < 1e-3 && (hi - 0.877).abs() < 1e-3, "{lo} {hi}");
        let (lo, hi) = wilson(0, 10);
        assert_eq!(lo, 0.0);
        assert!((hi - 0.2775).abs() < 1e-3, "{hi}");
        let (lo, hi) = wilson(10, 10);
        assert!((lo - 0.7225).abs() < 1e-3, "{lo}");
        assert_eq!(hi, 1.0);
        assert_eq!(wilson(0, 0), (0.0, 1.0));

        let rate = Rate::of(36, 46);
        assert_eq!((rate.k, rate.n), (36, 46));
        assert!((rate.rate - 36.0 / 46.0).abs() < 1e-12);
        assert_eq!((rate.lo, rate.hi), wilson(36, 46));
        assert_eq!(Rate::of(0, 0).rate, 0.0);
    }
}
