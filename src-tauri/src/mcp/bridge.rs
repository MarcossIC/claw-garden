use std::sync::Arc;
use dashmap::DashMap;
use super::circuit_breaker::{BreakerCheck, CircuitBreaker};
use crate::tools::errors::ToolErrorMessage;

/// Extracts the server name from a namespaced MCP tool name.
/// "mcp__filesystem__read_file" → Some("filesystem")
pub fn extract_server_name(tool_name: &str) -> Option<&str> {
    if let Some(rest) = tool_name.strip_prefix("mcp__") {
        let parts: Vec<&str> = rest.splitn(2, "__").collect();
        parts.first().copied()
    } else {
        None
    }
}

/// Constructs a namespaced MCP tool name.
/// ("filesystem", "read_file") → "mcp__filesystem__read_file"
pub fn mcp_tool_name(server: &str, tool: &str) -> String {
    format!("mcp__{server}__{tool}")
}

/// Bridge between the ToolExecutor and MCP servers.
/// Handles circuit breaker logic per server.
pub struct McpToolBridge {
    /// One circuit breaker per MCP server.
    breakers: DashMap<String, Arc<CircuitBreaker>>,
}

impl McpToolBridge {
    pub fn new() -> Self {
        Self {
            breakers: DashMap::new(),
        }
    }

    /// Get or create the circuit breaker for a server.
    pub fn breaker_for(&self, server_name: &str) -> Arc<CircuitBreaker> {
        self.breakers
            .entry(server_name.to_owned())
            .or_insert_with(|| Arc::new(CircuitBreaker::new(server_name)))
            .clone()
    }

    /// Check if a call should proceed, be rejected (circuit open), or use probe.
    /// Call this BEFORE making the actual MCP call.
    pub fn pre_call_check(&self, tool_name: &str) -> Result<(), ToolErrorMessage> {
        let server_name = match extract_server_name(tool_name) {
            Some(s) => s,
            None => return Ok(()), // Not an MCP tool
        };

        let breaker = self.breaker_for(server_name);
        match breaker.check() {
            BreakerCheck::Closed | BreakerCheck::HalfOpen => Ok(()),
            BreakerCheck::Open { retry_after } => Err(ToolErrorMessage::circuit_open(
                server_name,
                retry_after.as_secs(),
                &["read_file", "write_file", "glob_search", "grep_search"],
            )),
        }
    }

    /// Record the result of an MCP call (for circuit breaker tracking).
    /// `is_transient_error` should be true only for Timeout/ConnectionError.
    pub fn record_call_result(&self, tool_name: &str, success: bool, is_transient_error: bool) {
        let server_name = match extract_server_name(tool_name) {
            Some(s) => s,
            None => return,
        };
        let breaker = self.breaker_for(server_name);
        if success {
            breaker.record_success();
        } else if is_transient_error {
            breaker.record_failure();
        }
        // Non-transient errors (NotFound, InvalidInput) don't affect the breaker
    }
}

impl Default for McpToolBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_server_name_works() {
        assert_eq!(extract_server_name("mcp__filesystem__read_file"), Some("filesystem"));
        assert_eq!(extract_server_name("mcp__git__commit"), Some("git"));
        assert_eq!(extract_server_name("read_file"), None);
    }

    #[test]
    fn mcp_tool_name_works() {
        assert_eq!(mcp_tool_name("filesystem", "read_file"), "mcp__filesystem__read_file");
    }

    #[test]
    fn bridge_opens_circuit_after_failures() {
        let bridge = McpToolBridge::new();
        for _ in 0..5 {
            bridge.record_call_result("mcp__test__call", false, true);
        }
        let result = bridge.pre_call_check("mcp__test__call");
        assert!(result.is_err());
    }
}
