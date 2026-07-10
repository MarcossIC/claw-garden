use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crate::models::{
    AppError, LlmError, ChatRequest, ChatStreamResult, Role, StreamEvent, ToolCall, ToolDefinition,
    ToolFunction,
};
use crate::providers::sse::{SseLine, SseLineParser};
use crate::providers::{AiProvider, StreamSender};

#[derive(Debug, Clone)]
pub enum AnthropicAuth {
    ApiKey(String),
    Bearer(String),
}

#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    pub auth: AnthropicAuth,
    pub model: String,
    pub base_url: String,
}

pub struct AnthropicProvider {
    config: AnthropicConfig,
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(config: AnthropicConfig) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self { config, client }
    }
}

// --- Anthropic API types ---

const PROMPT_CACHING_BETA: &str = "prompt-caching-scope-2026-01-05";

#[derive(Serialize, Clone)]
struct CacheControl {
    #[serde(rename = "type")]
    kind: &'static str,
}

impl CacheControl {
    fn ephemeral() -> Self {
        Self { kind: "ephemeral" }
    }
}

#[derive(Serialize)]
struct AnthropicSystemBlock {
    #[serde(rename = "type")]
    kind: &'static str,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<Vec<AnthropicSystemBlock>>,
    max_tokens: u32,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: AnthropicContent,
}

#[derive(Serialize)]
#[serde(untagged)]
enum AnthropicContent {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

// SSE event types
#[derive(Deserialize)]
struct SseContentBlockStart {
    content_block: SseContentBlock,
}

#[derive(Deserialize)]
struct SseContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct SseContentDelta {
    delta: SseDelta,
}

#[derive(Deserialize)]
struct SseDelta {
    #[serde(rename = "type")]
    delta_type: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    partial_json: Option<String>,
}

fn build_anthropic_messages(request: &ChatRequest) -> Vec<AnthropicMessage> {
    let mut messages = Vec::new();

    for msg in &request.messages {
        match msg.role {
            Role::System => continue, // system is a separate parameter
            Role::Tool => {
                // Tool results: extract tool_use_id from tool_calls of the previous assistant message
                // We use the tool call ID stored in the content prefix "id:" if present,
                // otherwise fall back to a generic ID
                let tool_use_id = msg
                    .tool_calls
                    .as_ref()
                    .and_then(|tcs| tcs.first())
                    .map(|tc| tc.id.clone())
                    .unwrap_or_else(|| "tool_result".to_string());

                messages.push(AnthropicMessage {
                    role: "user".to_string(),
                    content: AnthropicContent::Blocks(vec![AnthropicContentBlock::ToolResult {
                        tool_use_id,
                        content: msg.content.clone(),
                    }]),
                });
            }
            Role::Assistant => {
                if let Some(tool_calls) = &msg.tool_calls {
                    let blocks: Vec<AnthropicContentBlock> = tool_calls
                        .iter()
                        .map(|tc| AnthropicContentBlock::ToolUse {
                            id: tc.id.clone(),
                            name: tc.function.name.clone(),
                            input: tc.function.arguments.clone(),
                        })
                        .collect();
                    messages.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: AnthropicContent::Blocks(blocks),
                    });
                } else {
                    messages.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: AnthropicContent::Text(msg.content.clone()),
                    });
                }
            }
            Role::User => {
                messages.push(AnthropicMessage {
                    role: "user".to_string(),
                    content: AnthropicContent::Text(msg.content.clone()),
                });
            }
        }
    }

    messages
}

fn convert_tools(tools: &[ToolDefinition]) -> Vec<AnthropicTool> {
    let last_idx = tools.len().saturating_sub(1);
    tools
        .iter()
        .enumerate()
        .map(|(i, t)| AnthropicTool {
            name: t.function.name.clone(),
            description: t.function.description.clone(),
            input_schema: t.function.parameters.clone(),
            // Mark only the last tool — Anthropic caches everything up to (and including) this block
            cache_control: if i == last_idx {
                Some(CacheControl::ephemeral())
            } else {
                None
            },
        })
        .collect()
}

#[async_trait]
impl AiProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn chat_stream(
        &self,
        request: &ChatRequest,
        sender: StreamSender,
    ) -> Result<ChatStreamResult, AppError> {
        let model = if request.model.is_empty() {
            &self.config.model
        } else {
            &request.model
        };

        let anthropic_tools = request.tools.as_ref().map(|t| convert_tools(t));

        // Build system as cacheable block array.
        //
        // Two-block strategy for Anthropic prefix caching:
        //   Block 1 (with cache_control): BASE_RULES + role template — identical for all agents
        //             of the same role, so the cache hit is shared across agents.
        //   Block 2 (with cache_control): custom_instructions — per-agent but also static,
        //             cached separately so block 1's prefix remains stable.
        //
        // Both blocks are marked ephemeral because neither changes between requests.
        // Anthropic caches the longest stable prefix, so block 1 gets the most reuse.
        let system_blocks = request.system_prompt.as_ref().map(|text| {
            let mut blocks = vec![AnthropicSystemBlock {
                kind: "text",
                text: text.clone(),
                cache_control: Some(CacheControl::ephemeral()),
            }];
            if let Some(instructions) = &request.custom_instructions {
                if !instructions.trim().is_empty() {
                    blocks.push(AnthropicSystemBlock {
                        kind: "text",
                        text: format!("Additional instructions:\n{instructions}"),
                        cache_control: Some(CacheControl::ephemeral()),
                    });
                }
            }
            blocks
        });

        let body = AnthropicRequest {
            model: model.to_string(),
            messages: build_anthropic_messages(request),
            system: system_blocks,
            max_tokens: crate::services::model_meta::max_output_tokens("anthropic", model),
            stream: true,
            tools: anthropic_tools,
        };

        let mut request = self
            .client
            .post(format!("{}/v1/messages", self.config.base_url))
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", PROMPT_CACHING_BETA)
            .header("content-type", "application/json");

        request = match &self.config.auth {
            AnthropicAuth::ApiKey(key) => request.header("x-api-key", key),
            AnthropicAuth::Bearer(token) => request.bearer_auth(token),
        };

        let response = request
            .json(&body)
            .send()
            .await
            .map_err(LlmError::Network)?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(match status.as_u16() {
                401 | 403 => LlmError::Authentication(format!("Anthropic {status}: {text}")),
                429 => LlmError::RateLimit {
                    retry_after_secs: crate::services::retry::parse_retry_after_secs(&text),
                },
                400 => LlmError::InvalidRequest(format!("{status}: {text}")),
                _ => LlmError::ProviderUnavailable(format!("Anthropic {status}: {text}")),
            }
            .into());
        }

        sender(StreamEvent::State {
            state: "typing".to_string(),
        })
        .map_err(AppError::Streaming)?;

        let mut stream_state = SseStreamState::default();
        let mut byte_stream = response.bytes_stream();
        let mut parser = SseLineParser::new();

        while let Some(chunk) = byte_stream.next().await {
            let chunk =
                chunk.map_err(|e| AppError::Streaming(format!("Stream read error: {e}")))?;
            parser.push(&chunk);

            while let Some(line) = parser.next_line() {
                match line {
                    SseLine::Done => break,
                    SseLine::Data(data) => {
                        self.process_sse_event(&data, &sender, &mut stream_state);
                    }
                }
            }
        }

        if !stream_state.tool_calls.is_empty() {
            Ok(ChatStreamResult::ToolCalls(stream_state.tool_calls))
        } else {
            Ok(ChatStreamResult::Text(stream_state.full_response))
        }
    }
}

#[derive(Default)]
struct SseStreamState {
    full_response: String,
    tool_calls: Vec<ToolCall>,
    current_tool_name: Option<String>,
    current_tool_id: Option<String>,
    current_tool_input: String,
    input_tokens: u32,
    output_tokens: u32,
    cache_creation_tokens: u32,
    cache_read_tokens: u32,
}

impl AnthropicProvider {
    fn process_sse_event(&self, data: &str, sender: &StreamSender, state: &mut SseStreamState) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
            return;
        };
        let event_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match event_type {
            "content_block_start" => {
                if let Ok(parsed) = serde_json::from_str::<SseContentBlockStart>(data) {
                    if parsed.content_block.block_type == "tool_use" {
                        state.current_tool_name = parsed.content_block.name;
                        state.current_tool_id = parsed.content_block.id.clone();
                        state.current_tool_input.clear();
                    }
                }
            }
            "content_block_delta" => {
                if let Ok(parsed) = serde_json::from_str::<SseContentDelta>(data) {
                    match parsed.delta.delta_type.as_str() {
                        "text_delta" => {
                            if let Some(text) = &parsed.delta.text {
                                state.full_response.push_str(text);
                                let _ = sender(StreamEvent::Delta {
                                    delta: text.clone(),
                                });
                            }
                        }
                        "input_json_delta" => {
                            if let Some(json) = &parsed.delta.partial_json {
                                state.current_tool_input.push_str(json);
                            }
                        }
                        _ => {}
                    }
                }
            }
            "content_block_stop" => {
                if let Some(name) = state.current_tool_name.take() {
                    let id = state.current_tool_id.take().unwrap_or_else(|| {
                        format!("toolu_{}", uuid::Uuid::new_v4().as_simple())
                    });
                    let arguments = serde_json::from_str(&state.current_tool_input)
                        .unwrap_or(serde_json::Value::Object(Default::default()));
                    state.tool_calls.push(ToolCall {
                        id,
                        function: ToolFunction { name, arguments },
                    });
                    state.current_tool_input.clear();
                }
            }
            "message_start" => {
                let usage = value.pointer("/message/usage");
                if let Some(tokens) = usage
                    .and_then(|u| u.get("input_tokens"))
                    .and_then(|v| v.as_u64())
                {
                    state.input_tokens = tokens as u32;
                }
                if let Some(tokens) = usage
                    .and_then(|u| u.get("cache_creation_input_tokens"))
                    .and_then(|v| v.as_u64())
                {
                    state.cache_creation_tokens = tokens as u32;
                }
                if let Some(tokens) = usage
                    .and_then(|u| u.get("cache_read_input_tokens"))
                    .and_then(|v| v.as_u64())
                {
                    state.cache_read_tokens = tokens as u32;
                }
            }
            "message_delta" => {
                if let Some(tokens) = value
                    .pointer("/usage/output_tokens")
                    .and_then(|v| v.as_u64())
                {
                    state.output_tokens = tokens as u32;
                    let _ = sender(StreamEvent::Usage {
                        input_tokens: state.input_tokens,
                        output_tokens: state.output_tokens,
                        cache_creation_tokens: state.cache_creation_tokens,
                        cache_read_tokens: state.cache_read_tokens,
                    });
                }
            }
            _ => {}
        }
    }
}
