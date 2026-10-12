//! Trajectory miner — pipeline + Golden Trajectory gating.
//!
//! Run on session close. Always delegates to `episodic_writer::run_episodic_writer_core` to INSERT the
//! episodic trajectory, then — only if `success_score` clears
//! [`GOLDEN_SCORE_THRESHOLD`] — extracts tool-call sequences and mines
//! [`MinedSkillCandidate`]s, promoting recurring ones into `procedural_skill`
//! via `promote_to_procedural_skill`.
//!
//! # Design constraints
//! - The gate is `draft.success_score > GOLDEN_SCORE_THRESHOLD` (strict `>`).
//!   Structurally, this is a BINARY gate: `ended_clean` sessions always score
//!   exactly `0.9` and pass; non-clean sessions always score `<= 0.4` and
//!   never pass — the band `(0.85, 0.9)` is unreachable with real history.
//! - Naming is heuristic-first: the candidate is built with a deterministic
//!   heuristic `skill_name`/`trigger` BEFORE any LLM call, and a single LLM
//!   naming attempt only ever OVERRIDES it — timeout/error/parse-fail/absent
//!   provider all leave the heuristic in place. The candidate is never
//!   dropped or emptied because of the namer.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::memory::db::MemoryStore;
use crate::memory::embedder::{EmbedRole, Embedder};
use crate::memory::episodic_writer::{run_episodic_writer_core, TrajectoryDraft};
use crate::models::chat::Role;
use crate::models::{ChatMessage, ChatRequest, ChatStreamResult};
use crate::providers::{AiProvider, StreamSender};

/// Golden Trajectory gate threshold. The gate uses strict `>` —
/// `success_score == 0.85` exactly does NOT pass.
///
/// Visible to the crate so `episodic_writer` — which produces the scores this
/// compares against — can assert the two agree. Its
/// `success_score_lands_only_in_two_disjoint_bands` shows the reachable scores
/// are `[0.0, 0.4]` and exactly `0.9`, so this value's real job is to sit in
/// the empty gap between them; a threshold moved into either band would change
/// the gate from "ended on an assistant message" into something the score has
/// no resolution to answer.
pub(crate) const GOLDEN_SCORE_THRESHOLD: f64 = 0.85;

/// Maximum wall-clock time allowed for the namer LLM call (same budget as `EXTRACTOR_TIMEOUT`).
pub const NAMER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Root-cause category vocabulary. Closed to exactly these 3 snake_case
/// values; no `unclassified` sentinel is ever
/// written.
const RCA_MISSING_CONTEXT: &str = "missing_context";
const RCA_MODEL_LIMITATION: &str = "model_limitation";
const RCA_PROCEDURAL_ERROR: &str = "procedural_error";

/// Per-item character cap applied inside `build_rca_prompt`. A SEPARATE
/// const from episodic_writer's `ERROR_ITEM_CHAR_CAP` — same value, parallel
/// frontier, per the codebase's per-layer cap doctrine (each frontier
/// applies its own cap).
const RCA_ERROR_ITEM_CHAR_CAP: usize = 500;

/// Maximum number of error items included in the RCA prompt.
/// Only the FIRST `RCA_MAX_ERROR_ITEMS` in flattened/chronological order are
/// kept; items beyond the cap are dropped, not summarized.
const RCA_MAX_ERROR_ITEMS: usize = 20;

/// Cap (in tool-sequence steps) above which `heuristic_skill_name` switches
/// from the full `→`-joined name to the abbreviated "first 2 + last + count"
/// form. Fixed at 4: abbreviating only shortens the string once it elides
/// >= 2 names, i.e. from `n >= 5`.
const MAX_SKILL_NAME_STEPS: usize = 4;

/// One mined skill candidate — the input to `promote_to_procedural_skill`,
/// which upserts it into `procedural_skill`.
#[derive(Debug, Clone, PartialEq)]
pub struct MinedSkillCandidate {
    pub source_trajectory_id: i64,
    pub session_id: String,
    pub agent_id: String,
    pub task: String,
    pub normalized_sequence: Vec<String>,
    pub skill_name: String,
    pub trigger: String,
}

/// Collapses every run of CONSECUTIVE identical names to a single occurrence.
/// Pure, no I/O. Non-adjacent repeats are preserved.
pub(crate) fn normalize_tool_sequence(seq: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(seq.len());
    for name in seq {
        if out.last() != Some(name) {
            out.push(name.clone());
        }
    }
    out
}

/// At most 1 sequence per trajectory: the FULL `tool_calls` sequence,
/// normalized, or `vec![]` if `tool_calls` is empty. No n-grams/sub-sequences.
pub(crate) fn extract_tool_sequences(draft: &TrajectoryDraft) -> Vec<Vec<String>> {
    let normalized = normalize_tool_sequence(&draft.tool_calls);
    if normalized.is_empty() {
        Vec::new()
    } else {
        vec![normalized]
    }
}

/// Recurrence threshold: a candidate promotes iff
/// `1 (actual) + filter_recurring(...).len() >= RECURRING_THRESHOLD`.
const RECURRING_THRESHOLD: usize = 2;

/// Bound on the number of prior Golden trajectories fetched per candidate.
const PRIOR_TRAJECTORIES_LIMIT: i64 = 500;

/// Single producer of the text embedded for a procedural skill row — trigger
/// FIRST, then skill_name. Shared between the live write path
/// (below) and `reembed_all`'s backfill: the two MUST build the identical
/// string from the same `(trigger, skill_name)` pair, or the backfill
/// silently re-infers a vector for a different string than the one the row
/// was originally written against, changing what the row means without
/// anyone deciding that.
pub(crate) fn skill_embedding_text(trigger: &str, skill_name: &str) -> String {
    format!("{trigger} {skill_name}")
}

/// Single producer of the dedup key / `tool_sequence` column value — used on
/// BOTH the SELECT (dedup check) and INSERT/UPDATE paths. Inline
/// `serde_json::to_string` calls for this purpose are prohibited outside
/// this helper.
pub(crate) fn serialize_tool_sequence(seq: &[String]) -> String {
    serde_json::to_string(seq).expect("Vec<String> serialises infallibly")
}

/// `episodic_trajectory.id`s of PRIOR entries whose sequence equals
/// `candidate_seq` by exact element-wise match. Pure, no I/O — no
/// fuzzy/similarity matching.
pub(crate) fn filter_recurring(candidate_seq: &[String], prior: &[(i64, Vec<String>)]) -> Vec<i64> {
    prior.iter().filter(|(_, seq)| seq == candidate_seq).map(|(id, _)| *id).collect()
}

/// Deterministic heuristic name for a (non-empty) normalized tool sequence.
/// Invariant: `extract_tool_sequences` never produces an empty sequence, but
/// this function is still defensive for `seq.is_empty()` (returns `""`).
fn heuristic_skill_name(seq: &[String]) -> String {
    let n = seq.len();
    if n == 0 {
        return String::new();
    }
    if n <= MAX_SKILL_NAME_STEPS {
        seq.join("→")
    } else {
        format!("{}→{}→…→{} ({} steps)", seq[0], seq[1], seq[n - 1], n)
    }
}

fn heuristic_trigger(task: &str) -> String {
    task.to_string()
}

/// System prompt for the namer LLM call (molde `EXTRACT_SYS`).
const NAMER_SYS: &str = "You name a reusable coding skill mined from a successful agent trajectory.\n\
    Return ONLY a JSON object (no prose, no markdown fences):\n\
    {\"skill_name\": <string>, \"trigger\": <string>}\n\
    - skill_name: a short imperative name for the reusable procedure (e.g. \"Fix failing test\", \"Add REST endpoint\"). 2-6 words. NOT a list of tool names.\n\
    - trigger: one phrase describing WHEN to apply this skill (the situation/goal), e.g. \"when a unit test fails\".\n\
    Base both strictly on the task and the tool sequence given. Return your single best guess. Never return prose outside the JSON object.";

/// System prompt for the RCA classifier LLM call. Closed vocabulary of
/// exactly 3 categories — the model must never return anything else.
const RCA_SYS: &str = "You classify why an AI coding agent's trajectory failed.\n\
    Return ONLY a JSON object (no prose, no markdown fences):\n\
    {\"root_cause\": <one of: \"missing_context\", \"model_limitation\", \"procedural_error\">}\n\
    - missing_context: the agent lacked information it needed (files, docs, prior state) to succeed.\n\
    - model_limitation: the agent had the information but reasoned or generated incorrectly.\n\
    - procedural_error: the agent skipped or misordered a required step/tool call.\n\
    Base your classification strictly on the task, the errors encountered, and the resolution (if given). Never return a value outside those three.";

/// Build the user-turn content for the namer LLM call.
fn build_namer_prompt(task: &str, sequence: &[String], resolution: Option<&str>) -> String {
    let mut out = format!("TASK: {task}\nTOOL SEQUENCE: {}\n", sequence.join(" → "));
    if let Some(r) = resolution {
        out.push_str(&format!("RESOLUTION: {r}\n"));
    }
    out
}

/// Parse the namer LLM's raw text response into `(skill_name, trigger)`.
///
/// Defensive by construction (molde `parse_extraction`, but object not
/// array): tolerates markdown fences by slicing from the first `{` to the
/// last `}`; any malformed JSON, missing/empty field after trim → `None`
/// (heuristic persists at the call site).
pub(crate) fn parse_skill_naming(txt: &str) -> Option<(String, String)> {
    let (Some(start), Some(end)) = (txt.find('{'), txt.rfind('}')) else {
        log::warn!("miner: no JSON object found in namer response");
        return None;
    };
    if end < start {
        log::warn!("miner: malformed braces in namer response");
        return None;
    }
    let value: serde_json::Value = match serde_json::from_str(&txt[start..=end]) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("miner: failed to parse namer JSON: {e}");
            return None;
        }
    };
    let skill_name = value.get("skill_name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let trigger = value.get("trigger").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if skill_name.is_empty() || trigger.is_empty() {
        log::warn!("miner: namer produced empty skill_name/trigger; keeping heuristic");
        return None;
    }
    Some((skill_name, trigger))
}

/// Build the user-turn content for the RCA classifier LLM call.
///
/// Applies the RCA-prompt-frontier caps: at most [`RCA_MAX_ERROR_ITEMS`]
/// errors, each truncated to
/// [`RCA_ERROR_ITEM_CHAR_CAP`] chars — items beyond the cap are dropped, not
/// summarized. `resolution` stays uncapped (matches the `llm_name`
/// precedent). Pure, no I/O.
pub(crate) fn build_rca_prompt(task: &str, errors: &[String], resolution: Option<&str>) -> String {
    let mut out = format!("TASK: {task}\nERRORS:\n");
    for e in errors.iter().take(RCA_MAX_ERROR_ITEMS) {
        let capped: String = e.chars().take(RCA_ERROR_ITEM_CHAR_CAP).collect();
        out.push_str(&format!("- {capped}\n"));
    }
    if let Some(r) = resolution {
        out.push_str(&format!("RESOLUTION: {r}\n"));
    }
    out
}

/// Parse the RCA LLM's raw text response into a closed-vocabulary category.
///
/// Mirror of `parse_skill_naming` (slice first `{`..last `}`, tolerate
/// markdown fences, malformed JSON/missing/empty field → `None`), with one
/// deliberate divergence: the parsed value is lowercased before the
/// exact-match check, so the closed vocabulary tolerates model casing (e.g.
/// `"Missing_Context"`). Any value outside the 3-value vocabulary → `None`.
pub(crate) fn parse_root_cause(txt: &str) -> Option<String> {
    let (Some(start), Some(end)) = (txt.find('{'), txt.rfind('}')) else {
        log::warn!("miner: no JSON object found in RCA response");
        return None;
    };
    if end < start {
        log::warn!("miner: malformed braces in RCA response");
        return None;
    }
    let value: serde_json::Value = match serde_json::from_str(&txt[start..=end]) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("miner: failed to parse RCA JSON: {e}");
            return None;
        }
    };
    let root_cause = value.get("root_cause").and_then(|v| v.as_str()).unwrap_or("").trim().to_lowercase();
    match root_cause.as_str() {
        RCA_MISSING_CONTEXT | RCA_MODEL_LIMITATION | RCA_PROCEDURAL_ERROR => Some(root_cause),
        _ => {
            log::warn!("miner: RCA response outside closed vocabulary: {root_cause:?}");
            None
        }
    }
}

/// One LLM naming attempt. Isolated from `&MemoryStore` so it can be tested
/// with a paused clock without any DB I/O between `advance` calls.
///
/// `Some((skill_name, trigger))` on a well-formed, on-time response; `None`
/// on timeout / provider error / `ToolCalls` / parse-fail. Never panics.
async fn llm_name(
    provider: &Arc<dyn AiProvider>,
    task: &str,
    sequence: &[String],
    resolution: Option<&str>,
    timeout: Duration,
) -> Option<(String, String)> {
    let req = ChatRequest {
        model: String::new(),
        messages: vec![ChatMessage {
            id: crate::models::chat::generate_message_id(),
            role: Role::User,
            content: build_namer_prompt(task, sequence, resolution),
            tool_calls: None,
            timestamp_ms: 0,
            is_error: None,
        }],
        system_prompt: Some(NAMER_SYS.to_string()),
        custom_instructions: None,
        tools: None,
        extensions: Default::default(),
    };
    let sender: StreamSender = Box::new(|_e| Ok(()));
    let text = match tokio::time::timeout(timeout, provider.chat_stream(&req, sender)).await {
        Ok(Ok(ChatStreamResult::Text(txt))) => txt,
        Ok(Ok(ChatStreamResult::ToolCalls(_))) => {
            log::warn!("miner: namer returned ToolCalls; keeping heuristic");
            return None;
        }
        Ok(Err(e)) => {
            log::warn!("miner: namer provider error: {e}; keeping heuristic");
            return None;
        }
        Err(_elapsed) => {
            log::warn!("miner: namer timeout; keeping heuristic");
            return None;
        }
    };
    parse_skill_naming(&text)
}

/// One RCA classification attempt. Mirror of `llm_name`: isolated
/// from `&MemoryStore` so it can be tested with a paused clock without any DB
/// I/O between `advance` calls.
///
/// `Some(category)` on a well-formed, on-time, in-vocabulary response;
/// `None` on timeout / provider error / `ToolCalls` / parse-fail /
/// out-of-vocabulary. Never panics.
async fn llm_root_cause(
    provider: &Arc<dyn AiProvider>,
    task: &str,
    errors: &[String],
    resolution: Option<&str>,
    timeout: Duration,
) -> Option<String> {
    let req = ChatRequest {
        model: String::new(),
        messages: vec![ChatMessage {
            id: crate::models::chat::generate_message_id(),
            role: Role::User,
            content: build_rca_prompt(task, errors, resolution),
            tool_calls: None,
            timestamp_ms: 0,
            is_error: None,
        }],
        system_prompt: Some(RCA_SYS.to_string()),
        custom_instructions: None,
        tools: None,
        extensions: Default::default(),
    };
    let sender: StreamSender = Box::new(|_e| Ok(()));
    let text = match tokio::time::timeout(timeout, provider.chat_stream(&req, sender)).await {
        Ok(Ok(ChatStreamResult::Text(txt))) => txt,
        Ok(Ok(ChatStreamResult::ToolCalls(_))) => {
            log::warn!("miner: RCA returned ToolCalls; leaving root_cause NULL");
            return None;
        }
        Ok(Err(e)) => {
            log::warn!("miner: RCA provider error: {e}; leaving root_cause NULL");
            return None;
        }
        Err(_elapsed) => {
            log::warn!("miner: RCA timeout; leaving root_cause NULL");
            return None;
        }
    };
    parse_root_cause(&text)
}

/// Testable core of the mining pipeline — takes explicit dependencies, so it
/// can be unit tested without any host. Mirrors `extractor::run_extraction_core` / `run_episodic_writer_core`.
///
/// Flow: (1) delegate to `run_episodic_writer_core` — the episodic trajectory
/// is INSERTed unconditionally, before any gate; `None` short-circuits to
/// `vec![]`. (2) Golden gate: `draft.success_score <= GOLDEN_SCORE_THRESHOLD`
/// short-circuits to `vec![]` — no extraction, no LLM call. (3) extract at
/// most 1 tool-call sequence. (4) build each candidate with a heuristic
/// name/trigger, then attempt ONE LLM override (never blocking, never
/// dropping the candidate).
// 8 params is the fixed contract this testable-core split needs to keep
// every dependency explicit — narrowing it would break that split.
#[allow(clippy::too_many_arguments)]
pub async fn mine_session_core(
    session_id: String,
    history: Vec<(String, Vec<ChatMessage>)>,
    store: &MemoryStore,
    embedder: Option<Arc<dyn Embedder>>,
    project_by_agent: &HashMap<String, Option<String>>,
    provider_by_agent: &HashMap<String, Arc<dyn AiProvider>>,
    timeout: Duration,
    now_ms: u64,
) -> Vec<MinedSkillCandidate> {
    // Counts LLM calls issued below (RCA + namer), logged once at every exit
    // point as `aux_llm session={id} job=miner calls={n}`. Cloned before
    // `session_id` is moved into `run_episodic_writer_core` so the earliest
    // exit can still log it.
    let mut calls: u64 = 0;
    let session_id_for_log = session_id.clone();
    let Some((trajectory_id, draft)) =
        run_episodic_writer_core(session_id, history, store, embedder.clone(), project_by_agent, now_ms).await
    else {
        log::info!("aux_llm session={session_id_for_log} job=miner calls={calls}");
        return Vec::new();
    };

    // Gate uses `<=` (clippy-clean equivalent of `!(score > THRESHOLD)`);
    // `success_score` is always clamped [0,1], never NaN.
    if draft.success_score <= GOLDEN_SCORE_THRESHOLD {
        // Root Cause Analysis (fire-and-forget): attempt RCA iff there is
        // at least one error to classify. Every
        // failure path (absent provider, timeout, parse-fail, OOV, DB error)
        // warns and leaves `root_cause` NULL — never panics, never blocks
        // session close.
        if !draft.errors_encountered.is_empty() {
            match provider_by_agent.get(&draft.agent_id) {
                Some(provider) => {
                    calls += 1;
                    if let Some(cause) = llm_root_cause(
                        provider,
                        &draft.task,
                        &draft.errors_encountered,
                        draft.resolution.as_deref(),
                        timeout,
                    )
                    .await
                    {
                        if let Err(e) = store.update_trajectory_root_cause(trajectory_id, &cause).await {
                            log::warn!("miner: update_trajectory_root_cause failed id={trajectory_id}: {e}");
                        }
                    }
                    // llm_root_cause returning None already logged; leave NULL.
                }
                None => {
                    log::warn!("miner: no provider for agent={}; skipping RCA", draft.agent_id);
                }
            }
        }
        log::info!("aux_llm session={} job=miner calls={calls}", draft.session_id);
        return Vec::new();
    }

    let sequences = extract_tool_sequences(&draft);

    // Golden priors for recurrence matching. Errors are recoverable —
    // `log::warn!` + empty list, mining must never block session close.
    let priors_raw = store
        .fetch_prior_trajectories(&draft.agent_id, trajectory_id, GOLDEN_SCORE_THRESHOLD, PRIOR_TRAJECTORIES_LIMIT)
        .await
        .unwrap_or_else(|e| {
            log::warn!("miner: fetch_prior_trajectories failed for agent={}: {e}", draft.agent_id);
            Vec::new()
        });
    // Parse + normalize at query-time; malformed JSON is skipped with a
    // warning, not fatal.
    let priors: Vec<(i64, Vec<String>)> = priors_raw
        .iter()
        .filter_map(|(id, tool_calls_json)| match serde_json::from_str::<Vec<String>>(tool_calls_json) {
            Ok(v) => Some((*id, normalize_tool_sequence(&v))),
            Err(e) => {
                log::warn!("miner: malformed prior tool_calls JSON for trajectory_id={id}: {e}");
                None
            }
        })
        .collect();

    let mut candidates = Vec::with_capacity(sequences.len());
    for seq in sequences {
        let mut candidate = MinedSkillCandidate {
            source_trajectory_id: trajectory_id,
            session_id: draft.session_id.clone(),
            agent_id: draft.agent_id.clone(),
            task: draft.task.clone(),
            skill_name: heuristic_skill_name(&seq),
            trigger: heuristic_trigger(&draft.task),
            normalized_sequence: seq,
        };

        match provider_by_agent.get(&draft.agent_id) {
            Some(provider) => {
                calls += 1;
                if let Some((name, trig)) = llm_name(
                    provider,
                    &candidate.task,
                    &candidate.normalized_sequence,
                    draft.resolution.as_deref(),
                    timeout,
                )
                .await
                {
                    candidate.skill_name = name;
                    candidate.trigger = trig;
                }
                // llm_name returning None already logged; heuristic persists.
            }
            None => {
                log::warn!("miner: no provider for agent={}; keeping heuristic name", draft.agent_id);
            }
        }

        log::info!(
            "miner: mined skill candidate session_id={} trajectory_id={} skill_name={:?} steps={}",
            candidate.session_id,
            candidate.source_trajectory_id,
            candidate.skill_name,
            candidate.normalized_sequence.len(),
        );

        // Recurrence + promotion: total occurrences = 1 (actual) + matched
        // priors. Threshold not met => log only, no persistence.
        let matched = filter_recurring(&candidate.normalized_sequence, &priors);
        if 1 + matched.len() >= RECURRING_THRESHOLD {
            let mut source_trajectory_ids = vec![candidate.source_trajectory_id];
            source_trajectory_ids.extend(matched);

            // Embed "{trigger} {skill_name}" (trigger first), via the single
            // shared producer `skill_embedding_text` — `reembed_all` calls
            // the same function so a backfilled vector always matches what a
            // live write would have produced. Passage: a skill row is stored
            // content, retrieved later by a Query-role search.
            // Absent embedder / embed failure => None, never fatal.
            let embedding = match &embedder {
                Some(e) => match e
                    .embed_with_role(
                        &skill_embedding_text(&candidate.trigger, &candidate.skill_name),
                        EmbedRole::Passage,
                    )
                    .await
                {
                    Ok(v) => Some(v),
                    Err(e) => {
                        log::warn!(
                            "miner: embed failed for new skill candidate session_id={}: {e}",
                            candidate.session_id
                        );
                        None
                    }
                },
                None => None,
            };

            let rec = crate::memory::store_insert::NewProceduralSkillRecord {
                agent_id: candidate.agent_id.clone(),
                // RAW project_by_agent lookup — no derive, matches the
                // episodic writer's map but NOT its derived project_id value.
                project_id: project_by_agent.get(&candidate.agent_id).cloned().flatten(),
                skill_name: candidate.skill_name.clone(),
                trigger: candidate.trigger.clone(),
                tool_sequence_json: serialize_tool_sequence(&candidate.normalized_sequence),
                source_trajectory_ids,
                importance: 8.0,
                now_ms,
            };

            match store.promote_to_procedural_skill(rec, embedding).await {
                Ok(outcome) => log::info!(
                    "miner: promote_to_procedural_skill outcome={outcome:?} session_id={}",
                    candidate.session_id
                ),
                Err(e) => log::warn!(
                    "miner: promote_to_procedural_skill failed session_id={}: {e}",
                    candidate.session_id
                ),
            }
        } else {
            log::info!(
                "miner: candidate not recurrent (matched_priors={}) session_id={}",
                matched.len(),
                candidate.session_id
            );
        }

        candidates.push(candidate);
    }
    log::info!("aux_llm session={} job=miner calls={calls}", draft.session_id);
    candidates
}


#[cfg(test)]
mod tests {
    use super::*;

    // ── Pinned constant values ───────────────────────────────────────────────

    #[test]
    fn consts_have_pinned_values() {
        assert_eq!(GOLDEN_SCORE_THRESHOLD, 0.85);
        assert_eq!(NAMER_TIMEOUT, std::time::Duration::from_secs(30));
        assert_eq!(MAX_SKILL_NAME_STEPS, 4);
    }

    // ── RCA constant values ──────────────────────────────────────────────────

    #[test]
    fn rca_consts_have_pinned_values() {
        assert_eq!(RCA_MISSING_CONTEXT, "missing_context");
        assert_eq!(RCA_MODEL_LIMITATION, "model_limitation");
        assert_eq!(RCA_PROCEDURAL_ERROR, "procedural_error");
        assert_eq!(RCA_ERROR_ITEM_CHAR_CAP, 500);
        assert_eq!(RCA_MAX_ERROR_ITEMS, 20);
    }

    // ── RECURRING_THRESHOLD, serialize_tool_sequence, filter_recurring ───────

    #[test]
    fn recurring_threshold_pinned_value() {
        assert_eq!(RECURRING_THRESHOLD, 2);
    }

    #[test]
    fn serialize_tool_sequence_locks_json() {
        let seq = vec!["a".to_string(), "b".to_string()];
        assert_eq!(serialize_tool_sequence(&seq), "[\"a\",\"b\"]");
    }

    #[test]
    fn filter_recurring_exact_match() {
        let prior = vec![
            (7i64, vec!["A".to_string(), "B".to_string(), "C".to_string()]),
            (9i64, vec!["A".to_string(), "B".to_string()]),
        ];
        let candidate = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        assert_eq!(filter_recurring(&candidate, &prior), vec![7i64]);
    }

    #[test]
    fn filter_recurring_no_match_order_or_extra_step() {
        let prior = vec![(7i64, vec!["B".to_string(), "A".to_string(), "C".to_string()])];
        let candidate = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        assert_eq!(filter_recurring(&candidate, &prior), Vec::<i64>::new());
    }

    #[test]
    fn filter_recurring_multi_prior_all_matching() {
        let candidate = vec!["A".to_string(), "B".to_string()];
        let prior = vec![
            (1i64, candidate.clone()),
            (2i64, candidate.clone()),
            (3i64, vec!["X".to_string()]),
        ];
        assert_eq!(filter_recurring(&candidate, &prior), vec![1i64, 2i64]);
    }

    #[test]
    fn filter_recurring_empty_priors_returns_empty() {
        let candidate = vec!["A".to_string()];
        let prior: Vec<(i64, Vec<String>)> = vec![];
        assert_eq!(filter_recurring(&candidate, &prior), Vec::<i64>::new());
    }

    // ── normalize_tool_sequence — colapso de corridas adyacentes ─────────────

    #[test]
    fn collapse_adjacent_run() {
        let seq = vec![
            "read".to_string(),
            "edit".to_string(),
            "edit".to_string(),
            "edit".to_string(),
            "test".to_string(),
        ];
        assert_eq!(
            normalize_tool_sequence(&seq),
            vec!["read".to_string(), "edit".to_string(), "test".to_string()]
        );
    }

    #[test]
    fn preserve_non_adjacent() {
        let seq = vec!["read".to_string(), "edit".to_string(), "read".to_string()];
        assert_eq!(normalize_tool_sequence(&seq), seq);
    }

    #[test]
    fn edges_empty_and_all_same() {
        let empty: Vec<String> = vec![];
        assert_eq!(normalize_tool_sequence(&empty), Vec::<String>::new());

        let all_same = vec!["read".to_string(), "read".to_string(), "read".to_string()];
        assert_eq!(normalize_tool_sequence(&all_same), vec!["read".to_string()]);
    }

    // ── extract_tool_sequences — máx 1 secuencia por trayectoria ─────────────

    fn draft_with_tool_calls(tool_calls: Vec<String>) -> TrajectoryDraft {
        TrajectoryDraft {
            session_id: "sess".to_string(),
            agent_id: "agent".to_string(),
            task: "task".to_string(),
            tool_calls,
            errors_encountered: vec![],
            resolution: None,
            success_score: 0.9,
        }
    }

    #[test]
    fn nonempty_yields_one_sequence() {
        let draft = draft_with_tool_calls(vec![
            "read_file".to_string(),
            "edit_file".to_string(),
            "edit_file".to_string(),
            "test".to_string(),
        ]);
        let sequences = extract_tool_sequences(&draft);
        assert_eq!(
            sequences,
            vec![vec!["read_file".to_string(), "edit_file".to_string(), "test".to_string()]]
        );
    }

    #[test]
    fn empty_yields_zero_sequences() {
        let draft = draft_with_tool_calls(vec![]);
        assert_eq!(extract_tool_sequences(&draft), Vec::<Vec<String>>::new());
    }

    // ── heuristic_skill_name / heuristic_trigger ──────────────────────────────

    #[test]
    fn heuristic_name_join_short_sequence() {
        let seq = vec!["read_file".to_string(), "edit_file".to_string(), "run_tests".to_string()];
        assert_eq!(heuristic_skill_name(&seq), "read_file→edit_file→run_tests");
        assert_eq!(heuristic_trigger("fix the login bug"), "fix the login bug");
    }

    #[test]
    fn heuristic_name_exact_boundary_n_equals_4_not_abbreviated() {
        let seq = vec!["a".to_string(), "b".to_string(), "c".to_string(), "d".to_string()];
        assert_eq!(heuristic_skill_name(&seq), "a→b→c→d");
    }

    #[test]
    fn heuristic_name_caps_long_sequence() {
        let seq = vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
            "e".to_string(),
        ];
        assert_eq!(heuristic_skill_name(&seq), "a→b→…→e (5 steps)");
    }

    // ── MinedSkillCandidate — shape ────────────────────────────────────────────

    #[test]
    fn mined_skill_candidate_shape() {
        let c = MinedSkillCandidate {
            source_trajectory_id: 1,
            session_id: "sess".to_string(),
            agent_id: "agent".to_string(),
            task: "task".to_string(),
            normalized_sequence: vec!["read_file".to_string()],
            skill_name: "read_file".to_string(),
            trigger: "task".to_string(),
        };
        assert_eq!(c.source_trajectory_id, 1);
        assert_eq!(c.normalized_sequence, vec!["read_file".to_string()]);
    }

    // ── mine_session_core (pipeline + Golden gating) ─────────────────────────

    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use crate::memory::db::MemoryStore;
    use crate::memory::embedder::{Embedder, FakeEmbedder};
    use crate::models::chat::{Role, ToolCall, ToolFunction};
    use crate::models::{AppError, ChatMessage, ChatRequest, ChatStreamResult};
    use crate::providers::{AiProvider, StreamSender};

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

    /// Fake provider that returns a fixed text response and counts calls
    /// (molde extractor.rs `FakeProvider`).
    struct FakeProvider {
        response: String,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl AiProvider for FakeProvider {
        fn name(&self) -> &str {
            "fake"
        }
        async fn chat_stream(
            &self,
            _request: &ChatRequest,
            _sender: StreamSender,
        ) -> Result<ChatStreamResult, AppError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ChatStreamResult::Text(self.response.clone()))
        }
    }

    async fn count_episodic(store: &MemoryStore) -> i64 {
        store
            .conn
            .call(|c| c.query_row("SELECT COUNT(*) FROM episodic_trajectory", [], |r| r.get(0)))
            .await
            .unwrap()
    }

    async fn count_procedural(store: &MemoryStore) -> i64 {
        store
            .conn
            .call(|c| c.query_row("SELECT COUNT(*) FROM procedural_skill", [], |r| r.get(0)))
            .await
            .unwrap()
    }

    fn clean_history_with_tools(tool_batches: &[&[&str]]) -> Vec<(String, Vec<ChatMessage>)> {
        let mut msgs = vec![msg(Role::User, "fix the bug", 0)];
        let mut ts = 1u64;
        for batch in tool_batches {
            msgs.push(assistant_with_tools("", ts, batch));
            ts += 1;
            for _ in *batch {
                msgs.push(tool_result("ok", ts, false));
                ts += 1;
            }
        }
        msgs.push(msg(Role::Assistant, "done", ts));
        vec![("agent-a".to_string(), msgs)]
    }

    // core embedder=None → vec![], 0 filas, provider nunca invocado.
    #[tokio::test]
    async fn core_none_short_circuits() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let provider: Arc<dyn AiProvider> =
            Arc::new(FakeProvider { response: "{}".to_string(), calls: calls.clone() });
        let mut provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        provider_by_agent.insert("agent-a".to_string(), provider);
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let history = clean_history_with_tools(&[&["read_file"]]);

        let candidates = mine_session_core(
            "sess-none".to_string(),
            history,
            &store,
            None,
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            0,
        )
        .await;

        assert_eq!(candidates, Vec::new());
        assert_eq!(count_episodic(&store).await, 0);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "provider must never be invoked");
    }

    // low score (no-clean, 1 unresolved / 2 tool_calls => 0.15) → 1 row,
    // vec![] candidates, 0 namer LLM calls. The SAME
    // provider_by_agent is reused for RCA below the gate (no new spawn, no
    // re-fetch) — this history has 1 error_encountered, so RCA
    // DOES call the provider once; the namer itself is still never invoked
    // (extract_tool_sequences/heuristic naming only run past the golden gate).
    #[tokio::test]
    async fn low_score_only_inserts_zero_candidates_namer_never_invoked() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let provider: Arc<dyn AiProvider> =
            Arc::new(FakeProvider { response: "{}".to_string(), calls: calls.clone() });
        let mut provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        provider_by_agent.insert("agent-a".to_string(), provider);
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

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

        let candidates = mine_session_core(
            "sess-low".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            0,
        )
        .await;

        assert_eq!(candidates, Vec::new(), "gate must still short-circuit to no mined candidates");
        assert_eq!(count_episodic(&store).await, 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "the 1 call is RCA (errors_encountered non-empty), not the namer");
        assert!(
            fetch_root_cause(&store, "sess-low").await.is_none(),
            "response `{{}}` has no root_cause field => parse fails => stays NULL"
        );
    }

    // ── Boundary del gate — worked examples sobre score COMPUTADO ────────────

    #[tokio::test]
    async fn gate_clean_session_passes() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let history = clean_history_with_tools(&[&["read_file"]]);

        let candidates = mine_session_core(
            "sess-clean".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            0,
        )
        .await;

        assert_eq!(candidates.len(), 1, "clean session (score 0.9) must pass the gate");
    }

    #[tokio::test]
    async fn gate_resolved_error_passes() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

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

        let candidates = mine_session_core(
            "sess-resolved".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            0,
        )
        .await;

        assert_eq!(candidates.len(), 1, "resolved error before last Assistant must pass the gate");
    }

    #[tokio::test]
    async fn gate_unresolved_nonclean_does_not_pass() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        // 10 tool_calls, 1 trailing unresolved error (no Assistant after it)
        // => score = 0.4 - 0.5*(1/10) = 0.35 (real f64 arithmetic).
        let mut msgs = vec![msg(Role::User, "fix the bug", 0)];
        let mut ts = 1u64;
        for i in 0..10 {
            msgs.push(assistant_with_tools("", ts, &[&format!("tool_{i}")]));
            ts += 1;
            let is_last = i == 9;
            msgs.push(tool_result(if is_last { "boom" } else { "ok" }, ts, is_last));
            ts += 1;
        }
        let history = vec![("agent-a".to_string(), msgs)];

        // Boundary case: assert the score COMPUTED by
        // build_trajectory for this exact history, with epsilon — expected
        // derived from the pinned formula, not hardcoded: BASE 0.4 + bonus
        // 0.0 (last message is Role::Tool, not ended_clean) − 0.5·(1
        // unresolved / 10 tool_calls) = 0.35.
        let expected = 0.4 - 0.5 * (1.0 / 10.0);
        let draft = crate::memory::episodic_writer::build_trajectory("sess-nonclean", &history)
            .expect("history has a non-empty Role::User — draft must build");
        assert!(
            (draft.success_score - expected).abs() < 1e-9,
            "expected computed score ~{expected}, got {}",
            draft.success_score
        );
        assert!(
            draft.success_score <= GOLDEN_SCORE_THRESHOLD,
            "computed non-clean score must not clear the gate"
        );

        let candidates = mine_session_core(
            "sess-nonclean".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            0,
        )
        .await;

        assert_eq!(candidates, Vec::new(), "non-clean trailing error must NOT pass the gate");
        assert_eq!(count_episodic(&store).await, 1, "INSERT must still happen regardless of the gate");
    }

    #[tokio::test]
    async fn gate_long_clean_session_passes() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let names: Vec<String> = (0..20).map(|i| format!("tool_{i}")).collect();

        let mut msgs = vec![msg(Role::User, "fix the bug", 0)];
        let mut ts = 1u64;
        for name in &names {
            msgs.push(assistant_with_tools("", ts, &[name.as_str()]));
            ts += 1;
            msgs.push(tool_result("ok", ts, false));
            ts += 1;
        }
        msgs.push(msg(Role::Assistant, "done", ts));
        let history = vec![("agent-a".to_string(), msgs)];

        let candidates = mine_session_core(
            "sess-long-clean".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            0,
        )
        .await;

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].normalized_sequence.len(), 20, "20 distinct tool names, no collapse");
    }

    #[test]
    fn gate_strict_boundary_const_level() {
        // `std::hint::black_box` keeps these opaque to clippy's constant-folding
        // (assertions_on_constants) — the values ARE compile-time constants, but
        // the point of this test is to lock the boundary AT RUNTIME comparison.
        // `<=` (not `!(a > b)`) mirrors the production form and avoids `neg_cmp_op_on_partial_ord`.
        let at_boundary = std::hint::black_box(0.85_f64);
        let above_boundary = std::hint::black_box(0.9_f64);
        assert!(at_boundary <= GOLDEN_SCORE_THRESHOLD, "exact 0.85 must NOT pass — strict `>`");
        assert!(above_boundary > GOLDEN_SCORE_THRESHOLD, "0.9 must pass");
    }

    #[test]
    fn gate_is_effectively_binary() {
        use crate::memory::episodic_writer::build_trajectory;

        // Non-clean: any unresolved error implies the session did NOT end on
        // Assistant, so completion_bonus is always 0 and score <= 0.4.
        let non_clean = vec![(
            "agent-a".to_string(),
            vec![
                msg(Role::User, "t", 0),
                assistant_with_tools("", 1, &["c1"]),
                tool_result("boom", 2, true),
            ],
        )];
        let draft = build_trajectory("s", &non_clean).unwrap();
        assert!(!draft.success_score.is_nan());
        assert!(draft.success_score <= 0.4, "non-clean ceiling is 0.4, got {}", draft.success_score);

        // Clean: ended_clean forces unresolved=0 => score is exactly 0.9.
        let clean = vec![("agent-a".to_string(), vec![msg(Role::User, "t", 0), msg(Role::Assistant, "done", 1)])];
        let draft = build_trajectory("s", &clean).unwrap();
        assert!((draft.success_score - 0.9).abs() < 1e-9);
    }

    // ── Golden con tool_calls → 1 candidate normalizado ───────────────────────

    #[tokio::test]
    async fn golden_with_tool_calls_one_normalized_candidate() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let provider: Arc<dyn AiProvider> = Arc::new(FakeProvider {
            response: "not valid json".to_string(),
            calls: Arc::new(AtomicUsize::new(0)),
        });
        let mut provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        provider_by_agent.insert("agent-a".to_string(), provider);
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let history = clean_history_with_tools(&[&["read_file", "edit_file", "edit_file", "edit_file", "test"]]);

        let candidates = mine_session_core(
            "sess-golden-tools".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            0,
        )
        .await;

        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].normalized_sequence,
            vec!["read_file".to_string(), "edit_file".to_string(), "test".to_string()]
        );

        let rowid: i64 = store
            .conn
            .call(|c| c.query_row("SELECT rowid FROM episodic_trajectory", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(candidates[0].source_trajectory_id, rowid);
    }

    #[tokio::test]
    async fn golden_no_tool_calls_zero_candidates() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let history =
            vec![("agent-a".to_string(), vec![msg(Role::User, "hi", 0), msg(Role::Assistant, "hello", 1)])];

        let candidates = mine_session_core(
            "sess-golden-chat".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            0,
        )
        .await;

        assert_eq!(candidates, Vec::new());
        assert_eq!(count_episodic(&store).await, 1);
    }

    #[tokio::test]
    async fn golden_candidate_fully_populated() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let provider: Arc<dyn AiProvider> = Arc::new(FakeProvider {
            response: "not valid json".to_string(),
            calls: Arc::new(AtomicUsize::new(0)),
        });
        let mut provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        provider_by_agent.insert("agent-a".to_string(), provider);
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let history = clean_history_with_tools(&[&["read_file", "edit_file"]]);

        let candidates = mine_session_core(
            "sess-populated".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            0,
        )
        .await;

        assert_eq!(candidates.len(), 1);
        let c = &candidates[0];
        assert!(c.source_trajectory_id > 0);
        assert!(!c.session_id.is_empty());
        assert!(!c.agent_id.is_empty());
        assert!(!c.task.is_empty());
        assert!(!c.skill_name.is_empty());
        assert!(!c.trigger.is_empty());
        assert!(!c.normalized_sequence.is_empty());
    }

    // ── Recurrence and promotion to procedural_skill ──────────────────────────

    fn seeded_prior_record(
        agent_id: &str,
        session_id: &str,
        tool_names: &[&str],
        success_score: f64,
        created_at_ms: u64,
    ) -> crate::memory::store_insert::NewEpisodicTrajectoryRecord {
        crate::memory::store_insert::NewEpisodicTrajectoryRecord {
            scope: "agent",
            agent_id: Some(agent_id.to_string()),
            project_id: None,
            session_id: session_id.to_string(),
            task: "fix the bug".to_string(),
            tool_calls_json: serde_json::to_string(tool_names).unwrap(),
            errors_json: "[]".to_string(),
            resolution: Some("done".to_string()),
            success_score,
            created_by_type: crate::memory::store_insert::CreatedByType::System,
            created_by_ref: agent_id.to_string(),
            importance: 5.0,
            created_at_ms,
        }
    }

    // 1 Golden prior with a consecutive-dup raw sequence (proves query-time
    // normalization) recurring against the candidate ⇒
    // count_procedural==1, success_count==2, source_trajectory_ids==[current,prior],
    // tier==1, importance≈8.0, 1 procedural_vectors row.
    #[tokio::test]
    async fn recurrence_met_promotes_one_row() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let prior_record =
            seeded_prior_record("agent-a", "prior-sess", &["read_file", "read_file", "edit_file"], 0.9, 500);
        let prior_id = store.insert_episodic_trajectory(prior_record, vec![0.1_f32; 384]).await.unwrap();

        let history = clean_history_with_tools(&[&["read_file", "edit_file"]]);

        let candidates = mine_session_core(
            "sess-recurrent".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            1000,
        )
        .await;

        assert_eq!(candidates.len(), 1);
        assert_eq!(count_procedural(&store).await, 1, "recurring candidate must promote exactly 1 row");

        let (success_count, source_ids_json, tier, importance): (i64, String, i64, f64) = store
            .conn
            .call(|c| {
                c.query_row(
                    "SELECT success_count, source_trajectory_ids, tier, importance FROM procedural_skill",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
            })
            .await
            .unwrap();
        assert_eq!(success_count, 2);
        let source_ids: Vec<i64> = serde_json::from_str(&source_ids_json).unwrap();
        assert_eq!(source_ids, vec![candidates[0].source_trajectory_id, prior_id]);
        assert_eq!(tier, 1);
        assert!((importance - 8.0).abs() < 1e-9, "importance must be 8.0, got {importance}");

        let vec_count: i64 = store
            .conn
            .call(|c| c.query_row("SELECT COUNT(*) FROM procedural_vectors", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(vec_count, 1);
    }

    // Replaces the eliminated `procedural_skill_untouched`: 0 priors ⇒ total
    // occurrences = 1 < RECURRING_THRESHOLD ⇒ no promotion; candidate is
    // still returned/logged.
    #[tokio::test]
    async fn single_occurrence_does_not_promote() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let history = clean_history_with_tools(&[&["read_file", "edit_file"]]);

        let candidates = mine_session_core(
            "sess-single".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            0,
        )
        .await;

        assert_eq!(candidates.len(), 1, "candidate must still be logged");
        assert_eq!(count_procedural(&store).await, 0, "0 priors must not promote");
    }

    // A prior with an identical sequence but success_score=0.4 (non-clean
    // path) is filtered out at the SQL layer, so it never reaches
    // filter_recurring — no promotion.
    #[tokio::test]
    async fn prior_non_golden_does_not_count() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let prior_record = seeded_prior_record("agent-a", "prior-nongolden", &["read_file", "edit_file"], 0.4, 500);
        store.insert_episodic_trajectory(prior_record, vec![0.1_f32; 384]).await.unwrap();

        let history = clean_history_with_tools(&[&["read_file", "edit_file"]]);

        let candidates = mine_session_core(
            "sess-nongolden".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            1000,
        )
        .await;

        assert_eq!(candidates.len(), 1);
        assert_eq!(count_procedural(&store).await, 0, "non-Golden prior must not count toward recurrence");
    }

    // A Golden prior with a DIFFERENT sequence never matches
    // filter_recurring — no promotion.
    #[tokio::test]
    async fn prior_different_sequence_does_not_count() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let prior_record = seeded_prior_record("agent-a", "prior-diffseq", &["write_file"], 0.9, 500);
        store.insert_episodic_trajectory(prior_record, vec![0.1_f32; 384]).await.unwrap();

        let history = clean_history_with_tools(&[&["read_file", "edit_file"]]);

        let candidates = mine_session_core(
            "sess-diffseq".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            1000,
        )
        .await;

        assert_eq!(candidates.len(), 1);
        assert_eq!(count_procedural(&store).await, 0, "a Golden prior with a different sequence must not count");
    }

    // This is the test that distinguishes the pinned invariant
    // `success_count == len(distinct source_trajectory_ids)` from the
    // rejected `+= total_occurrences` reading: 3 sessions with the identical
    // normalized sequence, mined in order ⇒ after the 3rd, exactly 1 row,
    // success_count == 3 (NOT 5), source_trajectory_ids has exactly the 3
    // distinct trajectory ids.
    #[tokio::test]
    async fn dedup_third_session_success_count_equals_distinct_ids() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let history = || clean_history_with_tools(&[&["read_file", "edit_file"]]);
        let embedder = || Some(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>);

        let c1 = mine_session_core(
            "sess-dedup-1".to_string(),
            history(),
            &store,
            embedder(),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            1000,
        )
        .await;
        let id1 = c1[0].source_trajectory_id;
        assert_eq!(count_procedural(&store).await, 0, "1st occurrence must not promote");

        let c2 = mine_session_core(
            "sess-dedup-2".to_string(),
            history(),
            &store,
            embedder(),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            2000,
        )
        .await;
        let id2 = c2[0].source_trajectory_id;
        assert_eq!(count_procedural(&store).await, 1, "2nd occurrence must promote a new row");

        let c3 = mine_session_core(
            "sess-dedup-3".to_string(),
            history(),
            &store,
            embedder(),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            3000,
        )
        .await;
        let id3 = c3[0].source_trajectory_id;
        assert_eq!(count_procedural(&store).await, 1, "3rd occurrence must be a dedup UPDATE, not a 2nd row");

        let (success_count, source_ids_json): (i64, String) = store
            .conn
            .call(|c| {
                c.query_row("SELECT success_count, source_trajectory_ids FROM procedural_skill", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
            })
            .await
            .unwrap();
        assert_eq!(
            success_count, 3,
            "invariant success_count == len(distinct source_trajectory_ids); NOT += total_occurrences (would be 5)"
        );
        let mut source_ids: Vec<i64> = serde_json::from_str(&source_ids_json).unwrap();
        source_ids.sort();
        let mut expected = vec![id1, id2, id3];
        expected.sort();
        assert_eq!(source_ids, expected);
    }

    // ── Skill embed text is exactly "{trigger} {skill_name}" ─────────────────

    /// Wraps FakeEmbedder and records every `(role, text)` pair — lets tests
    /// assert the EXACT string AND role handed to `embed_with_role` (trigger
    /// first, then skill_name; role Passage), which the stored vector alone
    /// cannot prove readably.
    struct CapturingEmbedder {
        inner: FakeEmbedder,
        calls: Arc<std::sync::Mutex<Vec<(EmbedRole, String)>>>,
    }

    #[async_trait::async_trait]
    impl Embedder for CapturingEmbedder {
        async fn embed_with_role(
            &self,
            text: &str,
            role: EmbedRole,
        ) -> Result<Vec<f32>, crate::memory::embedder::EmbedError> {
            self.calls.lock().unwrap().push((role, text.to_string()));
            self.inner.embed_with_role(text, role).await
        }
        fn dimensions(&self) -> usize {
            384
        }
        fn model_id(&self) -> &str {
            "capturing-fake"
        }
        fn query_prefix(&self) -> &str {
            ""
        }
        fn passage_prefix(&self) -> &str {
            ""
        }
    }

    // The NEW-skill embedding input must be exactly
    // `format!("{trigger} {skill_name}")` — trigger FIRST. With no provider,
    // the heuristic name/trigger are deterministic, so the expected string is
    // fully known; the swapped-order string is asserted UNEQUAL to prove an
    // order swap would fail this test.
    #[tokio::test]
    async fn skill_embed_text_is_trigger_then_skill_name() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let embedder: Arc<dyn Embedder> =
            Arc::new(CapturingEmbedder { inner: FakeEmbedder::default(), calls: calls.clone() });
        let provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let prior_record = seeded_prior_record("agent-a", "prior-embed", &["read_file", "edit_file"], 0.9, 500);
        store.insert_episodic_trajectory(prior_record, vec![0.1_f32; 384]).await.unwrap();

        let history = clean_history_with_tools(&[&["read_file", "edit_file"]]);

        let candidates = mine_session_core(
            "sess-embed-text".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            1000,
        )
        .await;

        assert_eq!(candidates.len(), 1);
        assert_eq!(count_procedural(&store).await, 1, "promotion must have happened for the embed to be captured");

        // No provider ⇒ heuristic name/trigger persist: trigger = task,
        // skill_name = "→"-joined normalized sequence.
        let expected = format!("{} {}", "fix the bug", "read_file→edit_file");
        let swapped = format!("{} {}", "read_file→edit_file", "fix the bug");
        let captured = calls.lock().unwrap().clone();
        assert_eq!(captured.len(), 2, "exactly 2 embeds: task (episodic) + skill (procedural), got {captured:?}");
        assert_eq!(captured[1].1, expected, "skill embed text must be trigger FIRST, then skill_name");
        assert_ne!(captured[1].1, swapped, "an order swap must be detectable by this test");
        assert_eq!(captured[0].0, EmbedRole::Passage, "episodic task embed must be role Passage");
        assert_eq!(captured[1].0, EmbedRole::Passage, "skill embed must be role Passage");
    }

    // ── skill_embedding_text — pure, shared with reembed_all's backfill ──────

    #[test]
    fn skill_embedding_text_is_trigger_then_skill_name() {
        assert_eq!(skill_embedding_text("t", "s"), "t s");
    }

    // ── project_id is the RAW project_by_agent value, not derived ────────────

    // Positive scenario: `project_by_agent["agent-a"] = Some("proj-x")` ⇒
    // the promoted row stores the RAW value "proj-x" (no derive; a derived
    // id would fail this assert).
    #[tokio::test]
    async fn promoted_skill_project_id_is_raw_map_value() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        let mut project_by_agent: HashMap<String, Option<String>> = HashMap::new();
        project_by_agent.insert("agent-a".to_string(), Some("proj-x".to_string()));

        let prior_record = seeded_prior_record("agent-a", "prior-projid", &["read_file", "edit_file"], 0.9, 500);
        store.insert_episodic_trajectory(prior_record, vec![0.1_f32; 384]).await.unwrap();

        let history = clean_history_with_tools(&[&["read_file", "edit_file"]]);

        let candidates = mine_session_core(
            "sess-projid".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            1000,
        )
        .await;

        assert_eq!(candidates.len(), 1);
        assert_eq!(count_procedural(&store).await, 1);

        let project_id: Option<String> = store
            .conn
            .call(|c| c.query_row("SELECT project_id FROM procedural_skill", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(
            project_id.as_deref(),
            Some("proj-x"),
            "procedural_skill.project_id must be the RAW project_by_agent value, not a derived id"
        );
    }

    // ── Resilience: a malformed prior JSON must not block the rest ───────────

    // A malformed prior `tool_calls` JSON (written directly, bypassing the
    // normal writer's own serialization) must be skipped with a `log::warn!`
    // — NOT panic, NOT abort the whole mining pass — while a second, valid
    // Golden prior with the identical sequence still drives promotion.
    #[tokio::test]
    async fn malformed_prior_json_skipped_remaining_processed() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let mut malformed = seeded_prior_record("agent-a", "prior-malformed", &["read_file", "edit_file"], 0.9, 400);
        malformed.tool_calls_json = "not valid json".to_string();
        store.insert_episodic_trajectory(malformed, vec![0.1_f32; 384]).await.unwrap();

        let valid = seeded_prior_record("agent-a", "prior-valid", &["read_file", "edit_file"], 0.9, 500);
        let valid_id = store.insert_episodic_trajectory(valid, vec![0.1_f32; 384]).await.unwrap();

        let history = clean_history_with_tools(&[&["read_file", "edit_file"]]);

        let candidates = mine_session_core(
            "sess-malformed".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            1000,
        )
        .await;

        assert_eq!(candidates.len(), 1, "mining must complete without panic despite the malformed prior");
        assert_eq!(count_procedural(&store).await, 1, "the valid prior alone must still drive promotion");

        let source_ids_json: String = store
            .conn
            .call(|c| c.query_row("SELECT source_trajectory_ids FROM procedural_skill", [], |r| r.get(0)))
            .await
            .unwrap();
        let source_ids: Vec<i64> = serde_json::from_str(&source_ids_json).unwrap();
        assert_eq!(
            source_ids,
            vec![candidates[0].source_trajectory_id, valid_id],
            "the malformed prior's id must NOT appear in source_trajectory_ids"
        );
    }

    // ── Namer LLM — deeper coverage ───────────────────────────────────────────
    //
    // `parse_skill_naming`/`llm_name` are already implemented above this
    // point (needed so `mine_session_core` compiles end-to-end). This section
    // does NOT reimplement them — it deepens coverage over the existing
    // implementation: the parse matrix, explicit override/keep-heuristic
    // paths through `mine_session_core`, and the timeout path (previously
    // written but never exercised by a test).

    // ── parse_skill_naming — matriz de parse defensivo ────────────────────────

    #[test]
    fn parse_skill_naming_valid() {
        let txt = r#"{"skill_name":"Fix login flow","trigger":"when login fails"}"#;
        assert_eq!(
            parse_skill_naming(txt),
            Some(("Fix login flow".to_string(), "when login fails".to_string()))
        );
    }

    #[test]
    fn parse_skill_naming_valid_with_fences() {
        let txt = "```json\n{\"skill_name\":\"Fix login flow\",\"trigger\":\"when login fails\"}\n```";
        assert_eq!(
            parse_skill_naming(txt),
            Some(("Fix login flow".to_string(), "when login fails".to_string()))
        );
    }

    #[test]
    fn parse_skill_naming_empty_field_none() {
        let txt = r#"{"skill_name":"","trigger":"when login fails"}"#;
        assert_eq!(parse_skill_naming(txt), None);
    }

    #[test]
    fn parse_skill_naming_no_braces_none() {
        let txt = "Sure, here's a name for that skill: Fix login flow.";
        assert_eq!(parse_skill_naming(txt), None);
    }

    #[test]
    fn parse_skill_naming_malformed_json_none() {
        let txt = r#"{"skill_name": "Fix login flow", "trigger": }"#;
        assert_eq!(parse_skill_naming(txt), None);
    }

    // ── parse_root_cause — defensive parse matrix ─────────────────────────────

    #[test]
    fn parse_root_cause_valid_missing_context() {
        let txt = r#"{"root_cause":"missing_context"}"#;
        assert_eq!(parse_root_cause(txt), Some("missing_context".to_string()));
    }

    #[test]
    fn parse_root_cause_valid_model_limitation() {
        let txt = r#"{"root_cause":"model_limitation"}"#;
        assert_eq!(parse_root_cause(txt), Some("model_limitation".to_string()));
    }

    #[test]
    fn parse_root_cause_valid_procedural_error() {
        let txt = r#"{"root_cause":"procedural_error"}"#;
        assert_eq!(parse_root_cause(txt), Some("procedural_error".to_string()));
    }

    #[test]
    fn parse_root_cause_case_insensitive_matches() {
        let txt = r#"{"root_cause":"Missing_Context"}"#;
        assert_eq!(parse_root_cause(txt), Some("missing_context".to_string()));
    }

    #[test]
    fn parse_root_cause_out_of_vocabulary_none() {
        let txt = r#"{"root_cause":"hardware_failure"}"#;
        assert_eq!(parse_root_cause(txt), None);
    }

    #[test]
    fn parse_root_cause_markdown_fences_some() {
        let txt = "```json\n{\"root_cause\":\"procedural_error\"}\n```";
        assert_eq!(parse_root_cause(txt), Some("procedural_error".to_string()));
    }

    #[test]
    fn parse_root_cause_malformed_json_none() {
        let txt = r#"{"root_cause": }"#;
        assert_eq!(parse_root_cause(txt), None);
    }

    #[test]
    fn parse_root_cause_missing_or_empty_field_none() {
        assert_eq!(parse_root_cause(r#"{"other_field":"x"}"#), None);
        assert_eq!(parse_root_cause(r#"{"root_cause":""}"#), None);
    }

    #[test]
    fn parse_root_cause_no_braces_none() {
        let txt = "Sure, this looks like a missing_context issue.";
        assert_eq!(parse_root_cause(txt), None);
    }

    // ── build_rca_prompt — per-item and per-list caps ─────────────────────────

    #[test]
    fn build_rca_prompt_truncates_error_item_to_500_chars() {
        let long_error = "e".repeat(600);
        let prompt = build_rca_prompt("fix the bug", &[long_error], None);
        // The 600-char item must appear truncated to exactly 500 chars —
        // i.e. a run of 500 'e's followed by NOT another 'e' (line break/end).
        let truncated = "e".repeat(RCA_ERROR_ITEM_CHAR_CAP);
        assert!(prompt.contains(&truncated), "prompt must contain the 500-char truncated item");
        let full = "e".repeat(600);
        assert!(!prompt.contains(&full), "prompt must NOT contain the full untruncated 600-char item");
    }

    // The RCA prompt carries all three classification inputs.
    #[test]
    fn build_rca_prompt_contains_task_errors_and_resolution() {
        let errors = vec!["boom".to_string(), "kaboom".to_string()];
        let prompt = build_rca_prompt("fix the login bug", &errors, Some("patched the auth check"));
        assert!(prompt.contains("fix the login bug"), "prompt must contain the task");
        assert!(prompt.contains("boom"), "prompt must contain every error string");
        assert!(prompt.contains("kaboom"), "prompt must contain every error string");
        assert!(prompt.contains("patched the auth check"), "prompt must contain the resolution");
    }

    #[test]
    fn build_rca_prompt_caps_at_first_20_error_items() {
        let errors: Vec<String> = (0..25).map(|i| format!("error-{i}")).collect();
        let prompt = build_rca_prompt("fix the bug", &errors, None);
        assert!(prompt.contains("error-0"), "1st item must be present");
        assert!(prompt.contains("error-19"), "20th item (index 19) must be present");
        assert!(!prompt.contains("error-20"), "21st item (index 20) must be dropped");
        assert!(!prompt.contains("error-24"), "25th item must be dropped");
    }

    // ── llm_name vía mine_session_core ─────────────────────────────────────────

    #[tokio::test]
    async fn llm_override_applied() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let provider: Arc<dyn AiProvider> = Arc::new(FakeProvider {
            response: r#"{"skill_name":"Fix login flow","trigger":"when login fails"}"#.to_string(),
            calls: Arc::new(AtomicUsize::new(0)),
        });
        let mut provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        provider_by_agent.insert("agent-a".to_string(), provider);
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let history = clean_history_with_tools(&[&["read_file", "edit_file"]]);

        let candidates = mine_session_core(
            "sess-llm-override".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            0,
        )
        .await;

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].skill_name, "Fix login flow");
        assert_eq!(candidates[0].trigger, "when login fails");
    }

    #[tokio::test]
    async fn llm_parsefail_keeps_heuristic() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let provider: Arc<dyn AiProvider> =
            Arc::new(FakeProvider { response: "not valid json at all".to_string(), calls: calls.clone() });
        let mut provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        provider_by_agent.insert("agent-a".to_string(), provider);
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let history = clean_history_with_tools(&[&["read_file", "edit_file"]]);

        let candidates = mine_session_core(
            "sess-llm-parsefail".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            0,
        )
        .await;

        assert_eq!(candidates.len(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "LLM must still be attempted once");
        assert_eq!(
            candidates[0].skill_name, "read_file→edit_file",
            "parse failure must leave the heuristic name in place"
        );
        assert_eq!(candidates[0].trigger, "fix the bug");
    }

    #[tokio::test]
    async fn provider_absent_keeps_heuristic() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new(); // no entry for agent-a
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let history = clean_history_with_tools(&[&["read_file", "edit_file"]]);

        let candidates = mine_session_core(
            "sess-no-provider".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            0,
        )
        .await;

        assert_eq!(candidates.len(), 1, "candidate must still be returned without a provider");
        assert_eq!(candidates[0].skill_name, "read_file→edit_file");
    }

    // ── timeout — reloj pausado, testeado a nivel llm_name AISLADO ────────────

    /// Provider whose `chat_stream` future never resolves — used for the
    /// timeout test (molde resolver.rs `NeverProvider`).
    struct NeverProvider;

    #[async_trait::async_trait]
    impl AiProvider for NeverProvider {
        fn name(&self) -> &str {
            "never"
        }
        async fn chat_stream(
            &self,
            _request: &ChatRequest,
            _sender: StreamSender,
        ) -> Result<ChatStreamResult, AppError> {
            std::future::pending::<Result<ChatStreamResult, AppError>>().await
        }
    }

    #[tokio::test(start_paused = true)]
    async fn llm_name_timeout_returns_none() {
        let provider: Arc<dyn AiProvider> = Arc::new(NeverProvider);

        // Spawn so the test task can advance the paused clock independently.
        // Single `advance` past NAMER_TIMEOUT, zero DB I/O between — llm_name
        // takes no `&MemoryStore` for exactly this reason.
        let task = tokio::spawn({
            let p = provider.clone();
            async move { llm_name(&p, "task", &["read_file".to_string()], None, NAMER_TIMEOUT).await }
        });

        tokio::time::advance(Duration::from_secs(31)).await;

        let result = task.await.expect("llm_name task must not panic");
        assert_eq!(result, None, "LLM timeout must yield None (heuristic persists at the call site)");
    }

    // ── llm_root_cause — isolated from &MemoryStore, paused-clock testable ───

    // Isolated timeout test: paused clock, NeverProvider, single advance(31s),
    // NO DB I/O between advances — llm_root_cause takes no &MemoryStore for
    // exactly this reason.
    #[tokio::test(start_paused = true)]
    async fn llm_root_cause_timeout_returns_none() {
        let provider: Arc<dyn AiProvider> = Arc::new(NeverProvider);

        let task = tokio::spawn({
            let p = provider.clone();
            async move {
                llm_root_cause(&p, "task", &["boom".to_string()], None, NAMER_TIMEOUT).await
            }
        });

        tokio::time::advance(Duration::from_secs(31)).await;

        let result = task.await.expect("llm_root_cause task must not panic");
        assert_eq!(result, None, "LLM timeout must yield None");
    }

    #[tokio::test]
    async fn llm_root_cause_unparseable_text_returns_none() {
        let provider: Arc<dyn AiProvider> =
            Arc::new(FakeProvider { response: "not json".to_string(), calls: Arc::new(AtomicUsize::new(0)) });
        let result = llm_root_cause(&provider, "task", &["boom".to_string()], None, Duration::from_secs(5)).await;
        assert_eq!(result, None, "unparseable text must yield None");
    }

    /// Provider that always returns ToolCalls instead of Text.
    struct ToolCallsProvider;

    #[async_trait::async_trait]
    impl AiProvider for ToolCallsProvider {
        fn name(&self) -> &str {
            "tool-calls"
        }
        async fn chat_stream(
            &self,
            _request: &ChatRequest,
            _sender: StreamSender,
        ) -> Result<ChatStreamResult, AppError> {
            Ok(ChatStreamResult::ToolCalls(vec![]))
        }
    }

    #[tokio::test]
    async fn llm_root_cause_tool_calls_returns_none() {
        let provider: Arc<dyn AiProvider> = Arc::new(ToolCallsProvider);
        let result = llm_root_cause(&provider, "task", &["boom".to_string()], None, Duration::from_secs(5)).await;
        assert_eq!(result, None, "ToolCalls response must yield None");
    }

    // ── mine_session_core — RCA gate/integration ──────────────────────────────

    async fn fetch_root_cause(store: &MemoryStore, session_id: &str) -> Option<String> {
        let session_id = session_id.to_string();
        store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT root_cause FROM episodic_trajectory WHERE session_id = ?1",
                    rusqlite::params![session_id],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap()
    }

    fn nonclean_history_with_errors(errors: &[bool]) -> Vec<(String, Vec<ChatMessage>)> {
        // Builds a non-ended-clean history (last message is Role::Tool, so
        // completion_bonus is always 0 => score <= 0.4 <= GOLDEN_SCORE_THRESHOLD)
        // with one Tool result per `errors` entry, marked is_error per entry.
        let mut msgs = vec![msg(Role::User, "fix the bug", 0)];
        let mut ts = 1u64;
        for (i, &is_err) in errors.iter().enumerate() {
            msgs.push(assistant_with_tools("", ts, &[&format!("tool_{i}")]));
            ts += 1;
            let content = if is_err { "boom" } else { "ok" };
            msgs.push(tool_result(content, ts, is_err));
            ts += 1;
        }
        vec![("agent-a".to_string(), msgs)]
    }

    // Golden/clean (0.9) → root_cause NULL, provider not
    // called for RCA. Chat-only history (no tool_calls) so the namer's
    // llm_name is also never invoked — isolates the assertion to RCA.
    #[tokio::test]
    async fn rca_golden_session_skips_rca_provider_uncalled() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let provider: Arc<dyn AiProvider> =
            Arc::new(FakeProvider { response: r#"{"root_cause":"model_limitation"}"#.to_string(), calls: calls.clone() });
        let mut provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        provider_by_agent.insert("agent-a".to_string(), provider);
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let history =
            vec![("agent-a".to_string(), vec![msg(Role::User, "hi", 0), msg(Role::Assistant, "hello", 1)])];

        mine_session_core(
            "sess-rca-golden".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            0,
        )
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 0, "provider must never be invoked for a golden session");
        assert!(fetch_root_cause(&store, "sess-rca-golden").await.is_none(), "root_cause must stay NULL");
    }

    // Non-clean + zero errors → NULL, no LLM call.
    #[tokio::test]
    async fn rca_nonclean_zero_errors_skips_rca() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let provider: Arc<dyn AiProvider> =
            Arc::new(FakeProvider { response: r#"{"root_cause":"model_limitation"}"#.to_string(), calls: calls.clone() });
        let mut provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        provider_by_agent.insert("agent-a".to_string(), provider);
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        // Ends on a non-error Tool result: bonus=0, penalty=0 (0 unresolved) => score=0.4.
        let history = nonclean_history_with_errors(&[false]);

        mine_session_core(
            "sess-rca-nonclean-noerr".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            0,
        )
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 0, "no errors => RCA must never call the provider");
        assert!(fetch_root_cause(&store, "sess-rca-nonclean-noerr").await.is_none());
    }

    // Non-clean + errors + FakeProvider valid category =>
    // root_cause == category.
    #[tokio::test]
    async fn rca_nonclean_with_errors_valid_category_persists() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let provider: Arc<dyn AiProvider> = Arc::new(FakeProvider {
            response: r#"{"root_cause":"model_limitation"}"#.to_string(),
            calls: Arc::new(AtomicUsize::new(0)),
        });
        let mut provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        provider_by_agent.insert("agent-a".to_string(), provider);
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let history = nonclean_history_with_errors(&[true]);

        mine_session_core(
            "sess-rca-valid".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            0,
        )
        .await;

        assert_eq!(
            fetch_root_cause(&store, "sess-rca-valid").await.as_deref(),
            Some("model_limitation"),
            "valid category must persist verbatim"
        );
    }

    // Out-of-vocabulary response => root_cause stays NULL.
    #[tokio::test]
    async fn rca_out_of_vocabulary_response_stays_null() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let provider: Arc<dyn AiProvider> = Arc::new(FakeProvider {
            response: r#"{"root_cause":"hardware_failure"}"#.to_string(),
            calls: Arc::new(AtomicUsize::new(0)),
        });
        let mut provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        provider_by_agent.insert("agent-a".to_string(), provider);
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let history = nonclean_history_with_errors(&[true]);

        mine_session_core(
            "sess-rca-oov".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            0,
        )
        .await;

        assert!(fetch_root_cause(&store, "sess-rca-oov").await.is_none(), "OOV category must leave root_cause NULL");
    }

    // No provider entry for the agent => NULL, no panic.
    #[tokio::test]
    async fn rca_no_provider_for_agent_stays_null() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new(); // no entry
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let history = nonclean_history_with_errors(&[true]);

        mine_session_core(
            "sess-rca-noprovider".to_string(),
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
            0,
        )
        .await;

        assert!(fetch_root_cause(&store, "sess-rca-noprovider").await.is_none());
    }
}
