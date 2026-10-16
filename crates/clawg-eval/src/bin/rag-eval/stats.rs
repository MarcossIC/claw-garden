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

#[cfg(test)]
mod tests {
    use super::*;

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
