/// Progress event emitted by a tool during long-running execution.
/// Used to update the UI (e.g. animate the agent) while waiting for the result.
#[derive(Debug, Clone)]
pub enum ToolProgress {
    BashProgress {
        stdout_chunk: String,
        stderr_chunk: String,
    },
    WebFetchProgress {
        bytes_read: u64,
        url: String,
    },
    Generic {
        message: String,
    },
}
