//! What indexing a project's docs cost: the `index` block of a docs results
//! file and the lines of the report that read it.

use clawg_core::eval_api::{EvalCpu, EvalIndexRun, EvalMemoReport};
use serde::Serialize;

/// Seconds a refresh spent in each of its phases, summed over its docs. The
/// removal of docs that are gone counts as a write.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Phases {
    pub scan: f64,
    pub read: f64,
    pub mask: f64,
    pub split: f64,
    pub lookup: f64,
    pub embed: f64,
    pub write: f64,
}

impl Phases {
    pub fn of(run: &EvalIndexRun) -> Self {
        let secs = |f: fn(&clawg_core::eval_api::EvalPhases) -> std::time::Duration| {
            run.docs.iter().map(|d| f(&d.phases)).sum::<std::time::Duration>().as_secs_f64()
        };
        Self {
            scan: run.scan.as_secs_f64(),
            read: secs(|p| p.read),
            mask: secs(|p| p.mask),
            split: secs(|p| p.split),
            lookup: secs(|p| p.lookup),
            embed: secs(|p| p.embed),
            write: secs(|p| p.write) + run.remove.as_secs_f64(),
        }
    }

    /// One line for the report, in the order a refresh runs them.
    pub fn line(&self) -> String {
        format!(
            "phases: scan {:.2} s  read {:.2} s  mask {:.2} s  split {:.2} s  lookup {:.2} s  embed {:.2} s  write {:.2} s\n",
            self.scan, self.read, self.mask, self.split, self.lookup, self.embed, self.write,
        )
    }
}

/// The machine a run measured on: what decides the kernels of the model and
/// the threads its session runs with.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Machine {
    #[serde(flatten)]
    pub cpu: EvalCpu,
    pub threads: usize,
}

/// How many passages came from the vector memo and how many the model
/// embedded.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct MemoBlock {
    pub used: bool,
    pub from_memo: usize,
    pub embedded: usize,
    pub seconds: f64,
}

impl MemoBlock {
    /// A run with no memo embeds every passage of the corpus.
    fn of(memo: Option<&EvalMemoReport>, passages: usize) -> Self {
        match memo {
            Some(m) => Self { used: m.used, from_memo: m.from_memo, embedded: m.embedded, seconds: m.seconds },
            None => Self { used: false, from_memo: 0, embedded: passages, seconds: 0.0 },
        }
    }
}

/// The `index` block of a docs results file: numbers, flags and the names of
/// the build, the system and the architecture, never a path or a text.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IndexBlock {
    pub build: &'static str,
    pub machine: Machine,
    pub phases: Phases,
    pub memo: MemoBlock,
}

impl IndexBlock {
    pub fn new(run: &EvalIndexRun, memo: Option<&EvalMemoReport>, cpu: EvalCpu, threads: usize, debug: bool) -> Self {
        Self {
            build: if debug { "debug" } else { "release" },
            machine: Machine { cpu, threads },
            phases: Phases::of(run),
            memo: MemoBlock::of(memo, run.report.passages),
        }
    }
}
