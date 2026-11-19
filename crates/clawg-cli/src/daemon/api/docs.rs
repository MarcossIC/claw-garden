//! The project-docs routes: the background refresh of a project's docs and the search over them.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use clawg_core::memory::bootstrap::EmbedderStatus;
use clawg_core::memory::embedder::Embedder;
use clawg_core::memory::portability::timing::seconds;
use clawg_core::memory::project_docs::{DocHit, DocIndexError, RefreshGuard};
use clawg_core::memory::project_docs;
use clawg_core::metrics::{Recorder, Unit};
use clawg_core::paths::Profile;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{ApiError, ApiResult, AppCtx, ready_embedder};

/// Refreshes the project's documentation index in the background, once the
/// embedder has loaded: the bridge registers a root as its session starts,
/// usually before the model is ready.
pub(super) fn spawn_doc_refresh(app: Arc<AppCtx>, project_id: String) {
    tokio::spawn(async move {
        let mut status = app.runtime.embedder_status.subscribe();
        // A closed channel means the daemon is going away; the cell check
        // below then finds no embedder and gives up.
        let _ = status.wait_for(|s| !matches!(s, EmbedderStatus::Loading)).await.map(|_| ());
        let Some(embedder) = app.runtime.embedder.get().cloned() else {
            log::warn!("not indexing the docs of {project_id}: the embedder did not load");
            return;
        };
        if let Some(guard) = app.runtime.project_docs.begin(&project_id) {
            refresh_docs(&app, guard, embedder.as_ref(), &project_id).await;
        }
    });
}

/// Runs a claimed refresh in a task of its own, so it ends even when whoever
/// started it stops waiting.
fn refresh_docs_in_background(app: Arc<AppCtx>, guard: RefreshGuard, embedder: Arc<dyn Embedder>, project_id: String) {
    tokio::spawn(async move { refresh_docs(&app, guard, embedder.as_ref(), &project_id).await });
}

/// What the log says a refresh did; `reused` counts the passages whose vector
/// was kept instead of embedded again.
fn refresh_line(project_id: &str, r: &project_docs::RefreshReport) -> String {
    format!(
        "indexed the docs of {project_id}: {} docs, {} reindexed, {} removed, {} skipped, {} passages, {} reused",
        r.docs, r.indexed, r.removed, r.skipped, r.passages, r.reused
    )
}

/// What the log says when a refresh knows its plan, before its first file:
/// the one line a refresh that never ends leaves.
fn refresh_start_line(project_id: &str, trigger: project_docs::RefreshTrigger, plan: project_docs::PlannedRefresh) -> String {
    format!(
        "docs refresh of {project_id}: started by {}: {} to index, {} unchanged, {} to remove",
        trigger.as_str(),
        plan.index,
        plan.unchanged,
        plan.remove
    )
}

/// Where a refresh spent its time, the phases summed over its docs. `took` is
/// the total the metrics record too, so the line and the series agree.
fn refresh_timing_line(project_id: &str, took: Duration, timing: &project_docs::RefreshTiming) -> String {
    let p = timing.phases();
    format!(
        "docs refresh of {project_id}: done in {} s: scan {}, read {}, mask {}, split {}, lookup {}, embed {}, write {}",
        seconds(took),
        seconds(timing.scan),
        seconds(p.read),
        seconds(p.mask),
        seconds(p.split),
        seconds(p.lookup),
        seconds(p.embed),
        seconds(p.write),
    )
}

/// Nobody waits on the outcome: a failure is logged and kept for the next
/// search to name.
async fn refresh_docs(app: &AppCtx, guard: RefreshGuard, embedder: &dyn Embedder, project_id: &str) {
    let tokenizer = app.runtime.tokenizer.get().cloned();
    let started = Instant::now();
    let trigger = guard.trigger();
    let planned = |plan| log::info!("{}", refresh_start_line(project_id, trigger, plan));
    let refreshed = guard.run(&app.runtime.store, embedder, tokenizer, &planned).await;
    let took = started.elapsed();
    record_refresh(&app.runtime.metrics, took, refreshed.as_ref().ok().map(|run| &run.report));
    match refreshed {
        Ok(project_docs::RefreshRun { report: r, timing }) => {
            log::info!("{}", refresh_line(project_id, &r));
            log::info!("{}", refresh_timing_line(project_id, took, &timing));
            for f in &r.failed {
                log::warn!("left {} of {project_id} as it was: {}", f.path, f.reason);
            }
        }
        Err(e) => log::warn!("could not index the docs of {project_id}: {e}"),
    }
}

/// Records how long a refresh took and, when it finished, how many files it
/// indexed, found unchanged, removed and left as they were.
fn record_refresh(metrics: &Recorder, took: Duration, report: Option<&project_docs::RefreshReport>) {
    metrics.observe("docs.refresh.duration", Unit::Micros, &[], micros(took));
    if let Some(r) = report {
        for (state, files) in
            [("indexed", r.indexed), ("unchanged", r.unchanged), ("removed", r.removed), ("failed", r.failed.len())]
        {
            metrics.gauge("docs.refresh.files", &[("state", state)], files as f64);
        }
    }
}

pub(super) fn micros(took: Duration) -> u64 {
    u64::try_from(took.as_micros()).unwrap_or(u64::MAX)
}

/// How long a search waits for a project's first index before answering
/// with what there is. Longer than indexing a typical repository's docs takes,
/// and short of what an MCP host waits for a tool.
pub const FIRST_INDEX_WAIT: Duration = Duration::from_secs(20);

/// How long a search waits for the refresh it started over an index that
/// already holds docs before answering from it. About what one doc took in
/// the one full reindex measured (a debug build), so an edit is usually found
/// by the search that finds it, and a checkout costs that search no more.
pub const REFRESH_GRACE: Duration = Duration::from_secs(3);

async fn docs_in(store: &clawg_core::memory::MemoryStore, project_id: &str) -> ApiResult<usize> {
    project_docs::doc_count(store, project_id).await.map_err(ApiError::internal)
}

/// Largest `limit` a documentation search honours.
pub const MAX_DOCS_LIMIT: usize = 20;

/// `limit` of a documentation search when the caller does not say.
pub const DEFAULT_DOCS_LIMIT: usize = 5;

pub fn clamp_docs_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_DOCS_LIMIT).clamp(1, MAX_DOCS_LIMIT)
}

/// Strict for the same reason as [`ReaderQuery`](super::ReaderQuery).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocsSearchRequest {
    pub project_id: String,
    pub query: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DocsSearchBody {
    pub hits: Vec<DocHit>,
    /// Docs the project's index holds.
    pub docs: usize,
    /// A refresh of the project is still running, so the hits may be partial.
    pub indexing: bool,
    /// Why the project's last finished refresh failed, until one succeeds;
    /// the hits come from what was already indexed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// Why a project's docs are not searched, and the one command that changes it.
///
/// The user runs it, not the model: a tool argument the model could send on
/// its own would not be a choice the user made.
fn not_accepted(root: &std::path::Path, profile: Profile) -> String {
    let flag = match profile {
        Profile::Prod => String::new(),
        other => format!(" --profile {}", other.as_str()),
    };
    format!(
        "clawg reads the files of the projects the user added, and of a folder the MCP client has \
         open or a bridge was started in when that folder has a .git or a manifest and is not the \
         home directory or a filesystem root; {root} is neither. Ask the user to run `clawg projects \
         add \"{root}\"{flag}` if they want it searched, then call again",
        root = root.display()
    )
}

/// Searches a registered project's documentation. When the last refresh is
/// older than [`project_docs::REFRESH_INTERVAL`] it starts one in the
/// background and waits for it at most [`REFRESH_GRACE`].
pub(super) async fn search_docs(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<DocsSearchRequest>,
) -> ApiResult<Json<DocsSearchBody>> {
    if body.query.trim().is_empty() {
        return Err(ApiError::bad_request("query must not be empty"));
    }
    let store = &app.runtime.store;
    project_docs::root_of(store, &body.project_id).await.map_err(|e| match e {
        DocIndexError::UnknownProject(_) => ApiError::not_found(e.to_string()),
        DocIndexError::NotAccepted(root) => ApiError::new(StatusCode::FORBIDDEN, not_accepted(&root, app.profile)),
        other => ApiError::internal(other),
    })?;
    let embedder = ready_embedder(&app)?;
    let index = &app.runtime.project_docs;
    // An empty index waits longer: answering from it would read as "the docs
    // say nothing". The first index is usually started by the bridge's
    // registration a moment before this search.
    let wait = if let Some(guard) = index.begin_if_due(&body.project_id, Instant::now()) {
        refresh_docs_in_background(app.clone(), guard, embedder.clone(), body.project_id.clone());
        Some(if docs_in(store, &body.project_id).await? == 0 { FIRST_INDEX_WAIT } else { REFRESH_GRACE })
    } else if index.is_running(&body.project_id) && docs_in(store, &body.project_id).await? == 0 {
        Some(FIRST_INDEX_WAIT)
    } else {
        None
    };
    if let Some(limit) = wait {
        let started = Instant::now();
        index.wait_idle(&body.project_id, limit).await;
        app.runtime.metrics.observe("docs.search.inline_wait", Unit::Micros, &[], micros(started.elapsed()));
    }
    let hits = project_docs::search(store, embedder.as_ref(), &body.project_id, &body.query, clamp_docs_limit(body.limit))
        .await
        .map_err(ApiError::internal)?;
    let docs = docs_in(store, &body.project_id).await?;
    let warning = index.last_error(&body.project_id);
    Ok(Json(DocsSearchBody { hits, docs, indexing: index.is_running(&body.project_id), warning }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_docs_start_line_names_the_project_the_trigger_and_the_plan() {
        let plan = project_docs::PlannedRefresh { index: 36, unchanged: 865, remove: 2 };
        assert_eq!(
            refresh_start_line("repo-1a2b", project_docs::RefreshTrigger::Search, plan),
            "docs refresh of repo-1a2b: started by search: 36 to index, 865 unchanged, 2 to remove"
        );
        assert!(refresh_start_line("p", project_docs::RefreshTrigger::Registration, plan).contains("started by registration"));
    }

    #[test]
    fn the_docs_timing_line_names_every_phase() {
        let ms = Duration::from_millis;
        let timing = project_docs::RefreshTiming {
            scan: ms(500),
            docs: vec![
                project_docs::DocTiming {
                    path: "a.md".into(),
                    kind: project_docs::corpus::DocKind::Markdown,
                    passages: 2,
                    phases: project_docs::DocPhases {
                        read: ms(100),
                        mask: ms(300),
                        split: ms(600),
                        lookup: Duration::ZERO,
                        embed: ms(2000),
                        write: ms(250),
                    },
                },
                project_docs::DocTiming {
                    path: "b.md".into(),
                    kind: project_docs::corpus::DocKind::Markdown,
                    passages: 1,
                    phases: project_docs::DocPhases {
                        read: ms(150),
                        mask: ms(200),
                        split: ms(400),
                        lookup: Duration::ZERO,
                        embed: ms(1000),
                        write: ms(250),
                    },
                },
            ],
            remove: ms(250),
        };
        assert_eq!(
            refresh_timing_line("p", ms(6000), &timing),
            "docs refresh of p: done in 6.0 s: scan 0.5, read 0.2, mask 0.5, split 1.0, lookup 0.0, embed 3.0, write 0.8"
        );
    }

    #[test]
    fn the_docs_log_line_shows_the_reused_passages() {
        let report = project_docs::RefreshReport {
            docs: 4,
            indexed: 2,
            removed: 1,
            skipped: 3,
            passages: 9,
            reused: 7,
            ..Default::default()
        };
        assert_eq!(
            refresh_line("p", &report),
            "indexed the docs of p: 4 docs, 2 reindexed, 1 removed, 3 skipped, 9 passages, 7 reused"
        );
    }

    #[test]
    fn the_refusal_names_the_command_for_the_daemons_profile() {
        let root = std::path::Path::new("/work/other");
        assert!(not_accepted(root, Profile::Prod).contains("`clawg projects add \"/work/other\"`"));
        assert!(not_accepted(root, Profile::Dev).contains("`clawg projects add \"/work/other\" --profile dev`"));
    }

    /// A folder the MCP client has open is read as the bridge's own
    /// directory is, so the refusal names both.
    #[test]
    fn the_refusal_names_what_is_read_without_the_user_adding_it() {
        let refusal = not_accepted(std::path::Path::new("/work/other"), Profile::Prod);
        assert!(refusal.contains("a folder the MCP client has open or a bridge was started in"), "{refusal}");
    }

    #[test]
    fn a_docs_search_returns_five_by_default_and_never_more_than_twenty() {
        assert_eq!(clamp_docs_limit(None), 5);
        assert_eq!(clamp_docs_limit(Some(0)), 1);
        assert_eq!(clamp_docs_limit(Some(12)), 12);
        assert_eq!(clamp_docs_limit(Some(500)), 20);
    }
}
