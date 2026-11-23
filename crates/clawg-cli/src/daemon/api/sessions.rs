//! The routes that register the host session an agent works in and count what it does there.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use clawg_core::memory::merge::MergeReport;
use clawg_core::metrics::sessions::{
    Activity, CLIENTS, END_REASONS, HookEvent, label, OTHER, SOURCES, UNSPECIFIED, Who,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::daemon::lifecycle::now_ms;
use crate::daemon::sessions::{check_session_id, tool_label};

use super::{ApiError, ApiResult, AppCtx, live_project};

#[derive(Debug, Deserialize)]
pub struct SessionRequest {
    pub agent_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    pub session_id: String,
    /// What the hook reports: `start` or `prompt`. Absent, as a hook from
    /// before the log of sessions sends it, the session is only registered.
    #[serde(default)]
    pub event: Option<String>,
    /// What started the session, with `start`.
    #[serde(default)]
    pub source: Option<String>,
    /// The client whose hook this is.
    #[serde(default)]
    pub client: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SessionEndRequest {
    pub session_id: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SummaryRequest {
    pub session_id: String,
}

#[derive(Debug, Serialize)]
pub struct SummaryBody {
    /// The hook asks the agent for the session's summary now.
    pub ask: bool,
}

/// The memory types that do not make a session worth summing up: the
/// summary itself, and a tool's failure, which says nothing of the work.
const NOT_WORTH_A_SUMMARY: [&str; 2] = ["session_summary", "tool_error"];

/// Whether a session with these accepted saves, by memory type, should save
/// its summary: it saved something worth one, and no summary yet.
pub fn summary_due(saves: &BTreeMap<String, u64>) -> bool {
    let saved = |memory_type: &str| saves.get(memory_type).is_some_and(|n| *n > 0);
    !saved("session_summary")
        && saves.iter().any(|(memory_type, n)| *n > 0 && !NOT_WORTH_A_SUMMARY.contains(&memory_type.as_str()))
}

#[derive(Debug, Deserialize)]
pub struct ToolCallRequest {
    pub agent_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    pub tool: String,
    #[serde(default)]
    pub failed: bool,
}

/// Records the host session `agent_id` works in for `project_id`, as a hook
/// reports it; the saves that follow are stamped with it. What the hook says
/// happened is counted in the row of that session.
pub(super) async fn register_session(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<SessionRequest>,
) -> ApiResult<StatusCode> {
    check_session_id(&body.session_id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let project_id = live_project(&app, body.project_id).await?;
    app.sessions.register(&body.agent_id, project_id.as_deref(), &body.session_id);
    let who = Who {
        session_id: &body.session_id,
        agent_id: &body.agent_id,
        project_id: project_id.as_deref(),
        client: body.client.as_deref().map_or(OTHER, |client| label(&CLIENTS, client)),
        utc_offset_s: crate::daemon::process::utc_offset_s(),
    };
    let event = match body.event.as_deref() {
        Some("start") => HookEvent::Start(body.source.as_deref().map_or(UNSPECIFIED, |source| label(&SOURCES, source))),
        Some("prompt") => HookEvent::Prompt,
        _ => HookEvent::Seen,
    };
    app.runtime.metrics.sessions().hook(who, now_ms() as i64, event);
    Ok(StatusCode::NO_CONTENT)
}

/// Closes the row of a session its client says ended, with the reason it
/// gives. It names no project, so a hook can send it without registering one.
pub(super) async fn end_session(State(app): State<Arc<AppCtx>>, Json(body): Json<SessionEndRequest>) -> ApiResult<StatusCode> {
    check_session_id(&body.session_id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let reason = body.reason.as_deref().map_or(OTHER, |reason| label(&END_REASONS, reason));
    app.runtime.metrics.sessions().end(&body.session_id, now_ms() as i64, reason);
    Ok(StatusCode::NO_CONTENT)
}

/// Says whether the hook at the end of a turn asks the agent of
/// `session_id` for its summary, once per session. It reads only the counts
/// of the session log: no project is registered and no model is loaded.
pub(super) async fn summary(State(app): State<Arc<AppCtx>>, Json(body): Json<SummaryRequest>) -> ApiResult<Json<SummaryBody>> {
    check_session_id(&body.session_id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let due = app.runtime.metrics.sessions().saves(&body.session_id).is_some_and(|saves| summary_due(&saves));
    let ask = due && app.sessions.ask_summary_once(&body.session_id);
    Ok(Json(SummaryBody { ask }))
}

/// Counts a call of one of the bridge's tools for the session its agent and
/// project are in, as the bridge reports it once the tool has answered.
pub(super) async fn count_tool_call(State(app): State<Arc<AppCtx>>, Json(body): Json<ToolCallRequest>) -> ApiResult<StatusCode> {
    let project_id = live_project(&app, body.project_id).await?;
    if let Some(session) = app.sessions.current(&body.agent_id, project_id.as_deref()) {
        let call = Activity::Call { tool: tool_label(&body.tool), failed: body.failed };
        app.runtime.metrics.sessions().activity(&session, now_ms() as i64, call);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Counts a search that found nothing for the session of `agent_id` in the
/// one project it read, or in none; a search over several is nobody's.
pub(super) fn count_empty_search(app: &AppCtx, agent_id: &str, projects: Option<&[String]>) {
    let project = match projects {
        None => None,
        Some([project]) => Some(project.as_str()),
        Some(_) => return,
    };
    if let Some(session) = app.sessions.current(agent_id, project) {
        app.runtime.metrics.sessions().activity(&session, now_ms() as i64, Activity::EmptySearch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saves(pairs: &[(&str, u64)]) -> BTreeMap<String, u64> {
        pairs.iter().map(|(t, n)| (t.to_string(), *n)).collect()
    }

    #[test]
    fn summary_due_counts_only_what_is_worth_a_summary() {
        assert!(!summary_due(&saves(&[])), "nothing saved");
        assert!(!summary_due(&saves(&[("tool_error", 2)])), "only a failure");
        assert!(summary_due(&saves(&[("semantic_fact", 1)])));
        assert!(summary_due(&saves(&[("tool_error", 1), ("architectural_decision", 1)])));
        assert!(summary_due(&saves(&[("unspecified", 1)])), "a save without a type counts");
        assert!(!summary_due(&saves(&[("session_summary", 1), ("semantic_fact", 3)])), "the summary is there");
        assert!(!summary_due(&saves(&[("session_summary", 1)])));
    }
}

/// The sessions of a merged project go to the one that stays, in the log
/// and in what the saves that follow are stamped with. A dry run, or a merge
/// that wrote nothing, moves none.
pub(super) fn sessions_follow(app: &AppCtx, merged: &MergeReport) {
    if merged.merge_id.is_some() && !merged.dry_run {
        app.runtime.metrics.sessions().merge_project(&merged.source, &merged.target);
        app.sessions.merge_project(&merged.source, &merged.target);
    }
}
