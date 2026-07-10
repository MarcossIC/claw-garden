use tauri::State;

use crate::hooks::permission::UserDecision;
use crate::state::AppState;

/// Resolve a pending HITL confirmation issued by the chat loop.
///
/// The chat loop emits a `StreamEvent::ConfirmationRequired { call_id, ... }`
/// and parks on the receiver side of a oneshot stored in
/// `AppState::pending_confirmations` keyed by `call_id`. The UI calls this
/// command with the user's decision; we pull the sender out of the registry
/// and fire it, unblocking the chat loop.
///
/// Idempotency: a missing entry returns Ok with a debug log — the user may
/// have answered after the loop already gave up (timeout, agent abort).
#[tauri::command]
pub fn resolve_tool_confirmation(
    call_id: String,
    decision: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let parsed = match decision.as_str() {
        "allow" => UserDecision::Allow,
        "deny" => UserDecision::Deny,
        other => return Err(format!("invalid decision: {other}")),
    };

    let sender = match state.pending_confirmations.lock() {
        Ok(mut map) => map.remove(&call_id),
        Err(e) => return Err(format!("pending_confirmations lock poisoned: {e}")),
    };

    match sender {
        Some(tx) => {
            // If the receiver was dropped (loop aborted), the send fails — fine.
            let _ = tx.send(parsed);
            Ok(())
        }
        None => {
            log::debug!(
                "resolve_tool_confirmation: no pending request for call_id={call_id} \
                 (already resolved or chat loop aborted)"
            );
            Ok(())
        }
    }
}
