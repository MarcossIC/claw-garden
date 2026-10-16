//! An embedder that lets its model go after a stretch without use and loads
//! it again on the next call.
//!
//! The loaded model is the daemon's memory floor (about 1 GB of private bytes
//! for `multilingual-e5-small`), and a daemon spends most of its life with no
//! one asking for a vector. [`IdleEmbedder`] sits in the embedder cell for
//! the whole life of the process, so every reader of the cell keeps working
//! unchanged; only the model behind it comes and goes. Dropping the inner
//! embedder is what unloads it: for `E5Embedder` the last `Arc` takes the
//! request channel with it and the owning thread drops the session.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as SyncMutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::future::BoxFuture;
use tokio::time::Instant;

use crate::memory::embedder::{EmbedError, EmbedRole, Embedder};

/// Loads the model again: the same files and the same load the bootstrap used.
pub type ReloadFn =
    Arc<dyn Fn() -> BoxFuture<'static, Result<Arc<dyn Embedder>, String>> + Send + Sync>;

/// Env var holding how many minutes without use the daemon keeps the model;
/// `0` keeps it for the life of the process.
pub const IDLE_MINUTES_VAR: &str = "CLAWG_EMBEDDER_IDLE_MINUTES";

/// How long the model stays loaded with nobody using it. A working session
/// asks for vectors every few minutes, so ten keeps a reload out of the middle
/// of a task and still frees the memory over a long pause.
pub const DEFAULT_IDLE: Duration = Duration::from_secs(10 * 60);

/// The idle time [`IDLE_MINUTES_VAR`] asks for: `None` keeps the model
/// loaded. Unset or empty is [`DEFAULT_IDLE`]; anything but a whole number
/// of minutes is an error, which the caller reports before falling back.
pub fn idle_after(raw: Option<&str>) -> Result<Option<Duration>, String> {
    match raw.map(str::trim) {
        None | Some("") => Ok(Some(DEFAULT_IDLE)),
        Some(text) => match text.parse::<u64>() {
            Ok(0) => Ok(None),
            Ok(minutes) => Ok(Some(Duration::from_secs(minutes.saturating_mul(60)))),
            Err(_) => Err(format!(
                "{IDLE_MINUTES_VAR}={text:?} is not a whole number of minutes"
            )),
        },
    }
}

/// Whether a model last used at `last_used` should go at `now`. A call in
/// flight, or a load other calls are waiting on, always keeps it.
pub fn should_unload(now: Instant, last_used: Instant, in_flight: usize, idle: Duration) -> bool {
    in_flight == 0 && now.saturating_duration_since(last_used) >= idle
}

/// How often the sweeper looks: often enough that the model goes within a
/// quarter of the idle time past its due, and never busier than once a second.
fn sweep_period(idle: Duration) -> Duration {
    (idle / 4).clamp(Duration::from_secs(1), Duration::from_secs(60))
}

struct Usage {
    last_used: Instant,
    in_flight: usize,
}

struct Shared {
    /// Locked across a load, so calls arriving while the model is away share
    /// one load instead of each starting its own.
    model: tokio::sync::Mutex<Option<Arc<dyn Embedder>>>,
    usage: SyncMutex<Usage>,
    /// Read by health without waiting on a load that holds `model`.
    resident: AtomicBool,
    reload: ReloadFn,
    idle: Duration,
    dimensions: usize,
    model_id: String,
    query_prefix: String,
    passage_prefix: String,
}

/// Counts one user of the model from before its load until the call returns,
/// so the sweeper never takes the model from under it; dropping it stamps the
/// use, which a cancelled call counts as too.
struct InFlight(Arc<Shared>);

impl Drop for InFlight {
    fn drop(&mut self) {
        let mut usage = self.0.usage.lock().unwrap_or_else(|e| e.into_inner());
        usage.in_flight -= 1;
        usage.last_used = Instant::now();
    }
}

impl Shared {
    async fn lease(self: &Arc<Self>) -> Result<(Arc<dyn Embedder>, InFlight), EmbedError> {
        self.usage.lock().unwrap_or_else(|e| e.into_inner()).in_flight += 1;
        let guard = InFlight(self.clone());
        let mut slot = self.model.lock().await;
        if let Some(model) = slot.as_ref() {
            return Ok((model.clone(), guard));
        }
        let started = std::time::Instant::now();
        let model = match (self.reload)().await.and_then(|model| self.same_model(model)) {
            Ok(model) => model,
            Err(e) => {
                // Not ready rather than failed: the save that asked stays
                // pending and the next call loads again.
                log::warn!("could not load the embedding model again: {e}");
                return Err(EmbedError::NotReady);
            }
        };
        log::info!("embedding model loaded again in {} ms", started.elapsed().as_millis());
        *slot = Some(model.clone());
        self.resident.store(true, Ordering::Release);
        Ok((model, guard))
    }

    /// A model that came back with another identity would put new vectors in
    /// another space beside the stored ones.
    fn same_model(&self, model: Arc<dyn Embedder>) -> Result<Arc<dyn Embedder>, String> {
        if model.model_id() != self.model_id || model.dimensions() != self.dimensions {
            return Err(format!(
                "it loaded as {} with {} dimensions, not {} with {}",
                model.model_id(),
                model.dimensions(),
                self.model_id,
                self.dimensions
            ));
        }
        Ok(model)
    }

    async fn unload_if_idle(&self) -> bool {
        let mut slot = self.model.lock().await;
        if slot.is_none() {
            return false;
        }
        let due = {
            let usage = self.usage.lock().unwrap_or_else(|e| e.into_inner());
            should_unload(Instant::now(), usage.last_used, usage.in_flight, self.idle)
        };
        if due {
            self.resident.store(false, Ordering::Release);
            *slot = None;
            log::info!("embedding model unloaded after {} s without use", self.idle.as_secs());
        }
        due
    }
}

/// See the module docs.
pub struct IdleEmbedder {
    shared: Arc<Shared>,
}

impl IdleEmbedder {
    /// Wraps the `loaded` model, counting it as just used, and spawns on
    /// `runtime` the sweeper that unloads it once it sat `idle` unused. The
    /// sweeper ends when the embedder is dropped.
    pub fn new(
        loaded: Arc<dyn Embedder>,
        reload: ReloadFn,
        idle: Duration,
        runtime: &tokio::runtime::Handle,
    ) -> Self {
        let shared = Arc::new(Shared {
            dimensions: loaded.dimensions(),
            model_id: loaded.model_id().to_string(),
            query_prefix: loaded.query_prefix().to_string(),
            passage_prefix: loaded.passage_prefix().to_string(),
            model: tokio::sync::Mutex::new(Some(loaded)),
            usage: SyncMutex::new(Usage { last_used: Instant::now(), in_flight: 0 }),
            resident: AtomicBool::new(true),
            reload,
            idle,
        });
        let weak = Arc::downgrade(&shared);
        runtime.spawn(async move {
            let mut tick = tokio::time::interval(sweep_period(idle));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let Some(shared) = weak.upgrade() else { return };
                shared.unload_if_idle().await;
            }
        });
        Self { shared }
    }
}

#[cfg(any(test, feature = "testing"))]
impl IdleEmbedder {
    /// Lets the model go now, whatever its last use, so another crate's test
    /// can reach the unloaded state without waiting out the idle time.
    pub async fn unload(&self) {
        let mut slot = self.shared.model.lock().await;
        self.shared.resident.store(false, Ordering::Release);
        *slot = None;
    }
}

#[async_trait]
impl Embedder for IdleEmbedder {
    async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
        let (model, _in_flight) = self.shared.lease().await?;
        model.embed_with_role(text, role).await
    }

    async fn embed_batch(&self, texts: &[String], role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
        let (model, _in_flight) = self.shared.lease().await?;
        model.embed_batch(texts, role).await
    }

    fn dimensions(&self) -> usize {
        self.shared.dimensions
    }

    fn model_id(&self) -> &str {
        &self.shared.model_id
    }

    fn query_prefix(&self) -> &str {
        &self.shared.query_prefix
    }

    fn passage_prefix(&self) -> &str {
        &self.shared.passage_prefix
    }

    fn resident(&self) -> bool {
        self.shared.resident.load(Ordering::Acquire)
    }

    fn wake(&self) {
        if self.resident() {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
        let shared = self.shared.clone();
        runtime.spawn(async move {
            let _ = shared.lease().await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::embedder::FakeEmbedder;
    use std::sync::atomic::AtomicUsize;

    const IDLE: Duration = Duration::from_secs(600);

    fn fake() -> Arc<dyn Embedder> {
        Arc::new(FakeEmbedder::default())
    }

    /// A reload that counts its calls and yields once, the way a real load
    /// lets other callers queue behind it.
    fn counting_reload(loads: Arc<AtomicUsize>) -> ReloadFn {
        Arc::new(move || {
            let loads = loads.clone();
            Box::pin(async move {
                loads.fetch_add(1, Ordering::SeqCst);
                tokio::task::yield_now().await;
                Ok(fake())
            })
        })
    }

    fn idle_embedder(loads: Arc<AtomicUsize>) -> IdleEmbedder {
        IdleEmbedder::new(fake(), counting_reload(loads), IDLE, &tokio::runtime::Handle::current())
    }

    async fn pass(duration: Duration) {
        tokio::time::sleep(duration).await;
        // Lets the sweeper run the tick that came due.
        tokio::task::yield_now().await;
    }

    #[test]
    fn the_idle_time_defaults_to_ten_minutes_and_zero_keeps_the_model() {
        assert_eq!(idle_after(None), Ok(Some(DEFAULT_IDLE)));
        assert_eq!(idle_after(Some(" ")), Ok(Some(DEFAULT_IDLE)));
        assert_eq!(idle_after(Some("3")), Ok(Some(Duration::from_secs(180))));
        assert_eq!(idle_after(Some("0")), Ok(None));
    }

    #[test]
    fn an_idle_time_that_is_not_whole_minutes_is_refused() {
        for raw in ["ten", "-1", "1.5"] {
            let err = idle_after(Some(raw)).expect_err(raw);
            assert!(err.contains(IDLE_MINUTES_VAR), "{err}");
        }
    }

    #[test]
    fn a_model_goes_only_after_the_idle_time_with_nothing_in_flight() {
        let start = Instant::now();
        assert!(!should_unload(start + IDLE - Duration::from_secs(1), start, 0, IDLE));
        assert!(should_unload(start + IDLE, start, 0, IDLE));
        assert!(!should_unload(start + IDLE * 3, start, 1, IDLE));
    }

    #[tokio::test(start_paused = true)]
    async fn an_unused_model_is_unloaded_after_the_idle_time() {
        let loads = Arc::new(AtomicUsize::new(0));
        let embedder = idle_embedder(loads.clone());
        pass(IDLE - Duration::from_secs(60)).await;
        assert!(embedder.resident(), "still inside the idle time");
        pass(Duration::from_secs(60 + 150)).await;
        assert!(!embedder.resident());
        assert_eq!(loads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn each_use_restarts_the_idle_time() {
        let loads = Arc::new(AtomicUsize::new(0));
        let embedder = idle_embedder(loads);
        for _ in 0..3 {
            pass(IDLE / 2).await;
            embedder.embed_with_role("still working", EmbedRole::Query).await.expect("embed");
        }
        assert!(embedder.resident());
    }

    #[tokio::test(start_paused = true)]
    async fn a_call_after_an_unload_loads_the_model_again_and_gets_the_same_vector() {
        let loads = Arc::new(AtomicUsize::new(0));
        let embedder = idle_embedder(loads.clone());
        let before = embedder.embed_with_role("a text", EmbedRole::Query).await.expect("embed");
        pass(IDLE * 2).await;
        assert!(!embedder.resident());

        let after = embedder.embed_with_role("a text", EmbedRole::Query).await.expect("embed");
        assert_eq!(after, before);
        assert!(embedder.resident());
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn calls_arriving_while_the_model_is_away_share_one_load() {
        let loads = Arc::new(AtomicUsize::new(0));
        let embedder = Arc::new(idle_embedder(loads.clone()));
        pass(IDLE * 2).await;

        let calls = (0..8).map(|i| {
            let embedder = embedder.clone();
            tokio::spawn(async move {
                embedder.embed_batch(&[format!("text {i}")], EmbedRole::Passage).await
            })
        });
        for call in futures::future::join_all(calls).await {
            call.expect("task").expect("embed");
        }
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_call_in_flight_keeps_the_model_past_the_idle_time() {
        struct Slow;
        #[async_trait]
        impl Embedder for Slow {
            async fn embed_with_role(&self, _: &str, _: EmbedRole) -> Result<Vec<f32>, EmbedError> {
                tokio::time::sleep(IDLE * 3).await;
                Ok(vec![0.0; 384])
            }
            fn dimensions(&self) -> usize {
                384
            }
            fn model_id(&self) -> &str {
                "slow"
            }
            fn query_prefix(&self) -> &str {
                ""
            }
            fn passage_prefix(&self) -> &str {
                ""
            }
        }
        let reload: ReloadFn = Arc::new(|| Box::pin(async { Ok(Arc::new(Slow) as Arc<dyn Embedder>) }));
        let embedder = Arc::new(IdleEmbedder::new(
            Arc::new(Slow),
            reload,
            IDLE,
            &tokio::runtime::Handle::current(),
        ));
        let call = tokio::spawn({
            let embedder = embedder.clone();
            async move { embedder.embed_with_role("long", EmbedRole::Passage).await }
        });
        pass(IDLE * 2).await;
        assert!(embedder.resident(), "a call is still using the model");
        call.await.expect("task").expect("embed");
        pass(IDLE / 2).await;
        assert!(embedder.resident(), "the idle time starts when the call returns");
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_load_is_not_ready_and_the_next_call_tries_again() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let reload: ReloadFn = {
            let attempts = attempts.clone();
            Arc::new(move || {
                let first = attempts.fetch_add(1, Ordering::SeqCst) == 0;
                Box::pin(async move {
                    if first {
                        Err("the model file is gone".to_string())
                    } else {
                        Ok(fake())
                    }
                })
            })
        };
        let embedder = IdleEmbedder::new(fake(), reload, IDLE, &tokio::runtime::Handle::current());
        pass(IDLE * 2).await;

        let first = embedder.embed_with_role("a text", EmbedRole::Query).await;
        assert!(matches!(first, Err(EmbedError::NotReady)), "{first:?}");
        assert!(!embedder.resident());
        embedder.embed_with_role("a text", EmbedRole::Query).await.expect("second load works");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_model_that_comes_back_as_another_model_is_refused() {
        struct Other;
        #[async_trait]
        impl Embedder for Other {
            async fn embed_with_role(&self, _: &str, _: EmbedRole) -> Result<Vec<f32>, EmbedError> {
                Ok(vec![0.0; 384])
            }
            fn dimensions(&self) -> usize {
                384
            }
            fn model_id(&self) -> &str {
                "another-model"
            }
            fn query_prefix(&self) -> &str {
                ""
            }
            fn passage_prefix(&self) -> &str {
                ""
            }
        }
        let reload: ReloadFn = Arc::new(|| Box::pin(async { Ok(Arc::new(Other) as Arc<dyn Embedder>) }));
        let embedder = IdleEmbedder::new(fake(), reload, IDLE, &tokio::runtime::Handle::current());
        pass(IDLE * 2).await;

        let result = embedder.embed_with_role("a text", EmbedRole::Query).await;
        assert!(matches!(result, Err(EmbedError::NotReady)), "{result:?}");
        assert!(!embedder.resident());
    }

    #[tokio::test(start_paused = true)]
    async fn identity_and_prefixes_never_load_the_model() {
        let loads = Arc::new(AtomicUsize::new(0));
        let embedder = idle_embedder(loads.clone());
        pass(IDLE * 2).await;
        assert_eq!(embedder.model_id(), "fake-embedder");
        assert_eq!(embedder.dimensions(), 384);
        assert_eq!((embedder.query_prefix(), embedder.passage_prefix()), ("", ""));
        assert!(!embedder.resident());
        assert_eq!(loads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn wake_loads_the_model_without_a_caller_waiting() {
        let loads = Arc::new(AtomicUsize::new(0));
        let embedder = idle_embedder(loads.clone());
        pass(IDLE * 2).await;

        embedder.wake();
        assert!(!embedder.resident(), "wake returns before the load");
        pass(Duration::from_millis(1)).await;
        assert!(embedder.resident());
        assert_eq!(loads.load(Ordering::SeqCst), 1);

        embedder.wake();
        pass(Duration::from_millis(1)).await;
        assert_eq!(loads.load(Ordering::SeqCst), 1, "a resident model has nothing to load");
    }

    #[tokio::test(start_paused = true)]
    async fn a_woken_model_gets_a_full_idle_time() {
        let loads = Arc::new(AtomicUsize::new(0));
        let embedder = idle_embedder(loads);
        pass(IDLE * 2).await;
        embedder.wake();
        pass(IDLE / 2).await;
        assert!(embedder.resident());
    }

    #[tokio::test(start_paused = true)]
    async fn the_sweeper_ends_with_the_embedder() {
        let loads = Arc::new(AtomicUsize::new(0));
        let embedder = idle_embedder(loads);
        let weak = Arc::downgrade(&embedder.shared);
        drop(embedder);
        pass(IDLE).await;
        assert!(weak.upgrade().is_none());
    }
}
