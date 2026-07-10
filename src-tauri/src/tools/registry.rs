use crate::models::ToolDefinition;
use super::{ToolSpec, builtin::builtin_tools, rate_limiter::RateLimiter};

/// Global registry. Holds built-in tools. MCP tools added at session build time.
pub struct GlobalToolRegistry {
    builtins: Vec<ToolSpec>,
}

impl GlobalToolRegistry {
    pub fn new() -> Self {
        Self {
            builtins: builtin_tools(),
        }
    }

    pub fn find(&self, name: &str) -> Option<&ToolSpec> {
        self.builtins.iter().find(|t| t.name == name)
    }

    /// Build a RateLimiter pre-populated with all per-tool limits declared in ToolSpec.
    /// Call once per agent at session start — RateLimiter state is per-agent.
    pub fn build_rate_limiter(&self) -> RateLimiter {
        let mut limiter = RateLimiter::new();
        for spec in &self.builtins {
            if let Some(rl) = &spec.rate_limit {
                limiter.register_limit(spec.name, rl.clone());
            }
        }
        limiter
    }

    /// Build a frozen tool set for a session. Built-ins come first, sorted by name.
    /// MCP tools are appended after with "mcp__{server}__{tool}" namespace.
    pub fn build_session_tool_set(&self, mcp_tools: Vec<ToolDefinition>) -> SessionToolSet {
        let mut definitions: Vec<ToolDefinition> = self.builtins
            .iter()
            .map(|t| t.to_definition())
            .collect();

        // Built-ins sorted first (stable prefix for prompt cache)
        definitions.sort_by(|a, b| a.function.name.cmp(&b.function.name));

        // MCP tools appended after, also sorted
        let mut mcp_sorted = mcp_tools;
        mcp_sorted.sort_by(|a, b| a.function.name.cmp(&b.function.name));
        definitions.extend(mcp_sorted);

        SessionToolSet { definitions }
    }
}

impl Default for GlobalToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Frozen tool set for a session. Built once, never mutated.
/// Guarantees stable JSON for prompt cache hits.
pub struct SessionToolSet {
    definitions: Vec<ToolDefinition>,
}

impl SessionToolSet {
    /// Returns the frozen set of tool definitions for the API payload.
    /// Always returns the same slice — no dynamic filtering.
    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.definitions
    }
}
