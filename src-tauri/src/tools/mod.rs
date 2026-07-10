pub mod builtin;
pub mod cache;
pub mod concurrency;
pub mod dispatch;
pub mod errors;
pub mod executor;
pub mod file_locks;
pub mod permission;
pub mod progress;
pub mod rate_limiter;
pub mod registry;
pub mod storage;
pub mod validation;

pub use errors::{ErrorKind, ToolErrorMessage};
pub use permission::PermissionEnforcer;
pub use progress::ToolProgress;
pub use registry::{GlobalToolRegistry, SessionToolSet};

use std::time::Duration;
use serde_json::Value;

/// Full specification of a native tool. Includes API contract + runtime metadata.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    // API contract (visible to the model)
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,

    // Runtime enforcement (invisible to model)
    pub required_permission: PermissionMode,
    pub is_concurrency_safe: bool,
    pub max_result_size_bytes: Option<usize>,
    pub max_execution_time: Option<Duration>,

    // Hook matching field
    pub input_match_field: Option<&'static str>,

    // Caching
    pub cache_policy: Option<CachePolicy>,

    // Rate limiting
    pub rate_limit: Option<RateLimit>,

    // Retry policy
    pub retry_policy: Option<RetryPolicy>,

    // UI metadata
    pub is_read_only: bool,
    pub is_destructive: bool,
    pub interrupt_behavior: InterruptBehavior,
}

impl ToolSpec {
    /// Convert to the API-compatible ToolDefinition (what gets sent to the LLM).
    pub fn to_definition(&self) -> crate::models::ToolDefinition {
        crate::models::ToolDefinition {
            tool_type: "function".to_string(),
            function: crate::models::ToolFunctionDef {
                name: self.name.to_string(),
                description: self.description.to_string(),
                parameters: self.input_schema.clone(),
            },
        }
    }
}

/// Permission levels. Ordered: ReadOnly < WorkspaceWrite < DangerFullAccess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PermissionMode {
    ReadOnly = 0,
    WorkspaceWrite = 1,
    DangerFullAccess = 2,
}

/// Cache invalidation strategy for tool results.
#[derive(Debug, Clone)]
pub enum CachePolicy {
    /// Validate by checking file mtime. For single-file tools (read_file, list_directory).
    Mtime,
    /// Expire after a fixed duration. For multi-file tools (glob, grep) and web_fetch.
    Ttl(Duration),
}

/// Rate limiting policy. Token bucket, per-agent state.
#[derive(Debug, Clone)]
pub struct RateLimit {
    /// Bucket capacity (= max burst).
    pub max_calls: u32,
    /// Time to fully refill the bucket (defines calls/second rate).
    pub window: Duration,
    /// Max time to wait for a token before returning error.
    pub max_wait: Duration,
}

/// Retry policy for transient failures.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_delay: Duration,
    pub backoff: BackoffStrategy,
    pub retryable: RetryableErrors,
}

#[derive(Debug, Clone, Copy)]
pub enum BackoffStrategy {
    Exponential,
    Fixed,
}

#[derive(Debug, Clone, Copy)]
pub enum RetryableErrors {
    /// Only Timeout and NetworkError.
    Transient,
}

#[derive(Debug, Clone, Copy)]
pub enum InterruptBehavior {
    Cancel,
    Block,
}
