//! Pure statistics for comparing two latency distributions of one scenario:
//! unpaired, since two runs send different requests at different instants.
//! Written out, as in `rag-eval`'s `stats.rs`: a pinned value in a test is
//! easier to audit than a dependency's defaults.

/// The `q` quantile by nearest rank, as the bench summarises: the smallest
/// sample with at least `q` of them at or below it.
pub fn quantile(sorted: &[u64], q: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (q * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// The nearest-rank quantile of unsorted samples, reordering them in place.
fn select(samples: &mut [u64], q: f64) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    let rank = ((q * samples.len() as f64).ceil() as usize).clamp(1, samples.len());
    *samples.select_nth_unstable(rank - 1).1
}

/// A relative change, second over first minus one, and its interval.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Change {
    pub point: f64,
    pub lo: f64,
    pub hi: f64,
}

pub const RESAMPLES: usize = 4_000;

/// Fixed so the same two files always print the same interval.
pub const SEED: u64 = 0x5EED_0F_BE7C;

const ALPHA: f64 = 0.05;

/// The relative change of the `q` quantile from `a` to `b` and its 95 %
/// percentile bootstrap interval: each side drawn again with replacement on
/// its own, `resamples` times. Either side empty, or a first quantile of 0,
/// gives no change.
pub fn quantile_change(a: &[u64], b: &[u64], q: f64, resamples: usize, seed: u64) -> Change {
    let ratio = |qa: u64, qb: u64| if qa == 0 { 0.0 } else { qb as f64 / qa as f64 - 1.0 };
    if a.is_empty() || b.is_empty() || resamples == 0 {
        return Change::default();
    }
    let (mut sa, mut sb) = (a.to_vec(), b.to_vec());
    sa.sort_unstable();
    sb.sort_unstable();
    let point = ratio(quantile(&sa, q), quantile(&sb, q));
    let mut rng = SplitMix64::new(seed);
    let (mut ra, mut rb) = (vec![0; a.len()], vec![0; b.len()]);
    let mut changes: Vec<f64> = (0..resamples)
        .map(|_| {
            for x in ra.iter_mut() {
                *x = a[(rng.next_u64() % a.len() as u64) as usize];
            }
            for x in rb.iter_mut() {
                *x = b[(rng.next_u64() % b.len() as u64) as usize];
            }
            ratio(select(&mut ra, q), select(&mut rb, q))
        })
        .collect();
    changes.sort_by(f64::total_cmp);
    let lo = changes[((ALPHA / 2.0) * resamples as f64).floor() as usize];
    let hi = changes[((1.0 - ALPHA / 2.0) * resamples as f64).ceil() as usize - 1];
    Change { point, lo, hi }
}

/// Two-sided p-value of the Mann-Whitney U test: whether a sample from one
/// side tends to be larger than one from the other. The normal approximation
/// with the tie correction and a continuity correction of one half, which at
/// the bench's tens to hundreds of samples a side is close to exact.
pub fn mann_whitney_p(a: &[u64], b: &[u64]) -> f64 {
    let (na, nb) = (a.len() as f64, b.len() as f64);
    if a.is_empty() || b.is_empty() {
        return 1.0;
    }
    let mut all: Vec<(u64, bool)> = a.iter().map(|&x| (x, true)).chain(b.iter().map(|&x| (x, false))).collect();
    all.sort_unstable_by_key(|&(x, _)| x);
    let n = all.len();
    let (mut rank_sum_a, mut ties) = (0.0, 0.0);
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j < n && all[j].0 == all[i].0 {
            j += 1;
        }
        let t = (j - i) as f64;
        // Ranks i+1 to j, averaged over the tie.
        let rank = (i + 1 + j) as f64 / 2.0;
        rank_sum_a += rank * all[i..j].iter().filter(|(_, from_a)| *from_a).count() as f64;
        ties += t * t * t - t;
        i = j;
    }
    let u = rank_sum_a - na * (na + 1.0) / 2.0;
    let mean = na * nb / 2.0;
    let total = na + nb;
    let variance = na * nb / 12.0 * ((total + 1.0) - ties / (total * (total - 1.0)));
    if variance <= 0.0 {
        return 1.0;
    }
    let z = ((u - mean).abs() - 0.5).max(0.0) / variance.sqrt();
    erfc(z / std::f64::consts::SQRT_2).min(1.0)
}

/// The complementary error function, from the Chebyshev fit of Numerical
/// Recipes (`erfcc`): a relative error under 1.2e-7 everywhere.
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let poly = -z * z - 1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07 + t * (-1.135_203_98 + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    let r = t * poly.exp();
    if x >= 0.0 {
        r
    } else {
        2.0 - r
    }
}

/// SplitMix64, as `rag-eval` and `golden-mine` copy it: a fixed sequence
/// for a seed, whatever version of `rand`.
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

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn a_quantile_is_the_nearest_rank() {
        let sorted: Vec<u64> = (1..=100).collect();
        assert_eq!(quantile(&sorted, 0.50), 50);
        assert_eq!(quantile(&sorted, 0.95), 95);
        assert_eq!(quantile(&sorted, 1.0), 100);
        assert_eq!(quantile(&[3, 9], 0.5), 3);
        assert_eq!(quantile(&[], 0.5), 0);
        let mut unsorted = vec![9, 1, 5, 3, 7];
        assert_eq!(select(&mut unsorted, 0.5), 5);
    }

    #[test]
    fn erfc_matches_known_values() {
        assert!(close(erfc(0.0), 1.0, 1e-7));
        assert!(close(erfc(1.0), 0.157_299_207, 1e-7));
        assert!(close(erfc(2.0), 0.004_677_735, 1e-8));
        assert!(close(erfc(-1.0), 1.842_700_793, 1e-7));
    }

    #[test]
    fn mann_whitney_matches_a_value_computed_by_hand() {
        // U = 0, mean 12.5, variance 25 * 11 / 12, z = 12 / 4.7871 = 2.5067.
        let p = mann_whitney_p(&[1, 2, 3, 4, 5], &[6, 7, 8, 9, 10]);
        assert!(close(p, 0.012_186, 1e-5), "{p}");
        assert_eq!(p, mann_whitney_p(&[6, 7, 8, 9, 10], &[1, 2, 3, 4, 5]));
    }

    #[test]
    fn mann_whitney_sees_no_difference_between_equal_samples() {
        let a: Vec<u64> = (0..200).map(|i| 1_000 + (i * 37) % 500).collect();
        assert_eq!(mann_whitney_p(&a, &a), 1.0);
        assert_eq!(mann_whitney_p(&[5; 30], &[5; 30]), 1.0);
        assert_eq!(mann_whitney_p(&[], &a), 1.0);
    }

    #[test]
    fn mann_whitney_sees_a_shift_of_the_whole_distribution() {
        let a: Vec<u64> = (0..200).map(|i| 1_000 + (i * 37) % 500).collect();
        let b: Vec<u64> = a.iter().map(|x| x + 200).collect();
        assert!(mann_whitney_p(&a, &b) < 1e-6);
    }

    #[test]
    fn the_bootstrap_is_deterministic_for_a_seed() {
        let a: Vec<u64> = (0..100).map(|i| 1_000 + (i * 37) % 500).collect();
        let b: Vec<u64> = (0..100).map(|i| 1_050 + (i * 53) % 500).collect();
        let first = quantile_change(&a, &b, 0.95, 1_000, 7);
        assert_eq!(first, quantile_change(&a, &b, 0.95, 1_000, 7));
        assert_ne!(first, quantile_change(&a, &b, 0.95, 1_000, 8));
    }

    #[test]
    fn the_same_distribution_gives_an_interval_around_zero() {
        let a: Vec<u64> = (0..300).map(|i| 1_000 + (i * 37) % 500).collect();
        let change = quantile_change(&a, &a, 0.95, RESAMPLES, SEED);
        assert_eq!(change.point, 0.0);
        assert!(change.lo < 0.0 && change.hi > 0.0, "{change:?}");
    }

    #[test]
    fn a_slower_run_gives_an_interval_above_zero() {
        let a: Vec<u64> = (0..300).map(|i| 1_000 + (i * 37) % 500).collect();
        let b: Vec<u64> = a.iter().map(|x| x * 3 / 2).collect();
        let change = quantile_change(&a, &b, 0.95, RESAMPLES, SEED);
        assert!(close(change.point, 0.5, 0.01), "{change:?}");
        assert!(change.lo > 0.2, "{change:?}");
    }

    #[test]
    fn an_empty_side_or_a_zero_quantile_is_no_change() {
        assert_eq!(quantile_change(&[], &[1, 2], 0.5, 100, SEED), Change::default());
        assert_eq!(quantile_change(&[0, 0, 0], &[5, 5, 5], 0.5, 100, SEED), Change::default());
    }
}
