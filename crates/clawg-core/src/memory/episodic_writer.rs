//! Episodic-trajectory writer.
//!
//! `build_trajectory` and its testable core `run_episodic_writer_core` — the
//! latter consumed by `memory::miner::mine_session_core`.
//! `build_trajectory` is a **pure, deterministic** function (no LLM, no DB):
//! it aggregates the ENTIRE multi-agent session snapshot into a single
//! `TrajectoryDraft`, since `episodic_trajectory.session_id` is UNIQUE (one
//! row per session).
//!
//! # Constraints that are deliberate, not incidental
//! - `build_trajectory` never panics and performs no I/O.
//! - No `Role::User` (non-empty) anywhere in `history` → `None` (SKIP write).
//!   There is NO fallback to "first non-empty message of any role" — an
//!   empty/placeholder `task` would be NOT NULL-violating and would
//!   contaminate KNN with a meaningless embed.
//! - `success_score` is computed over the FLATTENED sequence of all agents in
//!   `Vec` order, messages in their own order (NOT sorted by timestamp) — see
//!   [`build_trajectory`] doc for the exact algorithm.

use std::collections::HashMap;
use std::sync::Arc;

use crate::memory::db::MemoryStore;
use crate::memory::embedder::{EmbedRole, Embedder};
use crate::memory::store_insert::{CreatedByType, NewEpisodicTrajectoryRecord};
use crate::models::chat::Role;
use crate::models::ChatMessage;

/// Hard cap (in `chars`) on the `task` column — mirrors `extractor.rs`'s
/// `TRANSCRIPT_CHAR_CAP` discipline of bounding LLM/DB-bound text.
const TASK_CHAR_CAP: usize = 512;

/// Hard cap (in `chars`) applied PER-ITEM to `errors_encountered` before
/// serializing to `errors_json`. Deliberately applied at the DB-boundary and
/// NOT inside `build_trajectory`: the pure
/// aggregation function keeps the full error content; the cap is a
/// persistence concern (mirrors the `task`/`TASK_CHAR_CAP` split, but scoped
/// to a different field and a different layer).
const ERROR_ITEM_CHAR_CAP: usize = 500;

/// `success_score` formula coefficients. Deliberate values, pinned by worked
/// examples in the tests below — changing one silently reshapes which sessions
/// get mined into skills.
///
///   `success_score = (BASE_SCORE + completion_bonus - unresolved_penalty).clamp(0.0, 1.0)`
///   where `completion_bonus = COMPLETION_BONUS` if the session ended on an
///   `Assistant` message, else `0.0`, and
///   `unresolved_penalty = UNRESOLVED_PENALTY_COEFF * (unresolved / tool_calls.max(1))`.
///
/// **This score is a boolean wearing a float's clothes, and reading it as a
/// gradient will mislead you.** `unresolved` counts errors positioned after the
/// LAST assistant message, so a session that ends on an assistant message has
/// no room for one: the bonus applies, the penalty cannot, and the score is
/// exactly 0.9. A session that ends any other way never gets the bonus and
/// lands in `[0.0, 0.4]`. The whole band between 0.4 and 0.9 is unreachable —
/// `success_score_lands_only_in_two_disjoint_bands` is what keeps that true —
/// and the miner's golden gate sits at 0.85, inside the gap. So the gate asks
/// one question: did this session end on an assistant message?
///
/// That is a reasonable question and the gate is fine. What is not fine is
/// treating the number as a quality measure: ranking sessions by it, picking a
/// different threshold, or averaging it produces confident nonsense. Give it
/// real resolution before asking it a real question.
const BASE_SCORE: f64 = 0.4;
const COMPLETION_BONUS: f64 = 0.5;
const UNRESOLVED_PENALTY_COEFF: f64 = 0.5;

/// One aggregated draft of an entire session's trajectory, ready to be
/// embedded (on `task`) and persisted via `insert_episodic_trajectory`.
/// Produced by [`build_trajectory`] — pure, no I/O.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TrajectoryDraft {
    pub session_id: String,
    pub agent_id: String,
    pub task: String,
    pub tool_calls: Vec<String>,
    pub errors_encountered: Vec<String>,
    pub resolution: Option<String>,
    pub success_score: f64,
}

/// Aggregates a multi-agent session snapshot into a single [`TrajectoryDraft`].
///
/// Pure and deterministic — no LLM, no DB. Returns `None` (SKIP write) when
/// there is no non-empty `Role::User` message anywhere in `history` (NO
/// fallback to another role — `task` is NOT NULL and an empty/placeholder
/// embed would contaminate KNN).
///
/// # Algorithm
/// 1. Flatten `history` into one sequence: agents in `Vec` order, messages in
///    their own order within each agent (this flattened order — NOT timestamp
///    order — is what "posterior a" / "último mensaje" mean below).
/// 2. `task` = content of the earliest (by `timestamp_ms`, tie-broken by
///    flattened position) non-empty `Role::User` message, truncated to
///    [`TASK_CHAR_CAP`] chars. `agent_id` = the agent that owns that message.
/// 3. `tool_calls` = names of every `ToolCall` on every `Assistant` message,
///    in flattened order (across ALL agents — this is what the trajectory
///    miner consumes as-is).
/// 4. `errors_encountered` = content of every `Role::Tool` message with
///    `is_error == Some(true)`, in flattened order.
/// 5. `unresolved` = count of those errors whose flattened index is AFTER the
///    last `Assistant` message's index (if there is no `Assistant` at all,
///    every error counts as unresolved).
/// 6. `ended_clean` = the very last message in the flattened sequence is
///    `Role::Assistant`.
/// 7. `success_score = (BASE_SCORE + completion_bonus - unresolved_penalty).clamp(0.0, 1.0)`.
pub(crate) fn build_trajectory(
    session_id: &str,
    history: &[(String, Vec<ChatMessage>)],
) -> Option<TrajectoryDraft> {
    let flattened: Vec<(&str, &ChatMessage)> = history
        .iter()
        .flat_map(|(agent_id, msgs)| msgs.iter().map(move |m| (agent_id.as_str(), m)))
        .collect();

    if flattened.is_empty() {
        return None;
    }

    // Earliest non-empty Role::User by timestamp_ms, tied
    // broken by flattened (Vec/message) position. None found → SKIP (None).
    let (_, (agent_id, task_msg)) = flattened
        .iter()
        .enumerate()
        .filter(|(_, (_, m))| m.role == Role::User && !m.content.trim().is_empty())
        .min_by_key(|(idx, (_, m))| (m.timestamp_ms, *idx))?;

    let agent_id = agent_id.to_string();
    let task: String = task_msg.content.chars().take(TASK_CHAR_CAP).collect();

    // tool_calls: names only, flattened across all agents in Vec/message order.
    let mut tool_calls: Vec<String> = Vec::new();
    for (_, m) in &flattened {
        if m.role == Role::Assistant {
            if let Some(calls) = &m.tool_calls {
                tool_calls.extend(calls.iter().map(|c| c.function.name.clone()));
            }
        }
    }

    // errors_encountered: Role::Tool && is_error==Some(true), flattened order.
    let error_indices: Vec<usize> = flattened
        .iter()
        .enumerate()
        .filter(|(_, (_, m))| m.role == Role::Tool && m.is_error == Some(true))
        .map(|(idx, _)| idx)
        .collect();
    let errors_encountered: Vec<String> =
        error_indices.iter().map(|&idx| flattened[idx].1.content.clone()).collect();

    // Last Assistant message in flattened order — anchors "unresolved" and
    // "resolution"; absence means every error is unresolved (no recovery point).
    let last_assistant_idx = flattened.iter().rposition(|(_, m)| m.role == Role::Assistant);

    let unresolved = match last_assistant_idx {
        Some(last) => error_indices.iter().filter(|&&idx| idx > last).count(),
        None => error_indices.len(),
    };

    let ended_clean = flattened.last().map(|(_, m)| m.role == Role::Assistant).unwrap_or(false);
    let resolution = last_assistant_idx.map(|idx| flattened[idx].1.content.clone());

    let completion_bonus = if ended_clean { COMPLETION_BONUS } else { 0.0 };
    let unresolved_penalty =
        UNRESOLVED_PENALTY_COEFF * (unresolved as f64 / tool_calls.len().max(1) as f64);
    let success_score = (BASE_SCORE + completion_bonus - unresolved_penalty).clamp(0.0, 1.0);

    Some(TrajectoryDraft {
        session_id: session_id.to_string(),
        agent_id,
        task,
        tool_calls,
        errors_encountered,
        resolution,
        success_score,
    })
}

/// Testable core of the episodic-trajectory write pipeline — takes explicit
/// dependencies, so it can be unit tested without any host. Mirrors
/// `extractor::run_extraction_core`.
///
/// Never panics: a missing embedder, an unbuildable draft (no `Role::User`),
/// an embed failure, or an insert failure are all `log::`'d and the function
/// returns early — no I/O happens beyond what's needed.
pub(crate) async fn run_episodic_writer_core(
    session_id: String,
    history: Vec<(String, Vec<ChatMessage>)>,
    store: &MemoryStore,
    embedder: Option<Arc<dyn Embedder>>,
    project_by_agent: &HashMap<String, Option<String>>,
    now_ms: u64,
) -> Option<(i64, TrajectoryDraft)> {
    if history.iter().all(|(_, m)| m.is_empty()) {
        return None;
    }

    // Check the embedder ONCE, BEFORE build_trajectory/insert — deliberately
    // ordered that way: it saves the whole aggregation when the embedder is not
    // ready, and a trajectory cannot be persisted without its vector anyway.
    // Same ordering as extractor.rs and worker.rs.
    let Some(embedder) = embedder else {
        log::debug!(
            "episodic_writer: embedder not ready; skipping trajectory write for session={session_id}"
        );
        return None;
    };

    let Some(draft) = build_trajectory(&session_id, &history) else {
        // No non-empty Role::User anywhere in history — SKIP write (no
        // fallback: an empty/placeholder task would violate
        // NOT NULL and contaminate KNN with a meaningless embed).
        return None;
    };

    // Passage: a trajectory's task text is stored content, retrieved later by
    // a Query-role search — not itself a query.
    let embedding = match embedder.embed_with_role(&draft.task, EmbedRole::Passage).await {
        Ok(v) => v,
        Err(e) => {
            log::warn!("episodic_writer: embed failed for session={session_id}: {e}");
            return None;
        }
    };

    // project_id resolution mirrors extractor.rs exactly: project_by_agent
    // holds the raw project PATH (or None); derive_project_id turns it into
    // the stable project identifier.
    let project_id = project_by_agent
        .get(&draft.agent_id)
        .cloned()
        .flatten()
        .filter(|s| !s.is_empty())
        .map(|s| crate::memory::identity::derive_project_id(std::path::Path::new(&s)));

    // Cap errors_encountered PER-ITEM to ERROR_ITEM_CHAR_CAP chars BEFORE
    // serializing — build_trajectory deliberately does not cap, so the pure
    // aggregation keeps the full error text and only the DB boundary trims it.
    let capped_errors: Vec<String> = draft
        .errors_encountered
        .iter()
        .map(|e| e.chars().take(ERROR_ITEM_CHAR_CAP).collect())
        .collect();

    let tool_calls_json =
        serde_json::to_string(&draft.tool_calls).expect("Vec<String> serialises infallibly");
    let errors_json =
        serde_json::to_string(&capped_errors).expect("Vec<String> serialises infallibly");

    let record = NewEpisodicTrajectoryRecord {
        scope: "agent",
        agent_id: Some(draft.agent_id.clone()),
        project_id,
        session_id: draft.session_id.clone(),
        task: draft.task.clone(),
        tool_calls_json,
        errors_json,
        resolution: draft.resolution.clone(),
        success_score: draft.success_score,
        created_by_type: CreatedByType::System,
        created_by_ref: draft.agent_id.clone(),
        importance: 5.0,
        created_at_ms: now_ms,
    };

    match store.insert_episodic_trajectory(record, embedding).await {
        Ok(rowid) => Some((rowid, draft)),
        Err(e) => {
            log::warn!(
                "episodic_writer: insert_episodic_trajectory failed for session={session_id}: {e}"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::miner::GOLDEN_SCORE_THRESHOLD;
    use crate::models::chat::{ToolCall, ToolFunction};

    fn msg(role: Role, content: &str, ts: u64) -> ChatMessage {
        ChatMessage {
            id: "t".to_string(),
            role,
            content: content.to_string(),
            tool_calls: None,
            timestamp_ms: ts,
            is_error: None,
        }
    }

    fn assistant_with_tools(content: &str, ts: u64, tool_names: &[&str]) -> ChatMessage {
        let calls = tool_names
            .iter()
            .map(|n| ToolCall {
                id: format!("tool-{n}"),
                function: ToolFunction { name: n.to_string(), arguments: serde_json::json!({}) },
            })
            .collect();
        ChatMessage {
            id: "t".to_string(),
            role: Role::Assistant,
            content: content.to_string(),
            tool_calls: Some(calls),
            timestamp_ms: ts,
            is_error: None,
        }
    }

    fn tool_result(content: &str, ts: u64, is_error: bool) -> ChatMessage {
        ChatMessage {
            id: "t".to_string(),
            role: Role::Tool,
            content: content.to_string(),
            tool_calls: None,
            timestamp_ms: ts,
            is_error: Some(is_error),
        }
    }

    /// The score never lands between 0.4 and 0.9, so the miner's 0.85 gate is a
    /// yes/no on "did this end on an assistant message" and nothing finer.
    ///
    /// Swept rather than reasoned about in one case: the argument is that
    /// `unresolved` counts errors AFTER the last assistant message, so ending on
    /// one forces the penalty to zero and pins the score at exactly 0.9. This
    /// enumerates the shapes that argument has to survive — varying tool count,
    /// error count, error position and how the session ends — and asserts the
    /// gap stays empty. Any coefficient change that opens a middle band shows up
    /// here, where the doc comment above claims it cannot happen.
    #[test]
    fn success_score_lands_only_in_two_disjoint_bands() {
        for tools in 0..4usize {
            for errors in 0..4usize {
                for ends_clean in [true, false] {
                    for errors_trailing in [true, false] {
                        let mut msgs = vec![msg(Role::User, "task", 0)];
                        let mut ts = 1u64;

                        if !errors_trailing {
                            for i in 0..errors {
                                msgs.push(tool_result(&format!("early err {i}"), ts, true));
                                ts += 1;
                            }
                        }
                        for i in 0..tools {
                            msgs.push(assistant_with_tools("", ts, &[&format!("tool_{i}")]));
                            ts += 1;
                        }
                        if errors_trailing {
                            for i in 0..errors {
                                msgs.push(tool_result(&format!("late err {i}"), ts, true));
                                ts += 1;
                            }
                        }
                        if ends_clean {
                            msgs.push(msg(Role::Assistant, "done", ts));
                        } else {
                            msgs.push(tool_result("final failure", ts, true));
                        }

                        let draft = build_trajectory("sweep", &vec![("a".to_string(), msgs)])
                            .expect("a User message is always present");
                        let s = draft.success_score;

                        assert!(
                            s <= 0.4 + 1e-9 || (s - 0.9).abs() < 1e-9,
                            "score {s} fell in the unreachable band \
                             (tools={tools} errors={errors} ends_clean={ends_clean} \
                              errors_trailing={errors_trailing})"
                        );
                        assert_eq!(
                            s > GOLDEN_SCORE_THRESHOLD,
                            ends_clean,
                            "the golden gate must track ONLY how the session ended \
                             (tools={tools} errors={errors} errors_trailing={errors_trailing})"
                        );
                    }
                }
            }
        }
    }

    // ── Sesión limpia: 2 tool_calls, 0 errores, termina Assistant ──

    #[test]
    fn clean_session_two_tool_calls_no_errors_yields_score_0_9() {
        let history = vec![(
            "agent-a".to_string(),
            vec![
                msg(Role::User, "fix the bug", 0),
                assistant_with_tools("", 1, &["read_file"]),
                tool_result("file contents", 2, false),
                assistant_with_tools("", 3, &["write_file"]),
                tool_result("written", 4, false),
                msg(Role::Assistant, "done", 5),
            ],
        )];

        let draft = build_trajectory("sess-1", &history).expect("expected Some draft");
        assert_eq!(draft.tool_calls, vec!["read_file", "write_file"]);
        assert!(draft.errors_encountered.is_empty());
        assert_eq!(draft.success_score, 0.9, "ceiling is 0.9, not 1.0");
    }

    // ── Error Tool resuelto antes del último Assistant no-error ──

    #[test]
    fn resolved_error_before_last_assistant_does_not_count_as_unresolved() {
        let history = vec![(
            "agent-a".to_string(),
            vec![
                msg(Role::User, "fix the bug", 0),
                assistant_with_tools("", 1, &["read_file"]),
                tool_result("boom", 2, true),
                assistant_with_tools("", 3, &["write_file"]),
                tool_result("ok", 4, false),
                msg(Role::Assistant, "resolved and done", 5),
            ],
        )];

        let draft = build_trajectory("sess-2", &history).expect("expected Some draft");
        assert_eq!(draft.errors_encountered, vec!["boom".to_string()]);
        assert_eq!(draft.success_score, 0.9, "resolved error must not penalize");
    }

    // ── Error Tool posterior al último Assistant (termina en error) ──

    #[test]
    fn unresolved_trailing_error_penalizes_score_strictly_below_s1_s2() {
        let history = vec![(
            "agent-a".to_string(),
            vec![
                msg(Role::User, "fix the bug", 0),
                assistant_with_tools("", 1, &["read_file"]),
                tool_result("ok", 2, false),
                assistant_with_tools("", 3, &["write_file"]),
                tool_result("boom", 4, true),
            ],
        )];

        let draft = build_trajectory("sess-3", &history).expect("expected Some draft");
        assert!(
            (draft.success_score - 0.15).abs() < 1e-9,
            "1 unresolved / 2 tool_calls => 0.4 - 0.25, got {}",
            draft.success_score
        );
        assert!(draft.success_score < 0.9, "an unresolved trailing error must score strictly below a clean session");
    }

    // ── Solo chat, 0 tool_calls, termina Assistant ──

    #[test]
    fn chat_only_no_tool_calls_ended_clean_yields_score_0_9() {
        let history = vec![(
            "agent-a".to_string(),
            vec![msg(Role::User, "hi", 0), msg(Role::Assistant, "hello", 1)],
        )];

        let draft = build_trajectory("sess-4", &history).expect("expected Some draft");
        assert!(draft.tool_calls.is_empty());
        assert_eq!(draft.success_score, 0.9);
    }

    // ── Sin ningún Role::User → None, sin panic ──

    #[test]
    fn no_user_message_anywhere_yields_none() {
        let history = vec![(
            "agent-a".to_string(),
            vec![msg(Role::Assistant, "hi", 0), tool_result("x", 1, false)],
        )];

        assert!(build_trajectory("sess-5", &history).is_none());
    }

    // ── History totalmente vacío → None ──

    #[test]
    fn totally_empty_history_yields_none() {
        let history: Vec<(String, Vec<ChatMessage>)> = vec![];
        assert!(build_trajectory("sess-6", &history).is_none());
    }

    #[test]
    fn history_with_agent_but_no_messages_yields_none() {
        let history: Vec<(String, Vec<ChatMessage>)> = vec![("agent-a".to_string(), vec![])];
        assert!(build_trajectory("sess-6b", &history).is_none());
    }

    // ── Agregación multi-agente: 1 solo draft; agent_id = dueño ──
    // ── del User más temprano; tool_calls aplanados en orden del Vec ─────────

    #[test]
    fn multi_agent_aggregates_into_one_draft_owned_by_earliest_user() {
        let history = vec![
            (
                "agent-a".to_string(),
                vec![msg(Role::User, "task-a", 10), assistant_with_tools("", 11, &["tool_a1", "tool_a2"])],
            ),
            (
                "agent-b".to_string(),
                vec![msg(Role::User, "task-b", 5), assistant_with_tools("", 6, &["tool_b"])],
            ),
        ];

        let draft = build_trajectory("sess-7", &history).expect("expected Some draft");
        assert_eq!(draft.agent_id, "agent-b", "earliest User (ts=5) belongs to agent-b");
        assert_eq!(draft.task, "task-b");
        assert_eq!(
            draft.tool_calls,
            vec!["tool_a1", "tool_a2", "tool_b"],
            "tool_calls must flatten in Vec order (agent-a then agent-b), not timestamp order"
        );
    }

    // ── Task truncado a TASK_CHAR_CAP=512 ──

    #[test]
    fn task_longer_than_cap_is_truncated_to_512_chars() {
        let long_task = "x".repeat(600);
        let history = vec![(
            "agent-a".to_string(),
            vec![msg(Role::User, &long_task, 0), msg(Role::Assistant, "ok", 1)],
        )];

        let draft = build_trajectory("sess-8", &history).expect("expected Some draft");
        assert_eq!(draft.task.chars().count(), 512);
        assert_eq!(draft.task, "x".repeat(512));
    }

    // ── Worked examples pinning the success_score coefficients ───────────────

    #[test]
    fn coefficients_have_pinned_values() {
        assert_eq!(BASE_SCORE, 0.4);
        assert_eq!(COMPLETION_BONUS, 0.5);
        assert_eq!(UNRESOLVED_PENALTY_COEFF, 0.5);
    }

    #[test]
    fn worked_example_limpia_yields_0_9() {
        let history = vec![(
            "a".to_string(),
            vec![
                msg(Role::User, "t", 0),
                assistant_with_tools("", 1, &["c1"]),
                tool_result("ok", 2, false),
                assistant_with_tools("", 3, &["c2"]),
                tool_result("ok", 4, false),
                msg(Role::Assistant, "done", 5),
            ],
        )];
        assert_eq!(build_trajectory("s", &history).unwrap().success_score, 0.9);
    }

    #[test]
    fn worked_example_recuperados_yields_0_9() {
        let history = vec![(
            "a".to_string(),
            vec![
                msg(Role::User, "t", 0),
                assistant_with_tools("", 1, &["c1"]),
                tool_result("boom", 2, true),
                assistant_with_tools("", 3, &["c2"]),
                tool_result("ok", 4, false),
                msg(Role::Assistant, "done", 5),
            ],
        )];
        assert_eq!(build_trajectory("s", &history).unwrap().success_score, 0.9);
    }

    #[test]
    fn worked_example_solo_chat_yields_0_9() {
        let history =
            vec![("a".to_string(), vec![msg(Role::User, "t", 0), msg(Role::Assistant, "done", 1)])];
        assert_eq!(build_trajectory("s", &history).unwrap().success_score, 0.9);
    }

    #[test]
    fn worked_example_no_resuelto_yields_0_15() {
        let history = vec![(
            "a".to_string(),
            vec![
                msg(Role::User, "t", 0),
                assistant_with_tools("", 1, &["c1"]),
                tool_result("ok", 2, false),
                assistant_with_tools("", 3, &["c2"]),
                tool_result("boom", 4, true),
            ],
        )];
        let score = build_trajectory("s", &history).unwrap().success_score;
        assert!((score - 0.15).abs() < 1e-9, "expected ~0.15, got {score}");
    }

    #[test]
    fn worked_example_fallo_total_yields_0_0() {
        let history = vec![(
            "a".to_string(),
            vec![msg(Role::User, "t", 0), assistant_with_tools("", 1, &["c1"]), tool_result("boom", 2, true)],
        )];
        assert_eq!(build_trajectory("s", &history).unwrap().success_score, 0.0);
    }

    // ── run_episodic_writer_core ─────────────────────────────────────────────

    use crate::memory::embedder::{EmbedError, FakeEmbedder};

    async fn count_episodic(store: &MemoryStore) -> i64 {
        store
            .conn
            .call(|c| c.query_row("SELECT COUNT(*) FROM episodic_trajectory", [], |r| r.get(0)))
            .await
            .unwrap()
    }

    /// Records every `(role, text)` pair handed to `embed_with_role`.
    struct RecordingEmbedder {
        calls: std::sync::Mutex<Vec<(EmbedRole, String)>>,
    }
    #[async_trait::async_trait]
    impl Embedder for RecordingEmbedder {
        async fn embed_with_role(&self, text: &str, role: EmbedRole) -> Result<Vec<f32>, EmbedError> {
            self.calls.lock().unwrap().push((role, text.to_string()));
            Ok(vec![0.1_f32; 384])
        }
        fn dimensions(&self) -> usize {
            384
        }
        fn model_id(&self) -> &str {
            "recording"
        }
        fn query_prefix(&self) -> &str {
            ""
        }
        fn passage_prefix(&self) -> &str {
            ""
        }
    }

    /// `run_episodic_writer_core` must embed the trajectory's task with role
    /// `Passage` — stored content, not a search query.
    #[tokio::test]
    async fn run_episodic_writer_core_embeds_task_with_role_passage() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let recording = Arc::new(RecordingEmbedder { calls: std::sync::Mutex::new(Vec::new()) });
        let embedder: Arc<dyn Embedder> = recording.clone();
        let history = vec![(
            "agent-a".to_string(),
            vec![msg(Role::User, "fix the bug", 0), msg(Role::Assistant, "done", 1)],
        )];
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        run_episodic_writer_core("sess-role".to_string(), history, &store, Some(embedder), &project_by_agent, 0)
            .await;

        let calls = recording.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "expected exactly one embed call, got {calls:?}");
        assert_eq!(calls[0].0, EmbedRole::Passage);
        assert_eq!(calls[0].1, "fix the bug");
    }

    // Embedder=None → 0 rows, return BEFORE build/insert.
    #[tokio::test]
    async fn missing_embedder_yields_zero_rows_returns_before_build() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let history = vec![(
            "agent-a".to_string(),
            vec![msg(Role::User, "fix the bug", 0), msg(Role::Assistant, "done", 1)],
        )];
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        run_episodic_writer_core("sess-w1".to_string(), history, &store, None, &project_by_agent, 0)
            .await;

        assert_eq!(count_episodic(&store).await, 0, "missing embedder must skip the write entirely");
    }

    // History vacío/todos vacíos → 0 filas, return temprano.
    #[tokio::test]
    async fn all_empty_history_yields_zero_rows() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let history: Vec<(String, Vec<ChatMessage>)> = vec![("agent-a".to_string(), vec![])];
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        run_episodic_writer_core(
            "sess-w2".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            0,
        )
        .await;

        assert_eq!(count_episodic(&store).await, 0, "all-empty history must insert zero rows");
    }

    // Build_trajectory devuelve None (sin User) → 0 filas insertadas.
    #[tokio::test]
    async fn no_user_message_yields_zero_rows() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let history = vec![("agent-a".to_string(), vec![msg(Role::Assistant, "hi", 0)])];
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        run_episodic_writer_core(
            "sess-w3".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            0,
        )
        .await;

        assert_eq!(count_episodic(&store).await, 0, "no Role::User must skip the write (None draft)");
    }

    // Caso feliz — embed(task), record armado, insert_episodic_trajectory
    // invocado, 1 fila con los campos esperados.
    #[tokio::test]
    async fn happy_path_inserts_one_row_with_expected_fields() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let history = vec![(
            "agent-a".to_string(),
            vec![
                msg(Role::User, "fix the bug", 0),
                assistant_with_tools("", 1, &["read_file"]),
                tool_result("ok", 2, false),
                msg(Role::Assistant, "done", 3),
            ],
        )];
        let mut project_by_agent: HashMap<String, Option<String>> = HashMap::new();
        project_by_agent.insert("agent-a".to_string(), Some("/some/project/path".to_string()));

        run_episodic_writer_core(
            "sess-w4".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            123_456,
        )
        .await;

        assert_eq!(count_episodic(&store).await, 1, "expected exactly 1 row inserted");

        let (task, tool_calls_json, errors_json, success_score, project_id): (
            String,
            String,
            String,
            f64,
            Option<String>,
        ) = store
            .conn
            .call(|c| {
                c.query_row(
                    "SELECT task, tool_calls, errors_encountered, success_score, project_id \
                     FROM episodic_trajectory",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
            })
            .await
            .unwrap();

        assert_eq!(task, "fix the bug");
        assert_eq!(tool_calls_json, "[\"read_file\"]");
        assert_eq!(errors_json, "[]");
        assert!((success_score - 0.9).abs() < 1e-9, "expected 0.9, got {success_score}");
        assert!(project_id.is_some(), "project_id must be derived from project_by_agent");
    }

    // errors_encountered is capped PER-ITEM to 500 chars before
    // being serialized into errors_json (done here, NOT in build_trajectory).
    #[tokio::test]
    async fn errors_encountered_capped_to_500_chars_per_item_before_serializing() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let long_error = "e".repeat(600);
        let history = vec![(
            "agent-a".to_string(),
            vec![
                msg(Role::User, "fix the bug", 0),
                assistant_with_tools("", 1, &["read_file"]),
                tool_result(&long_error, 2, true),
            ],
        )];
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        run_episodic_writer_core(
            "sess-w5".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            0,
        )
        .await;

        let errors_json: String = store
            .conn
            .call(|c| c.query_row("SELECT errors_encountered FROM episodic_trajectory", [], |r| r.get(0)))
            .await
            .unwrap();
        let parsed: Vec<String> = serde_json::from_str(&errors_json).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].chars().count(), 500, "each error item must be capped to 500 chars");
    }

    // ── Option<(i64, TrajectoryDraft)> return shape ──────────────────────────

    // core happy path: Some((rowid, draft)) with rowid > 0 and draft fields intact.
    #[tokio::test]
    async fn core_happy_path_returns_some_rowid_and_draft() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let history = vec![(
            "agent-a".to_string(),
            vec![msg(Role::User, "fix the bug", 0), msg(Role::Assistant, "done", 1)],
        )];
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let result = run_episodic_writer_core(
            "sess-core-happy".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            0,
        )
        .await;

        let (rowid, draft) = result.expect("expected Some((rowid, draft)) on happy path");
        assert!(rowid > 0, "expected positive rowid, got {rowid}");
        assert_eq!(draft.task, "fix the bug");
        assert!((draft.success_score - 0.9).abs() < 1e-9);
    }

    // core missing embedder: None, and no row inserted.
    #[tokio::test]
    async fn core_missing_embedder_returns_none() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let history = vec![(
            "agent-a".to_string(),
            vec![msg(Role::User, "fix the bug", 0), msg(Role::Assistant, "done", 1)],
        )];
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let result = run_episodic_writer_core(
            "sess-core-none-embedder".to_string(),
            history,
            &store,
            None,
            &project_by_agent,
            0,
        )
        .await;

        assert!(result.is_none(), "missing embedder must yield None");
        assert_eq!(count_episodic(&store).await, 0);
    }

    // core duplicate session_id: 2nd call violates UNIQUE -> None, no panic, 1 row total.
    #[tokio::test]
    async fn core_duplicate_session_id_returns_none() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let make_history = || {
            vec![(
                "agent-a".to_string(),
                vec![msg(Role::User, "fix the bug", 0), msg(Role::Assistant, "done", 1)],
            )]
        };
        let embedder1: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let embedder2: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let first = run_episodic_writer_core(
            "sess-core-dup".to_string(),
            make_history(),
            &store,
            Some(embedder1),
            &project_by_agent,
            0,
        )
        .await;
        assert!(first.is_some(), "first insert must succeed");

        let second = run_episodic_writer_core(
            "sess-core-dup".to_string(),
            make_history(),
            &store,
            Some(embedder2),
            &project_by_agent,
            0,
        )
        .await;

        assert!(second.is_none(), "duplicate session_id must yield None, not panic");
        assert_eq!(count_episodic(&store).await, 1, "only the first insert must persist");
    }

}
