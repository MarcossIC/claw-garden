use std::path::{Path, PathBuf};
use std::time::Duration;

use tauri::ipc::Channel;

use crate::compaction::{
    compact_history, estimate_history_tokens, micro_compact_tool_results,
    suggested_threshold_for_provider, CompactionConfig, CompactionResult,
};
use crate::hooks::permission::UserDecision;
use crate::hooks::{
    AgentContext, AgentHookConfig, FinalDecision, HookRunner, StderrDestination, StderrLevel,
};
use crate::memory::events::{MemoryEvent, MemorySource};
use crate::memory::identity::derive_project_id;
use crate::models::chat::{current_time_millis, generate_message_id};
use crate::models::{AppError, ChatMessage, ChatRequest, ChatStreamResult, Role, StreamEvent};
use crate::tools::executor::AgentToolContext;
use crate::tools::PermissionMode;
use crate::recovery::{
    ToolFailureKind, ToolRecoveryContext, ToolRecoveryResult, attempt_tool_recovery,
    build_error_context,
};
use crate::services::retry;
use crate::state::AppState;
use tokio::sync::oneshot;

const MAX_TOOL_ITERATIONS: usize = 5;
/// Cooldown after a 429 before the next request (circuit breaker).
const RATE_LIMIT_COOLDOWN_SECS: u64 = 60;

// ── Compaction ────────────────────────────────────────────────────────────────

/// Outcome of the two-phase auto-compaction analysis.
enum CompactAction {
    /// Micro-compaction was enough: truncated tool results, no messages removed.
    Micro {
        messages: Vec<ChatMessage>,
        tokens_freed: usize,
    },
    /// Full compaction needed (summary + message removal).
    Full {
        result: CompactionResult,
        original_tokens: usize,
    },
}

/// Check if auto-compaction should run and execute it if needed.
/// Two-phase approach:
///   Phase 1 (micro): truncate old, oversized tool results in-place.
///   Phase 2 (full): summary + message removal if still over threshold.
///
/// Lock order: session_manager → conversations (same as push_and_persist).
fn try_auto_compact(
    state: &AppState,
    agent_id: &str,
    provider_name: &str,
    on_event: &Channel<StreamEvent>,
    agent_project_path: Option<&str>,
) {
    // ── Analysis phase: snapshot the history, release the lock immediately ─────
    // We take an owned copy so expensive compaction work happens without holding
    // the conversations mutex.
    let snapshot = {
        let conversations = match state.conversations.lock() {
            Ok(c) => c,
            Err(_) => return,
        };
        let history = match conversations.get(agent_id) {
            Some(h) => h,
            None => return,
        };
        let threshold = suggested_threshold_for_provider(provider_name, "");
        let original_tokens = estimate_history_tokens(history);
        if original_tokens < threshold {
            return; // guard drops here, lock released
        }
        (original_tokens, history.to_vec(), threshold)
    }; // conversations lock released

    let (original_tokens, original_working, threshold) = snapshot;

    // Phase 1: micro-compact on a clone (keep the original for summary quality)
    let mut micro_working = original_working.clone();
    let (micro_modified, micro_freed) = micro_compact_tool_results(&mut micro_working);
    let tokens_after_micro = if micro_modified > 0 {
        estimate_history_tokens(&micro_working)
    } else {
        original_tokens
    };

    let action: Option<CompactAction> = if tokens_after_micro < threshold {
        if micro_modified > 0 {
            Some(CompactAction::Micro {
                messages: micro_working,
                tokens_freed: micro_freed,
            })
        } else {
            None
        }
    } else {
        // Phase 2: full compaction — use ORIGINAL messages so the summary is generated
        // from full tool results (not truncated previews). The "preserved recent" messages
        // are always within micro-compaction's PRESERVE_RECENT window, so they'd be
        // untouched by micro anyway.
        let config = CompactionConfig {
            preserve_recent: 4,
            max_estimated_tokens: 0, // threshold already checked
        };
        compact_history(&original_working, config).map(|result| CompactAction::Full {
            result,
            original_tokens,
        })
    };

    // ── Apply (session_manager → conversations) ───────────────────────────────
    match action {
        None => {}

        Some(CompactAction::Micro { messages, tokens_freed }) => {
            log::info!(
                "Micro-compaction for agent '{}': ~{} tokens freed",
                agent_id,
                tokens_freed,
            );

            // Persist atomically (snapshot rewrite, no compaction record)
            let persisted = state
                .session_manager
                .lock()
                .ok()
                .and_then(|sm| {
                    sm.save_agent_snapshot(agent_id, &messages)
                        .map_err(|e| log::error!("Failed to persist micro-compaction: {e}"))
                        .ok()
                })
                .is_some();

            // Update in-memory
            if let Ok(mut conversations) = state.conversations.lock() {
                if let Some(history) = conversations.get_mut(agent_id) {
                    *history = messages;
                }
            }

            if persisted {
                let _ = on_event.send(StreamEvent::Compaction {
                    removed_count: 0,
                    estimated_tokens_freed: tokens_freed,
                });
            }
        }

        Some(CompactAction::Full { result, original_tokens }) => {
            // Capture the disjoint delta BEFORE `result.compacted_messages` is
            // partial-moved below (drift note #4) — cheap, avoids fragility.
            let delta = result.new_summary.clone();
            let tokens_freed = original_tokens
                .saturating_sub(estimate_history_tokens(&result.compacted_messages));

            log::info!(
                "Full compaction for agent '{}': removed {} messages, ~{} tokens freed",
                agent_id,
                result.removed_count,
                tokens_freed,
            );

            let persisted = state
                .session_manager
                .lock()
                .ok()
                .and_then(|mut sm| {
                    sm.save_compacted_snapshot(
                        agent_id,
                        &result.compacted_messages,
                        result.removed_count,
                        &result.summary,
                    )
                    .map_err(|e| log::error!("Failed to persist compaction: {e}"))
                    .ok()
                })
                .is_some();

            if let Ok(mut conversations) = state.conversations.lock() {
                if let Some(history) = conversations.get_mut(agent_id) {
                    *history = result.compacted_messages;
                }
            }

            if persisted {
                let _ = on_event.send(StreamEvent::Compaction {
                    removed_count: result.removed_count,
                    estimated_tokens_freed: tokens_freed,
                });

                // REQ-5: feed the disjoint delta summary into the RAG memory
                // pipeline. Only on Full compaction, only after the snapshot
                // persisted — mirrors the StreamEvent::Compaction gate above.
                let project_id = project_id_from_path(agent_project_path);
                let evt = build_compaction_event(agent_id, project_id, delta, current_time_millis());
                if let Err(e) = state.memory_tx.send(evt) {
                    log::warn!("memory_tx send failed for compaction event (worker gone?): {e}");
                }
            }
        }
    }
}

// ── Persistence ───────────────────────────────────────────────────────────────

/// Push a message to the in-memory cache AND persist to disk.
///
/// Lock order: session_manager → conversations (matches session commands).
/// If persist fails, the in-memory push is rolled back.
/// R-NEW-1: `agent_project_path` is derived at the call site and forwarded to the
/// MemoryEvent builder. None means no project — project_id will be None in the event.
fn push_and_persist(
    state: &AppState,
    agent_id: &str,
    agent_project_path: Option<&str>,
    message: ChatMessage,
) {
    push_and_persist_inner(state, agent_id, agent_project_path, message);
}

/// Testable inner function — extracted from `push_and_persist` to allow unit
/// tests to call it directly without the Tauri command machinery.
pub(crate) fn push_and_persist_inner(
    state: &AppState,
    agent_id: &str,
    agent_project_path: Option<&str>,
    message: ChatMessage,
) {
    let mut session = match state.session_manager.lock() {
        Ok(s) => s,
        Err(e) => {
            log::error!("session_manager lock poisoned: {}", e);
            return;
        }
    };
    let mut conversations = match state.conversations.lock() {
        Ok(c) => c,
        Err(e) => {
            log::error!("conversations lock poisoned: {}", e);
            return;
        }
    };

    let entry = conversations.entry(agent_id.to_string()).or_default();
    entry.push(message);

    if let Err(e) = session.append_message(agent_id, entry.last().unwrap()) {
        entry.pop();
        log::error!("Session persist failed, rolled back: {}", e);
        return;
    }

    // ── Phase 3: emit MemoryEvent (design §4.d, R-NEW-1) ─────────────────────
    // Derive project_id from the path if provided.
    let project_id: Option<String> = agent_project_path
        .filter(|s| !s.is_empty())
        .map(|s| derive_project_id(Path::new(s)));

    if let Some(evt) =
        MemoryEvent::from_message(agent_id.to_string(), project_id, entry.last().unwrap(), MemorySource::Chat)
    {
        if let Err(e) = state.memory_tx.send(evt) {
            log::debug!("memory_tx send failed (worker gone?): {e}");
        }
    }
}

// ── Compaction → memory pipeline (Phase 7, REQ-5) ──────────────────────────────

/// Pure builder for the `MemoryEvent` sent to the RAG pipeline after a Full
/// compaction. Tauri-runtime-free — mirrors `MemoryEvent::from_message`'s
/// shape but is constructed directly (not via `from_message`, which returns
/// `None` for `Role::System`). `role: Role::System` here is a SENTINEL value
/// only: `NewMemoryRecord::from_event_and_chunk` branches on `source ==
/// MemorySource::Compaction` before it ever inspects `role`.
fn build_compaction_event(
    agent_id: &str,
    project_id: Option<String>,
    summary: String,
    ts: u64,
) -> MemoryEvent {
    MemoryEvent {
        agent_id: agent_id.to_string(),
        project_id,
        role: Role::System,
        content: summary,
        is_error: None,
        source: MemorySource::Compaction,
        timestamp_ms: ts,
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── 12-B: agent self-report (advisory) ────────────────────────────────────────

/// Handle the `report_anomaly` pseudo-tool. NO external side effect — emits
/// `StreamEvent::AnomalyReported` to the UI and returns an acknowledgement
/// that the model treats as a normal tool result, so the agentic loop
/// continues immediately without pausing.
///
/// Validation lives here (not in the executor pipeline) because we bypass the
/// executor entirely. The schema is small enough that a manual check is
/// cheaper than threading the schema validator.
fn handle_report_anomaly(
    input: &serde_json::Value,
    on_event: &Channel<StreamEvent>,
) -> (String, bool) {
    let anomaly_type = match input.get("anomaly_type").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => {
            return (
                "report_anomaly: missing required field 'anomaly_type'.".to_string(),
                true,
            );
        }
    };
    let description = match input.get("description").and_then(|v| v.as_str()) {
        Some(d) => d.to_string(),
        None => {
            return (
                "report_anomaly: missing required field 'description'.".to_string(),
                true,
            );
        }
    };
    let intended_action = input
        .get("intended_action")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let report_id = format!("anomaly-{}", uuid::Uuid::new_v4().as_simple());

    log::warn!(
        "Agent self-reported anomaly: type={anomaly_type}, description={description}"
    );

    let _ = on_event.send(StreamEvent::AnomalyReported {
        report_id,
        anomaly_type,
        description,
        intended_action,
    });

    (
        "Anomaly logged and shown to the user. Decide whether to proceed with the \
         original action, pivot to a safer alternative, or ask the user for guidance \
         before taking any further step."
            .to_string(),
        false,
    )
}

// ── Human-in-the-loop confirmation ────────────────────────────────────────────

/// Why a tool call was blocked before execution. Drives the message we feed
/// back to the LLM so it can decide whether to pivot, retry differently, or
/// ask the user — different reasons demand different model behavior.
#[derive(Debug, Clone)]
enum DenialKind {
    /// A PreToolUse hook returned Deny — policy block, not user-driven.
    HookBlock { reason: Option<String> },
    /// Hook returned Ask and the user explicitly denied in the modal.
    UserDeniedHookAsk,
    /// Native destructive tool, user denied in the modal.
    UserDeniedDestructive,
}

/// Park the chat loop until the UI resolves a `ConfirmationRequired` event.
///
/// Registers a oneshot in `AppState::pending_confirmations`, emits the request
/// to the UI, and awaits the user's decision. If the channel is dropped (UI
/// closed, app shutdown), defaults to `Deny` — fail-closed for safety.
async fn request_user_confirmation(
    state: &AppState,
    on_event: &Channel<StreamEvent>,
    tool_name: &str,
    arguments: &serde_json::Value,
    reason: Option<String>,
    source: &str,
) -> UserDecision {
    let call_id = format!("conf-{}", uuid::Uuid::new_v4().as_simple());
    let (tx, rx) = oneshot::channel::<UserDecision>();

    match state.pending_confirmations.lock() {
        Ok(mut map) => {
            map.insert(call_id.clone(), tx);
        }
        Err(e) => {
            log::error!(
                "pending_confirmations lock poisoned for '{tool_name}' — defaulting to Deny: {e}"
            );
            return UserDecision::Deny;
        }
    }

    let _ = on_event.send(StreamEvent::ConfirmationRequired {
        call_id: call_id.clone(),
        tool_name: tool_name.to_string(),
        arguments: arguments.clone(),
        reason,
        source: source.to_string(),
    });
    let _ = on_event.send(StreamEvent::State {
        state: "awaiting_confirmation".to_string(),
    });

    let decision = match rx.await {
        Ok(d) => d,
        Err(_) => {
            log::warn!(
                "Confirmation channel dropped for call_id={call_id} ({tool_name}) — defaulting to Deny"
            );
            // Defensive: clear the orphan sender (if still registered) so the map
            // doesn't grow unbounded across cancelled stream_chat invocations.
            if let Ok(mut map) = state.pending_confirmations.lock() {
                map.remove(&call_id);
            }
            UserDecision::Deny
        }
    };

    let _ = on_event.send(StreamEvent::ConfirmationResolved {
        call_id,
        decision: match decision {
            UserDecision::Allow => "allow".to_string(),
            UserDecision::Deny => "deny".to_string(),
        },
    });

    decision
}

// ── Memory pre-compute helper (Phase 4) ─────────────────────────────────────

/// Derives the project_id string from the agent's `project_path` (if set).
/// Used by both `stream_chat` and tests.
fn project_id_from_path(project_path: Option<&str>) -> Option<String> {
    project_path.and_then(|p| {
        if p.is_empty() {
            None
        } else {
            Some(crate::memory::identity::derive_project_id(Path::new(p)))
        }
    })
}

/// Computes the 4-block memory prefix (WORKSPACE / RECENT / LONG-TERM / CONFLICTS)
/// for the given agent and user query.
///
/// Regenerates the AUTO section of workspace.md unconditionally (D4 — cheap, ~2-5ms),
/// then calls `build_memory_prompt`. Any failure is degraded to an empty prefix so
/// chat ALWAYS proceeds (R-NEW-10 mitigation).
///
/// Returns `(memory_prefix, combined_system_prompt)` where `combined_system_prompt`
/// equals `agent_system_prompt` when the prefix is empty.
pub(crate) async fn compute_memory_prefix(
    state: &AppState,
    agent_system_prompt: &str,
    agent_id: &str,
    project_id_opt: Option<&str>,
    user_query: &str,
) -> String {
    let workspace_mgr = crate::memory::workspace::WorkspaceManager::new(
        state.base_dir.join("workspaces"),
    );

    // D4 — unconditional regen of AUTO section (cheap, ~2-5ms)
    if let Err(e) = workspace_mgr
        .regenerate_auto_section(agent_id, project_id_opt, &state.memory_store)
        .await
    {
        log::warn!("workspace regen failed: {e}");
    }

    let embedder_opt = state.embedder.get().cloned();
    let memory_prefix = crate::memory::prompt_builder::build_memory_prompt(
        &state.memory_store,
        &workspace_mgr,
        embedder_opt,
        agent_id,
        project_id_opt,
        user_query,
        Some(state.access_tracker.as_ref()),
    )
    .await
    .unwrap_or_default();

    if memory_prefix.is_empty() {
        agent_system_prompt.to_string()
    } else {
        format!("{}\n\n{}", memory_prefix, agent_system_prompt)
    }
}

// ── Command ───────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn stream_chat(
    agent_id: String,
    message: String,
    #[allow(non_snake_case)]
    isRetry: Option<bool>,
    on_event: Channel<StreamEvent>,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    let is_retry = isRetry.unwrap_or(false);
    // Get agent info
    let (agent_system_prompt, agent_custom_instructions, agent_provider_id, agent_project_path, agent_hooks) = {
        let agents = state
            .agents
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        match agents.iter().find(|a| a.id == agent_id) {
            Some(agent) => (
                agent.system_prompt.clone(),
                agent.custom_instructions.clone(),
                agent.provider_id.clone(),
                agent.project_path.clone(),
                agent.hooks.clone(),
            ),
            None => return Err(AppError::AgentNotFound(agent_id)),
        }
    };

    // Send "thinking" state
    on_event
        .send(StreamEvent::State {
            state: "thinking".to_string(),
        })
        .map_err(|e| AppError::Streaming(e.to_string()))?;

    // Capture message text for retrieval query BEFORE any moves.
    let message_text_for_query = message.clone();

    // Add user message to conversation history + persist
    // On retry the message is already in history from the first attempt
    if !is_retry {
        push_and_persist(
            &state,
            &agent_id,
            agent_project_path.as_deref(),
            ChatMessage {
                id: generate_message_id(),
                role: Role::User,
                content: message,
                tool_calls: None,
                timestamp_ms: current_time_millis(),
                is_error: None,
            },
        );
    }

    // Recovery context lives for the entire session — tracks attempts per tool/kind
    let mut recovery_ctx = ToolRecoveryContext::new();

    // Clean up stale rate-limit entries (older than 2 min) from the circuit breaker map.
    // unwrap_or_else: poison recovery — data is non-critical, continue with potentially
    // inconsistent state rather than crashing the chat command.
    {
        let mut tracker = state
            .rate_limit_tracker
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let cutoff = unix_now_secs().saturating_sub(120);
        tracker.retain(|_, &mut ts| ts > cutoff);
    }

    // Ensure an MCP connection for this agent's project path and get its tools.
    // Then build a frozen SessionToolSet: native built-ins first, MCP tools appended.
    // The set is built ONCE here and never mutated — stable for prompt cache hits.
    let mcp_tool_list = if let Some(ref project_path) = agent_project_path {
        let path_buf = PathBuf::from(project_path);
        if let Err(e) = state.mcp_pool.ensure_connected(&path_buf).await {
            log::warn!("MCP connect failed for {}: {}", project_path, e);
        }
        state.mcp_pool.get_tools(&path_buf).await.unwrap_or_default()
    } else {
        vec![]
    };
    let session_tool_set = state.tool_registry.build_session_tool_set(mcp_tool_list);
    let tools = Some(session_tool_set.definitions().to_vec());

    // Build per-session rate limiter pre-populated from each tool's declared limits.
    // State is per-invocation of stream_chat — resets between user messages.
    let rate_limiter = state.tool_registry.build_rate_limiter();

    // Resolve the session directory for tool result persistence.
    let session_dir = state
        .session_manager
        .lock()
        .map(|sm| sm.current_session_dir())
        .unwrap_or_else(|_| PathBuf::from("/tmp/claw-garden"));

    // ── Point 1: Pre-flight OAuth refresh ────────────────────────────────────
    // Must happen BEFORE get_agent_provider() so resolve_anthropic_auth() reads
    // the freshly written token from disk.
    match crate::services::auth::ensure_oauth_fresh(&state.oauth_refresh_lock).await {
        Ok(true) => {
            // Token was refreshed — evict the cached provider so the next call
            // picks up fresh credentials.
            state.invalidate_agent_provider(&agent_provider_id);
        }
        Err(e) => {
            log::warn!("OAuth pre-flight refresh failed: {e}");
            // Don't abort — let the provider fail naturally (may have an API key fallback)
        }
        Ok(false) => {}
    }

    // Build provider with potentially refreshed credentials
    let mut provider = state.get_agent_provider(&agent_provider_id);

    // Compile hook config once — paid at startup, not per tool call.
    // If compilation fails (invalid regex pattern in config), log and proceed without hooks.
    let hook_runner: Option<HookRunner> = match agent_hooks.compile() {
        Ok(compiled) => Some(HookRunner::new(compiled)),
        Err(e) => {
            log::error!("Failed to compile hooks for agent '{}': {}", agent_id, e);
            None
        }
    };

    let agent_ctx = AgentContext {
        agent_id: agent_id.clone(),
        agent_zone: None, // populated when zone system is wired up
    };

    // ── Phase 4: pre-loop memory pre-computation (D2) ────────────────────────
    // Runs ONCE before the agentic loop. The same combined_system_prompt is
    // reused for all MAX_TOOL_ITERATIONS iterations (D2 — one embed per turn).
    let project_id_opt = project_id_from_path(agent_project_path.as_deref());
    let combined_system_prompt = compute_memory_prefix(
        &state,
        &agent_system_prompt,
        &agent_id,
        project_id_opt.as_deref(),
        &message_text_for_query,
    )
    .await;
    // TODO Phase 5+: re-run retrieval per iteration when AccessTracker emits
    // synchronous tier-change events mid-loop (D2 forward-marker)

    // === AGENTIC LOOP ===
    for iteration in 0..MAX_TOOL_ITERATIONS {
        // Re-check OAuth freshness on each iteration (tool chains can span minutes)
        if let Ok(true) = crate::services::auth::ensure_oauth_fresh(&state.oauth_refresh_lock).await {
            log::info!("OAuth token refreshed mid-loop — invalidating cached provider");
            state.invalidate_agent_provider(&agent_provider_id);
            provider = state.get_agent_provider(&agent_provider_id);
        }

        // Build chat request from current history
        let chat_request = {
            let conversations = state
                .conversations
                .lock()
                .map_err(|e| AppError::Internal(e.to_string()))?;
            let history = conversations.get(&agent_id).cloned().unwrap_or_default();

            ChatRequest {
                model: String::new(),
                messages: history,
                system_prompt: Some(combined_system_prompt.clone()),
                custom_instructions: agent_custom_instructions.clone(),
                tools: tools.clone(),
                extensions: Default::default(),
            }
        };

        // ── Circuit breaker (per-provider) ────────────────────────────────────
        let provider_name = provider.name().to_string();
        let cooldown_wait = {
            let tracker = state.rate_limit_tracker.lock();
            if let Ok(tracker) = tracker {
                tracker.get(&provider_name).and_then(|&last_rl| {
                    let elapsed = unix_now_secs().saturating_sub(last_rl);
                    (elapsed < RATE_LIMIT_COOLDOWN_SECS)
                        .then(|| RATE_LIMIT_COOLDOWN_SECS - elapsed)
                })
            } else {
                None
            }
        }; // MutexGuard dropped here, BEFORE any .await

        if let Some(wait) = cooldown_wait {
            log::info!(
                "Circuit breaker active for '{}' — waiting {wait}s",
                provider_name
            );
            let _ = on_event.send(StreamEvent::State {
                state: format!("rate limited — waiting {wait}s"),
            });
            tokio::time::sleep(Duration::from_secs(wait)).await;
        }

        // ── Retry loop ────────────────────────────────────────────────────────
        let retry_config = retry::RetryConfig::default();
        let mut last_err: Option<AppError> = None;

        let stream_result: Result<ChatStreamResult, AppError> = 'retry: {
            for attempt in 0..=retry_config.max_retries {
                if attempt > 0 {
                    let delay = last_err
                        .as_ref()
                        .and_then(retry::extract_retry_after)
                        .unwrap_or_else(|| retry::compute_delay(&retry_config, attempt - 1));

                    log::info!(
                        "Retry {}/{} for agent '{}' after {}ms",
                        attempt,
                        retry_config.max_retries,
                        agent_id,
                        delay.as_millis()
                    );
                    let _ = on_event.send(StreamEvent::State {
                        state: format!("retrying ({attempt}/{})", retry_config.max_retries),
                    });
                    tokio::time::sleep(delay).await;

                    // Refresh OAuth before retry too
                    if let Ok(true) = crate::services::auth::ensure_oauth_fresh(
                        &state.oauth_refresh_lock,
                    ).await {
                        state.invalidate_agent_provider(&agent_provider_id);
                        provider = state.get_agent_provider(&agent_provider_id);
                    }
                }

                let channel_clone = on_event.clone();
                let sender: crate::providers::StreamSender = Box::new(move |event| {
                    channel_clone.send(event).map_err(|e| e.to_string())
                });

                match provider.chat_stream(&chat_request, sender).await {
                    Ok(result) => break 'retry Ok(result),
                    Err(e)
                        if retry::is_retryable(&e) && attempt < retry_config.max_retries =>
                    {
                        log::warn!(
                            "Retryable error on attempt {} for agent '{}': {e}",
                            attempt + 1,
                            agent_id
                        );
                        // Record rate-limit timestamp for this specific provider
                        if e.to_string().contains("returned 429") {
                            if let Ok(mut tracker) = state.rate_limit_tracker.lock() {
                                tracker.insert(provider_name.clone(), unix_now_secs());
                            }
                        }
                        last_err = Some(e);
                    }
                    Err(e) => break 'retry Err(e),
                }
            }
            // All attempts exhausted
            Err(last_err
                .unwrap_or_else(|| AppError::Internal("Retry exhausted".to_string())))
        };

        match stream_result {
            Ok(ChatStreamResult::Text(response_text)) => {
                crate::services::sycophancy::check_response(&agent_id, &response_text);

                push_and_persist(
                    &state,
                    &agent_id,
                    agent_project_path.as_deref(),
                    ChatMessage {
                        id: generate_message_id(),
                        role: Role::Assistant,
                        content: response_text.clone(),
                        tool_calls: None,
                        timestamp_ms: current_time_millis(),
                        is_error: None,
                    },
                );

                // Point 2: Two-phase auto-compaction after text response
                try_auto_compact(&state, &agent_id, provider.name(), &on_event, agent_project_path.as_deref());

                // Stop hook — always fire & forget, the agent has finished its turn.
                if let Some(ref runner) = hook_runner {
                    runner.run_stop("end_turn", Some(&response_text), &agent_ctx);
                }

                on_event
                    .send(StreamEvent::Done {
                        full_response: response_text,
                    })
                    .map_err(|e| AppError::Streaming(e.to_string()))?;

                return Ok(());
            }

            Ok(ChatStreamResult::ToolCalls(tool_calls)) => {
                push_and_persist(
                    &state,
                    &agent_id,
                    agent_project_path.as_deref(),
                    ChatMessage {
                        id: generate_message_id(),
                        role: Role::Assistant,
                        content: String::new(),
                        tool_calls: Some(tool_calls.clone()),
                        timestamp_ms: current_time_millis(),
                        is_error: None,
                    },
                );

                for tc in &tool_calls {
                    let tool_name = &tc.function.name;
                    let arguments = &tc.function.arguments;

                    let _ = on_event.send(StreamEvent::ToolUse {
                        tool_name: tool_name.clone(),
                        arguments: arguments.to_string(),
                    });
                    let _ = on_event.send(StreamEvent::State {
                        state: "reading".to_string(),
                    });

                    // ── FASE 1: PreToolUse hooks (run first; can mutate input) ────────
                    // All output channels are published BEFORE the permission decision,
                    // so the LLM and user have full context even when the tool is blocked.
                    let (effective_input, hook_decision, pre_llm_context) =
                        if let Some(ref runner) = hook_runner {
                            let pre = runner
                                .run_pre_tool_use(tool_name, arguments, &agent_ctx)
                                .await;

                            // 1a. stderr — routed by destination
                            for entry in &pre.stderr_lines {
                                match (&entry.level, &entry.destination) {
                                    (StderrLevel::Error, StderrDestination::Model) => {
                                        log::error!(
                                            "hook stderr [{tool_name}] {}: {}",
                                            entry.command,
                                            entry.content
                                        );
                                        // Model-bound stderr: prepend to LLM context below
                                    }
                                    (StderrLevel::Warning, StderrDestination::User) => {
                                        log::warn!(
                                            "hook stderr [{tool_name}] {}: {}",
                                            entry.command,
                                            entry.content
                                        );
                                        let _ = on_event.send(StreamEvent::HookWarning {
                                            message: entry.content.clone(),
                                        });
                                    }
                                    _ => {
                                        log::debug!(
                                            "hook stderr [{tool_name}] {}: {}",
                                            entry.command,
                                            entry.content
                                        );
                                    }
                                }
                            }

                            // 1b. garden_events → UI
                            for ev in &pre.garden_events {
                                let _ = on_event.send(StreamEvent::HookGardenEvent {
                                    move_to_zone: ev.move_to_zone.clone(),
                                    animation: ev.animation.clone(),
                                    notify: ev.notify.clone(),
                                });
                            }

                            // 1c. system_messages → user warning banner
                            for msg in &pre.system_messages {
                                let _ = on_event.send(StreamEvent::HookWarning {
                                    message: msg.clone(),
                                });
                            }

                            // 1d. Build LLM context: additional_contexts + model-bound stderr
                            let mut llm_parts: Vec<String> = pre
                                .stderr_lines
                                .iter()
                                .filter(|e| {
                                    matches!(e.destination, StderrDestination::Model)
                                })
                                .map(|e| {
                                    format!("[hook stderr — {}]: {}", e.command, e.content)
                                })
                                .collect();
                            llm_parts.extend(pre.additional_contexts.iter().cloned());
                            let llm_ctx = llm_parts.join("\n\n");

                            // Effective input — post-mutation. Used by both gates and execution.
                            let effective = pre
                                .updated_input
                                .clone()
                                .unwrap_or_else(|| arguments.clone());

                            (effective, pre.final_decision(), llm_ctx)
                        } else {
                            (arguments.clone(), FinalDecision::Allow, String::new())
                        };

                    // ── FASE 2: permission decision (Deny / Ask / Allow) ──────────────
                    // Resolution order:
                    //   1. Hook said Deny → block immediately.
                    //   2. Hook said Ask → request user confirmation (with hook's reason).
                    //   3. Hook said Allow → if the tool is a NATIVE destructive built-in,
                    //      gate on user confirmation regardless of hook config (12-A).
                    //   4. Otherwise → execute.
                    //
                    // Note: native-destructive gate runs on the EFFECTIVE input (post-hook
                    // mutation) so the user approves what will actually run, not the raw
                    // arguments the model produced.
                    let denial_kind: Option<DenialKind> = match hook_decision {
                        FinalDecision::Deny { reason } => {
                            Some(DenialKind::HookBlock { reason })
                        }
                        FinalDecision::Ask { reason } => {
                            let decision = request_user_confirmation(
                                &state,
                                &on_event,
                                tool_name,
                                &effective_input,
                                reason,
                                "hook_ask",
                            )
                            .await;
                            matches!(decision, UserDecision::Deny)
                                .then_some(DenialKind::UserDeniedHookAsk)
                        }
                        FinalDecision::Allow => {
                            let needs_destructive_gate = state
                                .tool_registry
                                .find(tool_name)
                                .map(|spec| spec.is_destructive)
                                .unwrap_or(false);
                            if needs_destructive_gate {
                                let decision = request_user_confirmation(
                                    &state,
                                    &on_event,
                                    tool_name,
                                    &effective_input,
                                    None,
                                    "destructive_tool",
                                )
                                .await;
                                matches!(decision, UserDecision::Deny)
                                    .then_some(DenialKind::UserDeniedDestructive)
                            } else {
                                None
                            }
                        }
                    };
                    let pre_denied = denial_kind.is_some();

                    // ── Execute tool (or short-circuit if denied) ─────────────────────
                    let (result_content, is_error) = if let Some(kind) = denial_kind {
                        let base = match kind {
                            DenialKind::HookBlock { reason } => match reason {
                                Some(r) => format!(
                                    "Tool '{tool_name}' was blocked by a hook policy: {r}"
                                ),
                                None => format!(
                                    "Tool '{tool_name}' was blocked by a hook policy before execution."
                                ),
                            },
                            DenialKind::UserDeniedHookAsk => format!(
                                "Tool '{tool_name}' was blocked: the user denied the confirmation \
                                 requested by hook policy. Do NOT retry the same call — consider an \
                                 alternative approach or ask the user what they prefer."
                            ),
                            DenialKind::UserDeniedDestructive => format!(
                                "Tool '{tool_name}' was blocked: the user denied confirmation for \
                                 this destructive operation. Do NOT retry the same call — consider \
                                 a non-destructive alternative or ask the user what they prefer."
                            ),
                        };
                        let tail = if pre_llm_context.is_empty() {
                            String::new()
                        } else {
                            format!("\n\nContext from hook:\n{pre_llm_context}")
                        };
                        (format!("{base}{tail}"), true)
                    } else if tool_name == "report_anomaly" {
                        // ── 12-B: advisory self-report — bypass the executor ─────────
                        // No side effect, no pipeline — emit event, return ack, let the
                        // agentic loop continue. PostToolUse hooks still run below so
                        // operators can plug in additional logging or telemetry.
                        handle_report_anomaly(&effective_input, &on_event)
                    } else if state.tool_registry.find(tool_name).is_some() {
                        // ── Native tool: go through ToolExecutor pipeline ─────────────
                        // Pipeline: validate → permission → cache → rate_limit →
                        //           concurrency_gate → dispatch → cache_store → persist
                        //
                        // Progress streaming: bash and web_fetch emit ToolProgress events
                        // as they run. A forwarder task translates them into
                        // StreamEvent::ToolProgress so the UI can animate the agent in
                        // real time instead of freezing until the tool returns. The task
                        // exits on its own when the ctx (and its sender) is dropped at
                        // the end of this arm.
                        let (progress_tx, mut progress_rx) =
                            tokio::sync::mpsc::unbounded_channel::<crate::tools::ToolProgress>();
                        {
                            let forward_channel = on_event.clone();
                            let forward_tool_name = tool_name.to_string();
                            tokio::spawn(async move {
                                while let Some(progress) = progress_rx.recv().await {
                                    let event = match progress {
                                        crate::tools::ToolProgress::BashProgress {
                                            stdout_chunk,
                                            stderr_chunk,
                                        } => StreamEvent::ToolProgress {
                                            tool: forward_tool_name.clone(),
                                            stdout_chunk: (!stdout_chunk.is_empty())
                                                .then_some(stdout_chunk),
                                            stderr_chunk: (!stderr_chunk.is_empty())
                                                .then_some(stderr_chunk),
                                            bytes_read: None,
                                            message: None,
                                        },
                                        crate::tools::ToolProgress::WebFetchProgress {
                                            bytes_read,
                                            url,
                                        } => StreamEvent::ToolProgress {
                                            tool: forward_tool_name.clone(),
                                            stdout_chunk: None,
                                            stderr_chunk: None,
                                            bytes_read: Some(bytes_read),
                                            message: Some(url),
                                        },
                                        crate::tools::ToolProgress::Generic { message } => {
                                            StreamEvent::ToolProgress {
                                                tool: forward_tool_name.clone(),
                                                stdout_chunk: None,
                                                stderr_chunk: None,
                                                bytes_read: None,
                                                message: Some(message),
                                            }
                                        }
                                    };
                                    let _ = forward_channel.send(event);
                                }
                            });
                        }

                        let agent_tool_ctx = AgentToolContext {
                            agent_id: agent_id.clone(),
                            // Orchestrators get DangerFullAccess; others WorkspaceWrite.
                            // TODO: derive from Agent.role when role-based permissions land.
                            permission_mode: PermissionMode::DangerFullAccess,
                            project_path: agent_project_path.clone(),
                            session_dir: session_dir.clone(),
                            progress_tx: Some(progress_tx),
                        };
                        state.tool_executor.execute(
                            tool_name,
                            &effective_input,
                            &agent_tool_ctx,
                            &rate_limiter,
                        ).await
                    } else {
                        // ── MCP tool: route to mcp_pool ───────────────────────────────
                        let tool_result = if let Some(ref project_path) = agent_project_path {
                            let path_buf = PathBuf::from(project_path);
                            // No lock held — McpPool manages per-server RwLocks internally
                            state
                                .mcp_pool
                                .call_tool(&path_buf, tool_name, effective_input.clone())
                                .await
                        } else {
                            Err(crate::models::McpError::InitializationFailed {
                                message: "No project path configured for this agent".into(),
                            }
                            .into())
                        };

                        match tool_result {
                            Ok(content) => (content, false),
                            Err(e) => {
                                let kind = ToolFailureKind::from_error(&e);

                                // All kinds flow through attempt_tool_recovery — the recipe
                                // decides the policy. Permanent failures (ToolNotFound,
                                // ToolInputInvalid) use [EscalateToLlm] and return immediately
                                // without ever calling retry_fn.
                                if let Some(ref project_path) = agent_project_path {
                                    let path_buf = PathBuf::from(project_path);

                                    // Clone captured vars once; each async move invocation
                                    // gets its own owned copies (lifetime/Send fix).
                                    let retry_path = path_buf.clone();
                                    let retry_name = tool_name.to_string();
                                    let retry_input = effective_input.clone();

                                    // Deref tauri::State<'_, AppState> → &AppState
                                    let app_state: &AppState = &state;

                                    // Wrap Channel::send into a plain closure so
                                    // attempt_tool_recovery doesn't depend on Tauri's IPC type.
                                    let emit_recovery = |ev: StreamEvent| { on_event.send(ev).ok(); };

                                    let result = attempt_tool_recovery(
                                        &mut recovery_ctx,
                                        tool_name,
                                        kind,
                                        app_state,
                                        &path_buf,
                                        &emit_recovery,
                                        move || {
                                            let pb = retry_path.clone();
                                            let tn = retry_name.clone();
                                            let ei = retry_input.clone();
                                            async move {
                                                app_state.mcp_pool.call_tool(&pb, &tn, ei).await
                                            }
                                        },
                                    )
                                    .await;

                                    match result {
                                        ToolRecoveryResult::Recovered { content } => {
                                            (content, false)
                                        }
                                        ToolRecoveryResult::EscalationRequired { reason } => {
                                            (build_error_context(tool_name, &e, &reason), true)
                                        }
                                        ToolRecoveryResult::PartialRecovery { .. } => (
                                            build_error_context(tool_name, &e, "partial-recovery"),
                                            true,
                                        ),
                                    }
                                } else {
                                    (build_error_context(tool_name, &e, "no-project-path"), true)
                                }
                            }
                        }
                    };

                    // ── PostToolUse hooks ─────────────────────────────────────────────
                    // Only run if the tool actually executed (not denied by pre-hook).
                    let effective_output = if !pre_denied {
                        if let Some(ref runner) = hook_runner {
                            let post = if is_error {
                                runner
                                    .run_post_tool_use_failure(
                                        tool_name,
                                        &effective_input,
                                        &result_content,
                                        &agent_ctx,
                                    )
                                    .await
                            } else {
                                runner
                                    .run_post_tool_use(
                                        tool_name,
                                        &effective_input,
                                        &result_content,
                                        &agent_ctx,
                                    )
                                    .await
                            };

                            // garden_events from post hook
                            for ev in &post.garden_events {
                                let _ = on_event.send(StreamEvent::HookGardenEvent {
                                    move_to_zone: ev.move_to_zone.clone(),
                                    animation: ev.animation.clone(),
                                    notify: ev.notify.clone(),
                                });
                            }

                            // system_messages from post hook
                            for msg in &post.system_messages {
                                let _ = on_event.send(StreamEvent::HookWarning {
                                    message: msg.clone(),
                                });
                            }

                            // updated_output: what the LLM sees (real output is unchanged)
                            post.updated_output.unwrap_or(result_content)
                        } else {
                            result_content
                        }
                    } else {
                        result_content
                    };

                    // Prepend pre-hook LLM context if present (injected before tool output)
                    let final_content = if !pre_llm_context.is_empty() && !pre_denied {
                        format!("{}\n\n{}", pre_llm_context, effective_output)
                    } else {
                        effective_output
                    };

                    push_and_persist(
                        &state,
                        &agent_id,
                        agent_project_path.as_deref(),
                        ChatMessage {
                            id: generate_message_id(),
                            role: Role::Tool,
                            content: final_content,
                            tool_calls: Some(vec![tc.clone()]),
                            timestamp_ms: current_time_millis(),
                            is_error: Some(is_error), // Phase 3 D1 — is_error in scope from line ~814
                        },
                    );
                }

                // Point 2: Two-phase auto-compaction after tool execution
                try_auto_compact(&state, &agent_id, provider.name(), &on_event, agent_project_path.as_deref());

                let _ = on_event.send(StreamEvent::State {
                    state: "thinking".to_string(),
                });

                log::info!(
                    "Agentic loop iteration {} — processed {} tool calls",
                    iteration + 1,
                    tool_calls.len()
                );
            }

            Err(e) => {
                let _ = on_event.send(StreamEvent::Error {
                    error: e.to_string(),
                });
                return Ok(());
            }
        }
    }

    let _ = on_event.send(StreamEvent::Error {
        error: format!("Max tool iterations ({MAX_TOOL_ITERATIONS}) reached"),
    });

    Ok(())
}

// ── T13-tests: push_and_persist MemoryEvent emission (REQ-INT-002) ────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::events::MemorySource;
    use crate::models::chat::Role;

    fn make_test_message(role: Role, content: &str, is_error: Option<bool>) -> ChatMessage {
        ChatMessage {
            id: "test-id".to_string(),
            role,
            content: content.to_string(),
            tool_calls: None,
            timestamp_ms: 1_000_000,
            is_error,
        }
    }

    #[tokio::test]
    async fn push_and_persist_emits_event_for_user_role() {
        let (state, mut rx) = crate::state::AppState::new_for_test_async().await;
        let msg = make_test_message(Role::User, "hello", None);
        push_and_persist_inner(&state, "agent-1", None, msg);
        let evt = rx.try_recv().expect("expected one MemoryEvent");
        assert_eq!(evt.role, Role::User);
        assert_eq!(evt.content, "hello");
    }

    #[tokio::test]
    async fn push_and_persist_skips_event_for_empty_assistant_tool_call() {
        let (state, mut rx) = crate::state::AppState::new_for_test_async().await;
        let mut msg = make_test_message(Role::Assistant, "", None);
        msg.tool_calls = Some(vec![]);
        push_and_persist_inner(&state, "agent-1", None, msg);
        assert!(rx.try_recv().is_err(), "expected no MemoryEvent for empty assistant tool-call");
    }

    #[tokio::test]
    async fn push_and_persist_emits_event_for_tool_with_is_error() {
        let (state, mut rx) = crate::state::AppState::new_for_test_async().await;
        let msg = make_test_message(Role::Tool, "error output", Some(true));
        push_and_persist_inner(&state, "agent-1", None, msg);
        let evt = rx.try_recv().expect("expected one MemoryEvent");
        assert_eq!(evt.role, Role::Tool);
        assert_eq!(evt.is_error, Some(true));
    }

    #[tokio::test]
    async fn push_and_persist_dropped_receiver_no_panic() {
        let (state, rx) = crate::state::AppState::new_for_test_async().await;
        drop(rx); // drop the receiver — sends must not panic
        let msg = make_test_message(Role::User, "hello", None);
        // Must not panic
        push_and_persist_inner(&state, "agent-1", None, msg);
    }

    // ── T6-test (REQ-5): pure build_compaction_event helper ───────────────────

    #[test]
    fn build_compaction_event_sets_compaction_source_and_system_sentinel_role() {
        let evt = build_compaction_event(
            "agent-1",
            Some("proj-hash".to_string()),
            "delta summary text".to_string(),
            12345,
        );
        assert_eq!(evt.agent_id, "agent-1");
        assert_eq!(evt.project_id, Some("proj-hash".to_string()));
        assert_eq!(evt.content, "delta summary text");
        assert_eq!(evt.timestamp_ms, 12345);
        assert_eq!(evt.source, MemorySource::Compaction);
        assert_eq!(evt.role, Role::System, "role is a sentinel — ignored downstream when source=Compaction");
    }

    #[test]
    fn build_compaction_event_none_project_id_stays_none() {
        let evt = build_compaction_event("agent-2", None, "text".to_string(), 1);
        assert_eq!(evt.project_id, None);
    }

    // ── T7-test (REQ-5): try_auto_compact Compaction-event gating ─────────────

    fn make_test_channel() -> Channel<StreamEvent> {
        Channel::new(|_| Ok(()))
    }

    fn make_long_user_msg(content_len: usize, tag: usize) -> ChatMessage {
        ChatMessage {
            id: format!("m-{tag}"),
            role: Role::User,
            content: format!("{}-{tag}", "x".repeat(content_len)),
            tool_calls: None,
            timestamp_ms: 1_000_000 + tag as u64,
            is_error: None,
        }
    }

    /// Drains ALL pending events off `rx` and returns only the ones sourced
    /// from Compaction — the signal under test in every T7 scenario.
    fn drain_compaction_events(
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<MemoryEvent>,
    ) -> Vec<MemoryEvent> {
        let mut out = vec![];
        while let Ok(evt) = rx.try_recv() {
            if evt.source == MemorySource::Compaction {
                out.push(evt);
            }
        }
        out
    }

    #[tokio::test]
    async fn try_auto_compact_full_arm_sends_exactly_one_compaction_event() {
        let (state, mut memory_rx) = crate::state::AppState::new_for_test_async().await;
        let agent_id = "agent-full";
        // 10 large messages, well above the ollama threshold (30_000 tokens),
        // more than preserve_recent(4) → guarantees a Full compaction.
        let messages: Vec<ChatMessage> = (0..10).map(|i| make_long_user_msg(15_000, i)).collect();
        {
            let mut conversations = state.conversations.lock().unwrap();
            conversations.insert(agent_id.to_string(), messages);
        }

        let channel = make_test_channel();
        let agent_project_path = Some("/some/project/path");
        try_auto_compact(&state, agent_id, "ollama", &channel, agent_project_path);

        let compaction_events = drain_compaction_events(&mut memory_rx);
        assert_eq!(
            compaction_events.len(), 1,
            "Full compaction must send exactly 1 Compaction event; got {}",
            compaction_events.len()
        );
        let expected_project_id = project_id_from_path(agent_project_path);
        assert_eq!(compaction_events[0].project_id, expected_project_id);
        assert_eq!(compaction_events[0].source, MemorySource::Compaction);
        assert!(!compaction_events[0].content.is_empty());
    }

    #[tokio::test]
    async fn try_auto_compact_micro_arm_sends_no_compaction_event() {
        let (state, mut memory_rx) = crate::state::AppState::new_for_test_async().await;
        let agent_id = "agent-micro";
        // 1 oversized tool result (old) + 6 tiny recent messages: total tokens
        // clear the ollama threshold pre-micro, but drop well below it once the
        // tool result is truncated/cleared — this is the Micro path, not Full.
        let mut messages = vec![ChatMessage {
            id: "tool-1".to_string(),
            role: Role::Tool,
            content: "x".repeat(130_000),
            tool_calls: None,
            timestamp_ms: 1,
            is_error: Some(false),
        }];
        for _ in 0..6 {
            messages.push(make_test_message(Role::User, "hi", None));
        }
        {
            let mut conversations = state.conversations.lock().unwrap();
            conversations.insert(agent_id.to_string(), messages);
        }

        let channel = make_test_channel();
        try_auto_compact(&state, agent_id, "ollama", &channel, None);

        let compaction_events = drain_compaction_events(&mut memory_rx);
        assert_eq!(
            compaction_events.len(), 0,
            "Micro compaction must send 0 Compaction events; got {}",
            compaction_events.len()
        );
        // Sanity: confirm the Micro branch actually ran (tool content cleared).
        let conversations = state.conversations.lock().unwrap();
        let history = conversations.get(agent_id).expect("history must exist");
        assert!(
            history[0].content.contains("cleared") || history[0].content.contains("truncated"),
            "expected micro-compaction to have modified the oversized tool result"
        );
    }

    #[tokio::test]
    async fn try_auto_compact_no_action_sends_no_compaction_event() {
        let (state, mut memory_rx) = crate::state::AppState::new_for_test_async().await;
        let agent_id = "agent-noop";
        let messages = vec![
            make_test_message(Role::User, "hi", None),
            make_test_message(Role::Assistant, "hello", None),
        ];
        {
            let mut conversations = state.conversations.lock().unwrap();
            conversations.insert(agent_id.to_string(), messages);
        }

        let channel = make_test_channel();
        try_auto_compact(&state, agent_id, "ollama", &channel, None);

        let compaction_events = drain_compaction_events(&mut memory_rx);
        assert_eq!(
            compaction_events.len(), 0,
            "No-action path must send 0 Compaction events; got {}",
            compaction_events.len()
        );
    }

    #[tokio::test]
    async fn try_auto_compact_full_arm_snapshot_failure_suppresses_event() {
        let (state, mut memory_rx) = crate::state::AppState::new_for_test_async().await;
        let agent_id = "agent-full-fail";
        let messages: Vec<ChatMessage> = (0..10).map(|i| make_long_user_msg(15_000, i)).collect();
        {
            let mut conversations = state.conversations.lock().unwrap();
            conversations.insert(agent_id.to_string(), messages);
        }

        // Force save_compacted_snapshot to fail WITHOUT touching the shared
        // sessions root (`AppState::new_for_test_async` ties every test in this
        // process to the same PID-keyed temp dir — deleting the root would race
        // concurrently-running tests, and even scanning for "the" session
        // subdirectory is ambiguous under concurrent session creation). Use the
        // public `current_session_dir()` accessor to get THIS state's exact
        // session directory, then pre-create a DIRECTORY at the exact path
        // `save_compacted_snapshot` will try to rename a file onto, for THIS
        // agent only: `fs::rename(tmp_file, jsonl_path)` fails when `jsonl_path`
        // already exists as a directory.
        let jsonl_path = {
            let sm = state.session_manager.lock().unwrap();
            sm.current_session_dir().join(format!("{agent_id}.jsonl"))
        };
        std::fs::create_dir_all(&jsonl_path).expect("failed to pre-create blocking directory");

        let channel = make_test_channel();
        try_auto_compact(&state, agent_id, "ollama", &channel, None);

        let compaction_events = drain_compaction_events(&mut memory_rx);
        assert_eq!(
            compaction_events.len(), 0,
            "snapshot persist failure must suppress the Compaction event (mirrors persisted gate); got {}",
            compaction_events.len()
        );
    }

    // T24-tests: compute_memory_prefix (pre-loop memory pre-compute) ─────────

    /// With no memories and no workspace file, compute_memory_prefix should
    /// return the agent_system_prompt unchanged (byte-equal).
    #[tokio::test]
    async fn stream_chat_with_no_memories_uses_unmodified_system_prompt() {
        let (state, _rx) = crate::state::AppState::new_for_test_async().await;
        let agent_system_prompt = "You are a test agent.";

        let result = compute_memory_prefix(
            &state,
            agent_system_prompt,
            "agent-1",
            None,
            "hello",
        ).await;

        assert_eq!(
            result, agent_system_prompt,
            "empty memory → combined_system_prompt must equal agent_system_prompt byte-for-byte"
        );
    }

    /// When workspace has content OR there are retrieval hits, the combined
    /// prompt should contain at least one memory block separator.
    #[tokio::test]
    async fn stream_chat_with_memories_prepends_blocks_to_system_prompt() {
        use crate::memory::workspace::{WorkspaceManager, AUTO_START, AUTO_END, USER_START, USER_END};

        let (state, _rx) = crate::state::AppState::new_for_test_async().await;
        let agent_system_prompt = "You are a helpful agent.";

        // Pre-write a workspace file with USER section content — the USER section
        // is preserved byte-for-byte by regenerate_auto_section, so it won't be
        // wiped even when the AUTO section is regenerated with empty DB data.
        let wm = WorkspaceManager::new(state.base_dir.join("workspaces"));
        let path = wm.workspace_path("agent-ws", None);
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        let ws_content = format!(
            "{}\n{}\n{}  my custom note\n{}\n",
            AUTO_START, AUTO_END, USER_START, USER_END
        );
        tokio::fs::write(&path, ws_content).await.unwrap();

        let result = compute_memory_prefix(
            &state,
            agent_system_prompt,
            "agent-ws",
            None,
            "test query",
        ).await;

        assert!(
            result.contains(crate::memory::prompt_builder::SEP_WORKSPACE_TEST),
            "combined_system_prompt should contain WORKSPACE separator when workspace has content; got: {}",
            &result[..result.len().min(200)]
        );
        assert!(
            result.ends_with(agent_system_prompt),
            "agent_system_prompt must be appended after memory prefix"
        );
    }

    /// A retrieval or workspace error must not cause compute_memory_prefix to
    /// panic or return an Err — it degrades to the plain agent_system_prompt.
    #[tokio::test]
    async fn stream_chat_retrieval_error_does_not_fail_chat() {
        let (state, _rx) = crate::state::AppState::new_for_test_async().await;
        let agent_system_prompt = "You are a helpful agent.";

        // Point base_dir to a path that doesn't exist — workspace regen will fail.
        // compute_memory_prefix must still return the plain system prompt.
        let result = compute_memory_prefix(
            &state,
            agent_system_prompt,
            "agent-err",
            None,
            "query",
        ).await;

        // Must not panic; should return something (either prefix+prompt or just prompt)
        assert!(
            result.contains(agent_system_prompt),
            "agent_system_prompt must appear in result even on errors"
        );
    }

    /// With an empty embedder OnceCell (not seeded), compute_memory_prefix must
    /// not panic and must return the plain system prompt.
    #[tokio::test]
    async fn stream_chat_with_embedder_not_ready_does_not_panic() {
        use once_cell::sync::OnceCell;
        use crate::memory::embedder::Embedder;

        let (mut state, _rx) = crate::state::AppState::new_for_test_async().await;
        // Replace embedder with empty OnceCell to simulate "not yet bootstrapped"
        state.embedder = std::sync::Arc::new(OnceCell::<std::sync::Arc<dyn Embedder>>::new());

        let agent_system_prompt = "You are a helpful agent.";
        let result = compute_memory_prefix(
            &state,
            agent_system_prompt,
            "agent-noembedder",
            None,
            "query",
        ).await;

        assert!(
            result.contains(agent_system_prompt),
            "should contain agent_system_prompt even with no embedder"
        );
    }
}
