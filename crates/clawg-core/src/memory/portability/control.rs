//! What the caller of an import can read while it runs: the step it is in and
//! how far each step got.

use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::memory::clock::TimestampMs;
use crate::memory::portability::timing::{Step, Timing};

/// Rows and notes an import went through, by step. None of it depends on how
/// many rows a batch holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counters {
    /// The rows of the file, once it is prepared.
    pub rows: usize,
    /// The notes of those rows: one for each row of a clawg file.
    pub notes: usize,
    pub rows_to_embed: usize,
    pub rows_embedded: usize,
    /// The rows of the write batches that were committed.
    pub rows_written: usize,
    pub notes_written: usize,
}

/// Where an import is, as one value, so a reader never sees a counter of one
/// moment next to another's.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Progress {
    pub step: Option<Step>,
    pub step_started_at_ms: i64,
    pub counters: Counters,
    /// The import's number in `memory_import`, once it wrote.
    pub import_id: Option<i64>,
}

/// Shared by the import and whoever follows it.
#[derive(Clone, Default)]
pub struct ImportControl(Arc<watch::Sender<Progress>>);

impl ImportControl {
    pub fn progress(&self) -> Progress {
        self.0.borrow().clone()
    }

    /// Publishes `step` as the one running and starts its clock.
    pub(super) fn begin(&self, step: Step) -> (Instant, Timing) {
        self.0.send_modify(|p| {
            p.step = Some(step);
            p.step_started_at_ms = TimestampMs::now().as_millis_i64();
        });
        (Instant::now(), Timing::new(step))
    }

    pub(super) fn count(&self, f: impl FnOnce(&mut Counters)) {
        self.0.send_modify(|p| f(&mut p.counters));
    }

    pub(super) fn numbered(&self, import_id: Option<i64>) {
        self.0.send_modify(|p| p.import_id = import_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_is_published_when_it_begins() {
        let control = ImportControl::default();
        assert_eq!(control.progress(), Progress::default());

        let (_, timing) = control.begin(Step::Embed);
        assert_eq!(timing.step, Step::Embed);
        let progress = control.progress();
        assert_eq!(progress.step, Some(Step::Embed));
        assert!(progress.step_started_at_ms > 0);

        control.count(|c| c.rows_embedded += 32);
        control.numbered(Some(7));
        let seen = control.clone().progress();
        assert_eq!((seen.counters.rows_embedded, seen.import_id), (32, Some(7)));
    }
}
