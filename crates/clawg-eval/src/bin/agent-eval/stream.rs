//! Reads the `stream-json` a client session prints, one JSON event per line,
//! into what the scoring needs: the tool calls with their results, the text
//! the hooks injected, the final answer and its cost. Pure: it takes the text.
//!
//! The format belongs to the client and changes with its versions, so the
//! reader is lenient: a line of a kind it does not know is counted, not an
//! error, and a line that is not JSON is reported by its number.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
    /// The result's text, `None` when no result came back.
    pub result: Option<String>,
    pub is_error: bool,
    /// Made by a subagent, not by the main session.
    pub subagent: bool,
    /// A `ToolSearch` call came before it in the session.
    pub after_tool_search: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Outcome {
    pub subtype: String,
    pub is_error: bool,
    pub text: String,
    pub num_turns: Option<u64>,
    pub duration_ms: Option<u64>,
    pub total_cost_usd: Option<f64>,
    pub usage: Usage,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct HookEvent {
    pub subtype: String,
    /// Every string the event carries, joined: what the hook put in front of
    /// the model is among them, whatever field the client names it with.
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Transcript {
    pub session_id: Option<String>,
    pub model: Option<String>,
    pub client_version: Option<String>,
    /// Each MCP server the session started with and its status.
    pub mcp_servers: Vec<(String, String)>,
    pub calls: Vec<ToolCall>,
    pub hooks: Vec<HookEvent>,
    /// The assistant's text blocks, in order.
    pub texts: Vec<String>,
    pub compactions: usize,
    pub result: Option<Outcome>,
    /// Lines of a kind this reader does not know.
    pub unknown_lines: usize,
    /// Numbers, from 1, of the lines that are not JSON objects.
    pub invalid_lines: Vec<usize>,
}

impl Transcript {
    pub fn tool_search_calls(&self) -> usize {
        self.calls.iter().filter(|c| c.name == "ToolSearch").count()
    }

    /// The calls to tools of the MCP server `server`.
    pub fn server_calls<'a>(&'a self, server: &'a str) -> impl Iterator<Item = &'a ToolCall> {
        let prefix = format!("mcp__{server}__");
        self.calls.iter().filter(move |c| c.name.starts_with(&prefix))
    }

    pub fn answer(&self) -> &str {
        self.result.as_ref().map_or("", |r| r.text.as_str())
    }
}

/// Whether `call_name` is the tool `name`: the same name, or an MCP tool of
/// any server whose own name is `name`.
pub fn is_tool(call_name: &str, name: &str) -> bool {
    call_name == name || (call_name.starts_with("mcp__") && call_name.ends_with(&format!("__{name}")))
}

pub fn parse(text: &str) -> Transcript {
    let mut t = Transcript::default();
    let mut searched = false;
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(Value::Object(event)) = serde_json::from_str::<Value>(line) else {
            t.invalid_lines.push(n + 1);
            continue;
        };
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        let subtype = event.get("subtype").and_then(Value::as_str).unwrap_or("");
        let subagent = event.get("parent_tool_use_id").is_some_and(|v| !v.is_null());
        match (kind, subtype) {
            ("system", "init") => {
                t.session_id = str_field(&event, "session_id");
                t.model = str_field(&event, "model");
                t.client_version = str_field(&event, "claude_code_version");
                for server in event.get("mcp_servers").and_then(Value::as_array).into_iter().flatten() {
                    let name = server.get("name").and_then(Value::as_str).unwrap_or("").to_owned();
                    let status = server.get("status").and_then(Value::as_str).unwrap_or("").to_owned();
                    t.mcp_servers.push((name, status));
                }
            }
            ("system", "compact_boundary") => t.compactions += 1,
            ("system", s) if s.starts_with("hook") => {
                let mut strings = Vec::new();
                collect_strings(&Value::Object(event.clone()), &mut strings);
                t.hooks.push(HookEvent { subtype: s.to_owned(), text: strings.join("\n") });
            }
            ("assistant", _) => {
                for block in content(&event) {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => t.texts.push(block.get("text").and_then(Value::as_str).unwrap_or("").to_owned()),
                        Some("tool_use") => {
                            let name = block.get("name").and_then(Value::as_str).unwrap_or("").to_owned();
                            let call = ToolCall {
                                id: block.get("id").and_then(Value::as_str).unwrap_or("").to_owned(),
                                input: block.get("input").cloned().unwrap_or(Value::Null),
                                subagent,
                                after_tool_search: searched,
                                name,
                                ..Default::default()
                            };
                            searched |= call.name == "ToolSearch";
                            t.calls.push(call);
                        }
                        _ => {}
                    }
                }
            }
            ("user", _) => {
                for block in content(&event) {
                    if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                        continue;
                    }
                    let id = block.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
                    if let Some(call) = t.calls.iter_mut().find(|c| c.id == id) {
                        call.result = Some(result_text(block.get("content")));
                        call.is_error = block.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                    }
                }
            }
            ("result", _) => {
                let usage = event.get("usage");
                let tokens = |name: &str| usage.and_then(|u| u.get(name)).and_then(Value::as_u64).unwrap_or(0);
                t.result = Some(Outcome {
                    subtype: subtype.to_owned(),
                    is_error: event.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                    text: str_field(&event, "result").unwrap_or_default(),
                    num_turns: event.get("num_turns").and_then(Value::as_u64),
                    duration_ms: event.get("duration_ms").and_then(Value::as_u64),
                    total_cost_usd: event.get("total_cost_usd").and_then(Value::as_f64),
                    usage: Usage {
                        input_tokens: tokens("input_tokens"),
                        output_tokens: tokens("output_tokens"),
                        cache_creation_input_tokens: tokens("cache_creation_input_tokens"),
                        cache_read_input_tokens: tokens("cache_read_input_tokens"),
                    },
                });
            }
            _ => t.unknown_lines += 1,
        }
    }
    t
}

fn str_field(event: &serde_json::Map<String, Value>, name: &str) -> Option<String> {
    event.get(name).and_then(Value::as_str).map(str::to_owned)
}

fn content(event: &serde_json::Map<String, Value>) -> impl Iterator<Item = &Value> {
    event.get("message").and_then(|m| m.get("content")).and_then(Value::as_array).into_iter().flatten()
}

/// A tool result's text: a string as it is, a list of blocks with their
/// texts joined.
fn result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn collect_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => out.push(s.clone()),
        Value::Array(items) => items.iter().for_each(|v| collect_strings(v, out)),
        Value::Object(map) => map.values().for_each(|v| collect_strings(v, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STREAM: &str = r##"{"type":"system","subtype":"init","session_id":"s1","model":"m1","claude_code_version":"2.1.300","mcp_servers":[{"name":"clawg-dev","status":"connected"}],"tools":["Read","ToolSearch"]}
{"type":"system","subtype":"hook_response","hook_event":"SessionStart","output":{"hookSpecificOutput":{"additionalContext":"=== LAST SESSION === port 48213"}}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Looking it up."},{"type":"tool_use","id":"t1","name":"ToolSearch","input":{"query":"select:mcp__clawg-dev__search_memory"}}]},"parent_tool_use_id":null}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"loaded"}]},"parent_tool_use_id":null}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"mcp__clawg-dev__search_memory","input":{"query":"staging port"}}]},"parent_tool_use_id":null}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":[{"type":"text","text":"#7 Staging listens on 48213"}]}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t3","name":"Read","input":{"file_path":"a.py"}}]},"parent_tool_use_id":"t9"}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t3","content":"no such file","is_error":true}]},"parent_tool_use_id":"t9"}
{"type":"system","subtype":"compact_boundary"}
{"type":"stream_event","event":{}}
not json
{"type":"result","subtype":"success","is_error":false,"result":"Port 48213.","num_turns":4,"duration_ms":9100,"total_cost_usd":0.21,"usage":{"input_tokens":12,"output_tokens":340,"cache_creation_input_tokens":20000,"cache_read_input_tokens":41000}}
"##;

    #[test]
    fn a_stream_reads_into_calls_hooks_and_an_outcome() {
        let t = parse(STREAM);
        assert_eq!(t.session_id.as_deref(), Some("s1"));
        assert_eq!(t.model.as_deref(), Some("m1"));
        assert_eq!(t.client_version.as_deref(), Some("2.1.300"));
        assert_eq!(t.mcp_servers, vec![("clawg-dev".to_owned(), "connected".to_owned())]);
        assert_eq!(t.texts, vec!["Looking it up.".to_owned()]);
        assert_eq!(t.calls.len(), 3);
        assert_eq!(t.tool_search_calls(), 1);
        let search = &t.calls[1];
        assert_eq!(search.result.as_deref(), Some("#7 Staging listens on 48213"));
        assert!(search.after_tool_search && !search.subagent && !search.is_error);
        assert!(!t.calls[0].after_tool_search);
        assert!(t.calls[2].subagent && t.calls[2].is_error);
        assert_eq!(t.server_calls("clawg-dev").count(), 1);
        assert_eq!(t.hooks.len(), 1);
        assert!(t.hooks[0].text.contains("=== LAST SESSION === port 48213"));
        assert_eq!(t.compactions, 1);
        assert_eq!(t.unknown_lines, 1);
        assert_eq!(t.invalid_lines, vec![11]);
        let outcome = t.result.as_ref().unwrap();
        assert_eq!(t.answer(), "Port 48213.");
        assert_eq!((outcome.num_turns, outcome.duration_ms, outcome.total_cost_usd), (Some(4), Some(9100), Some(0.21)));
        assert_eq!(outcome.usage.cache_read_input_tokens, 41000);
    }

    #[test]
    fn a_stream_cut_short_has_no_outcome_and_a_call_without_result() {
        let t = parse(r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{}}]}}"#);
        assert!(t.result.is_none());
        assert_eq!(t.answer(), "");
        assert_eq!(t.calls[0].result, None);
    }

    #[test]
    fn a_tool_is_named_bare_or_by_its_server() {
        assert!(is_tool("Read", "Read"));
        assert!(is_tool("mcp__clawg-dev__search_memory", "search_memory"));
        assert!(!is_tool("mcp__clawg-dev__search_memory_x", "search_memory"));
        assert!(!is_tool("search_memory_tool", "search_memory"));
    }
}
