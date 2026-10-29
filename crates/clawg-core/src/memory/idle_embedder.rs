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
//!
//! The same reload brings back a model whose inference worker is gone: a call
//! that finds it so drops it, loads it again and repeats itself once.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as SyncMutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::future::BoxFuture;
use tokio::time::Instant;

use crate::memory::embedder::{EmbedError, EmbedRole, Embedder};
use crate::metrics::{mib, MemoryProbes};

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

/// The least time between two loads that replace a model whose worker was
/// gone. A model that dies on every call would otherwise be read, checked and
/// built again on every call.
pub const RECOVERY_INTERVAL: Duration = Duration::from_secs(60);

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

/// Who asked for a load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cause {
    /// A project registration, ahead of the calls it expects.
    Wake,
    /// A call that needs a vector now.
    Call,
    /// A call that found the model's worker gone.
    Recovery,
}

/// The line a load logs: who asked, how long it took and the private memory
/// around it and the process's private peak before and after, when the daemon
/// gave a way to read them. The peak is the process's in the load's window,
/// so a task running meanwhile may be the one that raised it.
fn load_line(cause: Cause, took: Duration, memory: Option<(u64, u64)>, peak: Option<(u64, u64)>) -> String {
    let asked = match cause {
        Cause::Wake => "for a wake",
        Cause::Call => "for a call that embeds",
        Cause::Recovery => "for a call that found its worker gone",
    };
    let memory = memory.map(|(before, after)| format!("; private memory {} before, {} after", mib(before), mib(after)));
    let peak = peak.map(|(before, after)| {
        if after > before {
            format!("; private peak rose to {}", mib(after))
        } else {
            format!("; private peak stayed at {}, set before this load", mib(before))
        }
    });
    format!(
        "embedding model loaded again in {} ms {asked}{}{}",
        took.as_millis(),
        memory.unwrap_or_default(),
        peak.unwrap_or_default()
    )
}

/// What the sweeper did when the model went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Unloaded {
    /// A wake loaded it and no call used it since.
    woken_unused: bool,
}

struct Usage {
    last_used: Instant,
    in_flight: usize,
}

struct Slot {
    model: Option<Arc<dyn Embedder>>,
    /// The model was dropped because its worker was gone, so the next load
    /// is a recovery.
    recovering: bool,
    last_recovery: Option<Instant>,
    /// When a wake loaded the model in the slot, until a call uses it.
    woken_at: Option<Instant>,
}

struct Shared {
    /// Locked across a load, so calls arriving while the model is away share
    /// one load instead of each starting its own.
    model: tokio::sync::Mutex<Slot>,
    usage: SyncMutex<Usage>,
    /// Read by health without waiting on a load that holds `model`.
    resident: AtomicBool,
    reload: ReloadFn,
    memory: Option<MemoryProbes>,
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
    async fn lease(self: &Arc<Self>, mut cause: Cause) -> Result<(Arc<dyn Embedder>, InFlight), EmbedError> {
        self.usage.lock().unwrap_or_else(|e| e.into_inner()).in_flight += 1;
        let guard = InFlight(self.clone());
        let mut slot = self.model.lock().await;
        if let Some(model) = slot.model.clone() {
            if cause == Cause::Call {
                if let Some(woken) = slot.woken_at.take() {
                    log::info!("the model a wake loaded was first used {} s later", woken.elapsed().as_secs());
                }
            }
            return Ok((model, guard));
        }
        if slot.recovering {
            cause = Cause::Recovery;
            let now = Instant::now();
            if slot.last_recovery.is_some_and(|last| now.saturating_duration_since(last) < RECOVERY_INTERVAL) {
                return Err(EmbedError::NotReady);
            }
            slot.recovering = false;
            slot.last_recovery = Some(now);
        }
        let before = self.memory.and_then(|probes| (probes.private)());
        let peak_before = self.memory.and_then(|probes| (probes.private_peak)());
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
        let took = started.elapsed();
        let after = self.memory.and_then(|probes| (probes.private)());
        let peak_after = self.memory.and_then(|probes| (probes.private_peak)());
        log::info!("{}", load_line(cause, took, before.zip(after), peak_before.zip(peak_after)));
        slot.model = Some(model.clone());
        slot.woken_at = (cause == Cause::Wake).then(Instant::now);
        self.resident.store(true, Ordering::Release);
        Ok((model, guard))
    }

    /// Drops `dead`, whose worker is gone, unless another call already
    /// replaced it: that one's load must not be thrown away.
    async fn discard(&self, dead: &Arc<dyn Embedder>) {
        let mut slot = self.model.lock().await;
        if !slot.model.as_ref().is_some_and(|model| Arc::ptr_eq(model, dead)) {
            return;
        }
        log::warn!("the embedding model's inference worker is gone; the model is dropped and loaded again");
        self.resident.store(false, Ordering::Release);
        slot.model = None;
        slot.recovering = true;
    }

    /// Runs `call` on the model, and once more on a model loaded again when
    /// the first one's worker is gone.
    async fn with_model<T, F>(self: &Arc<Self>, call: impl Fn(Arc<dyn Embedder>) -> F) -> Result<T, EmbedError>
    where
        F: std::future::Future<Output = Result<T, EmbedError>>,
    {
        for _ in 0..2 {
            let (model, _in_flight) = self.lease(Cause::Call).await?;
            match call(model.clone()).await {
                Err(EmbedError::WorkerGone) => self.discard(&model).await,
                answer => return answer,
            }
        }
        Err(EmbedError::NotReady)
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

    async fn unload_if_idle(&self, idle: Duration) -> Option<Unloaded> {
        let mut slot = self.model.lock().await;
        slot.model.as_ref()?;
        let due = {
            let usage = self.usage.lock().unwrap_or_else(|e| e.into_inner());
            should_unload(Instant::now(), usage.last_used, usage.in_flight, idle)
        };
        if !due {
            return None;
        }
        self.resident.store(false, Ordering::Release);
        slot.model = None;
        let unloaded = Unloaded { woken_unused: slot.woken_at.take().is_some() };
        let unused = if unloaded.woken_unused { "; no call used it after the wake that loaded it" } else { "" };
        log::info!("embedding model unloaded after {} s without use{unused}", idle.as_secs());
        Some(unloaded)
    }
}

/// See the module docs.
pub struct IdleEmbedder {
    shared: Arc<Shared>,
}

impl IdleEmbedder {
    /// Wraps the `loaded` model, counting it as just used, and spawns on
    /// `runtime` the sweeper that unloads it once it sat `idle` unused. The
    /// sweeper ends when the embedder is dropped. With no `idle` there is no
    /// sweeper and the model only goes when its worker does. Each load logs
    /// the private memory and peak `memory` reads before and after it.
    pub fn new(
        loaded: Arc<dyn Embedder>,
        reload: ReloadFn,
        idle: Option<Duration>,
        memory: Option<MemoryProbes>,
        runtime: &tokio::runtime::Handle,
    ) -> Self {
        let shared = Arc::new(Shared {
            dimensions: loaded.dimensions(),
            model_id: loaded.model_id().to_string(),
            query_prefix: loaded.query_prefix().to_string(),
            passage_prefix: loaded.passage_prefix().to_string(),
            model: tokio::sync::Mutex::new(Slot {
                model: Some(loaded),
                recovering: false,
                last_recovery: None,
                woken_at: None,
            }),
            usage: SyncMutex::new(Usage { last_used: Instant::now(), in_flight: 0 }),
            resident: AtomicBool::new(true),
            reload,
            memory,
        });
        let Some(idle) = idle else { return Self { shared } };
        let weak = Arc::downgrade(&shared);
        runtime.spawn(async move {
            let mut tick = tokio::time::interval(sweep_period(idle));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let Some(shared) = weak.upgrade() else { return };
                shared.unload_if_idle(idle).await;
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
        slot.model = None;
        slot.woken_at = None;
    }
}

#[async_trait]
impl Embedder for IdleEmbedder {
    async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
        self.shared.with_model(|model| async move { model.embed_with_role(text, role).await }).await
    }

    async fn embed_batch(&self, texts: &[String], role: EmbedRole) -> Result<Vec<Vec<f32>>, EmbedError> {
        self.shared.with_model(|model| async move { model.embed_batch(texts, role).await }).await
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
            let _ = shared.lease(Cause::Wake).await;
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
        IdleEmbedder::new(fake(), counting_reload(loads), Some(IDLE), None, &tokio::runtime::Handle::current())
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
            Some(IDLE),
            None,
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
        let embedder = IdleEmbedder::new(fake(), reload, Some(IDLE), None, &tokio::runtime::Handle::current());
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
        let embedder = IdleEmbedder::new(fake(), reload, Some(IDLE), None, &tokio::runtime::Handle::current());
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

    /// An embedder with no sweeper of its own and its model already gone, so
    /// a test runs the sweep itself.
    async fn unswept() -> IdleEmbedder {
        let embedder =
            IdleEmbedder::new(fake(), counting_reload(Arc::new(AtomicUsize::new(0))), None, None, &tokio::runtime::Handle::current());
        embedder.unload().await;
        embedder
    }

    #[tokio::test(start_paused = true)]
    async fn the_sweep_tells_a_woken_model_nobody_used_from_one_a_call_used() {
        let embedder = unswept().await;
        embedder.wake();
        pass(IDLE).await;
        assert_eq!(embedder.shared.unload_if_idle(IDLE).await, Some(Unloaded { woken_unused: true }));

        embedder.wake();
        pass(Duration::from_millis(1)).await;
        embedder.embed_with_role("a text", EmbedRole::Query).await.expect("embed");
        pass(IDLE).await;
        assert_eq!(embedder.shared.unload_if_idle(IDLE).await, Some(Unloaded { woken_unused: false }));
        assert_eq!(embedder.shared.unload_if_idle(IDLE).await, None, "nothing left to unload");
    }

    #[tokio::test(start_paused = true)]
    async fn a_model_a_call_loaded_is_never_counted_as_woken() {
        let embedder = unswept().await;
        embedder.embed_with_role("a text", EmbedRole::Query).await.expect("embed");
        pass(IDLE).await;
        assert_eq!(embedder.shared.unload_if_idle(IDLE).await, Some(Unloaded { woken_unused: false }));
    }

    #[test]
    fn a_load_names_who_asked_and_the_memory_around_it() {
        assert_eq!(
            load_line(Cause::Wake, Duration::from_millis(9_234), Some((297_795_584, 775_946_240)), None),
            "embedding model loaded again in 9234 ms for a wake; private memory 284.0 MiB before, 740.0 MiB after"
        );
        assert_eq!(load_line(Cause::Call, Duration::from_millis(12), None, None), "embedding model loaded again in 12 ms for a call that embeds");
        assert!(load_line(Cause::Recovery, Duration::ZERO, None, None).ends_with("for a call that found its worker gone"));
    }

    const MIB: u64 = 1_048_576;

    #[test]
    fn a_load_line_names_the_peak_when_the_load_raised_it() {
        let line = load_line(
            Cause::Wake,
            Duration::from_millis(9_234),
            Some((297_795_584, 775_946_240)),
            Some((1_000 * MIB, 1_600 * MIB)),
        );
        assert_eq!(
            line,
            "embedding model loaded again in 9234 ms for a wake; private memory 284.0 MiB before, 740.0 MiB after; private peak rose to 1600.0 MiB"
        );
    }

    #[test]
    fn a_load_line_says_when_the_load_did_not_pass_the_peak() {
        let line = load_line(
            Cause::Call,
            Duration::from_millis(1_102),
            Some((300 * MIB, 760 * MIB)),
            Some((1_622 * MIB, 1_622 * MIB)),
        );
        assert!(line.ends_with("; private peak stayed at 1622.0 MiB, set before this load"), "{line}");
    }

    #[test]
    fn a_load_line_without_a_peak_probe_is_unchanged() {
        let memory = Some((297_795_584, 775_946_240));
        let today = "embedding model loaded again in 9234 ms for a wake; private memory 284.0 MiB before, 740.0 MiB after";
        assert_eq!(load_line(Cause::Wake, Duration::from_millis(9_234), memory, None), today);
        assert_eq!(
            load_line(Cause::Call, Duration::from_millis(12), None, None),
            "embedding model loaded again in 12 ms for a call that embeds"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn with_no_idle_time_the_model_stays_and_a_dead_worker_is_still_recovered() {
        let loads = Arc::new(AtomicUsize::new(0));
        let embedder =
            IdleEmbedder::new(fake(), counting_reload(loads.clone()), None, None, &tokio::runtime::Handle::current());
        pass(IDLE * 100).await;
        assert!(embedder.resident());
        assert_eq!(loads.load(Ordering::SeqCst), 0);

        let embedder =
            IdleEmbedder::new(dead(), counting_reload(loads.clone()), None, None, &tokio::runtime::Handle::current());
        embedder.embed_with_role("a text", EmbedRole::Query).await.expect("recovered");
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    /// A model whose inference worker is gone. With a barrier, each call
    /// waits there first, so callers can be made to hold it together.
    struct Dead(Option<Arc<tokio::sync::Barrier>>);

    #[async_trait]
    impl Embedder for Dead {
        async fn embed_with_role(&self, _: &str, _: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            if let Some(barrier) = &self.0 {
                barrier.wait().await;
            }
            Err(EmbedError::WorkerGone)
        }
        fn dimensions(&self) -> usize {
            384
        }
        fn model_id(&self) -> &str {
            "fake-embedder"
        }
        fn query_prefix(&self) -> &str {
            ""
        }
        fn passage_prefix(&self) -> &str {
            ""
        }
    }

    fn dead() -> Arc<dyn Embedder> {
        Arc::new(Dead(None))
    }

    #[tokio::test(start_paused = true)]
    async fn a_model_whose_worker_is_gone_is_loaded_again_within_the_call() {
        let loads = Arc::new(AtomicUsize::new(0));
        let embedder =
            IdleEmbedder::new(dead(), counting_reload(loads.clone()), Some(IDLE), None, &tokio::runtime::Handle::current());

        let vector = embedder.embed_with_role("a text", EmbedRole::Query).await.expect("the call recovers");
        assert_eq!(vector, fake().embed_with_role("a text", EmbedRole::Query).await.unwrap());
        assert!(embedder.resident());
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        embedder.embed_batch(&["another".to_string()], EmbedRole::Passage).await.expect("batch");
        assert_eq!(loads.load(Ordering::SeqCst), 1, "the recovered model stays");
    }

    #[tokio::test(start_paused = true)]
    async fn calls_that_find_the_worker_gone_together_share_one_reload() {
        let loads = Arc::new(AtomicUsize::new(0));
        let both_hold_it = Arc::new(tokio::sync::Barrier::new(2));
        let embedder = Arc::new(IdleEmbedder::new(
            Arc::new(Dead(Some(both_hold_it))),
            counting_reload(loads.clone()),
            Some(IDLE),
            None,
            &tokio::runtime::Handle::current(),
        ));

        let calls = (0..2).map(|i| {
            let embedder = embedder.clone();
            tokio::spawn(async move { embedder.embed_with_role(&format!("text {i}"), EmbedRole::Query).await })
        });
        for call in futures::future::join_all(calls).await {
            call.expect("task").expect("embed");
        }
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        assert!(embedder.resident());
    }

    #[tokio::test(start_paused = true)]
    async fn a_model_that_keeps_dying_is_reloaded_at_most_once_a_minute() {
        let loads = Arc::new(AtomicUsize::new(0));
        let reload: ReloadFn = {
            let loads = loads.clone();
            Arc::new(move || {
                loads.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Ok(dead()) })
            })
        };
        let embedder = IdleEmbedder::new(dead(), reload, Some(IDLE), None, &tokio::runtime::Handle::current());
        let call = || embedder.embed_with_role("a text", EmbedRole::Query);

        let first = call().await;
        assert!(matches!(first, Err(EmbedError::NotReady)), "{first:?}");
        assert!(!embedder.resident());
        assert_eq!(loads.load(Ordering::SeqCst), 1, "the first recovery does not wait");

        for wait in [Duration::ZERO, RECOVERY_INTERVAL - Duration::from_secs(1)] {
            pass(wait).await;
            let held = call().await;
            assert!(matches!(held, Err(EmbedError::NotReady)), "{held:?}");
            assert_eq!(loads.load(Ordering::SeqCst), 1, "held within the minute");
        }

        pass(Duration::from_secs(1)).await;
        let again = call().await;
        assert!(matches!(again, Err(EmbedError::NotReady)), "{again:?}");
        assert_eq!(loads.load(Ordering::SeqCst), 2, "a minute later it loads again");
        call().await.unwrap_err();
        assert_eq!(loads.load(Ordering::SeqCst), 2);
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
