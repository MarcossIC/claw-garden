//! What `clawg metrics report` prints: the stored rows of one metric, summed
//! by local day, by hour of the local day or by version.
//!
//! Pure over rows already read, so every grouping is tested without a file.

use std::collections::BTreeMap;

use serde::Serialize;

use super::histogram::Histogram;
use super::recorder::{parse_labels, Gauge};
use super::store::{GaugeRow, HistRow, RunRow};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum By {
    Day,
    Hour,
    Version,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    pub metric: String,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<&'static str>,
    pub by: By,
    pub since_ms: i64,
    pub runs: RunCount,
    pub groups: Vec<Group>,
}

/// Runs, and those of them without a recorded stop: killed, crashed or
/// still running.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct RunCount {
    pub total: usize,
    pub not_stopped: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Group {
    pub key: String,
    pub runs: usize,
    pub not_stopped: usize,
    #[serde(flatten)]
    pub stats: Stats,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Stats {
    Histogram { count: u64, mean: Option<f64>, min: u64, max: u64, p50: Option<u64>, p95: Option<u64>, p99: Option<u64> },
    Gauge { samples: u64, mean: f64, min: f64, max: f64 },
}

/// The rows of a metric, histograms or gauges.
pub enum Rows {
    Histograms(Vec<HistRow>),
    Gauges(Vec<GaugeRow>),
}

/// Groups `rows` by `by`, keeping only the series that hold every pair of
/// `labels`. The local time of a row is its run's (`utc_offset_s`).
pub fn report(metric: &str, rows: Rows, runs: &[RunRow], labels: &[(String, String)], by: By, since_ms: i64) -> Report {
    let run_of: BTreeMap<i64, &RunRow> = runs.iter().map(|r| (r.id, r)).collect();
    let keep = |series_labels: &str| {
        let pairs = parse_labels(series_labels);
        labels.iter().all(|(k, v)| pairs.contains(&(k.as_str(), v.as_str())))
    };
    let key_of = |run_id: i64, start_ms: i64| -> (i64, String) {
        let run = run_of.get(&run_id);
        let local_ms = start_ms + run.map_or(0, |r| r.utc_offset_s * 1000);
        match by {
            By::Day => {
                let (y, m, d) = crate::memory::backup::civil_from_days(local_ms.div_euclid(86_400_000));
                (0, format!("{y:04}-{m:02}-{d:02}"))
            }
            By::Hour => (0, format!("{:02}", local_ms.rem_euclid(86_400_000) / 3_600_000)),
            // Versions in the order they first ran, not as text.
            By::Version => (
                run.map_or(i64::MAX, |r| r.started_at_ms),
                run.map_or_else(|| "unknown".to_string(), |r| r.version.clone()),
            ),
        }
    };

    let mut run_ids: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    let (kind, unit, groups) = match rows {
        Rows::Histograms(rows) => {
            let unit = rows.first().map(|r| r.histogram.unit().as_str());
            let mut sums: BTreeMap<String, (i64, Histogram)> = BTreeMap::new();
            for row in rows.iter().filter(|r| keep(&r.key.labels)) {
                let (order, key) = key_of(row.run_id, row.start_ms);
                run_ids.entry(key.clone()).or_default().push(row.run_id);
                let entry = sums.entry(key).or_insert_with(|| (order, Histogram::new(row.histogram.unit())));
                entry.0 = entry.0.min(order);
                // Rows of one name share a unit; a stored row that does not
                // is left out rather than mixed in.
                let _ = entry.1.merge(&row.histogram);
            }
            let stats = sums
                .into_iter()
                .map(|(key, (order, h))| {
                    let stats = Stats::Histogram {
                        count: h.count(),
                        mean: h.mean(),
                        min: h.min(),
                        max: h.max(),
                        p50: h.quantile(0.5),
                        p95: h.quantile(0.95),
                        p99: h.quantile(0.99),
                    };
                    (order, key, stats)
                })
                .collect::<Vec<_>>();
            ("histogram", unit, stats)
        }
        Rows::Gauges(rows) => {
            let mut sums: BTreeMap<String, (i64, Option<Gauge>)> = BTreeMap::new();
            for row in rows.iter().filter(|r| keep(&r.key.labels)) {
                let (order, key) = key_of(row.run_id, row.start_ms);
                run_ids.entry(key.clone()).or_default().push(row.run_id);
                let entry = sums.entry(key).or_insert((order, None));
                entry.0 = entry.0.min(order);
                match &mut entry.1 {
                    Some(sum) => sum.merge(&row.gauge),
                    None => entry.1 = Some(row.gauge),
                }
            }
            let stats = sums
                .into_iter()
                .filter_map(|(key, (order, g))| {
                    let g = g?;
                    Some((order, key, Stats::Gauge { samples: g.count, mean: g.mean(), min: g.min, max: g.max }))
                })
                .collect::<Vec<_>>();
            ("gauge", None, stats)
        }
    };

    let mut groups: Vec<(i64, Group)> = groups
        .into_iter()
        .map(|(order, key, stats)| {
            let mut ids = run_ids.remove(&key).unwrap_or_default();
            ids.sort_unstable();
            ids.dedup();
            let not_stopped = ids.iter().filter(|id| run_of.get(id).is_some_and(|r| r.stopped_at_ms.is_none())).count();
            (order, Group { key, runs: ids.len(), not_stopped, stats })
        })
        .collect();
    groups.sort_by(|(a_order, a), (b_order, b)| a_order.cmp(b_order).then_with(|| a.key.cmp(&b.key)));

    let started: Vec<&RunRow> = runs.iter().filter(|r| r.started_at_ms >= since_ms).collect();
    Report {
        metric: metric.to_string(),
        kind,
        unit,
        by,
        since_ms,
        runs: RunCount { total: started.len(), not_stopped: started.iter().filter(|r| r.stopped_at_ms.is_none()).count() },
        groups: groups.into_iter().map(|(_, g)| g).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::histogram::Unit;
    use crate::metrics::recorder::SeriesKey;

    /// 2026-10-20 00:00 UTC, a Tuesday.
    const DAY0: i64 = 1_792_454_400_000;
    const HOUR: i64 = 3_600_000;

    fn run(id: i64, version: &str, started_at_ms: i64, stopped: bool, utc_offset_s: i64) -> RunRow {
        RunRow { id, started_at_ms, stopped_at_ms: stopped.then_some(started_at_ms + HOUR), version: version.into(), utc_offset_s }
    }

    fn hist(run_id: i64, start_ms: i64, labels: &str, values: &[u64]) -> HistRow {
        let mut histogram = Histogram::new(Unit::Micros);
        for v in values {
            histogram.record(*v);
        }
        HistRow { run_id, start_ms, key: SeriesKey { name: "http.duration".into(), labels: labels.into() }, histogram }
    }

    fn counts(report: &Report) -> Vec<(String, u64, usize, usize)> {
        report
            .groups
            .iter()
            .map(|g| match g.stats {
                Stats::Histogram { count, .. } => (g.key.clone(), count, g.runs, g.not_stopped),
                Stats::Gauge { samples, .. } => (g.key.clone(), samples, g.runs, g.not_stopped),
            })
            .collect()
    }

    #[test]
    fn a_report_groups_by_version_day_and_hour() {
        // Run 1 is three hours behind UTC: its 01:00 UTC row is 22:00 the day before.
        let runs = [run(1, "0.49.0", DAY0, true, -3 * 3600), run(2, "0.48.0", DAY0 - 24 * HOUR, false, 0)];
        let rows = || {
            Rows::Histograms(vec![
                hist(1, DAY0 + HOUR, "route=/v1/search", &[100, 200]),
                hist(1, DAY0 + 15 * HOUR, "route=/v1/search", &[300]),
                hist(2, DAY0 - 23 * HOUR, "route=/v1/search", &[400]),
            ])
        };

        let by_version = report("http.duration", rows(), &runs, &[], By::Version, DAY0 - 48 * HOUR);
        assert_eq!(by_version.kind, "histogram");
        assert_eq!(by_version.unit, Some("us"));
        assert_eq!(
            counts(&by_version),
            vec![("0.48.0".into(), 1, 1, 1), ("0.49.0".into(), 3, 1, 0)],
            "versions in the order they first ran"
        );
        assert_eq!(by_version.runs, RunCount { total: 2, not_stopped: 1 });

        let by_day = report("http.duration", rows(), &runs, &[], By::Day, 0);
        assert_eq!(counts(&by_day), vec![("2026-10-19".into(), 3, 2, 1), ("2026-10-20".into(), 1, 1, 0)]);

        let by_hour = report("http.duration", rows(), &runs, &[], By::Hour, 0);
        assert_eq!(counts(&by_hour), vec![("01".into(), 1, 1, 1), ("12".into(), 1, 1, 0), ("22".into(), 2, 1, 0)]);

        let Stats::Histogram { p50, max, .. } = &by_version.groups[1].stats else { panic!("a histogram") };
        assert_eq!(*max, 300);
        assert!(p50.is_some_and(|p| (100..=250).contains(&p)), "{p50:?}");
    }

    #[test]
    fn a_label_filter_keeps_only_matching_series() {
        let runs = [run(1, "0.49.0", DAY0, true, 0)];
        let rows = Rows::Histograms(vec![
            hist(1, DAY0, "route=/v1/search,transport=unix", &[1]),
            hist(1, DAY0, "route=/v1/search,transport=tcp", &[1, 2]),
            hist(1, DAY0, "route=/health,transport=tcp", &[1, 2, 3]),
        ]);
        let wanted = [("route".to_string(), "/v1/search".to_string()), ("transport".to_string(), "tcp".to_string())];

        let r = report("http.duration", rows, &runs, &wanted, By::Version, 0);

        assert_eq!(counts(&r), vec![("0.49.0".into(), 2, 1, 0)]);
    }

    #[test]
    fn a_gauge_report_gives_samples_and_extremes() {
        let runs = [run(1, "0.49.0", DAY0, false, 0)];
        let gauge = |count, sum, min, max| Gauge { count, sum, min, max, last: max };
        let row = |start_ms, g| GaugeRow { run_id: 1, start_ms, key: SeriesKey { name: "process.rss_bytes".into(), labels: String::new() }, gauge: g };
        let rows = Rows::Gauges(vec![row(DAY0, gauge(1, 10.0, 10.0, 10.0)), row(DAY0 + 60_000, gauge(1, 30.0, 30.0, 30.0))]);

        let r = report("process.rss_bytes", rows, &runs, &[], By::Version, 0);

        assert_eq!(r.kind, "gauge");
        assert_eq!(r.groups[0].stats, Stats::Gauge { samples: 2, mean: 20.0, min: 10.0, max: 30.0 });
        assert_eq!((r.groups[0].runs, r.groups[0].not_stopped), (1, 1));
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["groups"][0]["samples"], 2, "{json}");
        assert_eq!(json["by"], "version");
    }
}
