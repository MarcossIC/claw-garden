//! How long each step of an import took and what it went through, logged as
//! each step ends, so an import of twenty minutes shows where its time goes
//! while it runs and the steps add up against the request's own duration.
//! Names and numbers only: no line carries a row's text, an id or a path.

use std::fmt;
use std::time::{Duration, Instant};

/// A step of an import, in the order an import runs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The handler reads the file.
    Read,
    /// The ids a registration would give each root.
    Plan,
    /// A clawg export parsed, masked and made rows.
    Prepare,
    /// Another tool's notes masked, split and made rows.
    Translate,
    /// What the decision pass reads of the store.
    Load,
    /// The decision pass.
    Decide,
    Embed,
    /// The `pre-import` snapshot.
    Snapshot,
    /// The rows, in their write batches.
    Write,
    /// The links, with the journal's end.
    Links,
}

impl Step {
    fn name(self) -> &'static str {
        match self {
            Step::Read => "read",
            Step::Plan => "plan",
            Step::Prepare => "prepare",
            Step::Translate => "translate",
            Step::Load => "load",
            Step::Decide => "decide",
            Step::Embed => "embed",
            Step::Snapshot => "snapshot",
            Step::Write => "write",
            Step::Links => "links",
        }
    }
}

/// Calls of a step to the store's connection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Calls {
    pub count: usize,
    /// Inside the calls: how long they held the connection.
    pub in_sqlite: Duration,
    /// The rest of each call, waiting for the connection behind other calls.
    pub queued: Duration,
}

/// One step of an import, as it is logged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timing {
    pub step: Step,
    pub elapsed: Duration,
    pub rows: Option<usize>,
    pub batches: Option<usize>,
    pub calls: Option<Calls>,
    /// The call that held the connection longest, or the slowest batch.
    pub slowest: Option<Duration>,
}

impl Timing {
    pub fn new(step: Step) -> Self {
        Timing { step, elapsed: Duration::ZERO, rows: None, batches: None, calls: None, slowest: None }
    }

    /// Counts a call that took `waited` from being asked for to its answer,
    /// `ran` of it on the connection.
    pub fn call(&mut self, waited: Duration, ran: Duration) {
        let calls = self.calls.get_or_insert_default();
        calls.count += 1;
        calls.in_sqlite += ran;
        calls.queued += waited.saturating_sub(ran);
        self.slower(ran);
    }

    /// Counts a batch that took `took`.
    pub fn batch(&mut self, took: Duration) {
        *self.batches.get_or_insert(0) += 1;
        self.slower(took);
    }

    fn slower(&mut self, took: Duration) {
        self.slowest = Some(self.slowest.map_or(took, |s| s.max(took)));
    }
}

fn seconds(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64())
}

impl fmt::Display for Timing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if let Some(rows) = self.rows {
            parts.push(format!("{rows} rows"));
        }
        if let Some(batches) = self.batches {
            parts.push(format!("{batches} batches"));
            if let Some(rows) = self.rows.filter(|_| !self.elapsed.is_zero()) {
                parts.push(format!("{:.1} rows/s", rows as f64 / self.elapsed.as_secs_f64()));
            }
        }
        if let Some(calls) = self.calls {
            parts.push(if calls.count == 1 { "1 call".to_owned() } else { format!("{} calls", calls.count) });
            parts.push(format!("{} s in sqlite", seconds(calls.in_sqlite)));
            parts.push(format!("{} s queued", seconds(calls.queued)));
        }
        if let Some(slowest) = self.slowest {
            parts.push(format!("slowest {} ms", slowest.as_millis()));
        }
        write!(f, "{} {} s", self.step.name(), seconds(self.elapsed))?;
        if !parts.is_empty() {
            write!(f, " ({})", parts.join(", "))?;
        }
        Ok(())
    }
}

/// The steps of one import, from the moment the daemon took it up.
#[derive(Debug)]
pub struct ImportTimings {
    started: Instant,
    dry_run: bool,
    steps: Vec<Timing>,
}

impl ImportTimings {
    pub fn new(dry_run: bool) -> Self {
        ImportTimings { started: Instant::now(), dry_run, steps: Vec::new() }
    }

    /// Logs a step that has just ended.
    pub fn record(&mut self, timing: Timing) {
        log::info!("import: {timing}");
        self.steps.push(timing);
    }

    pub fn steps(&self) -> &[Timing] {
        &self.steps
    }

    pub fn summary(&self, total: Duration) -> Summary<'_> {
        Summary { timings: self, total }
    }

    /// Logs the summary of an import that ended, its total counted from
    /// [`ImportTimings::new`].
    pub fn finish(&self) {
        log::info!("{}", self.summary(self.started.elapsed()));
    }
}

/// The line that closes an import: its total and every step.
pub struct Summary<'a> {
    timings: &'a ImportTimings,
    total: Duration,
}

impl fmt::Display for Summary<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let dry = if self.timings.dry_run { "dry run " } else { "" };
        let steps: Vec<String> =
            self.timings.steps.iter().map(|t| format!("{} {}", t.step.name(), seconds(t.elapsed))).collect();
        write!(f, "import: {dry}done in {} s: {}", seconds(self.total), steps.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn timing(step: Step, elapsed: u64) -> Timing {
        Timing { elapsed: ms(elapsed), ..Timing::new(step) }
    }

    #[test]
    fn a_step_reads_as_its_numbers() {
        let mut load = Timing { rows: Some(812), ..timing(Step::Load, 3_140) };
        load.call(ms(500), ms(400));
        load.call(ms(2_200), ms(2_159));
        for _ in 0..3 {
            load.call(ms(10), ms(10));
        }
        load.call(ms(41), ms(41));
        assert_eq!(load.calls.unwrap().count, 6);
        assert_eq!(load.to_string(), "load 3.1 s (812 rows, 6 calls, 2.6 s in sqlite, 0.1 s queued, slowest 2159 ms)");

        let mut embed = Timing { rows: Some(64), ..timing(Step::Embed, 1_600) };
        embed.batch(ms(700));
        embed.batch(ms(900));
        assert_eq!(embed.to_string(), "embed 1.6 s (64 rows, 2 batches, 40.0 rows/s, slowest 900 ms)");

        let mut links = timing(Step::Links, 40);
        links.call(ms(40), ms(30));
        assert_eq!(links.to_string(), "links 0.0 s (1 call, 0.0 s in sqlite, 0.0 s queued, slowest 30 ms)");

        assert_eq!(timing(Step::Snapshot, 1_550).to_string(), "snapshot 1.6 s");
    }

    #[test]
    fn the_summary_names_every_step() {
        let steps = [
            (Step::Read, 1_800),
            (Step::Plan, 10),
            (Step::Translate, 61_000),
            (Step::Load, 3_100),
            (Step::Decide, 4_900),
            (Step::Embed, 1_142_000),
            (Step::Snapshot, 1_600),
            (Step::Write, 23_000),
            (Step::Links, 400),
        ];
        let mut timings = ImportTimings::new(false);
        for (step, elapsed) in steps {
            timings.record(timing(step, elapsed));
        }
        assert_eq!(
            timings.summary(ms(1_362_400)).to_string(),
            "import: done in 1362.4 s: read 1.8, plan 0.0, translate 61.0, load 3.1, decide 4.9, embed 1142.0, \
             snapshot 1.6, write 23.0, links 0.4"
        );

        let mut dry = ImportTimings::new(true);
        for (step, elapsed) in [(Step::Read, 50), (Step::Plan, 0), (Step::Prepare, 120), (Step::Load, 30), (Step::Decide, 70)] {
            dry.record(timing(step, elapsed));
        }
        assert_eq!(
            dry.summary(ms(300)).to_string(),
            "import: dry run done in 0.3 s: read 0.1, plan 0.0, prepare 0.1, load 0.0, decide 0.1"
        );
    }
}
