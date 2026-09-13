//! Globset-based hook pattern matching for the Native ToolRegistry.
//!
//! Uses `globset` (the crate used by ripgrep) instead of regex:
//! - O(n) guaranteed — no backtracking, no ReDoS vulnerability
//! - Compiles patterns at startup, fails fast on invalid patterns
//! - Applies globs against input[tool.input_match_field], NOT full JSON

use globset::{GlobBuilder, GlobMatcher};

#[derive(Debug)]
pub enum HookMatcherError {
    UnbalancedParens(String),
    InvalidGlob { pattern: String, reason: String },
}

impl std::fmt::Display for HookMatcherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnbalancedParens(s) => write!(f, "unbalanced parentheses in pattern: {s}"),
            Self::InvalidGlob { pattern, reason } => {
                write!(f, "invalid glob pattern '{pattern}': {reason}")
            }
        }
    }
}

/// A compiled hook rule that matches tool calls by name and optionally by input field value.
/// Created ONCE at config load time — never in the hot path.
///
/// Pattern syntax: `"tool_name"` or `"tool_name(glob_pattern)"`
///
/// Examples:
///   `"write_file(src/**)"` → matches write_file when path starts with src/
///   `"bash(git *)"` → matches bash when command starts with git
///   `"web_fetch"` → matches any web_fetch call
#[derive(Debug)]
pub struct HookRule {
    pub tool_name: String,
    /// None = match any input (no filter). Some = apply glob to input[input_match_field].
    pub input_matcher: Option<GlobMatcher>,
}

impl HookRule {
    /// Parse and compile a pattern string.
    /// Fails at startup if the pattern is invalid — never fails in production.
    pub fn parse(s: &str) -> Result<Self, HookMatcherError> {
        if let Some((tool_part, rest)) = s.split_once('(') {
            let pattern = rest
                .strip_suffix(')')
                .ok_or_else(|| HookMatcherError::UnbalancedParens(s.to_owned()))?;

            let glob = GlobBuilder::new(pattern)
                .case_insensitive(false)
                .literal_separator(false) // * can cross separators (for commands)
                .build()
                .map_err(|e| HookMatcherError::InvalidGlob {
                    pattern: pattern.to_owned(),
                    reason: e.to_string(),
                })?;

            Ok(HookRule {
                tool_name: tool_part.trim().to_owned(),
                input_matcher: Some(glob.compile_matcher()),
            })
        } else {
            // No parens — match any call to this tool
            Ok(HookRule {
                tool_name: s.trim().to_owned(),
                input_matcher: None,
            })
        }
    }

    /// Returns true if this rule applies to the given tool call.
    ///
    /// `input_field_value` should be `input[tool.input_match_field]` (the specific field
    /// declared in ToolSpec), NOT the full JSON string. This prevents false matches against
    /// other fields like `{"command": "git push", "timeout": "rm -rf"}`.
    pub fn matches(&self, tool_name: &str, input_field_value: &str) -> bool {
        if self.tool_name != tool_name {
            return false;
        }
        match &self.input_matcher {
            None => true,
            Some(glob) => glob.is_match(input_field_value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_tool_name_matches_any_input() {
        let rule = HookRule::parse("bash").unwrap();
        assert!(rule.matches("bash", "git push"));
        assert!(rule.matches("bash", "rm -rf /"));
        assert!(!rule.matches("write_file", "anything"));
    }

    #[test]
    fn bash_git_star_matches_git_commands() {
        let rule = HookRule::parse("bash(git *)").unwrap();
        assert!(rule.matches("bash", "git push origin main"));
        assert!(rule.matches("bash", "git commit -m 'fix'"));
        assert!(!rule.matches("bash", "cargo build"));
        assert!(!rule.matches("write_file", "git push"));
    }

    #[test]
    fn write_file_src_double_star_matches_nested() {
        let rule = HookRule::parse("write_file(src/**)").unwrap();
        assert!(rule.matches("write_file", "src/main.rs"));
        assert!(rule.matches("write_file", "src/tools/mod.rs"));
        assert!(!rule.matches("write_file", "tests/main.rs"));
    }

    #[test]
    fn invalid_glob_errors_at_parse_time() {
        // Not a real invalid glob, but an unclosed paren should fail
        let result = HookRule::parse("bash(git");
        assert!(result.is_err());
    }

    #[test]
    fn unbalanced_parens_errors() {
        let result = HookRule::parse("bash(git *");
        assert!(result.is_err());
    }
}
