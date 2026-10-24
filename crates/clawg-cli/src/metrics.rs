//! `clawg metrics report`: reads `metrics.db` without the daemon and without
//! the store's lock, as `export` reads `memory.db`.

use std::path::Path;
use std::time::Duration;

use clawg_core::metrics::report::{report as group, since_for, By, Rows};
use clawg_core::metrics::store::{MetricsDb, METRICS_FILE};

use clawg_core::preferences::Lang;

use crate::cli::ReportBy;

/// The report of `metric` in `base_dir`'s `metrics.db` as pretty JSON, or
/// why there is none, in `lang`. The JSON itself has no prose to translate.
pub fn report(
    lang: Lang,
    base_dir: &Path,
    metric: &str,
    labels: &[(String, String)],
    by: ReportBy,
    since: Duration,
    now_ms: i64,
) -> Result<String, String> {
    let path = base_dir.join(METRICS_FILE);
    if !path.is_file() {
        return Err(match lang {
            Lang::En => format!("there is no {}: the daemon writes it while it runs, unless CLAWG_METRICS=off", path.display()),
            Lang::Es => format!(
                "no existe {}: el daemon lo escribe mientras corre, salvo con CLAWG_METRICS=off",
                path.display()
            ),
        });
    }
    let failed = |e: clawg_core::metrics::store::MetricsError| match lang {
        Lang::En => format!("could not read {}: {e}", path.display()),
        Lang::Es => format!("no se pudo leer {}: {e}", path.display()),
    };
    let db = MetricsDb::open_read_only(&path).map_err(failed)?;
    let by = match by {
        ReportBy::Day => By::Day,
        ReportBy::Hour => By::Hour,
        ReportBy::Version => By::Version,
        ReportBy::Minute => By::Minute,
    };
    let since_ms = since_for(by, now_ms.saturating_sub(i64::try_from(since.as_millis()).unwrap_or(i64::MAX)), now_ms);

    let names = db.names().map_err(failed)?;
    if !names.iter().any(|n| n == metric) {
        let (path, names) = (path.display(), names.join(", "));
        return Err(match lang {
            Lang::En => format!("{path} has no metric named {metric:?}; it has {names}"),
            Lang::Es => format!("{path} no tiene ninguna métrica llamada {metric:?}; tiene {names}"),
        });
    }
    let hists = db.hist_rows_since(metric, since_ms).map_err(failed)?;
    let rows = if hists.is_empty() {
        let gauges = db.gauge_rows_since(metric, since_ms).map_err(failed)?;
        if gauges.is_empty() && is_histogram(&db, metric).map_err(failed)? {
            Rows::Histograms(hists)
        } else {
            Rows::Gauges(gauges)
        }
    } else {
        Rows::Histograms(hists)
    };
    let runs = db.runs_since(since_ms).map_err(failed)?;
    let report = group(metric, rows, &runs, labels, by, since_ms);
    serde_json::to_string_pretty(&report).map_err(|e| e.to_string())
}

/// Whether `metric` was ever stored as a histogram, so a window without rows
/// still reports the right kind.
fn is_histogram(db: &MetricsDb, metric: &str) -> Result<bool, clawg_core::metrics::store::MetricsError> {
    Ok(!db.hist_rows_since(metric, i64::MIN / 2)?.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawg_core::metrics::store::RunInfo;
    use clawg_core::metrics::{Recorder, Unit};
    use serde_json::Value;

    const NOW: i64 = 1_790_000_000_000;

    fn seeded(dir: &Path) {
        let mut db = MetricsDb::open(&dir.join(METRICS_FILE), NOW).unwrap();
        let run = db
            .start_run(&RunInfo {
                started_at_ms: NOW - 1000,
                version: "0.49.0".into(),
                dist_target: None,
                profile: "dev".into(),
                os: "linux".into(),
                arch: "x86_64".into(),
                cpus: 2,
                embedding_model: "m".into(),
                idle_minutes: Some(10),
                utc_offset_s: 0,
            })
            .unwrap();
        let r = Recorder::default();
        r.observe("http.duration", Unit::Micros, &[("route", "/v1/search")], 900);
        r.observe("http.duration", Unit::Micros, &[("route", "/health")], 90);
        r.gauge("process.rss_bytes", &[], 1024.0);
        db.write_minute(run, NOW - 1000, &r.take_minute()).unwrap();
    }

    #[test]
    fn a_report_reads_the_file_without_a_daemon() {
        let dir = tempfile::tempdir().unwrap();
        seeded(dir.path());
        let labels = [("route".to_string(), "/v1/search".to_string())];

        let out = report(Lang::En, dir.path(), "http.duration", &labels, ReportBy::Version, Duration::from_secs(3600), NOW).unwrap();

        let json: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(json["kind"], "histogram");
        assert_eq!(json["groups"][0]["key"], "0.49.0");
        assert_eq!(json["groups"][0]["count"], 1);
        assert_eq!(json["groups"][0]["not_stopped"], 1);
        assert_eq!(json["runs"]["total"], 1);
        let gauge: Value =
            serde_json::from_str(&report(Lang::En, dir.path(), "process.rss_bytes", &[], ReportBy::Day, Duration::from_secs(3600), NOW).unwrap())
                .unwrap();
        assert_eq!(gauge["kind"], "gauge");
        assert_eq!(gauge["groups"][0]["max"], 1024.0);
    }

    #[test]
    fn a_report_by_minute_reads_the_minutes_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        seeded(dir.path());

        let out = report(Lang::En, dir.path(), "process.rss_bytes", &[], ReportBy::Minute, Duration::from_secs(30 * 86_400), NOW).unwrap();

        let json: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(json["by"], "minute");
        assert_eq!(json["groups"][0]["key"], "2026-09-21 14:13");
        assert_eq!(json["since_ms"], NOW - clawg_core::metrics::store::MINUTES_KEPT_MS + 2_800_000, "{json}");
    }

    #[test]
    fn a_window_without_rows_keeps_the_kind_of_the_metric() {
        let dir = tempfile::tempdir().unwrap();
        seeded(dir.path());
        let later = NOW + 10 * 86_400_000;
        let json: Value =
            serde_json::from_str(&report(Lang::En, dir.path(), "http.duration", &[], ReportBy::Version, Duration::from_secs(3600), later).unwrap())
                .unwrap();
        assert_eq!(json["kind"], "histogram");
        assert_eq!(json["groups"], Value::Array(vec![]));
    }

    #[test]
    fn a_missing_file_or_metric_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let err = report(Lang::En, dir.path(), "http.duration", &[], ReportBy::Version, Duration::from_secs(60), NOW).unwrap_err();
        assert!(err.contains("metrics.db") && err.contains("CLAWG_METRICS=off"), "{err}");
        assert!(!dir.path().join(METRICS_FILE).exists(), "reading creates nothing");

        seeded(dir.path());
        let err = report(Lang::En, dir.path(), "http.durations", &[], ReportBy::Version, Duration::from_secs(60), NOW).unwrap_err();
        assert!(err.contains("\"http.durations\"") && err.contains("http.duration, process.rss_bytes"), "{err}");
    }

    #[test]
    fn a_missing_file_or_metric_is_named_in_spanish() {
        let dir = tempfile::tempdir().unwrap();
        let none = report(Lang::Es, dir.path(), "http.duration", &[], ReportBy::Version, Duration::from_secs(60), NOW).unwrap_err();
        assert!(none.starts_with("no existe ") && none.ends_with("salvo con CLAWG_METRICS=off"), "{none}");
        seeded(dir.path());
        let unknown = report(Lang::Es, dir.path(), "nope", &[], ReportBy::Version, Duration::from_secs(60), NOW).unwrap_err();
        assert!(unknown.contains("no tiene ninguna métrica llamada \"nope\"; tiene http.duration"), "{unknown}");
    }
}
