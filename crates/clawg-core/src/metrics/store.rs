//! `metrics.db`: the history of the daemon's metrics, in a file of its own.
//!
//! Never inside `memory.db`: it would take the store's only connection, travel
//! in its snapshots and change with `restore` or `purge`. Losing this file
//! loses only the history of the metrics, so a file SQLite does not recognise
//! is set aside and started again instead of stopping the daemon.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, ErrorCode, OpenFlags, OptionalExtension};

use super::histogram::{Histogram, Unit};
use super::event::{Event, EventRow};
use super::recorder::{Gauge, SeriesKey, Window};

pub const METRICS_FILE: &str = "metrics.db";

/// 2 added `event`, which a file of 1 gets when it is opened to write.
const SCHEMA_VERSION: i64 = 2;

const MINUTE_MS: i64 = 60_000;
const HOUR_MINUTES: i64 = 60;
const DAY_MS: i64 = 86_400_000;

/// How long the minutes are kept before they fold into their hour.
pub const MINUTES_KEPT_MS: i64 = 14 * DAY_MS;

/// How long the hours, and the runs left without any, are kept.
pub const HOURS_KEPT_MS: i64 = 400 * DAY_MS;

#[derive(Debug, thiserror::Error)]
pub enum MetricsError {
    #[error("{0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("metrics.db has schema {0}, newer than this clawg knows ({SCHEMA_VERSION})")]
    Newer(i64),
    #[error("a stored histogram of {0} does not fit its unit")]
    BadRow(String),
}

/// What a run records about itself when it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunInfo {
    pub started_at_ms: i64,
    pub version: String,
    pub dist_target: Option<String>,
    pub profile: String,
    pub os: String,
    pub arch: String,
    pub cpus: i64,
    pub embedding_model: String,
    pub idle_minutes: Option<i64>,
    /// Local time minus UTC when the run started, in seconds.
    pub utc_offset_s: i64,
}

/// A run as `report` reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRow {
    pub id: i64,
    pub started_at_ms: i64,
    pub stopped_at_ms: Option<i64>,
    pub version: String,
    pub utc_offset_s: i64,
}

/// A stored histogram of a minute or an hour, with the UTC start of its
/// period.
#[derive(Debug, Clone, PartialEq)]
pub struct HistRow {
    pub run_id: i64,
    pub start_ms: i64,
    pub key: SeriesKey,
    pub histogram: Histogram,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GaugeRow {
    pub run_id: i64,
    pub start_ms: i64,
    pub key: SeriesKey,
    pub gauge: Gauge,
}

/// What [`MetricsDb::prune`] changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pruned {
    pub folded_minutes: usize,
    pub dropped_hours: usize,
    pub dropped_events: usize,
    pub dropped_runs: usize,
}

pub struct MetricsDb {
    conn: Connection,
}

/// The two kinds of period a table holds.
#[derive(Clone, Copy)]
enum Period {
    Minute,
    Hour,
}

impl Period {
    fn hist_table(self) -> &'static str {
        match self {
            Period::Minute => "hist_minute",
            Period::Hour => "hist_hour",
        }
    }

    fn gauge_table(self) -> &'static str {
        match self {
            Period::Minute => "gauge_minute",
            Period::Hour => "gauge_hour",
        }
    }

    fn ms(self) -> i64 {
        match self {
            Period::Minute => MINUTE_MS,
            Period::Hour => MINUTE_MS * HOUR_MINUTES,
        }
    }
}

impl MetricsDb {
    /// Opens `path` to write, creating it (owner-only on Unix) with its
    /// schema. A file SQLite does not recognise, or finds corrupt, is renamed
    /// to `metrics.db.corrupt-<ms>` and a new one is created.
    pub fn open(path: &Path, now_ms: i64) -> Result<Self, MetricsError> {
        match Self::open_writable(path) {
            Err(MetricsError::Sqlite(e)) if is_unreadable(&e) => {
                let aside = set_aside_path(path, now_ms);
                std::fs::rename(path, &aside)?;
                log::warn!(
                    "{} was not a readable SQLite file ({e}); kept it as {} and started a new one",
                    path.display(),
                    aside.display()
                );
                Self::open_writable(path)
            }
            other => other,
        }
    }

    /// Opens `path` to read, without creating or changing anything.
    pub fn open_read_only(path: &Path) -> Result<Self, MetricsError> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(MetricsError::Newer(version));
        }
        Ok(Self { conn })
    }

    fn open_writable(path: &Path) -> Result<Self, MetricsError> {
        create_private(path)?;
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(MetricsError::Newer(version));
        }
        conn.execute_batch(SCHEMA)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(Self { conn })
    }

    pub fn start_run(&mut self, run: &RunInfo) -> Result<i64, MetricsError> {
        self.conn.execute(
            "INSERT INTO run (started_at_ms, version, dist_target, profile, os, arch, cpus, embedding_model, idle_minutes, utc_offset_s) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                run.started_at_ms,
                run.version,
                run.dist_target,
                run.profile,
                run.os,
                run.arch,
                run.cpus,
                run.embedding_model,
                run.idle_minutes,
                run.utc_offset_s
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn stop_run(&mut self, run_id: i64, at_ms: i64) -> Result<(), MetricsError> {
        self.conn.execute("UPDATE run SET stopped_at_ms = ?2 WHERE id = ?1", params![run_id, at_ms])?;
        Ok(())
    }

    /// Writes what `window` recorded as the minute that began at `start_ms`.
    /// A series that minute already has is added to, never replaced.
    pub fn write_minute(&mut self, run_id: i64, start_ms: i64, window: &Window) -> Result<(), MetricsError> {
        let minute = start_ms.div_euclid(MINUTE_MS);
        let tx = self.conn.transaction()?;
        for (key, histogram) in &window.histograms {
            upsert_hist(&tx, Period::Minute, run_id, minute, key, histogram)?;
        }
        for (key, gauge) in &window.gauges {
            upsert_gauge(&tx, Period::Minute, run_id, minute, key, gauge)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Keeps `events` of the run, in the same transaction for all of them.
    pub fn write_events(&mut self, run_id: i64, events: &[Event]) -> Result<(), MetricsError> {
        if events.is_empty() {
            return Ok(());
        }
        let tx = self.conn.transaction()?;
        for event in events {
            tx.execute(
                "INSERT INTO event (run_id, at_ms, kind, detail) VALUES (?1, ?2, ?3, ?4)",
                params![run_id, event.at_ms, event.kind.as_str(), event.detail_json()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The events from `since_ms` on, oldest first. A file of schema 1 has
    /// none.
    pub fn events_since(&self, since_ms: i64) -> Result<Vec<EventRow>, MetricsError> {
        let has_events: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'event')",
            [],
            |r| r.get(0),
        )?;
        if !has_events {
            return Ok(Vec::new());
        }
        let mut stmt =
            self.conn.prepare("SELECT run_id, at_ms, kind, detail FROM event WHERE at_ms >= ?1 ORDER BY at_ms, rowid")?;
        let rows = stmt.query_map([since_ms], |r| {
            let detail: String = r.get(3)?;
            Ok(EventRow {
                run_id: r.get(0)?,
                at_ms: r.get(1)?,
                kind: r.get(2)?,
                detail: serde_json::from_str(&detail).unwrap_or(serde_json::Value::Null),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Folds the minutes older than [`MINUTES_KEPT_MS`] into their hour and
    /// drops the hours and the events older than [`HOURS_KEPT_MS`], with the
    /// runs that have nothing left, in one transaction.
    pub fn prune(&mut self, now_ms: i64) -> Result<Pruned, MetricsError> {
        let minute_cut = (now_ms - MINUTES_KEPT_MS).div_euclid(MINUTE_MS);
        let hour_cut = (now_ms - HOURS_KEPT_MS).div_euclid(MINUTE_MS * HOUR_MINUTES);
        let tx = self.conn.transaction()?;

        let hists = read_hists(&tx, Period::Minute, "minute < ?1", minute_cut)?;
        for (run_id, minute, key, histogram) in &hists {
            upsert_hist(&tx, Period::Hour, *run_id, minute.div_euclid(HOUR_MINUTES), key, histogram)?;
        }
        let gauges = read_gauges(&tx, Period::Minute, "minute < ?1", minute_cut)?;
        for (run_id, minute, key, gauge) in &gauges {
            upsert_gauge(&tx, Period::Hour, *run_id, minute.div_euclid(HOUR_MINUTES), key, gauge)?;
        }
        let folded = tx.execute("DELETE FROM hist_minute WHERE minute < ?1", [minute_cut])?
            + tx.execute("DELETE FROM gauge_minute WHERE minute < ?1", [minute_cut])?;

        let dropped_hours = tx.execute("DELETE FROM hist_hour WHERE hour < ?1", [hour_cut])?
            + tx.execute("DELETE FROM gauge_hour WHERE hour < ?1", [hour_cut])?;
        let dropped_events = tx.execute("DELETE FROM event WHERE at_ms < ?1", [now_ms - HOURS_KEPT_MS])?;
        let dropped_runs = tx.execute(
            "DELETE FROM run WHERE started_at_ms < ?1 \
             AND id NOT IN (SELECT run_id FROM hist_minute UNION SELECT run_id FROM hist_hour \
                            UNION SELECT run_id FROM gauge_minute UNION SELECT run_id FROM gauge_hour \
                            UNION SELECT run_id FROM event)",
            [now_ms - HOURS_KEPT_MS],
        )?;
        tx.commit()?;
        Ok(Pruned { folded_minutes: folded, dropped_hours, dropped_events, dropped_runs })
    }

    /// The runs that started at `since_ms` or later, or that have rows from
    /// then on.
    pub fn runs_since(&self, since_ms: i64) -> Result<Vec<RunRow>, MetricsError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, started_at_ms, stopped_at_ms, version, utc_offset_s FROM run \
             WHERE started_at_ms >= ?1 OR stopped_at_ms IS NULL OR stopped_at_ms >= ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([since_ms], |r| {
            Ok(RunRow {
                id: r.get(0)?,
                started_at_ms: r.get(1)?,
                stopped_at_ms: r.get(2)?,
                version: r.get(3)?,
                utc_offset_s: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Every stored histogram named `name` whose period starts at `since_ms`
    /// or later, minutes and hours alike.
    pub fn hist_rows_since(&self, name: &str, since_ms: i64) -> Result<Vec<HistRow>, MetricsError> {
        let mut out = Vec::new();
        for period in [Period::Minute, Period::Hour] {
            let since = since_ms.div_euclid(period.ms());
            let column = period_column(period);
            let mut stmt = self.conn.prepare(&format!(
                "SELECT run_id, {column}, labels, unit, sum, min, max, buckets FROM {} WHERE name = ?1 AND {column} >= ?2",
                period.hist_table()
            ))?;
            let rows = stmt.query_map(params![name, since], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, Vec<u8>>(7)?,
                ))
            })?;
            for row in rows {
                let (run_id, at, labels, unit, sum, min, max, buckets) = row?;
                let histogram = decode(name, &unit, &buckets, sum, min, max)?;
                out.push(HistRow {
                    run_id,
                    start_ms: at * period.ms(),
                    key: SeriesKey { name: name.to_string(), labels },
                    histogram,
                });
            }
        }
        Ok(out)
    }

    pub fn gauge_rows_since(&self, name: &str, since_ms: i64) -> Result<Vec<GaugeRow>, MetricsError> {
        let mut out = Vec::new();
        for period in [Period::Minute, Period::Hour] {
            let since = since_ms.div_euclid(period.ms());
            let column = period_column(period);
            let mut stmt = self.conn.prepare(&format!(
                "SELECT run_id, {column}, labels, count, sum, min, max, last FROM {} WHERE name = ?1 AND {column} >= ?2",
                period.gauge_table()
            ))?;
            let rows = stmt.query_map(params![name, since], |r| {
                Ok(GaugeRow {
                    run_id: r.get(0)?,
                    start_ms: r.get::<_, i64>(1)? * period.ms(),
                    key: SeriesKey { name: name.to_string(), labels: r.get(2)? },
                    gauge: Gauge {
                        count: r.get::<_, i64>(3)? as u64,
                        sum: r.get(4)?,
                        min: r.get(5)?,
                        max: r.get(6)?,
                        last: r.get(7)?,
                    },
                })
            })?;
            out.extend(rows.collect::<Result<Vec<_>, _>>()?);
        }
        Ok(out)
    }

    /// Every metric name stored, histograms and gauges.
    pub fn names(&self) -> Result<Vec<String>, MetricsError> {
        let mut stmt = self.conn.prepare(
            "SELECT name FROM hist_minute UNION SELECT name FROM hist_hour \
             UNION SELECT name FROM gauge_minute UNION SELECT name FROM gauge_hour ORDER BY 1",
        )?;
        let names = stmt.query_map([], |r| r.get(0))?;
        Ok(names.collect::<Result<_, _>>()?)
    }
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS run (
    id INTEGER PRIMARY KEY,
    started_at_ms INTEGER NOT NULL,
    stopped_at_ms INTEGER,
    version TEXT NOT NULL,
    dist_target TEXT,
    profile TEXT NOT NULL,
    os TEXT NOT NULL,
    arch TEXT NOT NULL,
    cpus INTEGER NOT NULL,
    embedding_model TEXT NOT NULL,
    idle_minutes INTEGER,
    utc_offset_s INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS hist_minute (
    run_id INTEGER NOT NULL, minute INTEGER NOT NULL, name TEXT NOT NULL, labels TEXT NOT NULL,
    unit TEXT NOT NULL, count INTEGER NOT NULL, sum INTEGER NOT NULL, min INTEGER NOT NULL,
    max INTEGER NOT NULL, buckets BLOB NOT NULL,
    PRIMARY KEY (run_id, minute, name, labels)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS hist_hour (
    run_id INTEGER NOT NULL, hour INTEGER NOT NULL, name TEXT NOT NULL, labels TEXT NOT NULL,
    unit TEXT NOT NULL, count INTEGER NOT NULL, sum INTEGER NOT NULL, min INTEGER NOT NULL,
    max INTEGER NOT NULL, buckets BLOB NOT NULL,
    PRIMARY KEY (run_id, hour, name, labels)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS gauge_minute (
    run_id INTEGER NOT NULL, minute INTEGER NOT NULL, name TEXT NOT NULL, labels TEXT NOT NULL,
    count INTEGER NOT NULL, sum REAL NOT NULL, min REAL NOT NULL, max REAL NOT NULL, last REAL NOT NULL,
    PRIMARY KEY (run_id, minute, name, labels)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS gauge_hour (
    run_id INTEGER NOT NULL, hour INTEGER NOT NULL, name TEXT NOT NULL, labels TEXT NOT NULL,
    count INTEGER NOT NULL, sum REAL NOT NULL, min REAL NOT NULL, max REAL NOT NULL, last REAL NOT NULL,
    PRIMARY KEY (run_id, hour, name, labels)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS event (
    run_id INTEGER NOT NULL, at_ms INTEGER NOT NULL, kind TEXT NOT NULL, detail TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS event_by_time ON event (at_ms);
CREATE INDEX IF NOT EXISTS hist_minute_by_name ON hist_minute (name, minute);
CREATE INDEX IF NOT EXISTS gauge_minute_by_name ON gauge_minute (name, minute);
";

fn period_column(period: Period) -> &'static str {
    match period {
        Period::Minute => "minute",
        Period::Hour => "hour",
    }
}

fn decode(name: &str, unit: &str, buckets: &[u8], sum: i64, min: i64, max: i64) -> Result<Histogram, MetricsError> {
    let bad = || MetricsError::BadRow(name.to_string());
    let unit = Unit::parse(unit).ok_or_else(bad)?;
    Histogram::from_parts(unit, buckets, sum as u64, min as u64, max as u64).map_err(|_| bad())
}

fn upsert_hist(
    tx: &rusqlite::Transaction<'_>,
    period: Period,
    run_id: i64,
    at: i64,
    key: &SeriesKey,
    histogram: &Histogram,
) -> Result<(), MetricsError> {
    let (table, column) = (period.hist_table(), period_column(period));
    let existing = tx
        .query_row(
            &format!("SELECT unit, sum, min, max, buckets FROM {table} WHERE run_id = ?1 AND {column} = ?2 AND name = ?3 AND labels = ?4"),
            params![run_id, at, key.name, key.labels],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?, r.get::<_, Vec<u8>>(4)?)),
        )
        .optional()?;
    let mut total = match existing {
        Some((unit, sum, min, max, buckets)) => decode(&key.name, &unit, &buckets, sum, min, max)?,
        None => Histogram::new(histogram.unit()),
    };
    total.merge(histogram).map_err(|_| MetricsError::BadRow(key.name.clone()))?;
    tx.execute(
        &format!(
            "INSERT OR REPLACE INTO {table} (run_id, {column}, name, labels, unit, count, sum, min, max, buckets) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
        ),
        params![
            run_id,
            at,
            key.name,
            key.labels,
            total.unit().as_str(),
            total.count() as i64,
            total.sum() as i64,
            total.min() as i64,
            total.max() as i64,
            total.encode_buckets()
        ],
    )?;
    Ok(())
}

fn upsert_gauge(
    tx: &rusqlite::Transaction<'_>,
    period: Period,
    run_id: i64,
    at: i64,
    key: &SeriesKey,
    gauge: &Gauge,
) -> Result<(), MetricsError> {
    let (table, column) = (period.gauge_table(), period_column(period));
    let existing = tx
        .query_row(
            &format!("SELECT count, sum, min, max, last FROM {table} WHERE run_id = ?1 AND {column} = ?2 AND name = ?3 AND labels = ?4"),
            params![run_id, at, key.name, key.labels],
            |r| {
                Ok(Gauge {
                    count: r.get::<_, i64>(0)? as u64,
                    sum: r.get(1)?,
                    min: r.get(2)?,
                    max: r.get(3)?,
                    last: r.get(4)?,
                })
            },
        )
        .optional()?;
    let total = match existing {
        Some(mut earlier) => {
            earlier.merge(gauge);
            earlier
        }
        None => *gauge,
    };
    tx.execute(
        &format!(
            "INSERT OR REPLACE INTO {table} (run_id, {column}, name, labels, count, sum, min, max, last) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
        ),
        params![run_id, at, key.name, key.labels, total.count as i64, total.sum, total.min, total.max, total.last],
    )?;
    Ok(())
}

type Stored<T> = Vec<(i64, i64, SeriesKey, T)>;

/// The histograms of `period` whose period column passes `filter` with `?1`
/// bound to `value`, in the order they were recorded.
fn read_hists(tx: &rusqlite::Transaction<'_>, period: Period, filter: &str, value: i64) -> Result<Stored<Histogram>, MetricsError> {
    let column = period_column(period);
    let mut stmt = tx.prepare(&format!(
        "SELECT run_id, {column}, name, labels, unit, sum, min, max, buckets FROM {} WHERE {filter} ORDER BY {column}",
        period.hist_table()
    ))?;
    let rows = stmt.query_map([value], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            SeriesKey { name: r.get(2)?, labels: r.get(3)? },
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, i64>(6)?,
            r.get::<_, i64>(7)?,
            r.get::<_, Vec<u8>>(8)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (run_id, at, key, unit, sum, min, max, buckets) = row?;
        let histogram = decode(&key.name, &unit, &buckets, sum, min, max)?;
        out.push((run_id, at, key, histogram));
    }
    Ok(out)
}

fn read_gauges(tx: &rusqlite::Transaction<'_>, period: Period, filter: &str, value: i64) -> Result<Stored<Gauge>, MetricsError> {
    let column = period_column(period);
    let mut stmt = tx.prepare(&format!(
        "SELECT run_id, {column}, name, labels, count, sum, min, max, last FROM {} WHERE {filter} ORDER BY {column}",
        period.gauge_table()
    ))?;
    let rows = stmt.query_map([value], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            SeriesKey { name: r.get(2)?, labels: r.get(3)? },
            Gauge {
                count: r.get::<_, i64>(4)? as u64,
                sum: r.get(5)?,
                min: r.get(6)?,
                max: r.get(7)?,
                last: r.get(8)?,
            },
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn is_unreadable(e: &rusqlite::Error) -> bool {
    matches!(e.sqlite_error_code(), Some(ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt))
}

fn set_aside_path(path: &Path, now_ms: i64) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(format!(".corrupt-{now_ms}"));
    path.with_file_name(name)
}

/// Creates `path` empty and owner-only when it is missing, so SQLite opens a
/// file that was never readable by anyone else.
#[cfg(unix)]
fn create_private(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(not(unix))]
fn create_private(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// The histograms a window holds, keyed as `report` reads them; for tests.
#[cfg(test)]
fn by_key(rows: &[HistRow]) -> std::collections::BTreeMap<(i64, String), Histogram> {
    rows.iter().map(|r| ((r.start_ms, r.key.labels.clone()), r.histogram.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{EventKind, Recorder};

    const NOW: i64 = 1_790_000_000_000;

    fn run_info(started_at_ms: i64) -> RunInfo {
        RunInfo {
            started_at_ms,
            version: "0.49.0".into(),
            dist_target: None,
            profile: "dev".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            cpus: 8,
            embedding_model: "multilingual-e5-small/mean".into(),
            idle_minutes: Some(10),
            utc_offset_s: -10_800,
        }
    }

    fn window(values: &[u64], gauge: f64) -> Window {
        let r = Recorder::default();
        for v in values {
            r.observe("http.duration", Unit::Micros, &[("route", "/v1/search")], *v);
        }
        r.gauge("process.rss_bytes", &[], gauge);
        r.take_minute()
    }

    #[test]
    fn a_run_is_recorded_open_and_closed_on_stop() {
        let dir = crate::test_dir::test_dir();
        let path = dir.path().join(METRICS_FILE);
        let mut db = MetricsDb::open(&path, NOW).unwrap();
        let first = db.start_run(&run_info(NOW)).unwrap();
        let second = db.start_run(&run_info(NOW + 1)).unwrap();
        db.stop_run(second, NOW + 5).unwrap();
        drop(db);

        let db = MetricsDb::open_read_only(&path).unwrap();
        let runs = db.runs_since(NOW - 1).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!((runs[0].id, runs[0].stopped_at_ms), (first, None), "a run that never stopped stays open");
        assert_eq!(runs[1].stopped_at_ms, Some(NOW + 5));
        assert_eq!(runs[1].utc_offset_s, -10_800);
        assert_eq!(runs[1].version, "0.49.0");
    }

    #[test]
    fn a_minute_is_written_with_its_series() {
        let dir = crate::test_dir::test_dir();
        let path = dir.path().join(METRICS_FILE);
        let mut db = MetricsDb::open(&path, NOW).unwrap();
        let run = db.start_run(&run_info(NOW)).unwrap();
        db.write_minute(run, NOW, &window(&[100, 2_000], 7.0)).unwrap();
        db.write_minute(run, NOW + 1, &window(&[300], 9.0)).unwrap();

        let hists = db.hist_rows_since("http.duration", NOW - MINUTE_MS).unwrap();
        assert_eq!(hists.len(), 1, "a second write of the same minute adds to it");
        assert_eq!(hists[0].histogram.count(), 3);
        assert_eq!(hists[0].start_ms, NOW.div_euclid(MINUTE_MS) * MINUTE_MS);
        assert_eq!(hists[0].key.labels, "route=/v1/search");
        let gauges = db.gauge_rows_since("process.rss_bytes", NOW - MINUTE_MS).unwrap();
        assert_eq!((gauges[0].gauge.count, gauges[0].gauge.max, gauges[0].gauge.last), (2, 9.0, 9.0));
        assert_eq!(db.names().unwrap(), vec!["http.duration".to_string(), "process.rss_bytes".to_string()]);
        assert!(db.hist_rows_since("http.duration", NOW + MINUTE_MS).unwrap().is_empty());
    }

    #[test]
    fn a_corrupt_file_is_set_aside_and_recreated() {
        let dir = crate::test_dir::test_dir();
        let path = dir.path().join(METRICS_FILE);
        let garbage = vec![0x42u8; 8192];
        std::fs::write(&path, &garbage).unwrap();

        let mut db = MetricsDb::open(&path, NOW).unwrap();
        db.start_run(&run_info(NOW)).unwrap();

        let aside = dir.path().join(format!("{METRICS_FILE}.corrupt-{NOW}"));
        assert_eq!(std::fs::read(&aside).unwrap(), garbage, "the unreadable bytes are kept as they were");
        assert_eq!(db.runs_since(0).unwrap().len(), 1);
    }

    #[test]
    fn a_newer_schema_is_refused_untouched() {
        let dir = crate::test_dir::test_dir();
        let path = dir.path().join(METRICS_FILE);
        Connection::open(&path).unwrap().pragma_update(None, "user_version", 9).unwrap();
        assert!(matches!(MetricsDb::open(&path, NOW), Err(MetricsError::Newer(9))));
        assert!(matches!(MetricsDb::open_read_only(&path), Err(MetricsError::Newer(9))));
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_private_to_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::test_dir::test_dir();
        let path = dir.path().join(METRICS_FILE);
        MetricsDb::open(&path, NOW).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn old_minutes_fold_into_their_hour() {
        let dir = crate::test_dir::test_dir();
        let mut db = MetricsDb::open(&dir.path().join(METRICS_FILE), NOW).unwrap();
        let old = NOW - MINUTES_KEPT_MS - DAY_MS;
        let hour_start = old.div_euclid(3_600_000) * 3_600_000;
        let run = db.start_run(&run_info(hour_start)).unwrap();
        db.write_minute(run, hour_start, &window(&[100], 5.0)).unwrap();
        db.write_minute(run, hour_start + MINUTE_MS, &window(&[200_000, 900], 8.0)).unwrap();
        db.write_minute(run, NOW, &window(&[50], 1.0)).unwrap();

        let pruned = db.prune(NOW).unwrap();

        assert_eq!(pruned.folded_minutes, 4, "two minutes of a histogram and of a gauge");
        let rows = db.hist_rows_since("http.duration", 0).unwrap();
        let rows = by_key(&rows);
        let hour = &rows[&(hour_start, "route=/v1/search".to_string())];
        assert_eq!(hour.count(), 3, "the hour holds both minutes");
        assert_eq!((hour.min(), hour.max()), (100, 200_000));
        let recent = NOW.div_euclid(MINUTE_MS) * MINUTE_MS;
        assert_eq!(rows[&(recent, "route=/v1/search".to_string())].count(), 1, "a recent minute stays");
        let gauges = db.gauge_rows_since("process.rss_bytes", 0).unwrap();
        let folded = gauges.iter().find(|g| g.start_ms == hour_start).unwrap();
        assert_eq!((folded.gauge.count, folded.gauge.min, folded.gauge.max, folded.gauge.last), (2, 5.0, 8.0, 8.0));
    }

    /// The tables of schema 1 as clawg wrote them before events.
    const SCHEMA_1: &str = "
CREATE TABLE run (
    id INTEGER PRIMARY KEY, started_at_ms INTEGER NOT NULL, stopped_at_ms INTEGER, version TEXT NOT NULL,
    dist_target TEXT, profile TEXT NOT NULL, os TEXT NOT NULL, arch TEXT NOT NULL, cpus INTEGER NOT NULL,
    embedding_model TEXT NOT NULL, idle_minutes INTEGER, utc_offset_s INTEGER NOT NULL
);
CREATE TABLE hist_minute (
    run_id INTEGER NOT NULL, minute INTEGER NOT NULL, name TEXT NOT NULL, labels TEXT NOT NULL,
    unit TEXT NOT NULL, count INTEGER NOT NULL, sum INTEGER NOT NULL, min INTEGER NOT NULL,
    max INTEGER NOT NULL, buckets BLOB NOT NULL, PRIMARY KEY (run_id, minute, name, labels)
) WITHOUT ROWID;
CREATE TABLE hist_hour (
    run_id INTEGER NOT NULL, hour INTEGER NOT NULL, name TEXT NOT NULL, labels TEXT NOT NULL,
    unit TEXT NOT NULL, count INTEGER NOT NULL, sum INTEGER NOT NULL, min INTEGER NOT NULL,
    max INTEGER NOT NULL, buckets BLOB NOT NULL, PRIMARY KEY (run_id, hour, name, labels)
) WITHOUT ROWID;
CREATE TABLE gauge_minute (
    run_id INTEGER NOT NULL, minute INTEGER NOT NULL, name TEXT NOT NULL, labels TEXT NOT NULL,
    count INTEGER NOT NULL, sum REAL NOT NULL, min REAL NOT NULL, max REAL NOT NULL, last REAL NOT NULL,
    PRIMARY KEY (run_id, minute, name, labels)
) WITHOUT ROWID;
CREATE TABLE gauge_hour (
    run_id INTEGER NOT NULL, hour INTEGER NOT NULL, name TEXT NOT NULL, labels TEXT NOT NULL,
    count INTEGER NOT NULL, sum REAL NOT NULL, min REAL NOT NULL, max REAL NOT NULL, last REAL NOT NULL,
    PRIMARY KEY (run_id, hour, name, labels)
) WITHOUT ROWID;
PRAGMA user_version = 1;
";

    #[test]
    fn a_file_of_schema_1_gets_its_events_and_keeps_its_rows() {
        let dir = crate::test_dir::test_dir();
        let path = dir.path().join(METRICS_FILE);
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(SCHEMA_1).unwrap();
            c.execute(
                "INSERT INTO run (id, started_at_ms, version, profile, os, arch, cpus, embedding_model, utc_offset_s) \
                 VALUES (1, ?1, '0.66.0', 'dev', 'linux', 'x86_64', 8, 'm', 0)",
                [NOW],
            )
            .unwrap();
            c.execute(
                "INSERT INTO gauge_minute VALUES (1, ?1, 'process.rss_bytes', '', 1, 7.0, 7.0, 7.0, 7.0)",
                [NOW.div_euclid(MINUTE_MS)],
            )
            .unwrap();
        }
        let read = MetricsDb::open_read_only(&path).unwrap();
        assert!(read.events_since(0).unwrap().is_empty(), "a file of schema 1 reads as one without events");
        drop(read);

        let mut db = MetricsDb::open(&path, NOW).unwrap();
        let version: i64 = db.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, 2);
        assert_eq!(db.runs_since(0).unwrap().len(), 1, "its runs stay");
        assert_eq!(db.gauge_rows_since("process.rss_bytes", 0).unwrap()[0].gauge.last, 7.0, "its minutes stay");

        let run = db.start_run(&run_info(NOW)).unwrap();
        let event = |at_ms, kind| crate::metrics::Event {
            at_ms,
            kind,
            detail: std::collections::BTreeMap::from([("ms", 40u64.into()), ("reason", "manual".into())]),
        };
        db.write_events(run, &[event(NOW, EventKind::Backup), event(NOW + 1, EventKind::Shutdown)]).unwrap();
        let events = db.events_since(NOW).unwrap();
        assert_eq!(events.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), ["backup", "shutdown"]);
        assert_eq!(events[0].run_id, run);
        assert_eq!(events[0].detail, serde_json::json!({ "ms": 40, "reason": "manual" }));
        assert_eq!(db.events_since(NOW + 1).unwrap().len(), 1);
    }

    #[test]
    fn events_past_the_retention_go_with_their_run() {
        let dir = crate::test_dir::test_dir();
        let mut db = MetricsDb::open(&dir.path().join(METRICS_FILE), NOW).unwrap();
        let ancient = NOW - HOURS_KEPT_MS - DAY_MS;
        let gone = db.start_run(&run_info(ancient)).unwrap();
        let old = crate::metrics::Event { at_ms: ancient, kind: EventKind::Backup, detail: Default::default() };
        db.write_events(gone, &[old]).unwrap();
        let kept = db.start_run(&run_info(ancient)).unwrap();
        let recent = crate::metrics::Event { at_ms: NOW, kind: EventKind::Import, detail: Default::default() };
        db.write_events(kept, &[recent]).unwrap();

        let pruned = db.prune(NOW).unwrap();

        assert_eq!((pruned.dropped_events, pruned.dropped_runs), (1, 1));
        assert_eq!(db.runs_since(0).unwrap().iter().map(|r| r.id).collect::<Vec<_>>(), [kept], "a recent event keeps its run");
        assert_eq!(db.events_since(0).unwrap().len(), 1);
    }

    #[test]
    fn hours_past_the_retention_are_dropped() {
        let dir = crate::test_dir::test_dir();
        let mut db = MetricsDb::open(&dir.path().join(METRICS_FILE), NOW).unwrap();
        let ancient = NOW - HOURS_KEPT_MS - DAY_MS;
        let gone = db.start_run(&run_info(ancient)).unwrap();
        db.write_minute(gone, ancient, &window(&[100], 1.0)).unwrap();
        let kept = db.start_run(&run_info(NOW)).unwrap();
        db.write_minute(kept, NOW, &window(&[100], 1.0)).unwrap();

        let pruned = db.prune(NOW).unwrap();

        assert_eq!(pruned.dropped_hours, 2);
        assert_eq!(pruned.dropped_runs, 1);
        let runs = db.runs_since(0).unwrap();
        assert_eq!(runs.iter().map(|r| r.id).collect::<Vec<_>>(), vec![kept]);
        assert_eq!(db.hist_rows_since("http.duration", 0).unwrap().len(), 1);
    }
}
