/// Compaction configuration.
#[derive(Debug, Clone, Copy)]
pub struct CompactionConfig {
    /// Recent messages to preserve intact (default: 4).
    pub preserve_recent: usize,

    /// Estimated token threshold to trigger compaction.
    /// Used in manual compaction. In auto-compact this is ignored
    /// (compaction triggers whenever there are enough messages).
    pub max_estimated_tokens: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            preserve_recent: 4,
            max_estimated_tokens: 10_000,
        }
    }
}
