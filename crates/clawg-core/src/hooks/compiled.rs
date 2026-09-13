use std::collections::HashSet;
use std::time::Duration;

use regex::Regex;

use super::error::ConfigError;
use super::types::{AgentHookConfig, HookCommand, HookEvent, HookPhase, OnTimeoutPolicy};

// ── CompiledPattern ───────────────────────────────────────────────────────

/// Pre-compiled form of a pattern string (matcher or input_pattern in if_condition).
/// Created once when loading config; the instance is reused on every tool call.
#[derive(Debug, Clone)]
pub enum CompiledPattern {
    /// Absent or empty → always matches.
    Any,
    /// No wildcards, no delimiters → exact equality check, O(n).
    Exact(String),
    /// Contains `*` wildcards or `/regex/` delimiters → pre-compiled regex, O(n).
    Regex(Regex),
}

impl CompiledPattern {
    /// Builds a CompiledPattern from a raw string.
    ///
    /// Matching rules (in priority order):
    ///   - Empty or `"*"` → `Any`   (always matches)
    ///   - `/pattern/`    → `Regex`  (explicit regex, anchored by the caller's regex)
    ///   - Contains `*`   → `Regex`  (glob wildcard; all metacharacters are escaped
    ///                                per segment before joining with `.*`)
    ///   - Otherwise      → `Exact`  (literal equality — NO implicit substring)
    ///
    /// For substring matching use `"*keyword*"` or `"/.*keyword.*/"`.
    /// Implicit substring behavior is intentionally absent: `"Bash"` matches
    /// only `"Bash"`, NOT `"BashTool"`.
    ///
    /// Returns an error if the pattern is an invalid regex.
    pub fn compile(raw: &str) -> Result<Self, ConfigError> {
        Self::compile_impl(raw, true)
    }

    /// Compiles a pattern for substring/content matching (e.g. against JSON input).
    /// Wildcard globs are NOT anchored, so `git *` matches anywhere in the haystack.
    pub fn compile_for_content(raw: &str) -> Result<Self, ConfigError> {
        Self::compile_impl(raw, false)
    }

    fn compile_impl(raw: &str, anchored: bool) -> Result<Self, ConfigError> {
        if raw.is_empty() || raw == "*" {
            return Ok(Self::Any);
        }
        if raw.starts_with('/') && raw.ends_with('/') && raw.len() > 2 {
            let inner = &raw[1..raw.len() - 1];
            let re = Regex::new(inner).map_err(|e| ConfigError::InvalidPattern {
                raw: raw.to_string(),
                reason: e.to_string(),
            })?;
            return Ok(Self::Regex(re));
        }
        if raw.contains('*') {
            // Glob wildcard → regex.
            // Split on `*`, escape every literal segment individually so that
            // metacharacters like `.`, `+`, `(`, `[` etc. are treated as literals,
            // then re-join with `.*` to represent the wildcards.
            let regex_pat = raw
                .split('*')
                .map(regex::escape)
                .collect::<Vec<_>>()
                .join(".*");
            let pat = if anchored {
                format!("^{}$", regex_pat)
            } else {
                regex_pat
            };
            let re = Regex::new(&pat).map_err(|e| ConfigError::InvalidPattern {
                raw: raw.to_string(),
                reason: e.to_string(),
            })?;
            return Ok(Self::Regex(re));
        }
        // Default: exact match.
        // No implicit substring — unexpected partial matches like "Bash" hitting
        // "BashTool" are a common source of hard-to-find configuration bugs.
        Ok(Self::Exact(raw.to_string()))
    }

    pub fn matches(&self, value: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Exact(s) => s == value,
            Self::Regex(re) => re.is_match(value),
        }
    }
}

// ── CompiledIfCondition ───────────────────────────────────────────────────

/// Pre-compiled form of a "ToolName(input_pattern)" string.
/// tool_name_pattern can be exact ("Bash"), wildcard ("Bash*"), or regex ("/^Bash/").
/// input_pattern is optional and applied over the serialized JSON input of the tool.
#[derive(Debug, Clone)]
pub struct CompiledIfCondition {
    pub tool_name_pattern: CompiledPattern,
    pub input_pattern: Option<CompiledPattern>,
}

impl CompiledIfCondition {
    /// Parses and compiles "Bash(git *)" in a single pass.
    /// Returns an error if any pattern is an invalid regex.
    pub fn compile(raw: &str) -> Result<Self, ConfigError> {
        if let Some(paren) = raw.find('(') {
            if raw.ends_with(')') {
                let tool_part = raw[..paren].trim();
                let input_part = raw[paren + 1..raw.len() - 1].trim();
                return Ok(Self {
                    tool_name_pattern: CompiledPattern::compile(tool_part)?,
                    input_pattern: if input_part.is_empty() {
                        None
                    } else {
                        Some(CompiledPattern::compile_for_content(input_part)?)
                    },
                });
            }
        }
        Ok(Self {
            tool_name_pattern: CompiledPattern::compile(raw.trim())?,
            input_pattern: None,
        })
    }

    pub fn matches(&self, tool_name: &str, input_json: &str) -> bool {
        self.tool_name_pattern.matches(tool_name)
            && self
                .input_pattern
                .as_ref()
                .map_or(true, |p| p.matches(input_json))
    }
}

// ── CompiledHookCommand ───────────────────────────────────────────────────

/// Runtime form of HookCommand, ready for hot-path matching.
/// Created ONCE from HookCommand when loading agent config.
/// HookCommand is the serde form (JSON I/O).
/// CompiledHookCommand is the runtime form (matching + execution).
#[derive(Debug, Clone)]
pub struct CompiledHookCommand {
    pub command: String,

    // Pre-compiled filters
    /// HashSet for O(1) lookup instead of Vec O(n).
    pub tools: Option<HashSet<String>>,
    pub compiled_matcher: Option<CompiledPattern>,
    pub compiled_if: Option<CompiledIfCondition>,

    // Execution
    pub r#async: bool,
    pub async_rewake: bool,
    pub phase: HookPhase,
    pub timeout_secs: Option<u64>,
    pub on_timeout: OnTimeoutPolicy,
    pub status_message: Option<String>,
}

impl CompiledHookCommand {
    /// Hot-path matching — no parsing, no unnecessary allocations.
    pub fn matches_tool(&self, tool_name: &str, input_json: &str) -> bool {
        // 1. HashSet lookup O(1)
        if let Some(set) = &self.tools {
            if !set.contains(tool_name) {
                return false;
            }
        }
        // 2. Pre-compiled pattern over tool name
        if let Some(pat) = &self.compiled_matcher {
            if !pat.matches(tool_name) {
                return false;
            }
        }
        // 3. Pre-compiled IfCondition: tool name + input content
        if let Some(cond) = &self.compiled_if {
            if !cond.matches(tool_name, input_json) {
                return false;
            }
        }
        true
    }

    pub fn is_async(&self) -> bool {
        self.r#async
    }
    pub fn phase(&self) -> &HookPhase {
        &self.phase
    }
    pub fn command_str(&self) -> &str {
        &self.command
    }
    pub fn timeout_secs(&self) -> Option<u64> {
        self.timeout_secs
    }
    pub fn async_rewake(&self) -> bool {
        self.async_rewake
    }
}

impl HookCommand {
    /// Compiles the HookCommand into its runtime form.
    /// Returns an error if any matching pattern is invalid.
    /// Call once when loading agent config, not in the hot path.
    pub fn compile(&self) -> Result<CompiledHookCommand, ConfigError> {
        match self {
            HookCommand::Simple(cmd) => Ok(CompiledHookCommand {
                command: cmd.clone(),
                tools: None,
                compiled_matcher: None,
                compiled_if: None,
                r#async: false,
                async_rewake: false,
                phase: HookPhase::Main,
                timeout_secs: None,
                on_timeout: OnTimeoutPolicy::Fail,
                status_message: None,
            }),
            HookCommand::WithOptions {
                command,
                tools,
                matcher,
                if_condition,
                r#async,
                async_rewake,
                phase,
                timeout_secs,
                on_timeout,
                status_message,
            } => {
                let compiled_matcher = matcher
                    .as_deref()
                    .map(CompiledPattern::compile)
                    .transpose()?;

                let compiled_if = if_condition
                    .as_deref()
                    .map(CompiledIfCondition::compile)
                    .transpose()?;

                // Vec → HashSet: O(n) once, O(1) forever
                let tools_set = tools.as_ref().map(|list| {
                    list.iter().cloned().collect::<HashSet<String>>()
                });

                Ok(CompiledHookCommand {
                    command: command.clone(),
                    tools: tools_set,
                    compiled_matcher,
                    compiled_if,
                    r#async: *r#async,
                    async_rewake: *async_rewake,
                    phase: phase.clone(),
                    timeout_secs: *timeout_secs,
                    on_timeout: on_timeout.clone(),
                    status_message: status_message.clone(),
                })
            }
        }
    }
}

// ── CompiledAgentHookConfig ───────────────────────────────────────────────

/// Runtime form of AgentHookConfig.
/// AgentHookConfig is deserialized from JSON; then compile() is called once
/// to obtain CompiledAgentHookConfig, which is what HookRunner uses.
#[derive(Debug)]
pub struct CompiledAgentHookConfig {
    pub pre_tool_use: Vec<CompiledHookCommand>,
    pub post_tool_use: Vec<CompiledHookCommand>,
    pub post_tool_use_failure: Vec<CompiledHookCommand>,
    pub stop: Vec<CompiledHookCommand>,
    pub user_prompt_submit: Vec<CompiledHookCommand>,
    pub session_start: Vec<CompiledHookCommand>,
    pub session_end: Vec<CompiledHookCommand>,
    pub subagent_start: Vec<CompiledHookCommand>,
    pub subagent_stop: Vec<CompiledHookCommand>,
    pub pre_tool_use_timeout: Duration,
    pub post_tool_use_timeout: Duration,
    pub post_tool_use_failure_timeout: Duration,
}

impl CompiledAgentHookConfig {
    pub fn timeout_for_event(&self, event: HookEvent) -> Duration {
        match event {
            HookEvent::PreToolUse => self.pre_tool_use_timeout,
            HookEvent::PostToolUse => self.post_tool_use_timeout,
            HookEvent::PostToolUseFailure => self.post_tool_use_failure_timeout,
            _ => Duration::from_secs(30),
        }
    }

    /// Returns true if there are no hooks registered for any event.
    pub fn is_empty(&self) -> bool {
        self.pre_tool_use.is_empty()
            && self.post_tool_use.is_empty()
            && self.post_tool_use_failure.is_empty()
            && self.stop.is_empty()
            && self.user_prompt_submit.is_empty()
            && self.session_start.is_empty()
            && self.session_end.is_empty()
            && self.subagent_start.is_empty()
            && self.subagent_stop.is_empty()
    }
}

impl AgentHookConfig {
    /// Compiles the entire config into its runtime form.
    /// Should be called once when loading agent config.
    /// Returns a descriptive error if any pattern is invalid, naming the hook
    /// and the problematic field — the user knows what to fix in their config.
    pub fn compile(self) -> Result<CompiledAgentHookConfig, ConfigError> {
        Ok(CompiledAgentHookConfig {
            pre_tool_use: compile_list(self.pre_tool_use, "PreToolUse")?,
            post_tool_use: compile_list(self.post_tool_use, "PostToolUse")?,
            post_tool_use_failure: compile_list(
                self.post_tool_use_failure,
                "PostToolUseFailure",
            )?,
            stop: compile_list(self.stop, "Stop")?,
            user_prompt_submit: compile_list(self.user_prompt_submit, "UserPromptSubmit")?,
            session_start: compile_list(self.session_start, "SessionStart")?,
            session_end: compile_list(self.session_end, "SessionEnd")?,
            subagent_start: compile_list(self.subagent_start, "SubagentStart")?,
            subagent_stop: compile_list(self.subagent_stop, "SubagentStop")?,
            pre_tool_use_timeout: Duration::from_secs(self.pre_tool_use_timeout_secs),
            post_tool_use_timeout: Duration::from_secs(self.post_tool_use_timeout_secs),
            post_tool_use_failure_timeout: Duration::from_secs(
                self.post_tool_use_failure_timeout_secs,
            ),
        })
    }
}

fn compile_list(
    commands: Vec<HookCommand>,
    event_name: &str,
) -> Result<Vec<CompiledHookCommand>, ConfigError> {
    commands
        .into_iter()
        .enumerate()
        .map(|(i, cmd)| {
            cmd.compile().map_err(|e| ConfigError::HookCompile {
                event: event_name.to_string(),
                index: i,
                source: Box::new(e),
            })
        })
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::types::{AgentHookConfig, HookCommand, HookPhase, OnTimeoutPolicy};

    // ── CompiledPattern ───────────────────────────────────────────────────

    #[test]
    fn pattern_empty_compiles_to_any() {
        let p = CompiledPattern::compile("").unwrap();
        assert!(matches!(p, CompiledPattern::Any));
        assert!(p.matches("anything"));
    }

    #[test]
    fn pattern_star_compiles_to_any() {
        let p = CompiledPattern::compile("*").unwrap();
        assert!(matches!(p, CompiledPattern::Any));
        assert!(p.matches(""));
        assert!(p.matches("Bash"));
    }

    #[test]
    fn pattern_exact_no_implicit_substring() {
        // "Bash" compiles to Exact — only matches the literal string "Bash".
        let p = CompiledPattern::compile("Bash").unwrap();
        assert!(matches!(p, CompiledPattern::Exact(_)));
        assert!(p.matches("Bash"));
        assert!(!p.matches("BashTool")); // NOT a substring match
        assert!(!p.matches("edit_file"));
    }

    #[test]
    fn pattern_substring_requires_explicit_wildcard() {
        // For substring matching the user must be explicit: "*Bash*"
        let p = CompiledPattern::compile("*Bash*").unwrap();
        assert!(p.matches("Bash"));
        assert!(p.matches("BashTool"));
        assert!(p.matches("SomeBashTool"));
        assert!(!p.matches("edit_file"));
    }

    #[test]
    fn pattern_wildcard_matches() {
        let p = CompiledPattern::compile("Bash*").unwrap();
        assert!(p.matches("Bash"));
        assert!(p.matches("BashTool"));
        assert!(!p.matches("edit_file"));
    }

    #[test]
    fn pattern_wildcard_escapes_dot_metachar() {
        // "file.ts" without wildcard → Exact, not regex. But with wildcard:
        // "*.ts" should match "main.ts" but NOT "mainxts" (. is literal, not regex .)
        let p = CompiledPattern::compile("*.ts").unwrap();
        assert!(p.matches("main.ts"));
        assert!(p.matches("component.ts"));
        assert!(!p.matches("mainxts")); // dot is NOT a wildcard here
    }

    #[test]
    fn pattern_wildcard_escapes_plus_metachar() {
        // "a+b*" — the `+` is a regex metachar but must be treated as literal
        let p = CompiledPattern::compile("a+b*").unwrap();
        assert!(p.matches("a+b"));
        assert!(p.matches("a+btool"));
        assert!(!p.matches("ab")); // "a+b" requires the literal plus
        assert!(!p.matches("aaab")); // + is NOT "one or more a"
    }

    #[test]
    fn pattern_explicit_regex_exact_match() {
        let p = CompiledPattern::compile("/^Bash$/").unwrap();
        assert!(p.matches("Bash"));
        assert!(!p.matches("BashTool")); // ^ and $ anchor it
        assert!(!p.matches("edit_file"));
    }

    #[test]
    fn pattern_invalid_regex_errors() {
        let result = CompiledPattern::compile("/invalid[regex/");
        assert!(result.is_err());
    }

    // ── CompiledIfCondition ───────────────────────────────────────────────

    #[test]
    fn if_condition_tool_only_matches_any_input() {
        let c = CompiledIfCondition::compile("Bash").unwrap();
        assert!(c.matches("Bash", r#"{"command":"ls"}"#));
        assert!(c.matches("Bash", r#"{"command":"git push"}"#));
        assert!(!c.matches("edit_file", r#"{"command":"ls"}"#));
    }

    #[test]
    fn if_condition_bash_git_matches() {
        let c = CompiledIfCondition::compile("Bash(git *)").unwrap();
        assert!(c.matches("Bash", r#"{"command":"git push origin"}"#));
        assert!(!c.matches("Bash", r#"{"command":"ls -la"}"#));
        assert!(!c.matches("edit_file", r#"{"command":"git push"}"#));
    }

    #[test]
    fn if_condition_read_ts_matches() {
        let c = CompiledIfCondition::compile("Read(*.ts)").unwrap();
        assert!(c.matches("Read", r#"{"path":"main.ts"}"#));
        assert!(!c.matches("Read", r#"{"path":"main.js"}"#));
        assert!(!c.matches("Bash", r#"{"path":"main.ts"}"#));
    }

    #[test]
    fn if_condition_invalid_regex_errors() {
        let result = CompiledIfCondition::compile("Bash(/invalid[regex/)");
        assert!(result.is_err());
    }

    // ── CompiledHookCommand::matches_tool ─────────────────────────────────

    fn hook(tools: Option<Vec<&str>>, matcher: Option<&str>, if_cond: Option<&str>) -> CompiledHookCommand {
        CompiledHookCommand {
            command: "echo ok".to_string(),
            tools: tools.map(|v| v.into_iter().map(|s| s.to_string()).collect()),
            compiled_matcher: matcher.map(|m| CompiledPattern::compile(m).unwrap()),
            compiled_if: if_cond.map(|c| CompiledIfCondition::compile(c).unwrap()),
            r#async: false,
            async_rewake: false,
            phase: HookPhase::Main,
            timeout_secs: None,
            on_timeout: OnTimeoutPolicy::Fail,
            status_message: None,
        }
    }

    #[test]
    fn matches_tool_list_filters_correctly() {
        let h = hook(Some(vec!["Bash", "Read"]), None, None);
        assert!(h.matches_tool("Bash", "{}"));
        assert!(h.matches_tool("Read", "{}"));
        assert!(!h.matches_tool("edit_file", "{}"));
    }

    #[test]
    fn matches_tool_matcher_filters_correctly() {
        let h = hook(None, Some("Bash*"), None);
        assert!(h.matches_tool("Bash", "{}"));
        assert!(h.matches_tool("BashTool", "{}"));
        assert!(!h.matches_tool("edit_file", "{}"));
    }

    #[test]
    fn matches_tool_if_condition_filters() {
        let h = hook(None, None, Some("Bash(git *)"));
        assert!(h.matches_tool("Bash", r#"{"command":"git push"}"#));
        assert!(!h.matches_tool("Bash", r#"{"command":"ls"}"#));
    }

    #[test]
    fn matches_tool_all_three_filters_are_and() {
        // tools AND matcher AND if: all must pass
        let h = hook(Some(vec!["Bash"]), Some("Bash*"), Some("Bash(git *)"));
        assert!(h.matches_tool("Bash", r#"{"command":"git push"}"#));
        assert!(!h.matches_tool("Bash", r#"{"command":"ls"}"#)); // fails if
        assert!(!h.matches_tool("edit_file", r#"{"command":"git push"}"#)); // fails tools
    }

    #[test]
    fn matches_tool_no_filters_always_true() {
        let h = hook(None, None, None);
        assert!(h.matches_tool("anything", "{}"));
        assert!(h.matches_tool("edit_file", r#"{"path":"x.ts"}"#));
    }

    // ── HookCommand::compile ──────────────────────────────────────────────

    #[test]
    fn compile_simple_has_defaults() {
        let cmd = HookCommand::Simple("echo ok".to_string());
        let compiled = cmd.compile().unwrap();
        assert_eq!(compiled.command, "echo ok");
        assert!(compiled.tools.is_none());
        assert!(compiled.compiled_matcher.is_none());
        assert!(compiled.compiled_if.is_none());
        assert!(!compiled.r#async);
        assert!(!compiled.async_rewake);
        assert!(matches!(compiled.phase, HookPhase::Main));
        assert!(compiled.timeout_secs.is_none());
        assert!(matches!(compiled.on_timeout, OnTimeoutPolicy::Fail));
    }

    #[test]
    fn compile_with_options_valid_regex() {
        let cmd = HookCommand::WithOptions {
            command: "hook.sh".to_string(),
            tools: Some(vec!["Bash".to_string()]),
            matcher: Some("/^Bash$/".to_string()),
            if_condition: Some("Bash(git *)".to_string()),
            r#async: true,
            async_rewake: false,
            phase: HookPhase::Enforce,
            timeout_secs: Some(10),
            on_timeout: OnTimeoutPolicy::Continue,
            status_message: Some("running...".to_string()),
        };
        let compiled = cmd.compile().unwrap();
        assert_eq!(compiled.command, "hook.sh");
        assert!(compiled.tools.as_ref().unwrap().contains("Bash"));
        assert!(compiled.r#async);
        assert!(matches!(compiled.phase, HookPhase::Enforce));
        assert_eq!(compiled.timeout_secs, Some(10));
        assert!(matches!(compiled.on_timeout, OnTimeoutPolicy::Continue));
    }

    #[test]
    fn compile_invalid_matcher_regex_errors() {
        let cmd = HookCommand::WithOptions {
            command: "hook.sh".to_string(),
            tools: None,
            matcher: Some("/invalid[regex/".to_string()),
            if_condition: None,
            r#async: false,
            async_rewake: false,
            phase: HookPhase::Main,
            timeout_secs: None,
            on_timeout: OnTimeoutPolicy::Fail,
            status_message: None,
        };
        assert!(cmd.compile().is_err());
    }

    #[test]
    fn compile_invalid_if_condition_errors() {
        let cmd = HookCommand::WithOptions {
            command: "hook.sh".to_string(),
            tools: None,
            matcher: None,
            if_condition: Some("Bash(/invalid[/)".to_string()),
            r#async: false,
            async_rewake: false,
            phase: HookPhase::Main,
            timeout_secs: None,
            on_timeout: OnTimeoutPolicy::Fail,
            status_message: None,
        };
        assert!(cmd.compile().is_err());
    }

    #[test]
    fn compile_tools_vec_becomes_hashset() {
        let cmd = HookCommand::WithOptions {
            command: "hook.sh".to_string(),
            tools: Some(vec!["Bash".to_string(), "Read".to_string()]),
            matcher: None,
            if_condition: None,
            r#async: false,
            async_rewake: false,
            phase: HookPhase::Main,
            timeout_secs: None,
            on_timeout: OnTimeoutPolicy::Fail,
            status_message: None,
        };
        let compiled = cmd.compile().unwrap();
        let set = compiled.tools.unwrap();
        assert_eq!(set.len(), 2);
        assert!(set.contains("Bash"));
        assert!(set.contains("Read"));
    }

    // ── AgentHookConfig::compile ──────────────────────────────────────────

    #[test]
    fn agent_config_compile_empty_succeeds() {
        let config = AgentHookConfig::default();
        assert!(config.compile().is_ok());
    }

    #[test]
    fn agent_config_compile_fails_fast_on_first_error() {
        let mut config = AgentHookConfig::default();
        config.pre_tool_use = vec![
            HookCommand::WithOptions {
                command: "bad.sh".to_string(),
                tools: None,
                matcher: Some("/invalid[/".to_string()),
                if_condition: None,
                r#async: false,
                async_rewake: false,
                phase: HookPhase::Main,
                timeout_secs: None,
                on_timeout: OnTimeoutPolicy::Fail,
                status_message: None,
            },
            HookCommand::Simple("good.sh".to_string()),
        ];
        let err = config.compile().unwrap_err();
        // Error message must name the event and the index
        let msg = err.to_string();
        assert!(msg.contains("PreToolUse") || msg.contains("0"), "got: {msg}");
    }
}
