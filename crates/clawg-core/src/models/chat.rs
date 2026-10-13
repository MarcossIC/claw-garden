use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    #[serde(default = "generate_message_id")]
    pub id: String,
    pub role: Role,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default = "current_time_millis")]
    pub timestamp_ms: u64,
    /// Only `Some(true)` on `Role::Tool` messages whose underlying tool
    /// execution surfaced an error. Default `None` keeps previously persisted
    /// JSON deserializable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

pub fn generate_message_id() -> String {
    format!("msg-{}", uuid::Uuid::new_v4().as_simple())
}

pub fn current_time_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique ID for matching tool_use → tool_result (important for Anthropic API)
    #[serde(default = "generate_tool_id")]
    pub id: String,
    pub function: ToolFunction,
}

fn generate_tool_id() -> String {
    format!("toolu_{}", uuid::Uuid::new_v4().as_simple())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Events sent through a streaming channel.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(tag = "kind")]
pub enum StreamEvent {
    /// Embedder model bootstrap progress.
    ///
    /// Lifecycle on success: `start` -> `downloading` (1..N, rate-limited) ->
    /// `verifying` -> `done`. On error: any prefix, then exactly one terminal
    /// `error` event.
    #[serde(rename = "embedder_download")]
    EmbedderDownload {
        /// One of: `"start" | "downloading" | "verifying" | "done" | "error"`.
        phase: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        bytes_downloaded: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        bytes_total: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_error_field_defaults_to_none_on_missing_key() {
        let json = r#"{"role":"user","content":"hi","timestamp_ms":0}"#;
        let msg: ChatMessage = serde_json::from_str(json).expect("deserialize");
        assert_eq!(msg.is_error, None);
    }

    #[test]
    fn is_error_field_serializes_false_when_some_false() {
        let msg = ChatMessage {
            id: "x".into(),
            role: Role::Tool,
            content: "ok".into(),
            tool_calls: None,
            timestamp_ms: 0,
            is_error: Some(false),
        };
        let s = serde_json::to_string(&msg).expect("serialize");
        assert!(s.contains(r#""is_error":false"#), "got: {s}");
    }

    #[test]
    fn is_error_field_omitted_when_none() {
        let msg = ChatMessage {
            id: "x".into(),
            role: Role::User,
            content: "hi".into(),
            tool_calls: None,
            timestamp_ms: 0,
            is_error: None,
        };
        let s = serde_json::to_string(&msg).expect("serialize");
        assert!(!s.contains(r#""is_error""#), "is_error must be omitted when None: {s}");
    }

    #[test]
    fn is_error_field_some_true_serializes_correctly() {
        let msg = ChatMessage {
            id: "x".into(),
            role: Role::Tool,
            content: "fail".into(),
            tool_calls: None,
            timestamp_ms: 0,
            is_error: Some(true),
        };
        let s = serde_json::to_string(&msg).expect("serialize");
        assert!(s.contains(r#""is_error":true"#), "got: {s}");
    }

    #[test]
    fn embedder_download_serializes_skipping_none_fields() {
        let ev = StreamEvent::EmbedderDownload {
            phase: "downloading".into(),
            bytes_downloaded: Some(1024),
            bytes_total: Some(2048),
            error: None,
        };
        let s = serde_json::to_string(&ev).expect("serialize");
        assert!(s.contains(r#""kind":"embedder_download""#), "missing kind: {s}");
        assert!(s.contains(r#""phase":"downloading""#), "missing phase: {s}");
        assert!(s.contains(r#""bytes_downloaded":1024"#), "missing bytes_downloaded: {s}");
        assert!(s.contains(r#""bytes_total":2048"#), "missing bytes_total: {s}");
        assert!(!s.contains(r#""error""#), "error key must be omitted when None: {s}");
    }

    #[test]
    fn embedder_download_skips_all_optional_when_none() {
        let ev = StreamEvent::EmbedderDownload {
            phase: "verifying".into(),
            bytes_downloaded: None,
            bytes_total: None,
            error: None,
        };
        let s = serde_json::to_string(&ev).expect("serialize");
        // Must contain only kind + phase.
        assert!(s.contains(r#""kind":"embedder_download""#), "missing kind: {s}");
        assert!(s.contains(r#""phase":"verifying""#), "missing phase: {s}");
        assert!(!s.contains(r#""bytes_downloaded""#), "bytes_downloaded must be omitted: {s}");
        assert!(!s.contains(r#""bytes_total""#), "bytes_total must be omitted: {s}");
        assert!(!s.contains(r#""error""#), "error must be omitted: {s}");
    }

    #[test]
    fn embedder_download_roundtrip() {
        let ev = StreamEvent::EmbedderDownload {
            phase: "error".into(),
            bytes_downloaded: None,
            bytes_total: None,
            error: Some("boom".into()),
        };
        let s = serde_json::to_string(&ev).expect("serialize");
        let back: StreamEvent = serde_json::from_str(&s).expect("deserialize");
        let StreamEvent::EmbedderDownload { phase, bytes_downloaded, bytes_total, error } = back;
        assert_eq!(phase, "error");
        assert_eq!(bytes_downloaded, None);
        assert_eq!(bytes_total, None);
        assert_eq!(error, Some("boom".into()));
    }
}
