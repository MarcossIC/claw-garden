mod config;
pub mod micro;
mod result;
mod summarize;
mod tokens;

pub use config::CompactionConfig;
pub use micro::micro_compact_tool_results;
pub use result::CompactionResult;
pub use tokens::{estimate_history_tokens, estimate_message_tokens};

use crate::models::chat::{current_time_millis, generate_message_id};
use crate::models::{ChatMessage, Role};
use summarize::{build_continuation_message, merge_summaries, summarize_messages};

/// Default auto-compaction threshold (estimated tokens).
pub const DEFAULT_AUTO_COMPACT_THRESHOLD: usize = 80_000;

/// Suggested compaction threshold for a given provider/model combination.
/// These are conservative — we compact BEFORE hitting the real limit
/// to leave room for the model's response.
pub fn suggested_threshold_for_provider(provider: &str, model: &str) -> usize {
    match provider {
        "ollama" => {
            // Local models vary widely. Conservative default.
            // llama3.1: 128K context, but quality degrades much earlier.
            30_000
        }
        "anthropic" => {
            // Claude: 200K context. Compact at 80K to leave margin.
            80_000
        }
        "openrouter" => {
            if model.contains("claude") {
                80_000
            } else if model.contains("gpt-4") {
                60_000
            } else {
                40_000
            }
        }
        _ => DEFAULT_AUTO_COMPACT_THRESHOLD,
    }
}

/// Compacts a message history if it exceeds the configured thresholds.
///
/// Returns `None` if there's nothing to compact.
pub fn compact_history(
    messages: &[ChatMessage],
    config: CompactionConfig,
) -> Option<CompactionResult> {
    // Is there a prior summary at messages[0]?
    let has_prior_summary = messages
        .first()
        .map(|m| matches!(m.role, Role::System) && m.content.contains("<summary>"))
        .unwrap_or(false);

    let prefix_len = if has_prior_summary { 1 } else { 0 };
    let compactable = &messages[prefix_len..];

    // Worth compacting?
    if compactable.len() <= config.preserve_recent {
        return None;
    }

    let estimated = estimate_history_tokens(compactable);
    if estimated < config.max_estimated_tokens {
        return None;
    }

    // Split messages
    let keep_from = messages.len().saturating_sub(config.preserve_recent);
    let removed = &messages[prefix_len..keep_from];
    let preserved = &messages[keep_from..];

    if removed.is_empty() {
        return None;
    }

    // Generate summary
    let new_summary = summarize_messages(removed);
    // Clone the disjoint delta BEFORE the if/else below — the `else` branch MOVES
    // `new_summary` into `final_summary` on first compaction, so a naive read
    // after this point would not compile.
    let delta = new_summary.clone();
    let final_summary = if has_prior_summary {
        merge_summaries(&messages[0].content, &new_summary)
    } else {
        new_summary
    };

    let continuation = build_continuation_message(&final_summary, !preserved.is_empty());

    // Build compacted history
    let mut compacted = Vec::with_capacity(1 + preserved.len());
    compacted.push(ChatMessage {
        id: generate_message_id(),
        role: Role::System,
        content: continuation,
        tool_calls: None,
        timestamp_ms: current_time_millis(),
        is_error: None,
    });
    compacted.extend_from_slice(preserved);

    Some(CompactionResult {
        summary: final_summary,
        compacted_messages: compacted,
        removed_count: removed.len(),
        new_summary: delta,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::chat::{current_time_millis, generate_message_id};

    fn make_msg(role: Role, content: &str) -> ChatMessage {
        ChatMessage {
            id: generate_message_id(),
            role,
            content: content.to_string(),
            tool_calls: None,
            timestamp_ms: current_time_millis(),
            is_error: None,
        }
    }

    fn make_long_msg(role: Role, repeat: usize) -> ChatMessage {
        make_msg(role, &"x".repeat(repeat))
    }

    #[test]
    fn test_threshold_ollama_conservative() {
        let ollama = suggested_threshold_for_provider("ollama", "llama3.1");
        let anthropic = suggested_threshold_for_provider("anthropic", "claude-3.5-sonnet");
        assert!(ollama < anthropic);
    }

    #[test]
    fn test_threshold_anthropic() {
        assert_eq!(suggested_threshold_for_provider("anthropic", "claude-3"), 80_000);
    }

    #[test]
    fn test_compact_returns_none_below_threshold() {
        let msgs = vec![
            make_msg(Role::User, "hi"),
            make_msg(Role::Assistant, "hello"),
        ];
        let config = CompactionConfig::default();
        assert!(compact_history(&msgs, config).is_none());
    }

    #[test]
    fn test_compact_returns_none_few_messages() {
        let msgs = vec![
            make_long_msg(Role::User, 50_000),
            make_long_msg(Role::Assistant, 50_000),
        ];
        // Only 2 messages, preserve_recent=4 → nothing to remove
        let config = CompactionConfig {
            preserve_recent: 4,
            max_estimated_tokens: 1_000,
        };
        assert!(compact_history(&msgs, config).is_none());
    }

    #[test]
    fn test_compact_preserves_recent_n() {
        let msgs: Vec<_> = (0..10)
            .map(|i| make_long_msg(Role::User, 5_000 * (i + 1)))
            .collect();

        let config = CompactionConfig {
            preserve_recent: 4,
            max_estimated_tokens: 100, // Force compaction
        };

        let result = compact_history(&msgs, config).expect("should compact");

        // messages[0] is system summary, rest are preserved
        assert_eq!(result.compacted_messages.len(), 5); // 1 summary + 4 preserved
        assert!(matches!(result.compacted_messages[0].role, Role::System));
        assert!(result.compacted_messages[0].content.contains("<summary>"));
    }

    #[test]
    fn test_compact_summary_at_position_zero() {
        let msgs: Vec<_> = (0..8)
            .map(|_| make_long_msg(Role::User, 10_000))
            .collect();

        let config = CompactionConfig {
            preserve_recent: 2,
            max_estimated_tokens: 100,
        };

        let result = compact_history(&msgs, config).unwrap();
        assert!(matches!(result.compacted_messages[0].role, Role::System));
        assert!(result.compacted_messages[0].content.contains("compacted"));
    }

    #[test]
    fn test_compact_removed_count_correct() {
        let msgs: Vec<_> = (0..10)
            .map(|_| make_long_msg(Role::User, 5_000))
            .collect();

        let config = CompactionConfig {
            preserve_recent: 3,
            max_estimated_tokens: 100,
        };

        let result = compact_history(&msgs, config).unwrap();
        assert_eq!(result.removed_count, 7); // 10 - 3 preserved
    }

    #[test]
    fn test_recompaction_merges_summaries() {
        // First: a system message with prior summary
        let mut msgs = vec![make_msg(
            Role::System,
            "<summary>\nConversation summary:\n- Scope: 5 messages compacted.\n</summary>",
        )];
        // Then enough messages to trigger again
        for _ in 0..8 {
            msgs.push(make_long_msg(Role::User, 10_000));
        }

        let config = CompactionConfig {
            preserve_recent: 2,
            max_estimated_tokens: 100,
        };

        let result = compact_history(&msgs, config).unwrap();
        let summary_content = &result.compacted_messages[0].content;
        assert!(summary_content.contains("Previously compacted"));
        assert!(summary_content.contains("Newly compacted"));
    }

    // ── T1-test (REQ-1): CompactionResult.new_summary exposes the disjoint delta ──

    #[test]
    fn test_new_summary_equals_summary_on_first_compaction() {
        let msgs: Vec<_> = (0..8)
            .map(|_| make_long_msg(Role::User, 10_000))
            .collect();

        let config = CompactionConfig {
            preserve_recent: 2,
            max_estimated_tokens: 100,
        };

        let result = compact_history(&msgs, config).unwrap();
        assert_eq!(
            result.new_summary, result.summary,
            "first compaction: delta must equal the full summary"
        );
    }

    #[test]
    fn test_new_summary_excludes_prior_summary_on_recompaction() {
        let mut msgs = vec![make_msg(
            Role::System,
            "<summary>\nConversation summary:\n- Scope: 5 messages compacted.\n</summary>",
        )];
        for _ in 0..8 {
            msgs.push(make_long_msg(Role::User, 10_000));
        }

        let config = CompactionConfig {
            preserve_recent: 2,
            max_estimated_tokens: 100,
        };

        let result = compact_history(&msgs, config).unwrap();
        assert!(
            !result.new_summary.contains("Scope: 5 messages compacted"),
            "new_summary (delta) must NOT contain the prior summary's text: {}",
            result.new_summary
        );
        assert!(
            result.summary.contains("Previously compacted") && result.summary.contains("Newly compacted"),
            "summary (merged) must still contain both prior and new content"
        );
    }
}
