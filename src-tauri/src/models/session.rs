use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub version: u32,
    pub session_id: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub agents: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SessionRecord {
    #[serde(rename = "message")]
    Message { message: PersistedMessage },

    #[serde(rename = "compaction")]
    Compaction {
        count: u32,
        removed_message_count: usize,
        summary: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedMessage {
    pub id: String,
    pub role: super::chat::Role,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<super::chat::ToolCall>>,
    pub timestamp_ms: u64,
}
