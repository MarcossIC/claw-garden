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

/// Why an import was asked to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopCause {
    /// Its caller no longer wants it.
    Cancel,
    /// The process is going away.
    Shutdown,
}

#[derive(Default)]
struct Shared {
    stop: watch::Sender<Option<StopCause>>,
    progress: watch::Sender<Progress>,
    #[cfg(any(test, feature = "testing"))]
    on_step: std::sync::OnceLock<Box<dyn Fn(Step) + Send + Sync>>,
}

/// Shared by the import and whoever follows it or stops it.
#[derive(Clone, Default)]
pub struct ImportControl(Arc<Shared>);

impl ImportControl {
    pub fn progress(&self) -> Progress {
        self.0.progress.borrow().clone()
    }

    /// Asks the import to stop at the next safe point of its step. Returns
    /// whether this was the first cause: the first one stays.
    pub fn stop(&self, cause: StopCause) -> bool {
        self.0.stop.send_if_modified(|raised| {
            let first = raised.is_none();
            if first {
                *raised = Some(cause);
            }
            first
        })
    }

    pub fn cause(&self) -> Option<StopCause> {
        *self.0.stop.borrow()
    }

    /// Waits for a stop; one raised before the wait counts.
    pub(super) async fn raised(&self) -> StopCause {
        let mut stop = self.0.stop.subscribe();
        loop {
            if let Some(cause) = *stop.borrow_and_update() {
                return cause;
            }
            // The sender lives in `self`, so the channel never closes.
            if stop.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }

    /// Runs `f` as each step begins, once it is published, so a test can raise
    /// a stop exactly on entering a step. Only the first call sets it.
    #[cfg(any(test, feature = "testing"))]
    pub fn on_step(&self, f: impl Fn(Step) + Send + Sync + 'static) {
        let _ = self.0.on_step.set(Box::new(f));
    }

    /// Publishes `step` as the one running and starts its clock.
    pub(super) fn begin(&self, step: Step) -> (Instant, Timing) {
        self.0.progress.send_modify(|p| {
            p.step = Some(step);
            p.step_started_at_ms = TimestampMs::now().as_millis_i64();
        });
        #[cfg(any(test, feature = "testing"))]
        if let Some(hook) = self.0.on_step.get() {
            hook(step);
        }
        (Instant::now(), Timing::new(step))
    }

    pub(super) fn count(&self, f: impl FnOnce(&mut Counters)) {
        self.0.progress.send_modify(|p| f(&mut p.counters));
    }

    pub(super) fn numbered(&self, import_id: Option<i64>) {
        self.0.progress.send_modify(|p| p.import_id = import_id);
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

    #[test]
    fn the_first_stop_cause_wins() {
        let control = ImportControl::default();
        assert_eq!(control.cause(), None);
        assert!(control.stop(StopCause::Cancel));
        assert!(!control.stop(StopCause::Shutdown), "a second cause is not the first");
        assert_eq!(control.clone().cause(), Some(StopCause::Cancel));
    }

    #[tokio::test]
    async fn a_stop_raised_before_waiting_is_not_lost() {
        let control = ImportControl::default();
        control.stop(StopCause::Shutdown);
        assert_eq!(control.raised().await, StopCause::Shutdown);
    }

    #[tokio::test]
    async fn waiting_for_a_stop_ends_when_it_is_raised() {
        let control = ImportControl::default();
        let waiting = tokio::spawn({
            let control = control.clone();
            async move { control.raised().await }
        });
        tokio::task::yield_now().await;
        control.stop(StopCause::Cancel);
        assert_eq!(waiting.await.unwrap(), StopCause::Cancel);
    }

    #[test]
    fn a_step_hook_sees_each_step_as_it_begins() {
        let control = ImportControl::default();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        control.on_step(move |step| log.lock().unwrap().push(step));
        control.begin(Step::Load);
        control.begin(Step::Decide);
        assert_eq!(*seen.lock().unwrap(), vec![Step::Load, Step::Decide]);
    }
}
