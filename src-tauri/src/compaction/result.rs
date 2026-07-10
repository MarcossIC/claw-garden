use crate::models::ChatMessage;

/// Result of a compaction operation.
#[derive(Debug, Clone)]
pub struct CompactionResult {
    /// Generated summary (plain text with <summary> tags).
    pub summary: String,

    /// Compacted messages (summary + preserved).
    pub compacted_messages: Vec<ChatMessage>,

    /// Number of removed messages.
    pub removed_count: usize,

    /// Disjoint delta summary — ONLY the newly-generated summary text for this
    /// compaction pass (pre-merge), distinct from `summary` which may contain a
    /// merged "Previously compacted" + "Newly compacted" combination on
    /// re-compaction. Used to feed the RAG memory pipeline without duplicating
    /// content that was already ingested on a prior compaction.
    pub new_summary: String,
}
