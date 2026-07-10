use crate::models::{AppError, ChatMessage, SessionMeta};
use crate::session::SessionManager;
use crate::state::AppState;

#[tauri::command]
pub fn list_sessions(state: tauri::State<'_, AppState>) -> Result<Vec<SessionMeta>, AppError> {
    let session = state
        .session_manager
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    SessionManager::list_sessions(session.base_dir()).map_err(Into::into)
}

#[tauri::command]
pub fn current_session(state: tauri::State<'_, AppState>) -> Result<SessionMeta, AppError> {
    let session = state
        .session_manager
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(session.meta().clone())
}

/// Create a new session and fire-and-forget the contradiction resolver (REQ-07).
///
/// `app: tauri::AppHandle` is auto-injected by Tauri at the invoke boundary —
/// the frontend `invoke('new_session', {})` call does NOT change.
///
/// # Lock discipline
/// All `MutexGuard`s (`session`, `conversations`) are dropped inside the inner
/// block before the `spawn`. The resolver derives its own `store` handle via
/// `app.state::<AppState>()` — it needs neither lock.
#[tauri::command]
pub fn new_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<SessionMeta, AppError> {
    // Inner scope: all MutexGuards are dropped before the spawn.
    let meta = {
        let mut session = state
            .session_manager
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))?;

        let new = SessionManager::new_session(session.base_dir())?;
        let meta = new.meta().clone();

        let mut conversations = state
            .conversations
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        conversations.clear();

        *session = new;
        meta
    }; // session, conversations MutexGuards dropped here.

    // Fire-and-forget: spawn resolver on the Tauri async runtime.
    // `app.clone()` is `Send + 'static` — safe to move into the async block.
    // `tauri::State` is NEVER moved across the await boundary.
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::memory::resolver::run_resolver(handle).await;
    });

    Ok(meta)
}

/// Resume an existing session and fire-and-forget the contradiction resolver (REQ-07).
///
/// Identical lock-and-spawn discipline as `new_session`.
#[tauri::command]
pub fn resume_session(
    session_id: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<SessionMeta, AppError> {
    let meta = {
        let mut session = state
            .session_manager
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))?;

        let loaded = SessionManager::load_session(session.base_dir(), &session_id)?;
        let meta = loaded.meta().clone();

        let mut conversations = state
            .conversations
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        conversations.clear();
        for agent_id in &loaded.meta().agents {
            if let Ok(messages) = loaded.load_agent_messages(agent_id) {
                conversations.insert(agent_id.clone(), messages);
            }
        }

        *session = loaded;
        meta
    }; // session, conversations MutexGuards dropped here.

    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::memory::resolver::run_resolver(handle).await;
    });

    Ok(meta)
}

#[tauri::command]
pub fn delete_session(
    session_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    let session = state
        .session_manager
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    // Prevent deleting the current session
    if session.meta().session_id == session_id {
        return Err(AppError::Internal(
            "Cannot delete the active session".to_string(),
        ));
    }

    SessionManager::delete_session(session.base_dir(), &session_id)?;
    Ok(())
}

#[tauri::command]
pub fn get_agent_messages(
    agent_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ChatMessage>, AppError> {
    let conversations = state
        .conversations
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(conversations.get(&agent_id).cloned().unwrap_or_default())
}

// =============================================================================
// T26–T27 — compile-time signature proofs (Strict TDD, Phase 6)
// =============================================================================
//
// These tests verify two invariants without running any Tauri runtime:
//
//   1. `new_session` / `resume_session` have `app: tauri::AppHandle` as a
//      parameter (auto-injected by Tauri — frontend callers unchanged).
//
//   2. Both functions are plain `fn` (NOT `async fn`).  An `async fn` cannot
//      be coerced to a `fn`-pointer type; the coercion below would fail to
//      compile, proving that `tauri::async_runtime::spawn(…)` is truly
//      fire-and-forget and the command returns immediately.
//
// The function bodies are empty at runtime — the compiler check IS the test.
#[cfg(test)]
mod tests {
    use super::*;

    /// T26: `new_session(app: tauri::AppHandle, state: …)` compiles;
    /// the spawn is fire-and-forget (fn, not async fn).
    #[test]
    fn t26_new_session_accepts_app_handle_and_is_sync() {
        // `for<'r>` quantifies over all possible State lifetimes — this matches
        // the lifetime-elided `state: tauri::State<'_, AppState>` in the impl.
        //
        // Compile failures caught here:
        //   (a) missing `app: tauri::AppHandle` param → type mismatch
        //   (b) function declared `async fn` → cannot coerce async fn to fn ptr
        let _: for<'r> fn(
            tauri::AppHandle,
            tauri::State<'r, AppState>,
        ) -> Result<SessionMeta, AppError> = new_session;
    }

    /// T27: `resume_session(session_id: String, app: tauri::AppHandle, state: …)`
    /// compiles; the spawn is fire-and-forget (fn, not async fn).
    #[test]
    fn t27_resume_session_accepts_app_handle_and_is_sync() {
        let _: for<'r> fn(
            String,
            tauri::AppHandle,
            tauri::State<'r, AppState>,
        ) -> Result<SessionMeta, AppError> = resume_session;
    }
}
