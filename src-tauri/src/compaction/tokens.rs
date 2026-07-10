use crate::models::ChatMessage;

/// Estimates tokens for a single message. Heuristic: ~4 chars per token.
pub fn estimate_message_tokens(message: &ChatMessage) -> usize {
    let content_len = message.content.len();

    let tool_len = message
        .tool_calls
        .as_ref()
        .map(|calls| {
            calls
                .iter()
                .map(|tc| tc.function.name.len() + tc.function.arguments.to_string().len())
                .sum::<usize>()
        })
        .unwrap_or(0);

    (content_len + tool_len) / 4 + 1
}

/// Estimates total tokens for a message history.
pub fn estimate_history_tokens(messages: &[ChatMessage]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::chat::{current_time_millis, generate_message_id, Role, ToolCall, ToolFunction};

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

    #[test]
    fn test_estimate_short_message() {
        let msg = make_msg(Role::User, "hello");
        let tokens = estimate_message_tokens(&msg);
        // "hello" = 5 chars → 5/4 + 1 = 2
        assert_eq!(tokens, 2);
    }

    #[test]
    fn test_estimate_empty_message() {
        let msg = make_msg(Role::User, "");
        let tokens = estimate_message_tokens(&msg);
        // 0/4 + 1 = 1 minimum
        assert_eq!(tokens, 1);
    }

    #[test]
    fn test_estimate_with_tool_calls() {
        let mut msg = make_msg(Role::Assistant, "");
        msg.tool_calls = Some(vec![ToolCall {
            id: "t1".to_string(),
            function: ToolFunction {
                name: "read_file".to_string(),
                arguments: serde_json::json!({"path": "/src/main.rs"}),
            },
        }]);
        let tokens = estimate_message_tokens(&msg);
        // content=0, tool: "read_file"=9 + json_string_len > 0
        assert!(tokens > 1);
    }

    #[test]
    fn test_history_tokens_sums_all() {
        let msgs = vec![
            make_msg(Role::User, "hello world"),
            make_msg(Role::Assistant, "hi there, how can I help?"),
        ];
        let total = estimate_history_tokens(&msgs);
        let sum: usize = msgs.iter().map(|m| estimate_message_tokens(m)).sum();
        assert_eq!(total, sum);
    }
}
