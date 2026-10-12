use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

use crate::models::{
    AppError, LlmError, ChatRequest, ChatStreamResult, Role, StreamEvent, ToolCall, ToolDefinition,
    ToolFunction,
};
use crate::providers::sse::{SseLine, SseLineParser};
use crate::providers::{AiProvider, StreamSender};

#[derive(Debug, Clone)]
pub struct OpenRouterConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
}

pub struct OpenRouterProvider {
    config: OpenRouterConfig,
    client: reqwest::Client,
}

impl OpenRouterProvider {
    pub fn new(config: OpenRouterConfig) -> Self {
        let mut default_headers = HeaderMap::new();
        default_headers.insert(
            "HTTP-Referer",
            HeaderValue::from_static("https://clawgarden.app"),
        );
        default_headers.insert("X-Title", HeaderValue::from_static("Claw Garden"));

        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .default_headers(default_headers)
            .build()
            .unwrap_or_default();

        Self { config, client }
    }

    /// List available models from OpenRouter catalog
    pub async fn list_models(&self) -> Result<Vec<OpenRouterModel>, AppError> {
        let response = self
            .client
            .get(format!("{}/models", self.config.base_url))
            .bearer_auth(&self.config.api_key)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to fetch models: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "OpenRouter returned {status}: {text}"
            )));
        }

        let catalog: ModelsResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse models: {e}")))?;

        Ok(catalog.data)
    }

    /// Verify API key is valid
    pub async fn verify_key(&self) -> Result<bool, AppError> {
        let response = self
            .client
            .get(format!("{}/models?limit=1", self.config.base_url))
            .bearer_auth(&self.config.api_key)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Connection failed: {e}")))?;

        Ok(response.status().is_success())
    }
}

// --- OpenAI-compatible request types ---

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize)]
struct OpenAiMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCallSer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize)]
struct OpenAiToolCallSer {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OpenAiFunctionSer,
}

#[derive(Serialize)]
struct OpenAiFunctionSer {
    name: String,
    arguments: String,
}

// --- OpenAI-compatible response types (SSE) ---

#[derive(Deserialize)]
struct SseChunk {
    choices: Vec<SseChoice>,
    #[serde(default)]
    usage: Option<SseUsage>,
}

#[derive(Deserialize)]
struct SseUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

#[derive(Deserialize)]
struct SseChoice {
    delta: SseDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct SseDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<SseToolCall>>,
}

#[derive(Deserialize)]
struct SseToolCall {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<SseFunctionDelta>,
}

#[derive(Deserialize)]
struct SseFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

// --- Models catalog ---

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<OpenRouterModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterModel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub context_length: u32,
    #[serde(default)]
    pub pricing: Option<ModelPricing>,
    #[serde(default)]
    pub supported_parameters: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub completion: String,
}

// --- Message conversion ---

fn role_to_string(role: &Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "system",
        Role::Tool => "tool",
    }
}

fn build_openai_messages(request: &ChatRequest) -> Vec<OpenAiMessage> {
    let mut messages = Vec::new();

    if let Some(system) = &request.system_prompt {
        let content = match &request.custom_instructions {
            Some(ci) if !ci.trim().is_empty() => {
                format!("{system}\n\nAdditional instructions:\n{ci}")
            }
            _ => system.clone(),
        };
        messages.push(OpenAiMessage {
            role: "system".to_string(),
            content,
            tool_calls: None,
            tool_call_id: None,
        });
    }

    for msg in &request.messages {
        match msg.role {
            Role::System => continue,
            Role::Tool => {
                let tool_call_id = msg
                    .tool_calls
                    .as_ref()
                    .and_then(|tcs| tcs.first())
                    .map(|tc| tc.id.clone())
                    .unwrap_or_else(|| "call_0".to_string());

                messages.push(OpenAiMessage {
                    role: "tool".to_string(),
                    content: msg.content.clone(),
                    tool_calls: None,
                    tool_call_id: Some(tool_call_id),
                });
            }
            Role::Assistant => {
                if let Some(tool_calls) = &msg.tool_calls {
                    messages.push(OpenAiMessage {
                        role: "assistant".to_string(),
                        content: msg.content.clone(),
                        tool_calls: Some(
                            tool_calls
                                .iter()
                                .map(|tc| OpenAiToolCallSer {
                                    id: tc.id.clone(),
                                    call_type: "function".to_string(),
                                    function: OpenAiFunctionSer {
                                        name: tc.function.name.clone(),
                                        arguments: tc.function.arguments.to_string(),
                                    },
                                })
                                .collect(),
                        ),
                        tool_call_id: None,
                    });
                } else {
                    messages.push(OpenAiMessage {
                        role: "assistant".to_string(),
                        content: msg.content.clone(),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
            }
            Role::User => {
                messages.push(OpenAiMessage {
                    role: "user".to_string(),
                    content: msg.content.clone(),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
        }
    }

    messages
}

// --- Tool call accumulator (OpenAI streams tool calls in fragments) ---

struct ToolCallAccumulator {
    entries: Vec<(String, String, String)>, // (id, name, arguments_json)
}

impl ToolCallAccumulator {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    fn process(&mut self, tc: &SseToolCall) {
        let idx = tc.index;

        // Ensure we have enough entries
        while self.entries.len() <= idx {
            self.entries
                .push((String::new(), String::new(), String::new()));
        }

        if let Some(id) = &tc.id {
            self.entries[idx].0 = id.clone();
        }
        if let Some(func) = &tc.function {
            if let Some(name) = &func.name {
                self.entries[idx].1 = name.clone();
            }
            if let Some(args) = &func.arguments {
                self.entries[idx].2.push_str(args);
            }
        }
    }

    fn into_tool_calls(self) -> Vec<ToolCall> {
        self.entries
            .into_iter()
            .filter(|(_, name, _)| !name.is_empty())
            .map(|(id, name, args)| {
                let arguments = serde_json::from_str(&args)
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                let id = if id.is_empty() {
                    format!("call_{}", uuid::Uuid::new_v4().as_simple())
                } else {
                    id
                };
                ToolCall {
                    id,
                    function: ToolFunction { name, arguments },
                }
            })
            .collect()
    }
}

#[async_trait]
impl AiProvider for OpenRouterProvider {
    fn name(&self) -> &str {
        "openrouter"
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

        let body = OpenAiRequest {
            model: model.to_string(),
            messages: build_openai_messages(request),
            stream: true,
            tools: request.tools.clone(),
            max_tokens: Some(crate::services::model_meta::max_output_tokens("openrouter", model)),
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.config.base_url))
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(LlmError::Network)?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(match status.as_u16() {
                401 | 402 | 403 => LlmError::Authentication(format!("OpenRouter {status}: {text}")),
                429 => LlmError::RateLimit {
                    retry_after_secs: crate::services::retry::parse_retry_after_secs(&text),
                },
                400 => LlmError::InvalidRequest(format!("{status}: {text}")),
                _ => LlmError::ProviderUnavailable(format!("OpenRouter {status}: {text}")),
            }
            .into());
        }

        sender(StreamEvent::State {
            state: "typing".to_string(),
        })
        .map_err(AppError::Streaming)?;

        let mut full_response = String::new();
        let mut tool_accumulator = ToolCallAccumulator::new();
        let mut has_tool_calls = false;
        let mut byte_stream = response.bytes_stream();
        let mut parser = SseLineParser::new();

        while let Some(chunk) = byte_stream.next().await {
            let chunk =
                chunk.map_err(|e| AppError::Streaming(format!("Stream read error: {e}")))?;
            parser.push(&chunk);

            while let Some(line) = parser.next_line() {
                let data = match line {
                    SseLine::Done => break,
                    SseLine::Data(d) => d,
                };

                if let Ok(chunk) = serde_json::from_str::<SseChunk>(&data) {
                    for choice in &chunk.choices {
                        // Text content
                        if let Some(content) = &choice.delta.content {
                            if !content.is_empty() {
                                full_response.push_str(content);
                                let _ = sender(StreamEvent::Delta {
                                    delta: content.clone(),
                                });
                            }
                        }

                        // Tool calls (streamed in fragments)
                        if let Some(tool_calls) = &choice.delta.tool_calls {
                            has_tool_calls = true;
                            for tc in tool_calls {
                                tool_accumulator.process(tc);
                            }
                        }
                    }

                    // Token usage (typically in the last chunk)
                    if let Some(usage) = &chunk.usage {
                        let _ = sender(StreamEvent::Usage {
                            input_tokens: usage.prompt_tokens,
                            output_tokens: usage.completion_tokens,
                            cache_creation_tokens: 0,
                            cache_read_tokens: 0,
                        });
                    }
                }
            }
        }

        if has_tool_calls {
            Ok(ChatStreamResult::ToolCalls(
                tool_accumulator.into_tool_calls(),
            ))
        } else {
            Ok(ChatStreamResult::Text(full_response))
        }
    }
}
