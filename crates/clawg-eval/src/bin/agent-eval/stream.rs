//! Reads the `stream-json` a client session prints, one JSON event per line,
//! into what the scoring needs: the tool calls with their results, the text
//! the hooks injected, the final answer and its cost. Pure: it takes the text.
//!
//! The format belongs to the client and changes with its versions, so the
//! reader is lenient: a line of a kind it does not know is counted, not an
//! error, and a line that is not JSON is reported by its number.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
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
    /// The tokens it added to the context: how much the next request grew over
    /// its own, for a call that is alone in its message; the input, the result
    /// and the text the agent wrote with it. `None` when that cannot be told.
    pub added_tokens: Option<u64>,
}

/// One request of the main loop: a message of the assistant, which the client
/// may print in several events with one `id`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Request {
    pub id: Option<String>,
    /// The input, cache written and cache read of its `usage`, which is
    /// everything the model read to answer it.
    pub context: Option<u64>,
    /// Indexes into [`Transcript::calls`] of the calls it made.
    pub calls: Vec<usize>,
}

/// The tokens of the main loop. A subagent's are left out, and so is the
/// response that crossed `--max-budget-usd`: [`Outcome::model_usage`] has them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    /// The cache writes by how long they last, which bills them differently:
    /// an API key gets 5 minutes and a subscription 1 hour.
    #[serde(default)]
    pub cache_creation_5m_input_tokens: u64,
    #[serde(default)]
    pub cache_creation_1h_input_tokens: u64,
}

/// What one model spent in the whole session, its subagents included.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    /// The client's estimate, priced by its own table.
    pub cost_usd: Option<f64>,
    /// Which table priced it: `list`, `managed` or `unknown`.
    pub cost_basis: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub subtype: String,
    pub is_error: bool,
    pub text: String,
    pub num_turns: Option<u64>,
    pub duration_ms: Option<u64>,
    pub total_cost_usd: Option<f64>,
    pub usage: Usage,
    /// By model name.
    #[serde(default)]
    pub model_usage: BTreeMap<String, ModelUsage>,
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
    /// The credential that paid, such as `ANTHROPIC_API_KEY`.
    pub api_key_source: Option<String>,
    /// Each MCP server the session started with and its status.
    pub mcp_servers: Vec<(String, String)>,
    pub calls: Vec<ToolCall>,
    pub hooks: Vec<HookEvent>,
    /// The assistant's text blocks, in order.
    pub texts: Vec<String>,
    pub compactions: usize,
    /// Every request of the main loop, in order; a subagent's are left out.
    pub requests: Vec<Request>,
    /// The context of the first request of the main loop: what the session's
    /// context costs before the agent acts.
    pub first_request_tokens: Option<u64>,
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
                t.api_key_source = str_field(&event, "apiKeySource");
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
                if !subagent {
                    let message = event.get("message");
                    let id = message.and_then(|m| m.get("id")).and_then(Value::as_str).map(str::to_owned);
                    let context = message.and_then(|m| m.get("usage")).map(|u| {
                        let tokens = |name: &str| u.get(name).and_then(Value::as_u64).unwrap_or(0);
                        tokens("input_tokens") + tokens("cache_creation_input_tokens") + tokens("cache_read_input_tokens")
                    });
                    match t.requests.last_mut() {
                        Some(last) if id.is_some() && last.id == id => last.context = last.context.or(context),
                        _ => t.requests.push(Request { id, context, calls: Vec::new() }),
                    }
                }
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
                            if let (false, Some(request)) = (subagent, t.requests.last_mut()) {
                                request.calls.push(t.calls.len());
                            }
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
                let written = |name: &str| {
                    usage.and_then(|u| u.get("cache_creation")).and_then(|c| c.get(name)).and_then(Value::as_u64).unwrap_or(0)
                };
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
                        cache_creation_5m_input_tokens: written("ephemeral_5m_input_tokens"),
                        cache_creation_1h_input_tokens: written("ephemeral_1h_input_tokens"),
                    },
                    model_usage: event
                        .get("modelUsage")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                        .map(|(name, m)| (name.clone(), model_usage(m)))
                        .collect(),
                });
            }
            _ => t.unknown_lines += 1,
        }
    }
    t.first_request_tokens = t.requests.iter().find_map(|r| r.context);
    for i in 1..t.requests.len() {
        let (Some(before), Some(after)) = (t.requests[i - 1].context, t.requests[i].context) else { continue };
        if let [only] = t.requests[i - 1].calls[..] {
            t.calls[only].added_tokens = after.checked_sub(before);
        }
    }
    t
}

fn str_field(event: &serde_json::Map<String, Value>, name: &str) -> Option<String> {
    event.get(name).and_then(Value::as_str).map(str::to_owned)
}

/// One entry of `modelUsage`, whose fields the client names in camel case.
fn model_usage(m: &Value) -> ModelUsage {
    let tokens = |name: &str| m.get(name).and_then(Value::as_u64).unwrap_or(0);
    ModelUsage {
        input_tokens: tokens("inputTokens"),
        output_tokens: tokens("outputTokens"),
        cache_read_input_tokens: tokens("cacheReadInputTokens"),
        cache_creation_input_tokens: tokens("cacheCreationInputTokens"),
        cost_usd: m.get("costUSD").and_then(Value::as_f64),
        cost_basis: m.get("costBasis").and_then(Value::as_str).map(str::to_owned),
    }
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

    /// The fields as Claude Code 2.1.322 prints them.
    const PAID: &str = r#"{"type":"system","subtype":"init","session_id":"s2","model":"claude-sonnet-5-5","apiKeySource":"ANTHROPIC_API_KEY"}
{"type":"result","subtype":"success","is_error":false,"result":"done","total_cost_usd":0.0912,"usage":{"input_tokens":30,"output_tokens":800,"cache_creation_input_tokens":12000,"cache_read_input_tokens":50000,"cache_creation":{"ephemeral_5m_input_tokens":11000,"ephemeral_1h_input_tokens":1000}},"modelUsage":{"claude-sonnet-5-5":{"inputTokens":30,"outputTokens":800,"cacheReadInputTokens":50000,"cacheCreationInputTokens":12000,"webSearchRequests":0,"costUSD":0.09,"contextWindow":200000,"costBasis":"list"},"claude-haiku-5-5":{"inputTokens":400,"outputTokens":20,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"costUSD":0.0012}}}
"#;

    #[test]
    fn the_outcome_carries_the_tokens_of_each_model_and_the_cache_writes_by_duration() {
        let t = parse(PAID);
        assert_eq!(t.api_key_source.as_deref(), Some("ANTHROPIC_API_KEY"));
        let outcome = t.result.unwrap();
        assert_eq!((outcome.usage.cache_creation_5m_input_tokens, outcome.usage.cache_creation_1h_input_tokens), (11000, 1000));
        assert_eq!(outcome.model_usage.len(), 2);
        let sonnet = &outcome.model_usage["claude-sonnet-5-5"];
        assert_eq!(
            (sonnet.input_tokens, sonnet.output_tokens, sonnet.cache_read_input_tokens, sonnet.cache_creation_input_tokens),
            (30, 800, 50000, 12000)
        );
        assert_eq!((sonnet.cost_usd, sonnet.cost_basis.as_deref()), (Some(0.09), Some("list")));
        let haiku = &outcome.model_usage["claude-haiku-5-5"];
        assert_eq!((haiku.input_tokens, haiku.cost_usd, haiku.cost_basis.as_deref()), (400, Some(0.0012), None));
    }

    #[test]
    fn a_stream_without_the_newer_fields_reads_them_as_absent() {
        let t = parse(STREAM);
        assert_eq!(t.api_key_source, None);
        let outcome = t.result.unwrap();
        assert!(outcome.model_usage.is_empty());
        assert_eq!((outcome.usage.cache_creation_5m_input_tokens, outcome.usage.cache_creation_1h_input_tokens), (0, 0));
    }

    #[test]
    fn the_first_request_is_the_first_message_of_the_main_loop() {
        let t = parse(
            r#"{"type":"assistant","message":{"content":[],"usage":{"input_tokens":9,"cache_creation_input_tokens":90,"cache_read_input_tokens":900}},"parent_tool_use_id":"sub"}
{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":2,"cache_creation_input_tokens":8733,"cache_read_input_tokens":11728,"output_tokens":16}},"parent_tool_use_id":null}
{"type":"assistant","message":{"content":[],"usage":{"input_tokens":5,"cache_creation_input_tokens":5,"cache_read_input_tokens":5}},"parent_tool_use_id":null}"#,
        );
        assert_eq!(t.first_request_tokens, Some(2 + 8733 + 11728), "input, cache written and cache read; not the subagent's");
        assert_eq!(parse(STREAM).first_request_tokens, None, "messages without usage");
    }

    /// An event of the assistant: message `id`, a context of `read` tokens, and `blocks`.
    fn message(id: &str, read: u64, blocks: &str, parent: &str) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"id":"{id}","content":[{blocks}],"usage":{{"input_tokens":3,"cache_creation_input_tokens":7,"cache_read_input_tokens":{},"output_tokens":9}}}},"parent_tool_use_id":{parent}}}"#,
            read - 10
        ) + "\n"
    }

    fn call(id: &str) -> String {
        format!(r#"{{"type":"tool_use","id":"{id}","name":"Read","input":{{}}}}"#)
    }

    #[test]
    fn each_request_of_the_main_loop_is_read_once_with_its_context() {
        let stream = message("m1", 1000, r#"{"type":"text","text":"Reading."}"#, "null")
            + &message("m1", 1000, &call("a"), "null")
            + &message("s1", 77, &call("sub"), "\"agent\"")
            + &message("m2", 1800, &(call("b") + "," + &call("c")), "null")
            + r#"{"type":"assistant","message":{"content":[{"type":"text","text":"no usage"}]}}"#;
        let t = parse(&stream);
        assert_eq!(t.requests.len(), 3, "a message in two events is one request, a subagent's is none: {:?}", t.requests);
        assert_eq!(t.requests[0].context, Some(1000));
        assert_eq!(t.requests[0].calls, [0], "the call the second event of its message made");
        assert_eq!((t.requests[1].context, t.requests[1].calls.clone()), (Some(1800), vec![2, 3]));
        assert_eq!((t.requests[2].context, t.requests[2].calls.len()), (None, 0), "a message without usage has no context");
        assert_eq!(t.first_request_tokens, Some(1000));
    }

    #[test]
    fn a_call_alone_in_its_message_adds_what_the_next_request_grew() {
        let stream = message("m1", 1000, &call("a"), "null")
            + &message("m2", 1800, &call("b"), "null")
            + &message("m3", 2000, &call("c"), "null")
            + &message("m4", 2500, r#"{"type":"text","text":"Done."}"#, "null");
        let t = parse(&stream);
        let added: Vec<Option<u64>> = t.calls.iter().map(|c| c.added_tokens).collect();
        assert_eq!(added, [Some(800), Some(200), Some(500)]);
    }

    #[test]
    fn calls_that_share_a_message_have_no_tokens_of_their_own() {
        let stream = message("m1", 1000, &(call("a") + "," + &call("b")), "null")
            + &message("m2", 1800, &call("c"), "null")
            + &message("m3", 900, &call("d"), "null")
            + &message("m4", 5000, &call("e"), "null");
        let t = parse(&stream);
        let added: Vec<Option<u64>> = t.calls.iter().map(|c| c.added_tokens).collect();
        assert_eq!(added, [None, None, None, Some(4100), None], "two calls in a message, a context that shrank after c, and the last request");
        assert!(parse(STREAM).calls.iter().all(|c| c.added_tokens.is_none()), "a stream without usage measures nothing");
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
