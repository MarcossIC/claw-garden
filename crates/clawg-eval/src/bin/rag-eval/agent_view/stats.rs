//! Aggregates per-case agent-view samples into the per-tool summary.

use serde::{Deserialize, Serialize};

/// One case's reading of one tool.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub found: bool,
    /// Every expected row was visible, not just one.
    pub all_found: bool,
    pub tokens: usize,
    pub latency_ms: f64,
}

/// Summary of one tool across a set of cases.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ToolStats {
    pub n_cases: usize,
    /// Share of cases where an expected row was visible in the output.
    pub hit_rate: f64,
    /// Share of cases where every expected row was visible. A case that
    /// expects two rows and shows one counts in `hit_rate` and not here.
    /// Results written before it was recorded read 0.
    #[serde(default)]
    pub all_hit_rate: f64,
    pub mean_tokens: f64,
    pub max_tokens: usize,
    pub latency_p50_ms: f64,
    pub latency_p99_ms: f64,
}

impl ToolStats {
    /// No case contributed to this summary.
    pub fn is_empty(&self) -> bool {
        self.n_cases == 0
    }
}

/// Nearest-rank percentile: the smallest value with at least `p` percent of
/// the samples at or below it. Always one of the inputs, never interpolated,
/// so a reported p99 is a latency that actually happened. `0.0` for no
/// samples, the same all-zero degradation the other aggregators use.
pub fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// Summarizes `samples`; all zeros when there are none.
pub fn summarize(samples: &[Sample]) -> ToolStats {
    let n = samples.len();
    if n == 0 {
        return ToolStats::default();
    }
    let latencies: Vec<f64> = samples.iter().map(|s| s.latency_ms).collect();
    ToolStats {
        n_cases: n,
        hit_rate: samples.iter().filter(|s| s.found).count() as f64 / n as f64,
        all_hit_rate: samples.iter().filter(|s| s.all_found).count() as f64 / n as f64,
        mean_tokens: samples.iter().map(|s| s.tokens).sum::<usize>() as f64 / n as f64,
        max_tokens: samples.iter().map(|s| s.tokens).max().unwrap_or(0),
        latency_p50_ms: percentile(&latencies, 50.0),
        latency_p99_ms: percentile(&latencies, 99.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_of_nothing_is_zero() {
        assert_eq!(percentile(&[], 50.0), 0.0);
        assert_eq!(percentile(&[], 99.0), 0.0);
    }

    #[test]
    fn percentile_of_one_value_is_that_value() {
        assert_eq!(percentile(&[7.5], 50.0), 7.5);
        assert_eq!(percentile(&[7.5], 99.0), 7.5);
    }

    #[test]
    fn percentile_of_one_to_a_hundred_is_the_nearest_rank() {
        let values: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&values, 50.0), 50.0);
        assert_eq!(percentile(&values, 99.0), 99.0);
        assert_eq!(percentile(&values, 100.0), 100.0);
    }

    #[test]
    fn percentile_does_not_depend_on_input_order() {
        assert_eq!(percentile(&[9.0, 1.0, 5.0, 3.0, 7.0], 50.0), 5.0);
    }

    #[test]
    fn percentile_is_always_an_input_value() {
        assert_eq!(percentile(&[10.0, 20.0], 50.0), 10.0);
        assert_eq!(percentile(&[10.0, 20.0], 51.0), 20.0);
    }

    #[test]
    fn summarize_of_nothing_is_all_zero() {
        assert_eq!(summarize(&[]), ToolStats::default());
    }

    #[test]
    fn summarize_reports_hit_rate_token_mean_and_max() {
        let stats = summarize(&[
            Sample { found: true, all_found: true, tokens: 100, latency_ms: 1.0 },
            Sample { found: false, all_found: false, tokens: 300, latency_ms: 2.0 },
            Sample { found: true, all_found: false, tokens: 200, latency_ms: 3.0 },
            Sample { found: true, all_found: true, tokens: 0, latency_ms: 4.0 },
        ]);
        assert_eq!(stats.n_cases, 4);
        assert_eq!(stats.hit_rate, 0.75);
        assert_eq!(stats.all_hit_rate, 0.5);
        assert_eq!(stats.mean_tokens, 150.0);
        assert_eq!(stats.max_tokens, 300);
        assert_eq!(stats.latency_p50_ms, 2.0);
        assert_eq!(stats.latency_p99_ms, 4.0);
    }

    #[test]
    fn a_summary_without_all_hit_rate_still_reads() {
        let old = r#"{"n_cases":1,"hit_rate":1.0,"mean_tokens":10.0,"max_tokens":10,"latency_p50_ms":1.0,"latency_p99_ms":1.0}"#;
        let stats: ToolStats = serde_json::from_str(old).expect("a summary from before the strict rate");
        assert_eq!(stats.all_hit_rate, 0.0);
    }
}
