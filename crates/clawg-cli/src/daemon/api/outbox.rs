//! The routes that retry or discard a save the outbox could not finish.

use axum::Json;
use axum::extract::State;
use clawg_core::memory::outbox::OutboxActionError;
use clawg_core::memory::save::check_operation_id;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::{ApiError, ApiResult, AppCtx};

/// Names a save the daemon gave up on: its operation id, and its agent when
/// another agent holds the same id.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutboxRequest {
    pub operation_id: String,
    #[serde(default)]
    pub agent_id: Option<String>,
}

/// A failed save taken back to pending; `queued` is false when the queue was
/// full and it waits for the next start.
#[derive(Debug, Serialize)]
pub struct RetriedBody {
    pub operation_id: String,
    pub agent_id: String,
    pub queued: bool,
}

/// A failed save dropped, with the memories it had inserted, now deleted.
#[derive(Debug, Serialize)]
pub struct DiscardedBody {
    pub operation_id: String,
    pub agent_id: String,
    pub deleted: Vec<i64>,
}

fn outbox_error(e: OutboxActionError) -> ApiError {
    match e {
        OutboxActionError::NotFound(_) => ApiError::not_found(e.to_string()),
        OutboxActionError::NotFailed(_) | OutboxActionError::Ambiguous { .. } | OutboxActionError::Unreadable(_) => {
            ApiError::conflict(e.to_string())
        }
        OutboxActionError::Store(_) => ApiError::internal(e),
    }
}

/// The operation id of an outbox request, checked as a save checks it.
fn requested_operation(body: &OutboxRequest) -> ApiResult<String> {
    let id = check_operation_id(Some(body.operation_id.clone())).map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(id.unwrap_or_default())
}

pub(super) async fn retry_outbox(State(app): State<Arc<AppCtx>>, Json(body): Json<OutboxRequest>) -> ApiResult<Json<RetriedBody>> {
    let operation_id = requested_operation(&body)?;
    let retried = app.runtime.retry_failed(&operation_id, body.agent_id.as_deref()).await.map_err(outbox_error)?;
    Ok(Json(RetriedBody { operation_id, agent_id: retried.agent_id, queued: retried.queued }))
}

pub(super) async fn discard_outbox(State(app): State<Arc<AppCtx>>, Json(body): Json<OutboxRequest>) -> ApiResult<Json<DiscardedBody>> {
    let operation_id = requested_operation(&body)?;
    let discarded = app.runtime.discard_failed(&operation_id, body.agent_id.as_deref()).await.map_err(outbox_error)?;
    Ok(Json(DiscardedBody { operation_id, agent_id: discarded.agent_id, deleted: discarded.deleted }))
}
