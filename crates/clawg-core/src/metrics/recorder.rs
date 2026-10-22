//! Where the daemon records its metrics while it serves: the minute in
//! progress, which the writer takes once a minute, and the whole run, which
//! `GET /v1/metrics` reads.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use super::histogram::{Histogram, Unit};

/// A series: a metric name and its labels, written `k=v` sorted by key and
/// joined by `,`, so one set of labels is one series whatever order the
/// caller gave them in.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SeriesKey {
    pub name: String,
    pub labels: String,
}

impl SeriesKey {
    pub fn new(name: &str, labels: &[(&str, &str)]) -> Self {
        Self { name: name.to_string(), labels: canonical_labels(labels) }
    }

    /// The labels back as pairs, in their canonical order.
    pub fn label_pairs(&self) -> Vec<(&str, &str)> {
        parse_labels(&self.labels)
    }
}

/// `labels` as `k=v` pairs sorted by key and joined by `,`. A `,` or `=`
/// inside a value becomes `_`, so the text always splits back the same way.
pub fn canonical_labels(labels: &[(&str, &str)]) -> String {
    let mut pairs: Vec<(&str, String)> =
        labels.iter().map(|(k, v)| (*k, v.replace([',', '='], "_"))).collect();
    pairs.sort();
    pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(",")
}

pub fn parse_labels(labels: &str) -> Vec<(&str, &str)> {
    labels.split(',').filter(|p| !p.is_empty()).filter_map(|p| p.split_once('=')).collect()
}

/// The samples of a gauge over a window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gauge {
    pub count: u64,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
    pub last: f64,
}

impl Gauge {
    fn of(value: f64) -> Self {
        Self { count: 1, sum: value, min: value, max: value, last: value }
    }

    fn add(&mut self, value: f64) {
        self.count += 1;
        self.sum += value;
        self.min = self.min.min(value);
        self.max = self.max.max(value);
        self.last = value;
    }

    /// Folds a later window into this one: `last` is the later one's.
    pub fn merge(&mut self, later: &Gauge) {
        self.count += later.count;
        self.sum += later.sum;
        self.min = self.min.min(later.min);
        self.max = self.max.max(later.max);
        self.last = later.last;
    }

    pub fn mean(&self) -> f64 {
        if self.count == 0 { 0.0 } else { self.sum / self.count as f64 }
    }
}

/// What one window (a minute) recorded. Only the series with data are in it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Window {
    pub histograms: BTreeMap<SeriesKey, Histogram>,
    pub gauges: BTreeMap<SeriesKey, Gauge>,
}

impl Window {
    pub fn is_empty(&self) -> bool {
        self.histograms.is_empty() && self.gauges.is_empty()
    }
}

#[derive(Debug, Default)]
struct State {
    minute: Window,
    run: BTreeMap<SeriesKey, Histogram>,
    latest: BTreeMap<SeriesKey, f64>,
}

/// The metrics of one run of the daemon. A plain mutex: a few dozen series
/// and one lock per request cost nothing next to what a request does.
#[derive(Debug, Default)]
pub struct Recorder {
    state: Mutex<State>,
    writing: AtomicBool,
}

impl Recorder {
    pub fn observe(&self, name: &str, unit: Unit, labels: &[(&str, &str)], value: u64) {
        let key = SeriesKey::new(name, labels);
        let mut state = self.lock();
        state.minute.histograms.entry(key.clone()).or_insert_with(|| Histogram::new(unit)).record(value);
        state.run.entry(key).or_insert_with(|| Histogram::new(unit)).record(value);
    }

    pub fn gauge(&self, name: &str, labels: &[(&str, &str)], value: f64) {
        let key = SeriesKey::new(name, labels);
        let mut state = self.lock();
        match state.minute.gauges.get_mut(&key) {
            Some(gauge) => gauge.add(value),
            None => {
                state.minute.gauges.insert(key.clone(), Gauge::of(value));
            }
        }
        state.latest.insert(key, value);
    }

    /// The minute so far, leaving it empty; the run keeps its histograms.
    pub fn take_minute(&self) -> Window {
        std::mem::take(&mut self.lock().minute)
    }

    /// A copy of the minute so far.
    pub fn minute(&self) -> Window {
        self.lock().minute.clone()
    }

    /// Every histogram since the run began.
    pub fn run(&self) -> BTreeMap<SeriesKey, Histogram> {
        self.lock().run.clone()
    }

    /// The last value each gauge took, whatever minute it was in.
    pub fn latest_gauges(&self) -> BTreeMap<SeriesKey, f64> {
        self.lock().latest.clone()
    }

    /// Whether the minutes are being kept in `metrics.db`, as the writer
    /// last said.
    pub fn writing(&self) -> bool {
        self.writing.load(Ordering::Relaxed)
    }

    pub fn set_writing(&self, writing: bool) {
        self.writing.store(writing, Ordering::Relaxed);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        // A panic while the lock was held leaves numbers, never a broken
        // invariant: go on with them.
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taking_the_minute_empties_it_and_keeps_the_run() {
        let r = Recorder::default();
        r.observe("http.duration", Unit::Micros, &[("route", "/health")], 120);
        r.gauge("process.rss_bytes", &[], 10.0);

        let minute = r.take_minute();
        assert_eq!(minute.histograms.len(), 1);
        assert_eq!(minute.gauges.len(), 1);
        assert!(r.minute().is_empty(), "a series without data this minute is not in it");

        r.observe("http.duration", Unit::Micros, &[("route", "/health")], 80);
        let run = r.run();
        let health = &run[&SeriesKey::new("http.duration", &[("route", "/health")])];
        assert_eq!(health.count(), 2, "the run keeps what the minute gave away");
        assert_eq!(r.take_minute().histograms.values().next().unwrap().count(), 1);
        assert_eq!(r.latest_gauges()[&SeriesKey::new("process.rss_bytes", &[])], 10.0);
    }

    #[test]
    fn labels_are_canonical_whatever_their_order() {
        let a = SeriesKey::new("m", &[("status", "2xx"), ("route", "/v1/search")]);
        let b = SeriesKey::new("m", &[("route", "/v1/search"), ("status", "2xx")]);
        assert_eq!(a, b);
        assert_eq!(a.labels, "route=/v1/search,status=2xx");
        assert_eq!(a.label_pairs(), vec![("route", "/v1/search"), ("status", "2xx")]);
        assert_eq!(canonical_labels(&[("k", "a,b=c")]), "k=a_b_c", "a value cannot split into two labels");
        assert!(parse_labels("").is_empty());
    }

    #[test]
    fn a_gauge_keeps_its_extremes_and_last_value() {
        let r = Recorder::default();
        for v in [3.0, 9.0, 1.0, 4.0] {
            r.gauge("http.in_flight", &[], v);
        }
        let minute = r.take_minute();
        let g = minute.gauges[&SeriesKey::new("http.in_flight", &[])];
        assert_eq!((g.count, g.min, g.max, g.last), (4, 1.0, 9.0, 4.0));
        assert_eq!(g.mean(), 4.25);

        let mut earlier = Gauge::of(5.0);
        earlier.merge(&g);
        assert_eq!((earlier.count, earlier.min, earlier.max, earlier.last), (5, 1.0, 9.0, 4.0));
    }
}
