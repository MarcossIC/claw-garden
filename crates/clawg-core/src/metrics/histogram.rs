//! A histogram with fixed bucket bounds, so two of them add up.
//!
//! The bounds depend only on the unit, never on the samples: the histogram of
//! a day, an hour of the day or a version is the sum of its minutes, and any
//! percentile comes out of that sum within the width of one bucket.

/// What a histogram counts, which fixes its bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Unit {
    Micros,
    Bytes,
}

/// Upper bounds of the duration buckets, in microseconds. They hold the ones
/// OpenTelemetry recommends for `http.server.request.duration`, with finer
/// ones below a millisecond, where a request over the socket answers.
const MICROS_BOUNDS: [u64; 23] = [
    50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 25_000, 50_000, 75_000, 100_000, 250_000, 500_000, 750_000,
    1_000_000, 2_500_000, 5_000_000, 7_500_000, 10_000_000, 30_000_000, 60_000_000, 300_000_000,
];

const KIB: u64 = 1024;

/// Upper bounds of the size buckets, in bytes.
const BYTES_BOUNDS: [u64; 11] =
    [256, KIB, 4 * KIB, 16 * KIB, 64 * KIB, 256 * KIB, KIB * KIB, 4 * KIB * KIB, 16 * KIB * KIB, 64 * KIB * KIB, 256 * KIB * KIB];

impl Unit {
    /// The upper bound of every bucket but the last, which is open.
    pub fn bounds(self) -> &'static [u64] {
        match self {
            Unit::Micros => &MICROS_BOUNDS,
            Unit::Bytes => &BYTES_BOUNDS,
        }
    }

    /// How the unit is written in `metrics.db` and in the API.
    pub fn as_str(self) -> &'static str {
        match self {
            Unit::Micros => "us",
            Unit::Bytes => "bytes",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "us" => Some(Unit::Micros),
            "bytes" => Some(Unit::Bytes),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a histogram in {0:?} cannot be added to one in {1:?}")]
pub struct UnitMismatch(pub Unit, pub Unit);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the stored buckets do not fit the bounds of their unit")]
pub struct BadBuckets;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Histogram {
    unit: Unit,
    buckets: Vec<u64>,
    count: u64,
    sum: u64,
    min: u64,
    max: u64,
}

impl Histogram {
    pub fn new(unit: Unit) -> Self {
        Self { unit, buckets: vec![0; unit.bounds().len() + 1], count: 0, sum: 0, min: 0, max: 0 }
    }

    /// A histogram read back from its parts, as [`Histogram::encode_buckets`]
    /// wrote them.
    pub fn from_parts(unit: Unit, buckets: &[u8], sum: u64, min: u64, max: u64) -> Result<Self, BadBuckets> {
        let len = unit.bounds().len() + 1;
        if buckets.len() != len * 8 {
            return Err(BadBuckets);
        }
        let buckets: Vec<u64> = buckets
            .chunks_exact(8)
            .map(|b| u64::from_le_bytes(b.try_into().expect("chunks of eight")))
            .collect();
        let count = buckets.iter().fold(0u64, |acc, n| acc.saturating_add(*n));
        Ok(Self { unit, buckets, count, sum, min, max })
    }

    pub fn record(&mut self, value: u64) {
        let i = self.unit.bounds().partition_point(|&bound| bound < value);
        self.buckets[i] += 1;
        if self.count == 0 {
            self.min = value;
            self.max = value;
        } else {
            self.min = self.min.min(value);
            self.max = self.max.max(value);
        }
        self.count += 1;
        self.sum = self.sum.saturating_add(value);
    }

    /// Adds `other` in, as if its samples had been recorded here.
    pub fn merge(&mut self, other: &Histogram) -> Result<(), UnitMismatch> {
        if self.unit != other.unit {
            return Err(UnitMismatch(self.unit, other.unit));
        }
        if other.count == 0 {
            return Ok(());
        }
        for (mine, theirs) in self.buckets.iter_mut().zip(&other.buckets) {
            *mine = mine.saturating_add(*theirs);
        }
        if self.count == 0 {
            self.min = other.min;
            self.max = other.max;
        } else {
            self.min = self.min.min(other.min);
            self.max = self.max.max(other.max);
        }
        self.count = self.count.saturating_add(other.count);
        self.sum = self.sum.saturating_add(other.sum);
        Ok(())
    }

    /// The `q` quantile (0 to 1), interpolated inside the bucket that holds
    /// the sample of rank `ceil(q * count)` and kept within `min` and `max`.
    /// `None` when nothing was recorded.
    pub fn quantile(&self, q: f64) -> Option<u64> {
        if self.count == 0 {
            return None;
        }
        let rank = ((q.clamp(0.0, 1.0) * self.count as f64).ceil() as u64).max(1);
        let bounds = self.unit.bounds();
        let mut below = 0u64;
        for (i, &n) in self.buckets.iter().enumerate() {
            if n == 0 || below + n < rank {
                below += n;
                continue;
            }
            let lower = if i == 0 { 0 } else { bounds[i - 1] };
            let upper = bounds.get(i).copied().unwrap_or(self.max);
            let lower = lower.max(self.min);
            let upper = upper.min(self.max).max(lower);
            let within = (rank - below) as f64 / n as f64;
            return Some(lower + ((upper - lower) as f64 * within).round() as u64);
        }
        Some(self.max)
    }

    /// The bucket counts as little-endian `u64`s, how `metrics.db` keeps them.
    pub fn encode_buckets(&self) -> Vec<u8> {
        self.buckets.iter().flat_map(|n| n.to_le_bytes()).collect()
    }

    pub fn unit(&self) -> Unit {
        self.unit
    }

    pub fn buckets(&self) -> &[u64] {
        &self.buckets
    }

    pub fn count(&self) -> u64 {
        self.count
    }

    pub fn sum(&self) -> u64 {
        self.sum
    }

    pub fn min(&self) -> u64 {
        self.min
    }

    pub fn max(&self) -> u64 {
        self.max
    }

    pub fn mean(&self) -> Option<f64> {
        (self.count > 0).then(|| self.sum as f64 / self.count as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed sequence of durations spread over every order of magnitude
    /// the bounds cover, so the tests do not depend on a random generator.
    fn samples(seed: u64, n: usize) -> Vec<u64> {
        let mut state = seed;
        (0..n)
            .map(|_| {
                state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                let magnitude = (state >> 60) as u32 % 9;
                (state >> 20) % 10u64.pow(magnitude + 1)
            })
            .collect()
    }

    fn of(unit: Unit, values: &[u64]) -> Histogram {
        let mut h = Histogram::new(unit);
        for v in values {
            h.record(*v);
        }
        h
    }

    fn exact(values: &[u64], q: f64) -> u64 {
        let mut sorted = values.to_vec();
        sorted.sort_unstable();
        let rank = ((q * sorted.len() as f64).ceil() as usize).max(1);
        sorted[rank - 1]
    }

    /// The bucket bounds `(lower, upper]` that hold `value`.
    fn bucket_of(unit: Unit, value: u64) -> (u64, u64) {
        let bounds = unit.bounds();
        let i = bounds.partition_point(|&b| b < value);
        (if i == 0 { 0 } else { bounds[i - 1] }, bounds.get(i).copied().unwrap_or(u64::MAX))
    }

    #[test]
    fn merging_two_histograms_is_the_histogram_of_the_union() {
        let (a, b) = (samples(1, 700), samples(2, 300));
        let mut merged = of(Unit::Micros, &a);
        merged.merge(&of(Unit::Micros, &b)).expect("same unit");

        let union: Vec<u64> = a.iter().chain(&b).copied().collect();
        let whole = of(Unit::Micros, &union);
        assert_eq!(merged, whole);
        for q in [0.5, 0.95, 0.99] {
            assert_eq!(merged.quantile(q), whole.quantile(q));
        }
    }

    #[test]
    fn a_percentile_falls_in_the_bucket_of_the_exact_one() {
        let values = samples(3, 1000);
        let h = of(Unit::Micros, &values);
        for q in [0.01, 0.5, 0.9, 0.95, 0.99, 1.0] {
            let truth = exact(&values, q);
            let (lower, upper) = bucket_of(Unit::Micros, truth);
            let estimate = h.quantile(q).expect("samples");
            assert!(
                estimate >= lower && estimate <= upper,
                "p{q}: {estimate} outside ({lower}, {upper}] that holds the exact {truth}"
            );
            assert!(estimate >= h.min() && estimate <= h.max());
        }
    }

    #[test]
    fn a_single_value_is_every_percentile() {
        let h = of(Unit::Micros, &[1234]);
        assert_eq!(h.quantile(0.5), Some(1234));
        assert_eq!(h.quantile(0.99), Some(1234));
        assert_eq!(Histogram::new(Unit::Micros).quantile(0.5), None);
    }

    #[test]
    fn a_value_on_a_bound_belongs_to_the_bucket_it_closes() {
        let h = of(Unit::Micros, &[50, 51, 400_000_000]);
        assert_eq!(h.buckets()[0], 1);
        assert_eq!(h.buckets()[1], 1);
        assert_eq!(*h.buckets().last().unwrap(), 1, "past every bound is the open bucket");
    }

    #[test]
    fn histograms_of_different_units_do_not_merge() {
        let mut micros = of(Unit::Micros, &[1, 2]);
        let before = micros.clone();
        assert_eq!(micros.merge(&of(Unit::Bytes, &[3])), Err(UnitMismatch(Unit::Micros, Unit::Bytes)));
        assert_eq!(micros, before);
    }

    #[test]
    fn the_buckets_round_trip_through_bytes() {
        let h = of(Unit::Bytes, &samples(4, 200));
        let back = Histogram::from_parts(Unit::Bytes, &h.encode_buckets(), h.sum(), h.min(), h.max()).expect("valid");
        assert_eq!(back, h);
        assert_eq!(Histogram::from_parts(Unit::Micros, &h.encode_buckets(), 0, 0, 0), Err(BadBuckets));
        assert_eq!(Unit::parse(Unit::Bytes.as_str()), Some(Unit::Bytes));
        assert_eq!(Unit::parse(Unit::Micros.as_str()), Some(Unit::Micros));
    }
}
