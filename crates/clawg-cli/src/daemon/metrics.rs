//! What the daemon measures of itself: every HTTP request, and the writer
//! that samples the process once a minute and keeps each minute in
//! `metrics.db`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{MatchedPath, Request, State};
use axum::http::{HeaderMap, Method};
use axum::middleware::Next;
use axum::response::Response;
use clawg_core::metrics::store::{MetricsDb, RunInfo, METRICS_FILE};
use clawg_core::metrics::{Gauge, Histogram, Recorder, SeriesKey, Unit};
use serde::Serialize;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::lifecycle::now_ms;
use super::process::{self, ratio, ProcessSample, RuntimeSample};

/// Which listener a request came in on. The router is the same for both, so
/// `serve` marks each server's copy with this as an extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Unix,
    Tcp,
}

impl Transport {
    fn as_str(self) -> &'static str {
        match self {
            Transport::Unix => "unix",
            Transport::Tcp => "tcp",
        }
    }
}

/// The recorder and the requests in flight, shared by every copy of a
/// router.
pub struct HttpMeter {
    recorder: Arc<Recorder>,
    in_flight: AtomicU64,
}

impl HttpMeter {
    pub fn new(recorder: Arc<Recorder>) -> Self {
        Self { recorder, in_flight: AtomicU64::new(0) }
    }
}

/// Takes one off the requests in flight when the request ends, also when
/// its future is dropped because the client went away.
struct InFlight<'a>(&'a HttpMeter);

impl InFlight<'_> {
    fn enter(meter: &HttpMeter) -> InFlight<'_> {
        let now = meter.in_flight.fetch_add(1, Ordering::Relaxed) + 1;
        meter.recorder.gauge("http.in_flight", &[], now as f64);
        InFlight(meter)
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        let now = self.0.in_flight.fetch_sub(1, Ordering::Relaxed).saturating_sub(1);
        self.0.recorder.gauge("http.in_flight", &[], now as f64);
    }
}

/// Times every request, those the guard or the token refuse included. Every
/// label comes from a closed set or from the route template axum matched,
/// never from the URL, the query or the body.
pub async fn time_request(State(meter): State<Arc<HttpMeter>>, req: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = method_label(req.method());
    let route = req.extensions().get::<MatchedPath>().map(|p| p.as_str().to_string());
    let route = route.as_deref().unwrap_or("unmatched").to_string();
    let transport = req.extensions().get::<Transport>().copied().unwrap_or(Transport::Tcp);
    meter.recorder.observe(
        "http.request_bytes",
        Unit::Bytes,
        &[("method", method), ("route", &route)],
        content_length(req.headers()),
    );

    let in_flight = InFlight::enter(&meter);
    let response = next.run(req).await;
    drop(in_flight);

    let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    meter.recorder.observe(
        "http.duration",
        Unit::Micros,
        &[
            ("method", method),
            ("route", &route),
            ("status", status_class(response.status().as_u16())),
            ("transport", transport.as_str()),
        ],
        micros,
    );
    response
}

/// `CLAWG_METRICS`: `off` keeps `metrics.db` from being created or written.
pub const METRICS_VAR: &str = "CLAWG_METRICS";

/// Whether `metrics.db` is written, from the value of [`METRICS_VAR`]: only
/// `off`, in any case, turns it off.
pub fn writing_enabled(value: Option<&str>) -> bool {
    !value.is_some_and(|v| v.trim().eq_ignore_ascii_case("off"))
}

/// Where the metrics of `base_dir` go, or `None` when [`METRICS_VAR`] turned
/// them off.
pub fn metrics_path(base_dir: &Path, var: Option<&str>) -> Option<PathBuf> {
    writing_enabled(var).then(|| base_dir.join(METRICS_FILE))
}

const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 60 * MINUTE_MS;

/// The open file and the run this process recorded in it.
struct Sink {
    db: MetricsDb,
    run_id: i64,
}

/// Samples the process into the recorder and keeps each minute in
/// `metrics.db`. Its writes are blocking; the daemon runs them off the
/// runtime's threads.
pub struct Writer {
    recorder: Arc<Recorder>,
    sink: Option<Sink>,
    minute_start_ms: i64,
    last_prune_ms: i64,
    cpu: Option<(i64, Duration)>,
    busy: Option<(i64, Duration)>,
}

impl Writer {
    /// Opens `path` and records the run in it; `None` writes nothing. A file
    /// that cannot be opened turns writing off with one `warn`.
    pub fn start(path: Option<&Path>, recorder: Arc<Recorder>, run: &RunInfo, now_ms: i64) -> Self {
        let sink = path.and_then(|path| {
            let opened = MetricsDb::open(path, now_ms).and_then(|mut db| {
                if let Err(e) = db.prune(now_ms) {
                    log::warn!("could not fold the old minutes of {}: {e}", path.display());
                }
                let run_id = db.start_run(run)?;
                Ok(Sink { db, run_id })
            });
            opened
                .map_err(|e| log::warn!("metrics are not kept in {} until the next start: {e}", path.display()))
                .ok()
        });
        recorder.set_writing(sink.is_some());
        Self { recorder, sink, minute_start_ms: now_ms, last_prune_ms: now_ms, cpu: None, busy: None }
    }

    pub fn is_writing(&self) -> bool {
        self.sink.is_some()
    }

    /// Records what the process and the runtime are at now as gauges.
    pub fn sample(&mut self, now_ms: i64, process: ProcessSample, runtime: RuntimeSample, resident: Option<bool>) {
        let r = &self.recorder;
        let gauges = [
            ("process.rss_bytes", process.rss_bytes),
            ("process.rss_peak_bytes", process.rss_peak_bytes),
            ("process.private_bytes", process.private_bytes),
            ("process.private_peak_bytes", process.private_peak_bytes),
            ("process.threads", process.threads),
            ("process.open_files", process.open_files),
            ("process.handles", process.handles),
        ];
        for (name, value) in gauges {
            if let Some(value) = value {
                r.gauge(name, &[], value as f64);
            }
        }
        if let Some(resident) = resident {
            r.gauge("embedder.resident", &[], if resident { 1.0 } else { 0.0 });
        }
        r.gauge("rt.alive_tasks", &[], runtime.alive_tasks as f64);
        r.gauge("rt.global_queue_depth", &[], runtime.global_queue_depth as f64);
        if let Some(cpu) = process.cpu {
            if let Some(share) = since(&mut self.cpu, now_ms, cpu).and_then(|(spent, wall)| ratio(spent, wall, 1)) {
                r.gauge("process.cpu_ratio", &[], share);
            }
        }
        let busy = since(&mut self.busy, now_ms, runtime.busy);
        if let Some(share) = busy.and_then(|(spent, wall)| ratio(spent, wall, runtime.workers)) {
            r.gauge("rt.worker_busy_ratio", &[], share);
        }
    }

    /// Writes the minute so far as the minute it began in, and folds the
    /// old minutes once an hour. A failure turns writing off with one `warn`.
    pub fn flush(&mut self, now_ms: i64) {
        let window = self.recorder.take_minute();
        let started = std::mem::replace(&mut self.minute_start_ms, now_ms);
        let Some(sink) = self.sink.as_mut() else { return };
        let mut result = sink.db.write_minute(sink.run_id, started, &window);
        if result.is_ok() && now_ms - self.last_prune_ms >= HOUR_MS {
            self.last_prune_ms = now_ms;
            result = sink.db.prune(now_ms).map(drop);
        }
        if let Err(e) = result {
            log::warn!("could not write the metrics, so they are not kept until the next start: {e}");
            self.sink = None;
            self.recorder.set_writing(false);
        }
    }

    /// Writes the minute in progress and marks the run stopped.
    pub fn stop(mut self, now_ms: i64) {
        self.flush(now_ms);
        if let Some(mut sink) = self.sink.take() {
            if let Err(e) = sink.db.stop_run(sink.run_id, now_ms) {
                log::warn!("could not record the end of this run in the metrics: {e}");
            }
        }
    }
}

/// The time spent and the wall time since the last reading in `last`, which
/// becomes `now`. `None` on the first reading.
fn since(last: &mut Option<(i64, Duration)>, now_ms: i64, total: Duration) -> Option<(Duration, Duration)> {
    let previous = last.replace((now_ms, total));
    let (then_ms, then_total) = previous?;
    let wall = Duration::from_millis(u64::try_from(now_ms - then_ms).ok()?);
    Some((total.saturating_sub(then_total), wall))
}

/// The writer's task, and how to end it.
pub struct WriterTask {
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl WriterTask {
    /// Starts sampling and writing at every turn of the wall-clock minute.
    pub fn spawn(writer: Writer, resident: impl Fn() -> Option<bool> + Send + 'static) -> Self {
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(run_writer(writer, stopped, resident));
        Self { stop, task }
    }

    /// Writes the last minute and the end of the run, waiting at most
    /// `bound`; past it the run is left without an end, as a kill leaves it.
    pub async fn close(self, bound: Duration) {
        let _ = self.stop.send(());
        if tokio::time::timeout(bound, self.task).await.is_err() {
            log::warn!("the last minute of metrics was not written within {} ms", bound.as_millis());
        }
    }
}

async fn run_writer(mut writer: Writer, mut stopped: oneshot::Receiver<()>, resident: impl Fn() -> Option<bool>) {
    let handle = tokio::runtime::Handle::current();
    let now = || now_ms() as i64;
    loop {
        let wait = MINUTE_MS - now().rem_euclid(MINUTE_MS);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(wait as u64)) => {}
            _ = &mut stopped => break,
        }
        writer.sample(now(), process::sample(), process::runtime_sample(&handle), resident());
        writer = match tokio::task::spawn_blocking(move || {
            writer.flush(now());
            writer
        })
        .await
        {
            Ok(writer) => writer,
            Err(e) => {
                log::warn!("the metrics writer stopped: {e}");
                return;
            }
        };
    }
    writer.sample(now(), process::sample(), process::runtime_sample(&handle), resident());
    let _ = tokio::task::spawn_blocking(move || writer.stop(now())).await;
}

// ── Reading ─────────────────────────────────────────────────────────────────

/// The answer of `GET /v1/metrics`.
#[derive(Debug, Serialize)]
pub struct MetricsBody {
    pub version: String,
    pub started_at_ms: u64,
    /// Whether the minutes are kept in `metrics.db`.
    pub writing: bool,
    pub minute: MinuteBody,
    pub run: RunBody,
}

#[derive(Debug, Serialize)]
pub struct MinuteBody {
    pub histograms: Vec<HistogramBody>,
    pub gauges: Vec<GaugeBody>,
}

#[derive(Debug, Serialize)]
pub struct RunBody {
    pub histograms: Vec<HistogramBody>,
}

#[derive(Debug, Serialize)]
pub struct HistogramBody {
    pub name: String,
    pub labels: std::collections::BTreeMap<String, String>,
    pub unit: &'static str,
    pub count: u64,
    pub sum: u64,
    pub min: u64,
    pub max: u64,
    pub p50: Option<u64>,
    pub p95: Option<u64>,
    pub p99: Option<u64>,
    /// Each bucket's upper bound (`null` for the open one) and count.
    pub buckets: Vec<BucketBody>,
}

#[derive(Debug, Serialize)]
pub struct BucketBody {
    pub le: Option<u64>,
    pub count: u64,
}

#[derive(Debug, Serialize)]
pub struct GaugeBody {
    pub name: String,
    pub labels: std::collections::BTreeMap<String, String>,
    pub count: u64,
    pub mean: f64,
    pub min: f64,
    pub max: f64,
    pub last: f64,
}

fn labels_of(key: &SeriesKey) -> std::collections::BTreeMap<String, String> {
    key.label_pairs().into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn histogram_body(key: &SeriesKey, h: &Histogram) -> HistogramBody {
    let bounds = h.unit().bounds();
    HistogramBody {
        name: key.name.clone(),
        labels: labels_of(key),
        unit: h.unit().as_str(),
        count: h.count(),
        sum: h.sum(),
        min: h.min(),
        max: h.max(),
        p50: h.quantile(0.5),
        p95: h.quantile(0.95),
        p99: h.quantile(0.99),
        buckets: h.buckets().iter().enumerate().map(|(i, n)| BucketBody { le: bounds.get(i).copied(), count: *n }).collect(),
    }
}

fn gauge_body(key: &SeriesKey, g: &Gauge) -> GaugeBody {
    GaugeBody {
        name: key.name.clone(),
        labels: labels_of(key),
        count: g.count,
        mean: g.mean(),
        min: g.min,
        max: g.max,
        last: g.last,
    }
}

/// What the recorder holds now, as `GET /v1/metrics` answers it.
pub fn metrics_body(recorder: &Recorder, version: &str, started_at_ms: u64) -> MetricsBody {
    let minute = recorder.minute();
    MetricsBody {
        version: version.to_string(),
        started_at_ms,
        writing: recorder.writing(),
        minute: MinuteBody {
            histograms: minute.histograms.iter().map(|(k, h)| histogram_body(k, h)).collect(),
            gauges: minute.gauges.iter().map(|(k, g)| gauge_body(k, g)).collect(),
        },
        run: RunBody { histograms: recorder.run().iter().map(|(k, h)| histogram_body(k, h)).collect() },
    }
}

/// The histograms of the run and the last value of each gauge in the text
/// format Prometheus reads (version 0.0.4): durations in seconds, sizes in
/// bytes, buckets cumulative.
pub fn prometheus_text(recorder: &Recorder) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let mut last_name = String::new();
    for (key, h) in &recorder.run() {
        // Divided rather than multiplied: 50 / 1e6 prints 0.00005, 50 * 1e-6 does not.
        let (name, per_unit) = match h.unit() {
            Unit::Micros => (format!("clawg_{}_seconds", key.name.replace('.', "_")), 1e6),
            Unit::Bytes => (with_suffix(&format!("clawg_{}", key.name.replace('.', "_")), "_bytes"), 1.0),
        };
        if name != last_name {
            let _ = writeln!(out, "# TYPE {name} histogram");
            last_name = name.clone();
        }
        let labels = prometheus_labels(key);
        let mut cumulative = 0u64;
        for (i, n) in h.buckets().iter().enumerate() {
            cumulative += n;
            let le = match h.unit().bounds().get(i) {
                Some(bound) => format!("{}", *bound as f64 / per_unit),
                None => "+Inf".to_string(),
            };
            let _ = writeln!(out, "{name}_bucket{{{}le=\"{le}\"}} {cumulative}", with_comma(&labels));
        }
        let _ = writeln!(out, "{name}_sum{} {}", braced(&labels), h.sum() as f64 / per_unit);
        let _ = writeln!(out, "{name}_count{} {}", braced(&labels), h.count());
    }
    for (key, value) in &recorder.latest_gauges() {
        let name = format!("clawg_{}", key.name.replace('.', "_"));
        if name != last_name {
            let _ = writeln!(out, "# TYPE {name} gauge");
            last_name = name.clone();
        }
        let _ = writeln!(out, "{name}{} {value}", braced(&prometheus_labels(key)));
    }
    out
}

fn with_suffix(name: &str, suffix: &str) -> String {
    if name.ends_with(suffix) { name.to_string() } else { format!("{name}{suffix}") }
}

fn prometheus_labels(key: &SeriesKey) -> String {
    key.label_pairs()
        .iter()
        .map(|(k, v)| format!("{k}=\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")))
        .collect::<Vec<_>>()
        .join(",")
}

fn with_comma(labels: &str) -> String {
    if labels.is_empty() { String::new() } else { format!("{labels},") }
}

fn braced(labels: &str) -> String {
    if labels.is_empty() { String::new() } else { format!("{{{labels}}}") }
}

fn method_label(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::POST => "POST",
        Method::PATCH => "PATCH",
        Method::DELETE => "DELETE",
        Method::PUT => "PUT",
        Method::HEAD => "HEAD",
        Method::OPTIONS => "OPTIONS",
        _ => "other",
    }
}

fn status_class(status: u16) -> &'static str {
    match status {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        _ => "5xx",
    }
}

fn content_length(headers: &HeaderMap) -> u64 {
    headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_info() -> RunInfo {
        RunInfo {
            started_at_ms: NOW,
            version: "0.0.0".into(),
            dist_target: None,
            profile: "dev".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            cpus: 2,
            embedding_model: "m".into(),
            idle_minutes: Some(10),
            utc_offset_s: 0,
        }
    }

    const NOW: i64 = 1_790_000_000_000;

    fn runtime(busy_ms: u64) -> RuntimeSample {
        RuntimeSample { alive_tasks: 3, global_queue_depth: 0, workers: 2, busy: Duration::from_millis(busy_ms) }
    }

    #[test]
    fn metrics_off_is_the_only_way_to_turn_writing_off() {
        assert!(writing_enabled(None));
        assert!(writing_enabled(Some("on")));
        assert!(writing_enabled(Some("")));
        assert!(!writing_enabled(Some("off")));
        assert!(!writing_enabled(Some(" OFF ")));
        let dir = Path::new("/data");
        assert_eq!(metrics_path(dir, None), Some(dir.join("metrics.db")));
        assert_eq!(metrics_path(dir, Some("off")), None);
    }

    #[test]
    fn a_writer_turned_off_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = Arc::new(Recorder::default());
        let mut writer = Writer::start(None, recorder.clone(), &run_info(), NOW);
        recorder.observe("http.duration", Unit::Micros, &[], 5);
        writer.flush(NOW + MINUTE_MS);
        writer.stop(NOW + 2 * MINUTE_MS);
        assert!(!recorder.writing());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn each_minute_and_the_stop_are_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(METRICS_FILE);
        let recorder = Arc::new(Recorder::default());
        let mut writer = Writer::start(Some(&path), recorder.clone(), &run_info(), NOW);
        assert!(writer.is_writing() && recorder.writing());

        recorder.observe("http.duration", Unit::Micros, &[("route", "/health")], 100);
        let process = ProcessSample {
            rss_bytes: Some(4096),
            private_peak_bytes: Some(8192),
            cpu: Some(Duration::from_secs(1)),
            ..Default::default()
        };
        writer.sample(NOW, process, runtime(0), Some(true));
        writer.flush(NOW + MINUTE_MS);
        let process = ProcessSample { cpu: Some(Duration::from_secs(31)), ..process };
        writer.sample(NOW + MINUTE_MS, process, runtime(60_000), Some(false));
        recorder.observe("http.duration", Unit::Micros, &[("route", "/health")], 200);
        writer.stop(NOW + MINUTE_MS + 10);

        let db = MetricsDb::open_read_only(&path).unwrap();
        let runs = db.runs_since(0).unwrap();
        assert_eq!(runs[0].stopped_at_ms, Some(NOW + MINUTE_MS + 10));
        let hists = db.hist_rows_since("http.duration", 0).unwrap();
        assert_eq!(hists.len(), 2, "one row per minute");
        assert_eq!(hists.iter().map(|h| h.histogram.count()).sum::<u64>(), 2);
        let cpu = db.gauge_rows_since("process.cpu_ratio", 0).unwrap();
        assert_eq!(cpu.len(), 1, "a ratio needs two readings");
        assert_eq!(cpu[0].gauge.last, 0.5);
        let busy = db.gauge_rows_since("rt.worker_busy_ratio", 0).unwrap();
        assert_eq!(busy[0].gauge.last, 0.5, "a minute busy out of two workers' minutes");
        let resident = db.gauge_rows_since("embedder.resident", 0).unwrap();
        assert_eq!(resident.iter().map(|g| g.gauge.last).collect::<Vec<_>>(), vec![1.0, 0.0]);
        let peak = db.gauge_rows_since("process.private_peak_bytes", 0).unwrap();
        assert_eq!(peak.iter().map(|g| g.gauge.max).collect::<Vec<_>>(), vec![8192.0, 8192.0]);
        assert!(db.gauge_rows_since("process.handles", 0).unwrap().is_empty(), "an unknown value is left out");
    }

    #[test]
    fn a_failed_write_turns_writing_off_and_serving_goes_on() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(METRICS_FILE);
        let recorder = Arc::new(Recorder::default());
        let mut writer = Writer::start(Some(&path), recorder.clone(), &run_info(), NOW);
        rusqlite_drop_tables(&path);

        recorder.observe("http.duration", Unit::Micros, &[], 5);
        writer.flush(NOW + MINUTE_MS);

        assert!(!writer.is_writing() && !recorder.writing());
        recorder.observe("http.duration", Unit::Micros, &[], 7);
        writer.flush(NOW + 2 * MINUTE_MS);
        assert_eq!(recorder.run().values().next().unwrap().count(), 2, "recording goes on");
    }

    #[test]
    fn a_file_that_cannot_be_opened_turns_writing_off() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = Arc::new(Recorder::default());
        let writer = Writer::start(Some(dir.path()), recorder.clone(), &run_info(), NOW);
        assert!(!writer.is_writing() && !recorder.writing());
    }

    fn rusqlite_drop_tables(path: &Path) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch("DROP TABLE hist_minute; DROP TABLE gauge_minute;").unwrap();
    }

    #[test]
    fn a_label_value_is_escaped_for_prometheus() {
        let key = SeriesKey { name: "m".into(), labels: "route=a\"b\\c".into() };
        assert_eq!(prometheus_labels(&key), r#"route="a\"b\\c""#);
        assert_eq!(braced(""), "");
        assert_eq!(with_suffix("clawg_http_request_bytes", "_bytes"), "clawg_http_request_bytes");
    }

    #[test]
    fn a_status_falls_in_its_class() {
        assert_eq!(status_class(200), "2xx");
        assert_eq!(status_class(204), "2xx");
        assert_eq!(status_class(401), "4xx");
        assert_eq!(status_class(503), "5xx");
        assert_eq!(status_class(101), "1xx");
    }

    #[test]
    fn a_size_without_a_valid_content_length_is_zero() {
        let mut headers = HeaderMap::new();
        assert_eq!(content_length(&headers), 0);
        headers.insert(axum::http::header::CONTENT_LENGTH, "12".parse().unwrap());
        assert_eq!(content_length(&headers), 12);
        headers.insert(axum::http::header::CONTENT_LENGTH, "lots".parse().unwrap());
        assert_eq!(content_length(&headers), 0);
    }
}
