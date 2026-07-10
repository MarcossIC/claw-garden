use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

use super::{
    BackoffStrategy, GlobalToolRegistry, PermissionMode, RetryPolicy, ToolSpec,
    cache::TOOL_CACHE,
    concurrency::ConcurrencyGate,
    dispatch,
    errors::ToolErrorMessage,
    file_locks::FileLockRegistry,
    permission::PermissionEnforcer,
    progress::ToolProgress,
    rate_limiter::RateLimiter,
    storage::persist_if_large,
    validation::validate_input,
};

/// Context for a single agent executing a tool call.
pub struct AgentToolContext {
    pub agent_id: String,
    pub permission_mode: PermissionMode,
    pub project_path: Option<String>,
    pub session_dir: PathBuf,
    /// Optional channel for streaming progress events from long-running tools
    /// (bash stdout/stderr chunks, web_fetch byte counts). `None` = no streaming,
    /// tools produce only the final result. The caller owns the receiver and
    /// forwards to the UI; dropping the sender stops progress delivery.
    pub progress_tx: Option<UnboundedSender<ToolProgress>>,
}

/// Shared executor infrastructure (one per session/app instance).
pub struct ToolExecutor {
    pub registry: Arc<GlobalToolRegistry>,
    pub gate: Arc<ConcurrencyGate>,
    pub file_locks: Arc<FileLockRegistry>,
    pub http_client: reqwest::Client,
}

impl ToolExecutor {
    pub fn new(registry: Arc<GlobalToolRegistry>) -> Self {
        let file_locks = Arc::new(FileLockRegistry::new());

        // Background cleanup of path→RwLock entries nobody holds anymore.
        // Prevents unbounded growth of the lock map in long sessions.
        // Task uses a Weak<Self> internally so it stops when the registry is dropped.
        // Guarded by a Tokio-runtime check: in non-async test contexts the registry
        // is still fully functional without the cleanup task.
        if tokio::runtime::Handle::try_current().is_ok() {
            FileLockRegistry::spawn_cleanup_task(&file_locks);
        }

        Self {
            registry,
            gate: Arc::new(ConcurrencyGate::new()),
            file_locks,
            http_client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Execute a native tool call through the full pipeline.
    /// Returns (content_string, is_error).
    pub async fn execute(
        &self,
        tool_name: &str,
        input: &Value,
        agent_ctx: &AgentToolContext,
        rate_limiter: &RateLimiter,
    ) -> (String, bool) {
        match self.execute_inner(tool_name, input, agent_ctx, rate_limiter).await {
            Ok(content) => (content, false),
            Err(err) => (err.to_json_string(), true),
        }
    }

    async fn execute_inner(
        &self,
        tool_name: &str,
        input: &Value,
        agent_ctx: &AgentToolContext,
        rate_limiter: &RateLimiter,
    ) -> Result<String, ToolErrorMessage> {
        // Step a: find tool spec
        let spec = self.registry.find(tool_name)
            .ok_or_else(|| ToolErrorMessage::invalid_input(
                tool_name,
                &format!("unknown native tool '{tool_name}'"),
            ))?;

        // Step a: validate input against JSON schema
        validate_input(tool_name, &spec.input_schema, input)?;

        // Step b: permission check
        PermissionEnforcer::check(
            tool_name,
            spec.required_permission,
            agent_ctx.permission_mode,
        )?;

        // Step b2: cache lookup (read-only tools only)
        // A cache hit skips rate_limit, hooks, gate, dispatch, and post-hooks.
        if spec.is_read_only {
            if let Some(cached) = TOOL_CACHE.lookup(tool_name, input) {
                return Ok(cached);
            }
        }

        // Step c: rate limit (per-agent, token bucket)
        rate_limiter.consume(tool_name).await?;

        // Steps e+f+retry: concurrency gate → dispatch with timeout + retry policy.
        // The gate is acquired ONCE before the retry loop. Retries run inside the slot.
        let output = self.run_with_gate_and_retry(spec, tool_name, input, agent_ctx).await?;

        // Step g: cache store or invalidate
        if !spec.is_read_only {
            // Write tool: invalidate any cached read results that touched this path.
            if let Some(path_val) = input.get("path").and_then(|v| v.as_str()) {
                TOOL_CACHE.invalidate_touching(Path::new(path_val));
            }
        } else if let Some(policy) = &spec.cache_policy {
            let path = input.get("path").and_then(|v| v.as_str()).map(PathBuf::from);
            // touched_paths enables cross-tool cache invalidation:
            // write_file("src/main.rs") invalidates grep/glob results that included it.
            let touched_paths = extract_touched_paths(tool_name, input, &output);
            TOOL_CACHE.store(
                tool_name,
                input,
                output.clone(),
                policy.clone(),
                path,
                touched_paths,
            );
        }

        // Step h: persist to disk if output exceeds max_result_size_bytes.
        // Result is is_error: false — the tool succeeded, output is just large.
        let tool_use_id = uuid::Uuid::new_v4().simple().to_string();
        let content = persist_if_large(
            &output,
            spec.max_result_size_bytes,
            &agent_ctx.session_dir,
            &tool_use_id,
        );

        Ok(content.to_content_string())
    }

    /// Acquire the concurrency gate (read or write), then run dispatch with retry.
    /// The gate slot is held for the entire duration including backoff sleeps.
    async fn run_with_gate_and_retry(
        &self,
        spec: &ToolSpec,
        tool_name: &str,
        input: &Value,
        agent_ctx: &AgentToolContext,
    ) -> Result<String, ToolErrorMessage> {
        if spec.is_concurrency_safe {
            let _guard = self.gate.acquire_read().await;
            self.dispatch_with_retry(spec, tool_name, input, agent_ctx).await
        } else {
            let _guard = self.gate.acquire_write().await;
            // Non-idempotent tools (bash, write_file) never have retry_policy: None.
            // But we reuse the same path for consistency.
            self.dispatch_with_retry(spec, tool_name, input, agent_ctx).await
        }
    }

    /// Retry loop + per-call timeout. Runs inside an already-acquired gate slot.
    async fn dispatch_with_retry(
        &self,
        spec: &ToolSpec,
        tool_name: &str,
        input: &Value,
        agent_ctx: &AgentToolContext,
    ) -> Result<String, ToolErrorMessage> {
        let max_attempts = spec.retry_policy.as_ref().map_or(1, |p| p.max_attempts);

        let mut attempt = 0u32;
        loop {
            attempt += 1;

            // Dispatch with timeout enforcement from ToolSpec.max_execution_time.
            let result = self
                .dispatch_with_file_locks_and_timeout(spec, tool_name, input, agent_ctx)
                .await;

            match result {
                Ok(out) => return Ok(out),
                Err(e) => {
                    if let Some(policy) = &spec.retry_policy {
                        if e.is_transient() && attempt < max_attempts {
                            // Transient error with retries remaining — sleep and retry.
                            let delay = backoff_delay(policy, attempt);
                            tokio::time::sleep(delay).await;
                            continue;
                        }
                        if attempt >= max_attempts {
                            // All retry attempts exhausted — return a Timeout error so
                            // the model knows not to retry (is_retryable: false).
                            return Err(ToolErrorMessage::timeout(
                                tool_name,
                                spec.max_execution_time.map_or(120, |d| d.as_secs()),
                                attempt,
                            ));
                        }
                    }
                    // No retry policy, or non-transient error — fail immediately.
                    return Err(e);
                }
            }
        }
    }

    /// Single dispatch call with optional execution timeout from ToolSpec.
    /// File-level locks are acquired/released around the actual I/O.
    async fn dispatch_with_file_locks_and_timeout(
        &self,
        spec: &ToolSpec,
        tool_name: &str,
        input: &Value,
        agent_ctx: &AgentToolContext,
    ) -> Result<String, ToolErrorMessage> {
        let dispatch_fut = self.dispatch_with_file_locks(tool_name, input, agent_ctx);

        match spec.max_execution_time {
            Some(timeout) => tokio::time::timeout(timeout, dispatch_fut)
                .await
                // Elapsed error → Timeout with attempt=1 (callers handle attempt count).
                .map_err(|_| ToolErrorMessage::timeout(tool_name, timeout.as_secs(), 1))?,
            None => dispatch_fut.await,
        }
    }

    /// Acquire per-path file lock for the tool, then dispatch.
    async fn dispatch_with_file_locks(
        &self,
        tool_name: &str,
        input: &Value,
        agent_ctx: &AgentToolContext,
    ) -> Result<String, ToolErrorMessage> {
        let path = input.get("path").and_then(|v| v.as_str()).map(PathBuf::from);

        match tool_name {
            "read_file" | "list_directory" => {
                if let Some(p) = &path {
                    let _lock = self.file_locks.acquire_read(p).await;
                    dispatch_tool(tool_name, input, agent_ctx, &self.http_client).await
                } else {
                    dispatch_tool(tool_name, input, agent_ctx, &self.http_client).await
                }
            }
            "write_file" | "edit_file" => {
                if let Some(p) = &path {
                    let _lock = self.file_locks.acquire_write(p).await;
                    dispatch_tool(tool_name, input, agent_ctx, &self.http_client).await
                } else {
                    dispatch_tool(tool_name, input, agent_ctx, &self.http_client).await
                }
            }
            _ => dispatch_tool(tool_name, input, agent_ctx, &self.http_client).await,
        }
    }
}

async fn dispatch_tool(
    name: &str,
    input: &Value,
    ctx: &AgentToolContext,
    http_client: &reqwest::Client,
) -> Result<String, ToolErrorMessage> {
    // Only bash and web_fetch consume progress events today — the other tools
    // are fast enough that emitting incremental progress would be pure noise.
    let progress_tx = ctx.progress_tx.as_ref();

    match name {
        "read_file"      => dispatch::read_file(input).await,
        "write_file"     => dispatch::write_file(input).await,
        "edit_file"      => dispatch::edit_file(input).await,
        "glob_search"    => dispatch::glob_search(input).await,
        "grep_search"    => dispatch::grep_search(input).await,
        "bash"           => dispatch::bash(input, ctx.project_path.as_deref(), progress_tx).await,
        "list_directory" => dispatch::list_directory(input).await,
        "web_fetch"      => dispatch::web_fetch(input, http_client, progress_tx).await,
        other => Err(ToolErrorMessage::invalid_input(
            other,
            &format!("no native handler for '{other}'"),
        )),
    }
}

// ── Retry helpers ─────────────────────────────────────────────────────────────

/// Compute the sleep duration before the next attempt.
/// attempt is 1-indexed: first failure = attempt 1.
///   Exponential: 1s → 2s → 4s (initial * 2^(attempt-1))
///   Fixed:       always initial_delay
fn backoff_delay(policy: &RetryPolicy, attempt: u32) -> Duration {
    match policy.backoff {
        BackoffStrategy::Fixed => policy.initial_delay,
        BackoffStrategy::Exponential => {
            // Cap exponent at 10 (2^10 = 1024×) to prevent overflow.
            let exp = attempt.saturating_sub(1).min(10);
            let multiplier = 2u32.pow(exp);
            // saturating_mul prevents overflow for extreme values.
            policy.initial_delay.saturating_mul(multiplier)
        }
    }
}

// ── Cache touched_paths extraction ───────────────────────────────────────────

/// Extract the set of file paths that a tool's output "touches".
///
/// Used to populate `CacheEntry::touched_paths` so that subsequent write operations
/// can invalidate all cache entries that included the written file — including
/// multi-file tools like grep_search and glob_search.
///
/// Without this, write_file("src/main.rs") would only invalidate
/// read_file("src/main.rs") but NOT grep_search("fn main", "src/")
/// whose cached result included src/main.rs.
fn extract_touched_paths(tool_name: &str, input: &Value, output: &str) -> Vec<PathBuf> {
    let base = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");

    match tool_name {
        // Single-file tools: the path IS the only touched file.
        "read_file" | "list_directory" => {
            input.get("path")
                .and_then(|v| v.as_str())
                .map(|p| vec![PathBuf::from(p)])
                .unwrap_or_default()
        }

        // glob_search output: one relative path per line (e.g. "src/main.rs").
        // Lines starting with '[' are messages (no match / truncated), skip them.
        "glob_search" => {
            output
                .lines()
                .filter(|l| !l.starts_with('[') && !l.is_empty())
                .map(|l| PathBuf::from(base).join(l.trim()))
                .collect()
        }

        // grep_search output: "relative_path:line_num: content"
        // Parse the path prefix (everything before the first ":\d" sequence).
        "grep_search" => {
            let mut paths = HashSet::new();
            for line in output.lines() {
                if line.starts_with('[') || line.is_empty() {
                    continue;
                }
                if let Some(path_str) = parse_grep_path(line) {
                    paths.insert(PathBuf::from(base).join(path_str));
                }
            }
            paths.into_iter().collect()
        }

        _ => vec![],
    }
}

/// Parse the file path from a grep result line "path:line_num: content".
///
/// Strategy: scan for the first colon that is immediately followed by a decimal digit.
/// This avoids splitting on Windows drive letters while reliably finding the
/// line-number separator on any OS (grep results always use digits after the first colon).
///
/// Returns None if no such colon is found (e.g. the truncation message line).
fn parse_grep_path(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b':' {
            if let Some(&next) = bytes.get(i + 1) {
                if next.is_ascii_digit() {
                    return Some(&line[..i]);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── backoff_delay ─────────────────────────────────────────────────────────

    #[test]
    fn exponential_backoff_doubles_each_attempt() {
        let policy = RetryPolicy {
            max_attempts: 3,
            initial_delay: Duration::from_secs(1),
            backoff: BackoffStrategy::Exponential,
            retryable: super::super::RetryableErrors::Transient,
        };
        assert_eq!(backoff_delay(&policy, 1), Duration::from_secs(1)); // 1 * 2^0
        assert_eq!(backoff_delay(&policy, 2), Duration::from_secs(2)); // 1 * 2^1
        assert_eq!(backoff_delay(&policy, 3), Duration::from_secs(4)); // 1 * 2^2
    }

    #[test]
    fn fixed_backoff_is_constant() {
        let policy = RetryPolicy {
            max_attempts: 3,
            initial_delay: Duration::from_secs(2),
            backoff: BackoffStrategy::Fixed,
            retryable: super::super::RetryableErrors::Transient,
        };
        assert_eq!(backoff_delay(&policy, 1), Duration::from_secs(2));
        assert_eq!(backoff_delay(&policy, 2), Duration::from_secs(2));
        assert_eq!(backoff_delay(&policy, 3), Duration::from_secs(2));
    }

    // ── parse_grep_path ───────────────────────────────────────────────────────

    #[test]
    fn parse_grep_path_unix() {
        assert_eq!(
            parse_grep_path("src/main.rs:42: fn main() {"),
            Some("src/main.rs")
        );
    }

    #[test]
    fn parse_grep_path_nested() {
        assert_eq!(
            parse_grep_path("src/tools/mod.rs:10: pub fn foo() {"),
            Some("src/tools/mod.rs")
        );
    }

    #[test]
    fn parse_grep_path_skips_message_lines() {
        assert_eq!(parse_grep_path("[results truncated at 500 matches]"), None);
    }

    #[test]
    fn parse_grep_path_no_colon() {
        assert_eq!(parse_grep_path("no colon here"), None);
    }

    // ── extract_touched_paths ─────────────────────────────────────────────────

    #[test]
    fn glob_search_extracts_paths() {
        let input = serde_json::json!({"pattern": "**/*.rs", "path": "src"});
        let output = "main.rs\ntools/mod.rs\n[no more]";
        let paths = extract_touched_paths("glob_search", &input, output);
        assert!(paths.contains(&PathBuf::from("src/main.rs")));
        assert!(paths.contains(&PathBuf::from("src/tools/mod.rs")));
        // The "[no more]" message line must be skipped
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn grep_search_extracts_unique_paths() {
        let input = serde_json::json!({"pattern": "fn", "path": "src"});
        let output = "main.rs:1: fn main() {\nmain.rs:5: fn helper() {\nlib.rs:3: fn new() {";
        let paths = extract_touched_paths("grep_search", &input, output);
        // Two unique files
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&PathBuf::from("src/main.rs")));
        assert!(paths.contains(&PathBuf::from("src/lib.rs")));
    }
}
