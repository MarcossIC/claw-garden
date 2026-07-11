use std::collections::HashMap;

use crate::models::{AppError, ChatMessage, SessionMeta};
use crate::session::SessionManager;
use crate::state::AppState;

/// Snapshot every agent's messages BEFORE `conversations.clear()` runs.
///
/// Used by both `new_session` and `resume_session` so `run_extractor`
/// (memory/extractor.rs, Fase 1) receives the OUTGOING session's full history
/// instead of an empty map — the clear happens immediately after this snapshot
/// is taken, inside the same locked block.
fn snapshot_conversations(
    conversations: &HashMap<String, Vec<ChatMessage>>,
) -> Vec<(String, Vec<ChatMessage>)> {
    conversations.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// Captures the OUTGOING session's `session_id` BEFORE `session` is replaced
/// by a new/loaded `SessionManager` (Fase 2 — memoria-3-capas). Used by both
/// `new_session` and `resume_session` so `run_episodic_writer` receives the
/// id of the CLOSING session, not the incoming one — mirrors
/// `snapshot_conversations`'s "capture before the swap/clear" pattern, and is
/// extracted as a standalone pure helper for the same reason: this codebase
/// does not enable the `tauri` "test" feature, so the command bodies
/// themselves cannot be unit tested directly, but this helper can be.
fn outgoing_session_id(session: &SessionManager) -> String {
    session.meta().session_id.clone()
}

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
    let (meta, history, outgoing_id) = {
        let mut session = state
            .session_manager
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))?;

        // Capture the OUTGOING session's id BEFORE `*session = new` overwrites
        // it (Fase 2 — S10): run_episodic_writer must receive the CLOSING
        // session's id, not the incoming one.
        let outgoing_id = outgoing_session_id(&session);

        let new = SessionManager::new_session(session.base_dir())?;
        let meta = new.meta().clone();

        let mut conversations = state
            .conversations
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        // Snapshot the OUTGOING session's history BEFORE clearing it, so
        // run_extractor (Fase 1) sees the full conversation, not an empty map.
        let history = snapshot_conversations(&conversations);
        conversations.clear();

        *session = new;
        (meta, history, outgoing_id)
    }; // session, conversations MutexGuards dropped here.

    // history is cloned for the episodic writer — the 2nd consumer alongside
    // the extractor, which takes ownership of the original below.
    let history_for_episodic = history.clone();

    // Fire-and-forget: spawn resolver on the Tauri async runtime.
    // `app.clone()` is `Send + 'static` — safe to move into the async block.
    // `tauri::State` is NEVER moved across the await boundary.
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::memory::resolver::run_resolver(handle).await;
    });

    // Fire-and-forget: spawn the memory extractor (Fase 1) on the closing
    // session's snapshot, independent from the resolver above.
    let handle2 = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::memory::extractor::run_extractor(handle2, history).await;
    });

    // Fire-and-forget: spawn the episodic-trajectory writer (Fase 2) on the
    // closing session's snapshot, independent from resolver/extractor above.
    let handle3 = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::memory::episodic_writer::run_episodic_writer(handle3, outgoing_id, history_for_episodic)
            .await;
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
    let (meta, history, outgoing_id) = {
        let mut session = state
            .session_manager
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))?;

        // Capture the OUTGOING session's id BEFORE `*session = loaded`
        // overwrites it (Fase 2 — S11): this captures the session being
        // REPLACED (X), not the one being loaded (Y = `session_id` param).
        let outgoing_id = outgoing_session_id(&session);

        let loaded = SessionManager::load_session(session.base_dir(), &session_id)?;
        let meta = loaded.meta().clone();

        let mut conversations = state
            .conversations
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        // Snapshot the OUTGOING session's history BEFORE clearing it — this
        // captures the session being replaced, not the one being loaded.
        let history = snapshot_conversations(&conversations);
        conversations.clear();
        for agent_id in &loaded.meta().agents {
            if let Ok(messages) = loaded.load_agent_messages(agent_id) {
                conversations.insert(agent_id.clone(), messages);
            }
        }

        *session = loaded;
        (meta, history, outgoing_id)
    }; // session, conversations MutexGuards dropped here.

    // history is cloned for the episodic writer — the 2nd consumer alongside
    // the extractor, which takes ownership of the original below.
    let history_for_episodic = history.clone();

    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::memory::resolver::run_resolver(handle).await;
    });

    // Fire-and-forget: spawn the memory extractor (Fase 1) on the outgoing
    // session's snapshot, independent from the resolver above.
    let handle2 = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::memory::extractor::run_extractor(handle2, history).await;
    });

    // Fire-and-forget: spawn the episodic-trajectory writer (Fase 2) on the
    // outgoing session's snapshot, independent from resolver/extractor above.
    let handle3 = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::memory::episodic_writer::run_episodic_writer(handle3, outgoing_id, history_for_episodic)
            .await;
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

    // ── Phase 5 (memoria-3-capas-fase-1) — history snapshot before clear ────
    //
    // `new_session`/`resume_session` take `state: tauri::State<'_, AppState>`,
    // which (like `app: tauri::AppHandle` in T26/T27 below) cannot be
    // constructed without a running Tauri app — this codebase does not enable
    // the `tauri` "test" feature. So the snapshot-before-clear logic is
    // extracted into the pure `snapshot_conversations` helper (used by both
    // call sites just before `conversations.clear()`), which IS directly
    // testable. Task 5.1/5.3's "history reaches run_extractor non-empty"
    // requirement is satisfied structurally: both command bodies call
    // `snapshot_conversations(&conversations)` before `.clear()` (verified by
    // code inspection + these unit tests of the helper itself).

    // Task 5.1: new_session — snapshot taken before clear captures all agents'
    // messages non-empty.
    #[test]
    fn snapshot_conversations_captures_all_agents_before_clear() {
        use std::collections::HashMap;
        let mut conversations: HashMap<String, Vec<ChatMessage>> = HashMap::new();
        conversations.insert(
            "agent-a".to_string(),
            vec![ChatMessage {
                id: "m1".to_string(),
                role: crate::models::chat::Role::User,
                content: "hello".to_string(),
                tool_calls: None,
                timestamp_ms: 0,
                is_error: None,
            }],
        );
        conversations.insert(
            "agent-b".to_string(),
            vec![ChatMessage {
                id: "m2".to_string(),
                role: crate::models::chat::Role::Assistant,
                content: "hi there".to_string(),
                tool_calls: None,
                timestamp_ms: 1,
                is_error: None,
            }],
        );

        let snapshot = snapshot_conversations(&conversations);
        conversations.clear();

        assert_eq!(snapshot.len(), 2, "snapshot must capture both agents");
        assert!(conversations.is_empty(), "conversations must be empty after clear");
        let agent_a = snapshot.iter().find(|(id, _)| id == "agent-a").expect("agent-a present");
        assert_eq!(agent_a.1.len(), 1);
        assert_eq!(agent_a.1[0].content, "hello");
        let agent_b = snapshot.iter().find(|(id, _)| id == "agent-b").expect("agent-b present");
        assert_eq!(agent_b.1[0].content, "hi there");
    }

    // Task 5.3: resume_session — snapshot taken before clear captures the
    // OUTGOING resumed session's messages, not an empty post-clear state.
    #[test]
    fn snapshot_conversations_captures_outgoing_session_before_resume_clear() {
        use std::collections::HashMap;
        let mut conversations: HashMap<String, Vec<ChatMessage>> = HashMap::new();
        conversations.insert(
            "outgoing-agent".to_string(),
            vec![
                ChatMessage {
                    id: "m1".to_string(),
                    role: crate::models::chat::Role::User,
                    content: "first turn".to_string(),
                    tool_calls: None,
                    timestamp_ms: 0,
                    is_error: None,
                },
                ChatMessage {
                    id: "m2".to_string(),
                    role: crate::models::chat::Role::Assistant,
                    content: "second turn".to_string(),
                    tool_calls: None,
                    timestamp_ms: 1,
                    is_error: None,
                },
            ],
        );

        let snapshot = snapshot_conversations(&conversations);
        // Simulate resume_session's clear() + reload with a DIFFERENT session's data.
        conversations.clear();
        conversations.insert("new-agent".to_string(), vec![]);

        assert_eq!(snapshot.len(), 1, "snapshot must capture the outgoing session only");
        let (agent_id, msgs) = &snapshot[0];
        assert_eq!(agent_id, "outgoing-agent");
        assert_eq!(msgs.len(), 2, "outgoing session had 2 messages");
        assert!(
            !conversations.contains_key("outgoing-agent"),
            "post-clear map must no longer contain the outgoing agent"
        );
    }

    // ── Lote 6 (memoria-3-capas-fase-2) — outgoing_session_id ────────────────
    //
    // `new_session`/`resume_session` cannot be called directly in tests (no
    // `tauri` "test" feature — same limitation as the T26/T27 compile-proofs
    // below), so the session_id-capture logic is extracted into the pure
    // `outgoing_session_id` helper (mirrors `snapshot_conversations`'s
    // extraction), which IS directly testable here. Task 6.2/6.3 (S10/S11)
    // are satisfied structurally: both command bodies call
    // `outgoing_session_id(&session)` BEFORE `*session = new/loaded` (verified
    // by code inspection + this unit test of the helper itself).

    // Task 6.2/6.3 (S10/S11): outgoing_session_id captures the CURRENT
    // session's id, and that captured value survives being overwritten by a
    // different SessionManager instance (simulating `*session = new/loaded`).
    #[test]
    fn outgoing_session_id_survives_replacement() {
        let temp_dir = std::env::temp_dir().join(format!(
            "claw-garden-test-outgoing-session-id-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let outgoing = SessionManager::new_session(&temp_dir).expect("outgoing session");
        let captured = outgoing_session_id(&outgoing);
        assert_eq!(
            captured,
            outgoing.meta().session_id,
            "must capture the CURRENT (outgoing) session's id"
        );

        // Simulate `*session = new` — a different SessionManager instance,
        // necessarily with a different session_id (SessionManager::new_session
        // generates a fresh id every call).
        let incoming = SessionManager::new_session(&temp_dir).expect("incoming session");
        assert_ne!(
            captured, incoming.meta().session_id,
            "captured id must be the OUTGOING one, not whatever replaces it"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

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
