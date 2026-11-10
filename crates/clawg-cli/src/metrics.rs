//! `clawg metrics report` and `clawg metrics sessions`: read `metrics.db`
//! without the daemon and without the store's lock, as `export` reads
//! `memory.db`.

use std::path::Path;
use std::time::Duration;

use clawg_core::metrics::report::{report as group, since_for, By, Rows};
use clawg_core::metrics::session_report::{self, SessionsBy as GroupBy};
use clawg_core::metrics::store::{MetricsDb, MetricsError, METRICS_FILE};

use clawg_core::preferences::Lang;

use crate::cli::{MetricsAction, ReportBy, SessionsBy};
use crate::error::CliError;

/// `clawg metrics report`: the report of `--metric`, or with `--events` the
/// events of each run. `clawg metrics sessions`: the sessions by group, or
/// with `--session` one of them.
pub fn command(lang: Lang, base_dir: &Path, action: &MetricsAction, now_ms: i64) -> Result<String, CliError> {
    match action {
        MetricsAction::Report { metric: Some(metric), events: false, labels, by, since, .. } => {
            report(lang, base_dir, metric, labels, *by, *since, now_ms)
        }
        MetricsAction::Report { since, .. } => events(lang, base_dir, *since, now_ms),
        MetricsAction::Sessions { by, since, session, .. } => sessions(lang, base_dir, *by, *since, session.as_deref(), now_ms),
    }
    .map_err(CliError::Failed)
}

/// The sessions `base_dir`'s `metrics.db` holds, as pretty JSON: those heard
/// of since `since` ago summed by `by`, or with `session` every row of that
/// one. Projects are named by id; the file has no names and `memory.db` is
/// not opened for them.
pub fn sessions(lang: Lang, base_dir: &Path, by: SessionsBy, since: Duration, session: Option<&str>, now_ms: i64) -> Result<String, String> {
    let (db, failed) = open(lang, base_dir)?;
    let printed = match session {
        Some(session_id) => {
            let rows = db.sessions_of(session_id).map_err(&failed)?;
            serde_json::to_string_pretty(&session_report::one_session(session_id, &rows, now_ms))
        }
        None => {
            let by = match by {
                SessionsBy::Project => GroupBy::Project,
                SessionsBy::Agent => GroupBy::Agent,
                SessionsBy::Day => GroupBy::Day,
            };
            let since_ms = now_ms.saturating_sub(i64::try_from(since.as_millis()).unwrap_or(i64::MAX));
            let rows = db.sessions_since(since_ms).map_err(&failed)?;
            serde_json::to_string_pretty(&session_report::report(&rows, by, since_ms, now_ms))
        }
    };
    printed.map_err(|e| e.to_string())
}

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
    let (db, failed) = open(lang, base_dir)?;
    let path = base_dir.join(METRICS_FILE);
    let by = match by {
        ReportBy::Day => By::Day,
        ReportBy::Hour => By::Hour,
        ReportBy::Version => By::Version,
        ReportBy::Minute => By::Minute,
    };
    let since_ms = since_for(by, now_ms.saturating_sub(i64::try_from(since.as_millis()).unwrap_or(i64::MAX)), now_ms);

    let names = db.names().map_err(&failed)?;
    if !names.iter().any(|n| n == metric) {
        let (path, names) = (path.display(), names.join(", "));
        return Err(match lang {
            Lang::En => format!("{path} has no metric named {metric:?}; it has {names}"),
            Lang::Es => format!("{path} no tiene ninguna métrica llamada {metric:?}; tiene {names}"),
        });
    }
    let hists = db.hist_rows_since(metric, since_ms).map_err(&failed)?;
    let rows = if hists.is_empty() {
        let gauges = db.gauge_rows_since(metric, since_ms).map_err(&failed)?;
        if gauges.is_empty() && is_histogram(&db, metric).map_err(&failed)? {
            Rows::Histograms(hists)
        } else {
            Rows::Gauges(gauges)
        }
    } else {
        Rows::Histograms(hists)
    };
    let runs = db.runs_since(since_ms).map_err(&failed)?;
    let report = group(metric, rows, &runs, labels, by, since_ms);
    serde_json::to_string_pretty(&report).map_err(|e| e.to_string())
}

/// `metrics.db` of `base_dir` open to read, and how a failure to read it is
/// said in `lang`; or why it is not there.
fn open(lang: Lang, base_dir: &Path) -> Result<(MetricsDb, impl Fn(MetricsError) -> String), String> {
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
    let failed = move |e: MetricsError| match lang {
        Lang::En => format!("could not read {}: {e}", path.display()),
        Lang::Es => format!("no se pudo leer {}: {e}", path.display()),
    };
    let db = MetricsDb::open_read_only(&base_dir.join(METRICS_FILE)).map_err(&failed)?;
    Ok((db, failed))
}

/// The events of `base_dir`'s `metrics.db` from `since` ago, by run, as
/// pretty JSON: each run with its version, start and stop, and its events
/// oldest first.
pub fn events(lang: Lang, base_dir: &Path, since: Duration, now_ms: i64) -> Result<String, String> {
    let (db, failed) = open(lang, base_dir)?;
    let since_ms = now_ms.saturating_sub(i64::try_from(since.as_millis()).unwrap_or(i64::MAX));
    let events = db.events_since(since_ms).map_err(&failed)?;
    let runs = db.runs_since(i64::MIN / 2).map_err(&failed)?;
    let listed: Vec<serde_json::Value> = runs
        .iter()
        .filter_map(|run| {
            let of_run: Vec<serde_json::Value> = events
                .iter()
                .filter(|e| e.run_id == run.id)
                .map(|e| serde_json::json!({ "at_ms": e.at_ms, "kind": e.kind, "detail": e.detail }))
                .collect();
            (!of_run.is_empty()).then(|| {
                serde_json::json!({
                    "run": run.id,
                    "version": run.version,
                    "started_at_ms": run.started_at_ms,
                    "stopped_at_ms": run.stopped_at_ms,
                    "events": of_run,
                })
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({ "since_ms": since_ms, "runs": listed })).map_err(|e| e.to_string())
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
    use clawg_core::metrics::{EventKind, Recorder, Unit};
    use serde_json::Value;

    use crate::cli::{ProfileArg, SessionsBy};

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
    fn the_flags_choose_the_report_or_the_events() {
        let dir = tempfile::tempdir().unwrap();
        seeded(dir.path());
        let day = Duration::from_secs(86_400);
        let asked = |metric: Option<&str>, events: bool| MetricsAction::Report {
            metric: metric.map(str::to_owned),
            events,
            labels: vec![],
            by: ReportBy::Day,
            since: day,
            profile: ProfileArg::default(),
        };

        let of_metric = command(Lang::En, dir.path(), &asked(Some("http.duration"), false), NOW).unwrap();
        let of_events = command(Lang::En, dir.path(), &asked(None, true), NOW).unwrap();

        assert_eq!(of_metric, report(Lang::En, dir.path(), "http.duration", &[], ReportBy::Day, day, NOW).unwrap());
        assert_eq!(of_events, events(Lang::En, dir.path(), day, NOW).unwrap());
        let missing = tempfile::tempdir().unwrap();
        let err = command(Lang::Es, missing.path(), &asked(None, true), NOW).unwrap_err();
        assert_eq!(err.to_string(), events(Lang::Es, missing.path(), day, NOW).unwrap_err());
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
    fn the_events_are_listed_by_run() {
        let dir = tempfile::tempdir().unwrap();
        seeded(dir.path());
        let mut db = MetricsDb::open(&dir.path().join(METRICS_FILE), NOW).unwrap();
        let event = |at_ms, kind| clawg_core::metrics::Event {
            at_ms,
            kind,
            detail: std::collections::BTreeMap::from([("ms", 5u64.into())]),
        };
        db.write_events(1, &[event(NOW - 10 * 86_400_000, EventKind::Backup), event(NOW - 500, EventKind::Shutdown)]).unwrap();
        drop(db);

        let json: Value = serde_json::from_str(&events(Lang::En, dir.path(), Duration::from_secs(86_400), NOW).unwrap()).unwrap();
        let runs = json["runs"].as_array().unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0]["version"], "0.49.0");
        assert_eq!(runs[0]["stopped_at_ms"], Value::Null);
        assert_eq!(runs[0]["events"], serde_json::json!([{ "at_ms": NOW - 500, "kind": "shutdown", "detail": { "ms": 5 } }]));

        let older: Value = serde_json::from_str(&events(Lang::En, dir.path(), Duration::from_secs(30 * 86_400), NOW).unwrap()).unwrap();
        assert_eq!(older["runs"][0]["events"].as_array().map(Vec::len), Some(2));
        let empty = tempfile::tempdir().unwrap();
        assert!(events(Lang::Es, empty.path(), Duration::from_secs(60), NOW).unwrap_err().contains("no existe"));
    }

    /// Three rows in `dir`'s file: a session of project `p` that ended and
    /// went on in a second row, and one without a project, ten days old.
    fn seeded_sessions(dir: &Path) {
        use clawg_core::metrics::sessions::{CountKind, Pending, SessionRow};
        let row = |session_id: &str, project: Option<&str>, started_at_ms: i64, last_seen_at_ms: i64| SessionRow {
            session_id: session_id.into(),
            started_at_ms,
            agent_id: "claude-code".into(),
            project_id: project.map(String::from),
            client: "claude-code".into(),
            utc_offset_s: 0,
            last_seen_at_ms,
            last_save_at_ms: None,
            ended_at_ms: None,
            end_reason: None,
            prompts: 2,
            empty_searches: 0,
            counts: Default::default(),
        };
        let hour = 3_600_000;
        let mut first = row("s-1", Some("p"), NOW - 3 * hour, NOW - 2 * hour);
        first.ended_at_ms = Some(NOW - 2 * hour);
        first.end_reason = Some("clear".into());
        first.counts.insert((CountKind::Call, "search_memory".into()), 3);
        first.counts.insert((CountKind::Save, "session_summary".into()), 1);
        let again = row("s-1", Some("p"), NOW - hour, NOW - 60_000);
        let old = row("s-old", None, NOW - 10 * 24 * hour, NOW - 10 * 24 * hour);
        let mut db = MetricsDb::open(&dir.join(METRICS_FILE), NOW).unwrap();
        db.write_sessions(&Pending { changes: Vec::new(), rows: vec![first, again, old] }).unwrap();
    }

    #[test]
    fn the_sessions_report_reads_the_file_without_a_daemon() {
        let dir = tempfile::tempdir().unwrap();
        seeded_sessions(dir.path());
        let week = Duration::from_secs(7 * 86_400);
        let asked = |by: SessionsBy, since: Duration| -> Value {
            let action = MetricsAction::Sessions { by, since, session: None, profile: ProfileArg::default() };
            serde_json::from_str(&command(Lang::En, dir.path(), &action, NOW).unwrap()).unwrap()
        };

        let by_project = asked(SessionsBy::Project, week);
        assert_eq!((&by_project["by"], &by_project["since_ms"]), (&serde_json::json!("project"), &serde_json::json!(NOW - 7 * 86_400_000)));
        let groups = by_project["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 1, "the session of ten days ago is outside the week: {by_project}");
        let p = &groups[0];
        assert_eq!((&p["key"], &p["sessions"], &p["open"], &p["prompts"]), (&serde_json::json!("p"), &serde_json::json!(2), &serde_json::json!(1), &serde_json::json!(4)));
        assert_eq!(p["ended"], serde_json::json!({ "clear": 1 }));
        assert_eq!(p["calls"], serde_json::json!({ "search_memory": { "count": 3, "failed": 0 } }));
        assert_eq!((&p["with_summary"], &p["with_summary_pct"]), (&serde_json::json!(1), &serde_json::json!(50.0)));

        let month = asked(SessionsBy::Project, Duration::from_secs(30 * 86_400));
        let none = &month["groups"][0];
        assert_eq!((&none["key"], &none["ended"]), (&Value::Null, &serde_json::json!({ "idle": 1 })), "{month}");
        assert_eq!(asked(SessionsBy::Agent, week)["groups"][0]["key"], "claude-code");
        assert_eq!(asked(SessionsBy::Day, week)["by"], "day");

        let empty = tempfile::tempdir().unwrap();
        for (lang, said) in [(Lang::En, "CLAWG_METRICS=off"), (Lang::Es, "no existe")] {
            let err = sessions(lang, empty.path(), SessionsBy::Project, week, None, NOW).unwrap_err();
            assert!(err.contains(said), "{err}");
        }
        assert!(!empty.path().join(METRICS_FILE).exists(), "reading creates nothing");
        seeded(empty.path());
        assert_eq!(asked_in(empty.path(), week)["groups"], serde_json::json!([]), "a file without sessions has no groups");
    }

    fn asked_in(dir: &Path, since: Duration) -> Value {
        serde_json::from_str(&sessions(Lang::En, dir, SessionsBy::Project, since, None, NOW).unwrap()).unwrap()
    }

    #[test]
    fn one_session_is_listed_with_each_of_its_rows() {
        let dir = tempfile::tempdir().unwrap();
        seeded_sessions(dir.path());
        let one = |id: &str| -> Value {
            serde_json::from_str(&sessions(Lang::En, dir.path(), SessionsBy::Project, Duration::from_secs(3600), Some(id), NOW).unwrap()).unwrap()
        };

        let listed = one("s-1");
        let rows = listed["rows"].as_array().unwrap();
        assert_eq!((listed["session_id"].as_str(), rows.len()), (Some("s-1"), 2));
        assert_eq!((&rows[0]["end_reason"], &rows[0]["duration_ms"]), (&serde_json::json!("clear"), &serde_json::json!(3_600_000)));
        assert_eq!(rows[0]["saves"], serde_json::json!({ "session_summary": 1 }));
        assert_eq!((&rows[1]["end_reason"], &rows[1]["ended_at_ms"]), (&Value::Null, &Value::Null), "the second row is open");
        assert_eq!(one("s-old")["rows"][0]["end_reason"], "idle", "an age does not narrow one session");
        assert_eq!(one("nobody")["rows"], serde_json::json!([]));
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
