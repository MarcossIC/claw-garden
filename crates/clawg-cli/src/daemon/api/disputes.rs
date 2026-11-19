//! The routes that resolve, judge and list the pairs of memories in dispute.

use axum::Json;
use axum::extract::{Query, State};
use clawg_core::memory::dispute::{
    judge_pair, open_pairs, OpenPairs, Outcome, Resolution, resolve_dispute_core, ResolveError,
    Verdict,
};
use serde::Deserialize;
use std::sync::Arc;

use super::{ApiError, ApiResult, AppCtx, live_project, named_reader};

/// Closes the dispute between two `memory` rows. Disputes only ever pair
/// `memory` rows, so there is no `kind`.
#[derive(Debug, Deserialize)]
pub struct ResolveRequest {
    pub keep: i64,
    pub other: i64,
    /// Both rows stand instead of `other` being superseded by `keep`.
    #[serde(default)]
    pub keep_both: bool,
    /// The agent the resolution acts for, as in [`ReaderQuery`](super::ReaderQuery).
    #[serde(default)]
    pub agent_id: Option<String>,
}

/// A verdict on a pair the contradiction detector found.
#[derive(Debug, Deserialize)]
pub struct JudgeRequest {
    pub keep: i64,
    pub other: i64,
    /// `supersedes`, `compatible`, `scoped` or `conflicts`.
    pub verdict: String,
    /// Why, required for `scoped`.
    #[serde(default)]
    pub reason: Option<String>,
    /// The caller declares the user answered, with the answer in `reason`.
    #[serde(default)]
    pub user_answered: bool,
    /// The agent the verdict acts for, as in [`ReaderQuery`](super::ReaderQuery).
    #[serde(default)]
    pub agent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisputesQuery {
    /// The agent that judges, as in [`ReaderQuery`](super::ReaderQuery).
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Only the pairs whose newer row this project reaches, as a search reads
    /// it; absent, the whole store.
    #[serde(default)]
    pub project_id: Option<String>,
}

/// How many pending pairs `GET /v1/disputes` names.
const OPEN_PAIRS_SHOWN: usize = 5;


/// The answer every verdict route gives a refused one.
fn resolve_error(e: ResolveError) -> ApiError {
    match e {
            ResolveError::SameRow | ResolveError::ReasonRequired | ResolveError::AnswerRequired => {
                ApiError::bad_request(e.to_string())
            }
            ResolveError::NotFound(_) => ApiError::not_found(e.to_string()),
            ResolveError::NotInDispute { .. }
            | ResolveError::SeveralPairs { .. }
            | ResolveError::SupersededElsewhere { .. }
            | ResolveError::NeedsUser { .. } => ApiError::conflict(e.to_string()),
            ResolveError::Store(_) => ApiError::internal(e),
    }
}

pub(super) async fn resolve_dispute(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<ResolveRequest>,
) -> ApiResult<Json<Resolution>> {
    let outcome = if body.keep_both { Outcome::KeepBoth } else { Outcome::Supersede };
    let reader = named_reader(&app, body.agent_id.as_deref());
    resolve_dispute_core(&app.runtime.store, body.keep, body.other, outcome, reader.as_ref())
        .await
        .map(Json)
        .map_err(resolve_error)
}

pub(super) async fn judge_dispute(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<JudgeRequest>,
) -> ApiResult<Json<Resolution>> {
    let verdict: Verdict = body.verdict.parse().map_err(|e: clawg_core::memory::dispute::UnknownVerdict| {
        ApiError::bad_request(e.to_string())
    })?;
    let reader = named_reader(&app, body.agent_id.as_deref());
    judge_pair(&app.runtime.store, body.keep, body.other, verdict, body.reason, body.user_answered, reader.as_ref())
        .await
        .map(Json)
        .map_err(resolve_error)
}

/// The pairs not judged yet, counted, with the oldest pending ones.
pub(super) async fn disputes(
    State(app): State<Arc<AppCtx>>,
    Query(q): Query<DisputesQuery>,
) -> ApiResult<Json<OpenPairs>> {
    let reader = named_reader(&app, q.agent_id.as_deref());
    let project_id = live_project(&app, q.project_id).await?;
    open_pairs(&app.runtime.store, reader.as_ref(), project_id.as_deref(), OPEN_PAIRS_SHOWN)
        .await
        .map(Json)
        .map_err(ApiError::internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    use super::super::test_support::{answer_of, daemon_for_jobs, run_sql, seed_memory};

    /// The status and the whole body `resolve_dispute` answers with, for every
    /// refusal in the order it checks them and for both outcomes.
    #[tokio::test]
    async fn each_resolved_dispute_keeps_its_status_and_body() {
        use serde_json::{json, Value};

        async fn resolved(app: &Arc<AppCtx>, body: Value) -> (StatusCode, Value) {
            let body: ResolveRequest = serde_json::from_value(body).expect("a resolve");
            answer_of(resolve_dispute(State(app.clone()), Json(body)).await).await
        }
        let side = |id: i64, status: &str, by: Option<i64>| json!({ "id": id, "status": status, "superseded_by": by });

        let (app, dir) = daemon_for_jobs().await;
        let keep = seed_memory(&app, "a", "semantic_fact", "the engine is sqlite").await;
        let other = seed_memory(&app, "a", "semantic_fact", "the engine is postgres").await;
        let apart = seed_memory(&app, "a", "semantic_fact", "an unrelated fact").await;
        let preference = seed_memory(&app, "a", "user_preference", "tabs over spaces").await;
        let replaced = seed_memory(&app, "a", "semantic_fact", "a replaced fact").await;
        let replacement = seed_memory(&app, "a", "semantic_fact", "its replacement").await;
        let theirs = seed_memory(&app, "b", "semantic_fact", "their fact").await;
        run_sql(
            &app,
            format!(
                "INSERT INTO memory_conflict (new_id, old_id, distance, state, created_at) VALUES
                   ({other}, {keep}, 0.2, 'pending', 3), ({preference}, {keep}, 0.2, 'pending', 3),
                   ({theirs}, {keep}, 0.2, 'pending', 3), ({replaced}, {keep}, 0.2, 'pending', 3);
                 UPDATE memory SET status = 'superseded', superseded_by = {replacement} WHERE id = {replaced};"
            ),
        )
        .await;
        std::fs::write(dir.path().join("sharing.json"), r#"{"version":1,"hides":{"a":["b"]}}"#).expect("write the policy");

        let cases = [
            (json!({ "keep": keep, "other": keep }), (StatusCode::BAD_REQUEST, json!({ "error": "a memory cannot be resolved against itself" }))),
            (json!({ "keep": 999_999, "other": 999_999 }), (StatusCode::BAD_REQUEST, json!({ "error": "a memory cannot be resolved against itself" }))),
            (json!({ "keep": 999_998, "other": 999_999 }), (StatusCode::NOT_FOUND, json!({ "error": "no memory row with id 999998" }))),
            (json!({ "keep": keep, "other": 999_999 }), (StatusCode::NOT_FOUND, json!({ "error": "no memory row with id 999999" }))),
            (
                json!({ "keep": keep, "other": theirs, "agent_id": "a" }),
                (StatusCode::NOT_FOUND, json!({ "error": format!("no memory row with id {theirs}") })),
            ),
            (
                json!({ "keep": keep, "other": apart }),
                (StatusCode::CONFLICT, json!({ "error": format!("memories {keep} and {apart} are not in dispute with each other") })),
            ),
            (
                json!({ "keep": keep, "other": replaced }),
                (
                    StatusCode::CONFLICT,
                    json!({ "error": format!("memory {replaced} was replaced by memory {replacement}; resolve that pair instead") }),
                ),
            ),
            (
                json!({ "keep": keep, "other": preference }),
                (
                    StatusCode::CONFLICT,
                    json!({
                        "error": format!(
                            "memory {preference} is a preference or a decision, so {keep} supersedes it only on the user's word: \
                             judge the pair `conflicts` and ask the user, then call again with `user_answered` and their \
                             answer in `reason`"
                        ),
                    }),
                ),
            ),
        ];
        for (body, expected) in cases {
            assert_eq!(resolved(&app, body.clone()).await, expected, "{body}");
        }

        assert_eq!(
            resolved(&app, json!({ "keep": keep, "other": preference, "keep_both": true })).await,
            (StatusCode::OK, json!({ "keep": side(keep, "active", None), "other": side(preference, "active", None), "pair": "judged" }))
        );
        assert_eq!(
            resolved(&app, json!({ "keep": keep, "other": other, "agent_id": "a" })).await,
            (StatusCode::OK, json!({ "keep": side(keep, "active", None), "other": side(other, "superseded", Some(keep)), "pair": "judged" }))
        );
        assert_eq!(
            resolved(&app, json!({ "keep": other, "other": keep })).await,
            (StatusCode::OK, json!({ "keep": side(other, "active", None), "other": side(keep, "superseded", Some(other)), "pair": "judged" })),
            "a resolution is undone by the other verdict"
        );

        app.runtime.close().await.expect("the store closes");
        assert_eq!(
            resolved(&app, json!({ "keep": keep, "other": other })).await,
            (StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": "memory backend channel error: connection closed during judge_pair" }))
        );
    }
}
