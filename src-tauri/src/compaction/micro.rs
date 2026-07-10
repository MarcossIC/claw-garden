use crate::models::{ChatMessage, Role};

/// Tool results smaller than this are left untouched.
const TRUNCATE_THRESHOLD_CHARS: usize = 500;

/// Preview length preserved for truncated results (≤500 chars).
const PREVIEW_CHARS: usize = 200;

/// Tool results larger than this get cleared entirely (only metadata kept).
const AGGRESSIVE_THRESHOLD_CHARS: usize = 5_000;

/// How many recent messages to leave untouched.
const PRESERVE_RECENT: usize = 6;

/// Micro-compact: truncate old, oversized tool results in-place.
///
/// Operates on a working copy of the history — it NEVER removes messages,
/// only shrinks their content. This keeps the conversation structure intact
/// while cutting the dominant source of token waste (large file reads).
///
/// Returns `(modified_count, estimated_tokens_freed)`.
pub fn micro_compact_tool_results(messages: &mut [ChatMessage]) -> (usize, usize) {
    if messages.len() <= PRESERVE_RECENT {
        return (0, 0);
    }

    let cutoff = messages.len() - PRESERVE_RECENT;
    let mut modified = 0;
    let mut tokens_freed = 0;

    for msg in &mut messages[..cutoff] {
        if !matches!(msg.role, Role::Tool) {
            continue;
        }

        let content_len = msg.content.len();
        if content_len <= TRUNCATE_THRESHOLD_CHARS {
            continue;
        }

        let tool_name = msg
            .tool_calls
            .as_ref()
            .and_then(|tc| tc.first())
            .map(|tc| tc.function.name.as_str())
            .unwrap_or("tool");

        let replacement = if content_len > AGGRESSIVE_THRESHOLD_CHARS {
            // Large file reads: keep only metadata label
            format!("[{tool_name} result cleared — original {content_len} chars]")
        } else {
            // Medium results: keep a short preview for context
            let end = find_truncation_point(&msg.content, PREVIEW_CHARS);
            let preview = &msg.content[..end];
            format!("[{tool_name} result truncated — original {content_len} chars]\n{preview}\n[...]")
        };

        let freed = content_len.saturating_sub(replacement.len()) / 4;
        tokens_freed += freed;
        msg.content = replacement;
        modified += 1;
    }

    (modified, tokens_freed)
}

/// Find a UTF-8–safe truncation point near `target`, preferring newline breaks.
fn find_truncation_point(s: &str, target: usize) -> usize {
    if target >= s.len() {
        return s.len();
    }
    let mut pos = target;
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    // Prefer breaking at a newline for cleaner output
    if let Some(nl) = s[..pos].rfind('\n') {
        if nl > target / 2 {
            return nl + 1;
        }
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ToolCall, ToolFunction};
    use crate::models::chat::{current_time_millis, generate_message_id};

    fn tool_msg(content: &str, name: &str) -> ChatMessage {
        ChatMessage {
            id: generate_message_id(),
            role: Role::Tool,
            content: content.to_string(),
            tool_calls: Some(vec![ToolCall {
                id: "toolu_test".to_string(),
                function: ToolFunction {
                    name: name.to_string(),
                    arguments: serde_json::Value::Object(Default::default()),
                },
            }]),
            timestamp_ms: current_time_millis(),
            is_error: None,
        }
    }

    fn user_msg(content: &str) -> ChatMessage {
        ChatMessage {
            id: generate_message_id(),
            role: Role::User,
            content: content.to_string(),
            tool_calls: None,
            timestamp_ms: current_time_millis(),
            is_error: None,
        }
    }

    #[test]
    fn does_nothing_when_few_messages() {
        let mut msgs: Vec<ChatMessage> = (0..PRESERVE_RECENT)
            .map(|_| tool_msg(&"x".repeat(1000), "read_file"))
            .collect();
        let (modified, freed) = micro_compact_tool_results(&mut msgs);
        assert_eq!(modified, 0);
        assert_eq!(freed, 0);
    }

    #[test]
    fn does_nothing_to_small_tool_results() {
        let mut msgs: Vec<ChatMessage> = (0..10)
            .map(|i| {
                if i < 4 {
                    tool_msg("small content", "read_file")
                } else {
                    user_msg("hi")
                }
            })
            .collect();
        let (modified, _) = micro_compact_tool_results(&mut msgs);
        assert_eq!(modified, 0);
    }

    #[test]
    fn truncates_medium_tool_results() {
        let big_content = "x".repeat(2_000);
        let mut msgs: Vec<ChatMessage> = vec![
            tool_msg(&big_content, "read_file"),
            user_msg("a"),
            user_msg("b"),
            user_msg("c"),
            user_msg("d"),
            user_msg("e"),
            user_msg("f"),
        ];
        let (modified, freed) = micro_compact_tool_results(&mut msgs);
        assert_eq!(modified, 1);
        assert!(freed > 0);
        assert!(msgs[0].content.contains("truncated"));
        assert!(msgs[0].content.len() < big_content.len());
    }

    #[test]
    fn clears_large_tool_results() {
        let huge_content = "x".repeat(10_000);
        let mut msgs: Vec<ChatMessage> = vec![
            tool_msg(&huge_content, "read_file"),
            user_msg("a"),
            user_msg("b"),
            user_msg("c"),
            user_msg("d"),
            user_msg("e"),
            user_msg("f"),
        ];
        let (modified, freed) = micro_compact_tool_results(&mut msgs);
        assert_eq!(modified, 1);
        assert!(freed > 0);
        assert!(msgs[0].content.contains("cleared"));
        assert!(!msgs[0].content.contains("x".repeat(100).as_str()));
    }

    #[test]
    fn preserves_recent_messages_untouched() {
        let big = "x".repeat(10_000);
        let mut msgs: Vec<ChatMessage> = Vec::new();
        // Old messages
        for _ in 0..4 {
            msgs.push(user_msg("old"));
        }
        // Recent tool results (within PRESERVE_RECENT)
        for _ in 0..PRESERVE_RECENT {
            msgs.push(tool_msg(&big, "read_file"));
        }
        let (modified, _) = micro_compact_tool_results(&mut msgs);
        assert_eq!(modified, 0, "recent tool results must not be touched");
    }
}
