//! Session-close memory extractor.
//!
//! Run on session close. Extracts pivotal memories (`user_preference`,
//! `architectural_decision`, `semantic_fact`) from the outgoing conversation via an
//! LLM call, parses the response defensively, and inserts each memory with its
//! embedding through `insert_memory_with_vector` (which now computes the correct
//! initial `tier`). Clone of the `memory/resolver.rs` fire-and-forget pattern.
//!
//! # Design constraints
//! - `EXTRACTOR_TIMEOUT = 30s`.
//! - Embedder `OnceCell` empty → `log::debug!` + return EARLY, before the LLM call.
//! - `scope="agent"`, `created_by_type=System`, `created_by_ref=agent_id`.
//! - Never panics — recoverable errors are `log::warn!`'d and skipped.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::memory::db::MemoryStore;
use crate::memory::embedder::{EmbedRole, Embedder};
use crate::memory::events::MemorySource;
use crate::memory::store_insert::{CreatedByType, MemoryType, NewMemoryRecord};
use crate::models::chat::Role;
use crate::models::{ChatMessage, ChatRequest, ChatStreamResult};
use crate::providers::{AiProvider, StreamSender};

/// Maximum wall-clock time allowed for a single extraction LLM call.
pub const EXTRACTOR_TIMEOUT: Duration = Duration::from_secs(30);

/// Hard cap on the rendered transcript passed to the LLM (approx. chars, not tokens).
const TRANSCRIPT_CHAR_CAP: usize = 12_000;

/// System prompt for the extraction LLM call.
///
/// CRITICAL: `workspace.rs` requires `importance >= 8`
/// for `user_preference` / `architectural_decision` to appear in the workspace
/// AUTO section. This prompt pushes the LLM to reserve 8+ for genuine pivotals —
/// without it, rows insert correctly but stay invisible in workspace.md.
const EXTRACT_SYS: &str = "You extract durable memories from a conversation between a user and an AI coding agent.\n\
    Return ONLY a JSON array (no prose, no markdown fences). Each element:\n\
    {\"memory_type\": <type>, \"content\": <string>, \"importance\": <1-10>}\n\
    Types (use EXACTLY these strings):\n\
    - \"user_preference\": a stable preference/constraint about HOW the user wants work done (tools, style, conventions, language). NOT one-off task requests.\n\
    - \"architectural_decision\": a design/architecture decision made, with its rationale (pattern, library, structure chosen and why).\n\
    - \"semantic_fact\": a durable non-obvious fact about the codebase/domain (invariant, gotcha, naming convention).\n\
    Rules:\n\
    - Extract ONLY durable, reusable memories. Skip greetings and transient task chatter.\n\
    - importance 1-10: reserve 9-10 for memories that must ALWAYS be visible (a hard rule the user set, a load-bearing decision). Use 8 for solid preferences/decisions worth surfacing. 1-7 for weaker facts.\n\
    - If nothing qualifies, return [].\n\
    - content: one self-contained sentence, no pronouns referring to the chat.";

/// Render the user-turn transcript passed to the extraction LLM call.
///
/// One `role: content` line per message, capped at [`TRANSCRIPT_CHAR_CAP`] chars.
fn build_transcript(msgs: &[ChatMessage]) -> String {
    let mut out = String::new();
    for m in msgs {
        let role = match m.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
            Role::System => "system",
        };
        out.push_str(role);
        out.push_str(": ");
        out.push_str(&m.content);
        out.push('\n');
    }
    if out.len() > TRANSCRIPT_CHAR_CAP {
        out.truncate(TRANSCRIPT_CHAR_CAP);
    }
    out
}

/// Reverse-map of `MemoryType::as_str` — parses the LLM's `memory_type` string
/// into the typed enum. Unknown strings return `None` (caller skips the element).
pub(crate) fn memory_type_from_str(s: &str) -> Option<MemoryType> {
    match s {
        "user_preference" => Some(MemoryType::UserPreference),
        "architectural_decision" => Some(MemoryType::ArchitecturalDecision),
        "semantic_fact" => Some(MemoryType::SemanticFact),
        _ => None,
    }
}

/// One pivotal memory extracted from the LLM's JSON response, post-validation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExtractedMemory {
    pub memory_type: MemoryType,
    pub content: String,
    pub importance: f64,
}

/// Parse the LLM's raw text response into a list of validated `ExtractedMemory`.
///
/// Defensive by construction — mirrors `resolver::parse_verdict`'s philosophy of
/// never panicking on malformed LLM output:
/// 1. Tolerate markdown fences: slice from the first `[` to the last `]`. If
///    either bracket is missing, log a warning and return `vec![]`.
/// 2. Parse the slice as `Vec<serde_json::Value>`. A global parse failure
///    (malformed JSON) logs a warning and returns `vec![]`.
/// 3. Validate each element independently — an invalid element is skipped
///    (logged), the rest of the array survives:
///    - `memory_type`: must be a string that `memory_type_from_str` recognizes.
///    - `content`: must be a non-empty string after trimming.
///    - `importance`: must be numeric; clamped to `[1.0, 10.0]` (not skipped
///      when out of range — only skipped when absent/non-numeric).
pub(crate) fn parse_extraction(txt: &str) -> Vec<ExtractedMemory> {
    let (Some(start), Some(end)) = (txt.find('['), txt.rfind(']')) else {
        log::warn!("extractor: no JSON array found in LLM response");
        return Vec::new();
    };
    if end < start {
        log::warn!("extractor: malformed brackets in LLM response");
        return Vec::new();
    }
    let slice = &txt[start..=end];

    let values: Vec<serde_json::Value> = match serde_json::from_str(slice) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("extractor: failed to parse JSON array: {e}");
            return Vec::new();
        }
    };

    let mut out = Vec::with_capacity(values.len());
    for (i, v) in values.into_iter().enumerate() {
        let Some(memory_type) = v.get("memory_type").and_then(|m| m.as_str()).and_then(memory_type_from_str) else {
            log::warn!("extractor: element {i} has unknown/missing memory_type; skipping");
            continue;
        };
        let content = v.get("content").and_then(|c| c.as_str()).unwrap_or("").trim().to_string();
        if content.is_empty() {
            log::warn!("extractor: element {i} has empty content; skipping");
            continue;
        }
        let Some(importance_raw) = v.get("importance").and_then(|imp| imp.as_f64()) else {
            log::warn!("extractor: element {i} has non-numeric/missing importance; skipping");
            continue;
        };
        let importance = importance_raw.clamp(1.0, 10.0);
        out.push(ExtractedMemory { memory_type, content, importance });
    }
    out
}

/// Testable core of the extraction pipeline — takes explicit dependencies, so
/// it can be unit tested without any host.
///
/// `project_by_agent` / `provider_by_agent` are keyed by `agent_id` and are
/// expected to already contain an entry for every agent present in `history`.
/// An agent with no provider entry is skipped with `log::warn!`.
///
/// Never panics: LLM errors/timeouts, parse failures, and embed/store errors
/// are all `log::`'d and skipped — the loop always continues to the next item.
///
/// Returns the number of LLM calls issued — one per agent with non-empty
/// history that has a resolved provider — so the caller can log
/// `aux_llm job=extractor calls={n}`.
pub async fn run_extraction_core(
    history: Vec<(String, Vec<ChatMessage>)>,
    store: &MemoryStore,
    embedder: Option<Arc<dyn Embedder>>,
    project_by_agent: &HashMap<String, Option<String>>,
    provider_by_agent: &HashMap<String, Arc<dyn AiProvider>>,
    timeout: Duration,
) -> u64 {
    if history.iter().all(|(_, m)| m.is_empty()) {
        return 0;
    }

    // Check the embedder ONCE, BEFORE any LLM call — saves the call entirely
    // when the embedder is not ready, instead of running extraction on every
    // session in the batch only to find out at the end that none of it could
    // be stored.
    let Some(embedder) = embedder else {
        log::debug!("extractor: embedder not ready; skipping extraction for this session");
        return 0;
    };

    let mut calls = 0u64;

    for (agent_id, msgs) in history {
        if msgs.is_empty() {
            continue;
        }
        let Some(provider) = provider_by_agent.get(&agent_id) else {
            log::warn!("extractor: no provider resolved for agent={agent_id}; skipping");
            continue;
        };

        let transcript = build_transcript(&msgs);
        let req = ChatRequest {
            model: String::new(),
            messages: vec![ChatMessage {
                id: crate::models::chat::generate_message_id(),
                role: Role::User,
                content: transcript,
                tool_calls: None,
                timestamp_ms: 0,
                is_error: None,
            }],
            system_prompt: Some(EXTRACT_SYS.to_string()),
            custom_instructions: None,
            tools: None,
            extensions: Default::default(),
        };
        let sender: StreamSender = Box::new(|_e| Ok(()));

        calls += 1;
        let text = match tokio::time::timeout(timeout, provider.chat_stream(&req, sender)).await {
            Ok(Ok(ChatStreamResult::Text(txt))) => txt,
            Ok(Ok(ChatStreamResult::ToolCalls(_))) => {
                log::warn!("extractor: provider returned ToolCalls for agent={agent_id}; skipping");
                continue;
            }
            Ok(Err(e)) => {
                log::warn!("extractor: provider error for agent={agent_id}: {e}");
                continue;
            }
            Err(_elapsed) => {
                log::warn!("extractor: LLM timeout for agent={agent_id}");
                continue;
            }
        };

        let extracted = parse_extraction(&text);
        if extracted.is_empty() {
            continue;
        }

        let project_id = project_by_agent
            .get(&agent_id)
            .cloned()
            .flatten()
            .filter(|s| !s.is_empty())
            .map(|s| crate::memory::identity::derive_project_id(std::path::Path::new(&s)));

        let now_ms = crate::models::chat::current_time_millis();

        for em in extracted {
            let vec = match embedder.embed_with_role(&em.content, EmbedRole::Passage).await {
                Ok(v) => v,
                Err(e) => {
                    log::warn!("extractor: embed failed for agent={agent_id}: {e}; skipping memory");
                    continue;
                }
            };
            let record = NewMemoryRecord {
                scope: "agent",
                agent_id: Some(agent_id.clone()),
                project_id: project_id.clone(),
                content: em.content,
                memory_type: em.memory_type,
                source: MemorySource::Chat,
                created_by_type: CreatedByType::System,
                created_by_ref: agent_id.clone(),
                importance: em.importance,
                created_at_ms: now_ms,
                disputed_against: None,
            };
            if let Err(e) = store.insert_memory_with_vector(record, vec).await {
                log::warn!("extractor: insert_memory_with_vector failed for agent={agent_id}: {e}");
            }
        }
    }

    calls
}


#[cfg(test)]
mod tests {
    use super::*;

    // ── memory_type_from_str round trip ─────────────────────────────────────

    #[test]
    fn memory_type_from_str_round_trip() {
        assert_eq!(memory_type_from_str("user_preference"), Some(MemoryType::UserPreference));
        assert_eq!(memory_type_from_str("architectural_decision"), Some(MemoryType::ArchitecturalDecision));
        assert_eq!(memory_type_from_str("semantic_fact"), Some(MemoryType::SemanticFact));
        assert_eq!(memory_type_from_str("unknown_type"), None);
    }

    // ── parse_extraction (pure, defensive) ────────────────────────────────────

    // Prose with no JSON array → vec![], no panic.
    #[test]
    fn parse_extraction_non_json_prose_yields_empty_vec() {
        let out = parse_extraction("Sure, here is a summary of the conversation without any JSON.");
        assert_eq!(out, vec![]);
    }

    // JSON wrapped in ```json fences parses correctly.
    #[test]
    fn parse_extraction_markdown_fenced_json_parses() {
        let txt = "```json\n[{\"memory_type\":\"user_preference\",\"content\":\"Use voseo\",\"importance\":8}]\n```";
        let out = parse_extraction(txt);
        assert_eq!(
            out,
            vec![ExtractedMemory {
                memory_type: MemoryType::UserPreference,
                content: "Use voseo".to_string(),
                importance: 8.0,
            }]
        );
    }

    // Non-numeric importance skips ONLY that element.
    #[test]
    fn parse_extraction_non_numeric_importance_skips_only_that_element() {
        let txt = r#"[
            {"memory_type":"user_preference","content":"first","importance":8},
            {"memory_type":"semantic_fact","content":"bad one","importance":"high"},
            {"memory_type":"architectural_decision","content":"third","importance":7}
        ]"#;
        let out = parse_extraction(txt);
        assert_eq!(out.len(), 2, "expected 2 valid entries, got {out:?}");
        assert_eq!(out[0].content, "first");
        assert_eq!(out[1].content, "third");
    }

    // Out-of-range importance is clamped, not skipped.
    #[test]
    fn parse_extraction_out_of_range_importance_is_clamped() {
        let txt = r#"[
            {"memory_type":"user_preference","content":"too low","importance":0.5},
            {"memory_type":"semantic_fact","content":"too high","importance":12}
        ]"#;
        let out = parse_extraction(txt);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].importance, 1.0, "0.5 must clamp to 1.0");
        assert_eq!(out[1].importance, 10.0, "12 must clamp to 10.0");
    }

    // Unknown memory_type skips that element.
    #[test]
    fn parse_extraction_unknown_memory_type_skips_element() {
        let txt = r#"[
            {"memory_type":"episodic_trajectory","content":"nope","importance":8},
            {"memory_type":"semantic_fact","content":"kept","importance":6}
        ]"#;
        let out = parse_extraction(txt);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].content, "kept");
    }

    // Empty content (post-trim) skips that element.
    #[test]
    fn parse_extraction_empty_content_skips_element() {
        let txt = r#"[
            {"memory_type":"user_preference","content":"   ","importance":8},
            {"memory_type":"semantic_fact","content":"kept","importance":6}
        ]"#;
        let out = parse_extraction(txt);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].content, "kept");
    }

    // ── run_extraction_core ───────────────────────────────────────────────────

    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use crate::memory::db::MemoryStore;
    use crate::memory::embedder::{EmbedError, EmbedRole, Embedder, FakeEmbedder};
    use crate::models::chat::Role;

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

    /// `run_extraction_core` must embed each extracted memory's content with
    /// role `Passage` — the extracted memory is stored, not a search query.
    #[tokio::test]
    async fn run_extraction_core_embeds_extracted_memories_with_role_passage() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let recording = Arc::new(RecordingEmbedder { calls: std::sync::Mutex::new(Vec::new()) });
        let embedder: Arc<dyn Embedder> = recording.clone();
        let provider: Arc<dyn AiProvider> = Arc::new(FakeProvider {
            response: r#"[{"memory_type":"semantic_fact","content":"a durable fact","importance":6}]"#.to_string(),
            calls: Arc::new(AtomicUsize::new(0)),
        });
        let mut provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        provider_by_agent.insert("agent-a".to_string(), provider);
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();
        let history = vec![("agent-a".to_string(), vec![make_msg(Role::User, "hello")])];

        run_extraction_core(
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
        )
        .await;

        let calls = recording.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "expected exactly one embed call, got {calls:?}");
        assert_eq!(calls[0].0, EmbedRole::Passage);
        assert_eq!(calls[0].1, "a durable fact");
    }
    use crate::models::{AppError, ChatMessage, ChatRequest, ChatStreamResult};
    use crate::providers::{AiProvider, StreamSender};

    fn make_msg(role: Role, content: &str) -> ChatMessage {
        ChatMessage {
            id: "t".to_string(),
            role,
            content: content.to_string(),
            tool_calls: None,
            timestamp_ms: 0,
            is_error: None,
        }
    }

    /// Fake provider that returns a fixed text response and counts calls.
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

    async fn count_memory(store: &MemoryStore) -> i64 {
        store
            .conn
            .call(|c| c.query_row("SELECT COUNT(*) FROM memory", [], |r| r.get(0)))
            .await
            .unwrap()
    }

    // Empty history → 0 LLM calls, 0 inserts.
    #[tokio::test]
    async fn empty_history_yields_zero_calls_zero_inserts() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let provider: Arc<dyn AiProvider> = Arc::new(FakeProvider {
            response: "[]".to_string(),
            calls: calls.clone(),
        });
        let mut provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        provider_by_agent.insert("agent-a".to_string(), provider);
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());

        run_extraction_core(
            vec![],
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
        )
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 0, "empty history must trigger zero LLM calls");
        assert_eq!(count_memory(&store).await, 0, "empty history must insert zero rows");
    }

    // Embedder OnceCell empty (None here) → 0 LLM calls, 0 inserts,
    // early return BEFORE the LLM is ever consulted.
    #[tokio::test]
    async fn missing_embedder_yields_zero_calls_zero_inserts() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let provider: Arc<dyn AiProvider> = Arc::new(FakeProvider {
            response: r#"[{"memory_type":"semantic_fact","content":"should never insert","importance":8}]"#.to_string(),
            calls: calls.clone(),
        });
        let mut provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        provider_by_agent.insert("agent-a".to_string(), provider);
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let history = vec![("agent-a".to_string(), vec![make_msg(Role::User, "hello")])];

        run_extraction_core(
            history,
            &store,
            None, // embedder not ready
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
        )
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 0, "missing embedder must skip the LLM call entirely");
        assert_eq!(count_memory(&store).await, 0, "missing embedder must insert zero rows");
    }

    // Mixed history + fake provider returning all 3 pivotal types →
    // 3 `memory` rows, one per type.
    #[tokio::test]
    async fn mixed_history_fake_provider_three_types_yields_three_rows() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let response = r#"[
            {"memory_type":"user_preference","content":"Always use voseo","importance":9},
            {"memory_type":"architectural_decision","content":"Use hexagonal architecture","importance":8},
            {"memory_type":"semantic_fact","content":"tier=1 rows are excluded from KNN","importance":6}
        ]"#;
        let provider: Arc<dyn AiProvider> = Arc::new(FakeProvider {
            response: response.to_string(),
            calls: calls.clone(),
        });
        let mut provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        provider_by_agent.insert("agent-a".to_string(), provider);
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());

        let history = vec![(
            "agent-a".to_string(),
            vec![
                make_msg(Role::User, "I prefer voseo and hexagonal architecture"),
                make_msg(Role::Assistant, "Noted, I'll keep that in mind"),
            ],
        )];

        run_extraction_core(
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
        )
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 1, "expected exactly 1 LLM call for the single agent");
        assert_eq!(count_memory(&store).await, 3, "expected 3 rows, one per extracted memory");

        let types: Vec<String> = store
            .conn
            .call(|c| -> Result<Vec<String>, rusqlite::Error> {
                let mut stmt = c.prepare("SELECT memory_type FROM memory ORDER BY memory_type")?;
                let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
                rows.collect()
            })
            .await
            .unwrap();
        assert_eq!(
            types,
            vec!["architectural_decision", "semantic_fact", "user_preference"],
            "expected all 3 pivotal types present"
        );
    }

    // `run_extraction_core` returns the number of LLM calls it issued, one per
    // agent with non-empty history that has a resolved provider — logged by
    // the caller as `aux_llm job=extractor calls={n}`.
    #[tokio::test]
    async fn run_extraction_core_counts_one_call_per_agent_with_history() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());
        let provider_a: Arc<dyn AiProvider> =
            Arc::new(FakeProvider { response: "[]".to_string(), calls: Arc::new(AtomicUsize::new(0)) });
        let provider_b: Arc<dyn AiProvider> =
            Arc::new(FakeProvider { response: "[]".to_string(), calls: Arc::new(AtomicUsize::new(0)) });
        let mut provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        provider_by_agent.insert("agent-a".to_string(), provider_a);
        provider_by_agent.insert("agent-b".to_string(), provider_b);
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();

        let history = vec![
            ("agent-a".to_string(), vec![make_msg(Role::User, "hi")]),
            ("agent-b".to_string(), vec![make_msg(Role::User, "hello")]),
        ];

        let calls = run_extraction_core(
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
        )
        .await;

        assert_eq!(calls, 2, "one LLM call per agent with non-empty history");
    }


    // ── Integration: workspace bridge + retrieval KNN ─────────────────────────

    // A fake provider returns a pivotal (user_preference, importance=8,
    // tier=1 at insert) → after run_extraction_core + WorkspaceManager
    // regeneration, the content appears in workspace.md's AUTO section.
    #[tokio::test]
    async fn extracted_pivotal_appears_in_workspace_auto_section() {
        use crate::memory::workspace::WorkspaceManager;

        let store = MemoryStore::new_for_test().await.unwrap();
        let response = r#"[{"memory_type":"user_preference","content":"Always confirm before destructive commands","importance":8}]"#;
        let provider: Arc<dyn AiProvider> = Arc::new(FakeProvider {
            response: response.to_string(),
            calls: Arc::new(AtomicUsize::new(0)),
        });
        let mut provider_by_agent: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        provider_by_agent.insert("agent-ws".to_string(), provider);
        let project_by_agent: HashMap<String, Option<String>> = HashMap::new();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::default());

        let history = vec![(
            "agent-ws".to_string(),
            vec![make_msg(Role::User, "Please always confirm before running destructive commands")],
        )];

        run_extraction_core(
            history,
            &store,
            Some(embedder),
            &project_by_agent,
            &provider_by_agent,
            Duration::from_secs(5),
        )
        .await;

        let tmp = tempfile::tempdir().unwrap();
        let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
        wm.regenerate_auto_section("agent-ws", None, &store).await.unwrap();
        let content = wm.read("agent-ws", None).await.unwrap();

        assert!(
            content.contains("Always confirm before destructive commands"),
            "expected extracted pivotal in workspace AUTO section, got:\n{content}"
        );
    }

    // Retrieval by producer via search_all_memories.
    //
    // `search.rs::SEARCH_SQL_MEMORY` carries no tier filter — every tier,
    // including tier=1 (where `compute_tier` always places `user_preference`),
    // is KNN-retrievable. This test covers `architectural_decision` (tier=2)
    // and `semantic_fact` (tier=3); `user_preference` (tier=1) is covered
    // separately below by `user_preference_tier_1_is_retrievable_by_knn`,
    // which asserts the tier=1 case explicitly.
    #[tokio::test]
    async fn architectural_decision_and_semantic_fact_are_knn_retrievable() {
        use crate::memory::search::{search_all_memories, SearchScope};

        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder = FakeEmbedder::default();

        let arch_emb = embedder.embed_with_role("Use hexagonal architecture for the tools module", EmbedRole::Passage).await.unwrap();
        let arch_record = NewMemoryRecord {
            scope: "agent",
            agent_id: Some("agent-knn".to_string()),
            project_id: None,
            content: "Use hexagonal architecture for the tools module".to_string(),
            memory_type: MemoryType::ArchitecturalDecision,
            source: MemorySource::Chat,
            created_by_type: CreatedByType::System,
            created_by_ref: "agent-knn".to_string(),
            importance: 8.0,
            created_at_ms: 0,
            disputed_against: None,
        };
        let arch_id = store.insert_memory_with_vector(arch_record, arch_emb.clone()).await.unwrap();

        let fact_emb = embedder.embed_with_role("tier=1 rows are excluded from KNN search by design", EmbedRole::Passage).await.unwrap();
        let fact_record = NewMemoryRecord {
            scope: "agent",
            agent_id: Some("agent-knn".to_string()),
            project_id: None,
            content: "tier=1 rows are excluded from KNN search by design".to_string(),
            memory_type: MemoryType::SemanticFact,
            source: MemorySource::Chat,
            created_by_type: CreatedByType::System,
            created_by_ref: "agent-knn".to_string(),
            importance: 6.0,
            created_at_ms: 0,
            disputed_against: None,
        };
        let fact_id = store.insert_memory_with_vector(fact_record, fact_emb.clone()).await.unwrap();

        let arch_hits = search_all_memories(&store, &arch_emb, None, "agent-knn", SearchScope::all_default(), 10)
            .await
            .unwrap();
        assert!(
            arch_hits.iter().any(|h| h.id == arch_id),
            "architectural_decision (tier=2) must be KNN-retrievable"
        );

        let fact_hits = search_all_memories(&store, &fact_emb, None, "agent-knn", SearchScope::all_default(), 10)
            .await
            .unwrap();
        assert!(
            fact_hits.iter().any(|h| h.id == fact_id),
            "semantic_fact (tier=3) must be KNN-retrievable"
        );
    }

    // A `user_preference` lands at tier 1 by type, and the workspace channel
    // only carries the ones at importance >= 8. Leaving tier 1 out of the KNN
    // pool therefore made every weaker preference unreachable through BOTH
    // channels, and made the contradiction detector — which reuses this same
    // search — structurally unable to fire against a preference, which is the
    // case it exists for. Both channels now see tier 1; not delivering the
    // same row twice is enforced in `prompt_builder`, against the ids the
    // workspace actually emitted.
    #[tokio::test]
    async fn user_preference_tier_1_is_retrievable_by_knn() {
        use crate::memory::search::{search_all_memories, SearchScope};

        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder = FakeEmbedder::default();
        let pref_emb = embedder.embed_with_role("Always use voseo", EmbedRole::Passage).await.unwrap();
        let pref_record = NewMemoryRecord {
            scope: "agent",
            agent_id: Some("agent-knn-pref".to_string()),
            project_id: None,
            content: "Always use voseo".to_string(),
            memory_type: MemoryType::UserPreference,
            source: MemorySource::Chat,
            created_by_type: CreatedByType::System,
            created_by_ref: "agent-knn-pref".to_string(),
            importance: 8.0,
            created_at_ms: 0,
            disputed_against: None,
        };
        let pref_id = store.insert_memory_with_vector(pref_record, pref_emb.clone()).await.unwrap();

        let hits = search_all_memories(&store, &pref_emb, None, "agent-knn-pref", SearchScope::all_default(), 10)
            .await
            .unwrap();
        let hit = hits
            .iter()
            .find(|h| h.id == pref_id)
            .expect("user_preference must be reachable by KNN");
        assert_eq!(hit.tier, 1, "the point of this test is that the row sits at tier 1");
    }
}
