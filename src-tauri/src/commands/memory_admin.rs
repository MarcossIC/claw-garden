//! Tauri command wrappers for memory inspection/override.
//!
//! Thin by design: each `#[tauri::command]` resolves `state.memory_store` and
//! delegates to the matching `*_core` function in
//! [`clawg_core::memory::admin`], `?`-propagating via `AppError`'s
//! `#[from] MemoryError`. Not directly unit-testable (`State` cannot be
//! constructed without a Tauri "test" feature this codebase doesn't enable) —
//! verified by compilation + `lib.rs` registration, mirroring
//! `commands/session.rs` / `commands/embedder.rs`.

use serde::{Deserialize, Serialize};

use clawg_core::memory::admin::{
    delete_memory_core, get_failure_histogram_core, get_skill_steps_trend_core,
    list_memories_core, set_importance_core, FailureHistogramRow, ListParams, MemoryAdminRow,
    MemoryKind, SkillStepsTrendRow,
};
use clawg_core::memory::MemoryError;
use clawg_core::models::AppError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Tauri command wrappers — thin, resolve `state.memory_store`, delegate to
// the `*_core` functions above, `?`-propagate via `AppError`'s
// `#[from] MemoryError` (models/error.rs:97). Not directly unit-testable
// (`State` cannot be constructed without a Tauri "test" feature this
// codebase doesn't enable) — verified by compilation + `lib.rs` registration,
// mirroring `commands/session.rs` / `commands/embedder.rs`.
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_memories(
    state: tauri::State<'_, AppState>,
    kind: Option<MemoryKind>,
    tier: Option<i64>,
    memory_type: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<MemoryAdminRow>, AppError> {
    let params = ListParams { kind, tier, memory_type, limit, offset };
    list_memories_core(&state.memory_store, params)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_importance(
    state: tauri::State<'_, AppState>,
    id: i64,
    kind: MemoryKind,
    importance: f64,
) -> Result<usize, AppError> {
    set_importance_core(&state.memory_store, id, kind, importance)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_memory(
    state: tauri::State<'_, AppState>,
    id: i64,
    kind: MemoryKind,
) -> Result<usize, AppError> {
    delete_memory_core(&state.memory_store, id, kind)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_failure_histogram(
    state: tauri::State<'_, AppState>,
    bucket_ms: i64,
) -> Result<Vec<FailureHistogramRow>, AppError> {
    get_failure_histogram_core(&state.memory_store, bucket_ms)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_skill_steps_trend(
    state: tauri::State<'_, AppState>,
    skill_id: Option<i64>,
    limit: Option<u32>,
) -> Result<Vec<SkillStepsTrendRow>, AppError> {
    get_skill_steps_trend_core(&state.memory_store, skill_id, limit)
        .await
        .map_err(Into::into)
}

// ---------------------------------------------------------------------------
// reembed_all — operator-invoked E5-prefix backfill (memory/maintenance.rs)
// ---------------------------------------------------------------------------

/// Which [`clawg_core::memory::maintenance::ReembedMode`] a `reembed_all` call
/// means. A required, non-`Option` argument with no default variant: the
/// accident this type exists to prevent is a caller INTENDING a `ReInfer`
/// (paying for real inference, migrating the embedding space) and getting a
/// free `Normalize` no-op instead because an optional argument was left
/// unset. Every invocation must name its mode explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReembedModeArg {
    Normalize,
    ReInfer,
}

#[tauri::command]
pub async fn reembed_all(
    state: tauri::State<'_, AppState>,
    mode: ReembedModeArg,
) -> Result<clawg_core::memory::maintenance::ReembedReport, AppError> {
    let mode = match mode {
        ReembedModeArg::Normalize => clawg_core::memory::maintenance::ReembedMode::Normalize,
        ReembedModeArg::ReInfer => {
            // ReInfer needs a live embedder to derive the identity it will
            // write to `embedding_contract` on success — if the model
            // hasn't finished loading yet there is nothing correct to do
            // but refuse, not silently fall back to Normalize.
            let embedder = state.embedder.get().cloned().ok_or_else(|| {
                AppError::from(MemoryError::Backend(
                    "reembed_all: embedder not yet loaded — cannot ReInfer".into(),
                ))
            })?;
            clawg_core::memory::maintenance::ReembedMode::ReInfer(
                clawg_core::memory::maintenance::ReInferPlan::new(embedder),
            )
        }
    };
    Ok(clawg_core::memory::maintenance::reembed_all(&state.memory_store, mode).await)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── reembed_all — ReembedModeArg serde round-trip ────────────────────────
    //
    // No `Option<_>` or default anywhere on this type or the command that
    // takes it: the frontend must always state which mode it means, so
    // there is no path where an omitted argument silently resolves to
    // Normalize when a ReInfer was intended.

    #[test]
    fn reembed_mode_arg_serde_snake_case_roundtrip() {
        let cases = [
            (ReembedModeArg::Normalize, "\"normalize\""),
            (ReembedModeArg::ReInfer, "\"re_infer\""),
        ];
        for (mode, expected_json) in cases {
            let json = serde_json::to_string(&mode).expect("serialize ReembedModeArg");
            assert_eq!(json, expected_json, "unexpected JSON for {mode:?}");
            let back: ReembedModeArg =
                serde_json::from_str(&json).expect("deserialize ReembedModeArg");
            assert_eq!(back, mode, "round-trip mismatch for {mode:?}");
        }
    }
}
