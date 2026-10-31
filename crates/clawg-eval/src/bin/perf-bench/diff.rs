//! Two result files of the bench, scenario by scenario.

use std::fmt::Write;
use std::path::Path;

use serde::Deserialize;

use crate::stats::{mann_whitney_p, quantile, quantile_change, Change, RESAMPLES, SEED};

pub const FORMAT: &str = "clawg-perf-bench";
pub const VERSION: u32 = 1;

/// The p95 change past which a scenario that the bootstrap separates from
/// no change is a regression.
pub const THRESHOLD: f64 = 0.20;

/// Fewer samples on either side say nothing about a p95.
pub const MIN_SAMPLES: usize = 20;

/// The fields of a result file the comparison reads; the rest is for people.
#[derive(Debug, Clone, Deserialize)]
pub struct BenchFile {
    pub format: String,
    pub version: u32,
    pub clawg: String,
    pub build: String,
    pub embedder: String,
    pub machine: Machine,
    pub store: Store,
    pub scenarios: Vec<Scenario>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Machine {
    pub os: String,
    pub arch: String,
    pub threads: usize,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Store {
    pub rows: usize,
    pub seed: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Scenario {
    pub name: String,
    pub failed: usize,
    pub samples_us: Vec<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("{path}: {source}")]
    Io { path: String, source: std::io::Error },
    #[error("{path}: not a bench result: {source}")]
    Json { path: String, source: serde_json::Error },
    #[error("{path}: format {format:?} version {version}; this reads {FORMAT} version {VERSION}")]
    Format { path: String, format: String, version: u32 },
}

pub fn read(path: &Path) -> Result<BenchFile, ReadError> {
    let shown = path.display().to_string();
    let text = std::fs::read_to_string(path).map_err(|source| ReadError::Io { path: shown.clone(), source })?;
    parse(&text, &shown)
}

pub fn parse(text: &str, path: &str) -> Result<BenchFile, ReadError> {
    let file: BenchFile = serde_json::from_str(text).map_err(|source| ReadError::Json { path: path.into(), source })?;
    if file.format != FORMAT || file.version != VERSION {
        return Err(ReadError::Format { path: path.into(), format: file.format, version: file.version });
    }
    Ok(file)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The p95 rose past the threshold and the interval leaves out no change,
    /// or the second run has failures the first did not.
    Slower,
    /// The p95 fell past the threshold and the interval leaves out no change.
    Faster,
    /// Neither: within the threshold, or not separated from no change.
    NoChange,
    TooFew,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Self::Slower => "slower",
            Self::Faster => "faster",
            Self::NoChange => "no change",
            Self::TooFew => "too few samples",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioDiff {
    pub name: String,
    pub n: (usize, usize),
    pub failed: (usize, usize),
    pub p50_us: (u64, u64),
    pub p95_us: (u64, u64),
    pub p50_change: Change,
    pub p95_change: Change,
    pub rank_p: f64,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BenchDiff {
    pub scenarios: Vec<ScenarioDiff>,
    /// What makes the two runs measure different things.
    pub warnings: Vec<String>,
}

impl BenchDiff {
    pub fn regressed(&self) -> bool {
        self.scenarios.iter().any(|s| s.verdict == Verdict::Slower)
    }
}

pub fn verdict(n: (usize, usize), failed: (usize, usize), p95: Change) -> Verdict {
    if failed.1 > failed.0 {
        return Verdict::Slower;
    }
    if n.0.min(n.1) < MIN_SAMPLES {
        return Verdict::TooFew;
    }
    if p95.lo > 0.0 && p95.point > THRESHOLD {
        Verdict::Slower
    } else if p95.hi < 0.0 && p95.point < -THRESHOLD {
        Verdict::Faster
    } else {
        Verdict::NoChange
    }
}

fn scenario_diff(a: &Scenario, b: &Scenario) -> ScenarioDiff {
    let sorted = |s: &Scenario| {
        let mut v = s.samples_us.clone();
        v.sort_unstable();
        v
    };
    let (sa, sb) = (sorted(a), sorted(b));
    let n = (sa.len(), sb.len());
    let failed = (a.failed, b.failed);
    let p95_change = quantile_change(&sa, &sb, 0.95, RESAMPLES, SEED);
    ScenarioDiff {
        name: a.name.clone(),
        n,
        failed,
        p50_us: (quantile(&sa, 0.50), quantile(&sb, 0.50)),
        p95_us: (quantile(&sa, 0.95), quantile(&sb, 0.95)),
        p50_change: quantile_change(&sa, &sb, 0.50, RESAMPLES, SEED),
        p95_change,
        rank_p: mann_whitney_p(&sa, &sb),
        verdict: verdict(n, failed, p95_change),
    }
}

pub fn diff(a: &BenchFile, b: &BenchFile) -> BenchDiff {
    let mut warnings = Vec::new();
    let mut differs = |what: &str, x: String, y: String| {
        if x != y {
            warnings.push(format!("{what} differs: {x} against {y}"));
        }
    };
    differs("build", a.build.clone(), b.build.clone());
    differs("embedder", a.embedder.clone(), b.embedder.clone());
    differs("machine", format!("{:?}", a.machine), format!("{:?}", b.machine));
    differs("store rows", a.store.rows.to_string(), b.store.rows.to_string());
    differs("store seed", a.store.seed.to_string(), b.store.seed.to_string());
    let mut scenarios = Vec::new();
    for sa in &a.scenarios {
        match b.scenarios.iter().find(|sb| sb.name == sa.name) {
            Some(sb) => scenarios.push(scenario_diff(sa, sb)),
            None => warnings.push(format!("{} is only in the first run", sa.name)),
        }
    }
    for sb in b.scenarios.iter().filter(|sb| !a.scenarios.iter().any(|sa| sa.name == sb.name)) {
        warnings.push(format!("{} is only in the second run", sb.name));
    }
    BenchDiff { scenarios, warnings }
}

fn ms(us: u64) -> f64 {
    us as f64 / 1000.0
}

fn pct(c: Change) -> String {
    format!("{:+.1}% [{:+.1}, {:+.1}]", c.point * 100.0, c.lo * 100.0, c.hi * 100.0)
}

pub fn render(a: &BenchFile, b: &BenchFile, d: &BenchDiff) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "first:  clawg {} {} {} embedder, {} rows\nsecond: clawg {} {} {} embedder, {} rows",
        a.clawg, a.build, a.embedder, a.store.rows, b.clawg, b.build, b.embedder, b.store.rows
    );
    for w in &d.warnings {
        let _ = writeln!(out, "warning: {w}");
    }
    let _ = writeln!(
        out,
        "{:<28} {:>9} {:>17} {:>28} {:>17} {:>28} {:>8}  verdict",
        "scenario", "n", "p50 ms", "p50 change [95%]", "p95 ms", "p95 change [95%]", "rank p"
    );
    for s in &d.scenarios {
        let _ = writeln!(
            out,
            "{:<28} {:>9} {:>17} {:>28} {:>17} {:>28} {:>8.4}  {}",
            s.name,
            format!("{}/{}", s.n.0, s.n.1),
            format!("{:.2}/{:.2}", ms(s.p50_us.0), ms(s.p50_us.1)),
            pct(s.p50_change),
            format!("{:.2}/{:.2}", ms(s.p95_us.0), ms(s.p95_us.1)),
            pct(s.p95_change),
            s.rank_p,
            if s.failed.1 > s.failed.0 {
                format!("{} ({} failed, {} before)", s.verdict.label(), s.failed.1, s.failed.0)
            } else {
                s.verdict.label().to_string()
            }
        );
    }
    let _ = writeln!(
        out,
        "slower: the p95 rose over {:.0}% and its interval leaves out no change, or new failures; \
         fewer than {MIN_SAMPLES} samples a side decide nothing",
        THRESHOLD * 100.0
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(build: &str, threads: usize, scenarios: Vec<Scenario>) -> BenchFile {
        BenchFile {
            format: FORMAT.into(),
            version: VERSION,
            clawg: "0.0.0".into(),
            build: build.into(),
            embedder: "fake".into(),
            machine: Machine { os: "linux".into(), arch: "x86_64".into(), threads },
            store: Store { rows: 10_000, seed: 74 },
            scenarios,
        }
    }

    fn scenario(name: &str, samples: Vec<u64>, failed: usize) -> Scenario {
        Scenario { name: name.into(), failed, samples_us: samples }
    }

    fn spread(n: u64, base: u64) -> Vec<u64> {
        (0..n).map(|i| base + (i * 37) % (base / 2)).collect()
    }

    #[test]
    fn the_verdict_needs_the_threshold_and_an_interval_without_zero() {
        let n = (100, 100);
        let c = |point, lo, hi| Change { point, lo, hi };
        assert_eq!(verdict(n, (0, 0), c(0.30, 0.10, 0.50)), Verdict::Slower);
        assert_eq!(verdict(n, (0, 0), c(0.30, -0.05, 0.50)), Verdict::NoChange, "the interval holds no change");
        assert_eq!(verdict(n, (0, 0), c(0.15, 0.05, 0.25)), Verdict::NoChange, "under the threshold");
        assert_eq!(verdict(n, (0, 0), c(-0.30, -0.50, -0.10)), Verdict::Faster);
        assert_eq!(verdict(n, (0, 0), c(-0.15, -0.25, -0.05)), Verdict::NoChange);
        assert_eq!(verdict((19, 100), (0, 0), c(0.90, 0.50, 1.50)), Verdict::TooFew);
        assert_eq!(verdict((19, 100), (0, 3), c(0.0, 0.0, 0.0)), Verdict::Slower, "new failures");
        assert_eq!(verdict(n, (3, 3), c(0.0, -0.1, 0.1)), Verdict::NoChange, "the same failures");
    }

    #[test]
    fn the_same_run_twice_is_no_change() {
        let a = file("debug", 4, vec![scenario("search", spread(400, 20_000), 0)]);
        let d = diff(&a, &a);
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
        assert_eq!(d.scenarios[0].verdict, Verdict::NoChange);
        assert_eq!(d.scenarios[0].rank_p, 1.0);
        assert!(!d.regressed());
    }

    #[test]
    fn a_slower_scenario_is_a_regression() {
        let a = file("debug", 4, vec![scenario("search", spread(400, 20_000), 0), scenario("save_1", spread(100, 20_000), 0)]);
        let slower: Vec<u64> = spread(400, 20_000).iter().map(|x| x + 8_000).collect();
        let b = file("debug", 4, vec![scenario("search", slower, 0), scenario("save_1", spread(100, 20_000), 0)]);
        let d = diff(&a, &b);
        assert_eq!(d.scenarios[0].verdict, Verdict::Slower);
        assert!(d.scenarios[0].rank_p < 1e-6);
        assert_eq!(d.scenarios[1].verdict, Verdict::NoChange);
        assert!(d.regressed());
        let text = render(&a, &b, &d);
        assert!(text.lines().any(|l| l.starts_with("search") && l.ends_with("slower")), "{text}");
    }

    #[test]
    fn what_differs_between_the_runs_is_named() {
        let a = file("debug", 4, vec![scenario("search", spread(40, 20_000), 0), scenario("backup", spread(40, 20_000), 0)]);
        let b = file("release", 8, vec![scenario("search", spread(40, 20_000), 0), scenario("refresh", spread(40, 20_000), 0)]);
        let d = diff(&a, &b);
        let joined = d.warnings.join("\n");
        for expected in ["build differs", "machine differs", "backup is only in the first run", "refresh is only in the second run"] {
            assert!(joined.contains(expected), "{expected}: {joined}");
        }
        assert_eq!(d.scenarios.len(), 1);
        assert!(render(&a, &b, &d).contains("warning: build differs: debug against release"));
    }

    #[test]
    fn a_result_file_is_read_by_its_format_and_ignores_what_it_does_not_compare() {
        let text = r#"{"format":"clawg-perf-bench","version":1,"clawg":"0.70.0","build":"debug","embedder":"fake",
            "machine":{"os":"linux","arch":"x86_64","threads":4},"store":{"rows":10,"seed":74,"seed_seconds":1.0},
            "process":{},"scenarios":[{"name":"search","rate_per_s":40.0,"failed":0,"samples_us":[1,2,3],"summary":{}}]}"#;
        let read = parse(text, "a.json").expect("a result file");
        assert_eq!(read.scenarios[0].samples_us, [1, 2, 3]);
        let other = text.replace("\"version\":1", "\"version\":2");
        assert!(matches!(parse(&other, "a.json"), Err(ReadError::Format { .. })));
        assert!(matches!(parse("{}", "a.json"), Err(ReadError::Json { .. })));
    }
}
