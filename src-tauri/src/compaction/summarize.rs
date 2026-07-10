use crate::models::chat::{ChatMessage, Role};

/// Generates a heuristic summary of removed messages.
/// Does NOT call the LLM — deterministic and free.
pub fn summarize_messages(messages: &[ChatMessage]) -> String {
    let user_count = messages
        .iter()
        .filter(|m| matches!(m.role, Role::User))
        .count();
    let assistant_count = messages
        .iter()
        .filter(|m| matches!(m.role, Role::Assistant))
        .count();
    let tool_count = messages
        .iter()
        .filter(|m| matches!(m.role, Role::Tool))
        .count();

    let mut lines = vec![
        "<summary>".to_string(),
        "Conversation summary:".to_string(),
        format!(
            "- Scope: {} messages compacted (user={}, assistant={}, tool={}).",
            messages.len(),
            user_count,
            assistant_count,
            tool_count
        ),
    ];

    // Tools used
    let tool_names = collect_tool_names(messages);
    if !tool_names.is_empty() {
        lines.push(format!("- Tools used: {}.", tool_names.join(", ")));
    }

    // Last 3 user requests
    let recent_requests = collect_recent_user_requests(messages, 3);
    if !recent_requests.is_empty() {
        lines.push("- Recent user requests:".to_string());
        for req in &recent_requests {
            lines.push(format!("  - {}", truncate(req, 160)));
        }
    }

    // Referenced files (extracted from tool calls)
    let files = collect_referenced_files(messages);
    if !files.is_empty() {
        let display: Vec<_> = files.iter().take(8).cloned().collect();
        lines.push(format!("- Key files: {}.", display.join(", ")));
    }

    // Pending work (keywords: todo, next, pending, remaining)
    let pending = collect_pending_work(messages);
    if !pending.is_empty() {
        lines.push("- Pending work:".to_string());
        for item in &pending {
            lines.push(format!("  - {}", truncate(item, 160)));
        }
    }

    // Condensed timeline
    lines.push("- Key timeline:".to_string());
    for msg in messages {
        let role_label = match msg.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::System => "System",
            Role::Tool => "Tool",
        };
        let content = summarize_content(msg);
        lines.push(format!("  - {}: {}", role_label, truncate(&content, 160)));
    }

    lines.push("</summary>".to_string());
    lines.join("\n")
}

/// Builds the continuation message injected as System message.
pub fn build_continuation_message(summary: &str, has_preserved_messages: bool) -> String {
    let mut msg = format!(
        "This conversation was compacted to save context. \
         The summary below covers the earlier portion.\n\n{summary}"
    );

    if has_preserved_messages {
        msg.push_str("\n\nRecent messages are preserved verbatim.");
    }

    msg.push_str(
        "\nContinue naturally from where we left off. \
         Do not acknowledge this summary or recap what happened.",
    );

    msg
}

/// Merges an existing summary with a new one (re-compaction).
pub fn merge_summaries(existing: &str, new: &str) -> String {
    let mut lines = vec![
        "<summary>".to_string(),
        "Conversation summary:".to_string(),
        "- Previously compacted:".to_string(),
    ];

    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed == "<summary>"
            || trimmed == "</summary>"
            || trimmed == "Conversation summary:"
        {
            continue;
        }
        if !trimmed.is_empty() {
            lines.push(format!("  {}", trimmed));
        }
    }

    lines.push("- Newly compacted:".to_string());
    for line in new.lines() {
        let trimmed = line.trim();
        if trimmed == "<summary>"
            || trimmed == "</summary>"
            || trimmed == "Conversation summary:"
        {
            continue;
        }
        if !trimmed.is_empty() {
            lines.push(format!("  {}", trimmed));
        }
    }

    lines.push("</summary>".to_string());
    lines.join("\n")
}

// -- Private helpers --

fn collect_tool_names(messages: &[ChatMessage]) -> Vec<String> {
    let mut names: Vec<String> = messages
        .iter()
        .filter_map(|m| m.tool_calls.as_ref())
        .flatten()
        .map(|tc| tc.function.name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

fn collect_recent_user_requests(messages: &[ChatMessage], count: usize) -> Vec<String> {
    messages
        .iter()
        .rev()
        .filter(|m| matches!(m.role, Role::User) && !m.content.trim().is_empty())
        .take(count)
        .map(|m| m.content.clone())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn collect_referenced_files(messages: &[ChatMessage]) -> Vec<String> {
    let extensions = [
        ".rs", ".ts", ".tsx", ".js", ".json", ".md", ".toml", ".html", ".css",
    ];
    let mut files: Vec<String> = messages
        .iter()
        .filter_map(|m| m.tool_calls.as_ref())
        .flatten()
        .filter_map(|tc| {
            tc.function
                .arguments
                .get("path")
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .filter(|path| extensions.iter().any(|ext| path.ends_with(ext)))
        .collect();
    files.sort();
    files.dedup();
    files
}

fn collect_pending_work(messages: &[ChatMessage]) -> Vec<String> {
    let keywords = ["todo", "next", "pending", "remaining", "follow up"];
    messages
        .iter()
        .filter(|m| {
            let lower = m.content.to_lowercase();
            keywords.iter().any(|kw| lower.contains(kw))
        })
        .map(|m| m.content.clone())
        .collect()
}

fn summarize_content(msg: &ChatMessage) -> String {
    if let Some(calls) = &msg.tool_calls {
        let tool_parts: Vec<String> = calls
            .iter()
            .map(|tc| format!("{}({})", tc.function.name, tc.function.arguments))
            .collect();
        if msg.content.is_empty() {
            tool_parts.join(" | ")
        } else {
            format!("{} | {}", msg.content, tool_parts.join(" | "))
        }
    } else {
        msg.content.clone()
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::chat::{current_time_millis, generate_message_id, ToolCall, ToolFunction};

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
    fn test_summarize_includes_scope() {
        let msgs = vec![
            make_msg(Role::User, "do something"),
            make_msg(Role::Assistant, "done"),
            make_msg(Role::User, "another thing"),
        ];
        let summary = summarize_messages(&msgs);
        assert!(summary.contains("3 messages compacted"));
        assert!(summary.contains("user=2"));
        assert!(summary.contains("assistant=1"));
    }

    #[test]
    fn test_summarize_includes_tools() {
        let mut msg = make_msg(Role::Assistant, "");
        msg.tool_calls = Some(vec![ToolCall {
            id: "t1".to_string(),
            function: ToolFunction {
                name: "read_file".to_string(),
                arguments: serde_json::json!({"path": "/src/main.rs"}),
            },
        }]);
        let msgs = vec![make_msg(Role::User, "read the file"), msg];
        let summary = summarize_messages(&msgs);
        assert!(summary.contains("Tools used: read_file"));
    }

    #[test]
    fn test_summarize_includes_user_requests() {
        let msgs = vec![
            make_msg(Role::User, "first request"),
            make_msg(Role::Assistant, "ok"),
            make_msg(Role::User, "second request"),
            make_msg(Role::Assistant, "done"),
            make_msg(Role::User, "third request"),
            make_msg(Role::Assistant, "finished"),
            make_msg(Role::User, "fourth request"),
        ];
        let summary = summarize_messages(&msgs);
        assert!(summary.contains("second request"));
        assert!(summary.contains("third request"));
        assert!(summary.contains("fourth request"));
        // First request should NOT be in "Recent user requests" (only last 3)
    }

    #[test]
    fn test_summarize_includes_files() {
        let mut msg = make_msg(Role::Assistant, "");
        msg.tool_calls = Some(vec![ToolCall {
            id: "t1".to_string(),
            function: ToolFunction {
                name: "read_file".to_string(),
                arguments: serde_json::json!({"path": "/src/lib.rs"}),
            },
        }]);
        let msgs = vec![msg];
        let summary = summarize_messages(&msgs);
        assert!(summary.contains("Key files: /src/lib.rs"));
    }

    #[test]
    fn test_summarize_truncates_long_content() {
        let long_content = "x".repeat(500);
        let msgs = vec![make_msg(Role::User, &long_content)];
        let summary = summarize_messages(&msgs);
        // Timeline entry should be truncated
        assert!(summary.contains("..."));
    }

    #[test]
    fn test_continuation_message_format() {
        let summary = "<summary>\ntest\n</summary>";
        let msg = build_continuation_message(summary, true);
        assert!(msg.contains("compacted to save context"));
        assert!(msg.contains("<summary>"));
        assert!(msg.contains("preserved verbatim"));
        assert!(msg.contains("Continue naturally"));
    }

    #[test]
    fn test_merge_preserves_previous() {
        let existing = "<summary>\nConversation summary:\n- Scope: 5 messages.\n</summary>";
        let new = "<summary>\nConversation summary:\n- Scope: 3 messages.\n</summary>";
        let merged = merge_summaries(existing, new);
        assert!(merged.contains("Previously compacted"));
        assert!(merged.contains("Newly compacted"));
        assert!(merged.contains("5 messages"));
        assert!(merged.contains("3 messages"));
    }

    #[test]
    fn test_summarize_empty_messages() {
        let msgs: Vec<ChatMessage> = vec![];
        let summary = summarize_messages(&msgs);
        assert!(summary.contains("<summary>"));
        assert!(summary.contains("0 messages compacted"));
        assert!(summary.contains("</summary>"));
    }

    #[test]
    fn test_pending_work_detection() {
        let msgs = vec![
            make_msg(Role::User, "the next step is to add tests"),
            make_msg(Role::Assistant, "I'll do that"),
            make_msg(Role::User, "also todo: fix the build"),
        ];
        let summary = summarize_messages(&msgs);
        assert!(summary.contains("Pending work"));
        assert!(summary.contains("next step"));
        assert!(summary.contains("todo: fix"));
    }
}
