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
pub struct OllamaConfig {
    pub host: String,
    pub port: u16,
    pub model: String,
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 11434,
            model: "llama3.1".to_string(),
        }
    }
}

pub struct OllamaProvider {
    config: OllamaConfig,
    client: reqwest::Client,
}

impl OllamaProvider {
    pub fn new(config: OllamaConfig) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self { config, client }
    }

    fn base_url(&self) -> String {
        format!("http://{}:{}", self.config.host, self.config.port)
    }
}

// --- Ollama request/response types ---

#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolDefinition>>,
}

#[derive(Serialize)]
struct OllamaChatMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OllamaToolCall>>,
}

#[derive(Serialize)]
struct OllamaToolCall {
    function: OllamaToolFunction,
}

#[derive(Serialize)]
struct OllamaToolFunction {
    name: String,
    arguments: serde_json::Value,
}

#[derive(Deserialize)]
struct OllamaStreamLine {
    message: Option<OllamaResponseMessage>,
    #[allow(dead_code)]
    done: bool,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    #[serde(default)]
    content: String,
    #[serde(default)]
    tool_calls: Option<Vec<OllamaResponseToolCall>>,
}

#[derive(Deserialize)]
struct OllamaResponseToolCall {
    function: OllamaResponseFunction,
}

#[derive(Deserialize)]
struct OllamaResponseFunction {
    name: String,
    arguments: serde_json::Value,
}

fn role_to_string(role: &Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "system",
        Role::Tool => "tool",
    }
}

fn build_ollama_messages(request: &ChatRequest) -> Vec<OllamaChatMessage> {
    let mut messages = Vec::new();

    if let Some(system) = &request.system_prompt {
        let content = match &request.custom_instructions {
            Some(ci) if !ci.trim().is_empty() => {
                format!("{system}\n\nAdditional instructions:\n{ci}")
            }
            _ => system.clone(),
        };
        messages.push(OllamaChatMessage {
            role: "system".to_string(),
            content,
            tool_calls: None,
        });
    }

    for msg in &request.messages {
        messages.push(OllamaChatMessage {
            role: role_to_string(&msg.role).to_string(),
            content: msg.content.clone(),
            tool_calls: msg.tool_calls.as_ref().map(|tcs| {
                tcs.iter()
                    .map(|tc| OllamaToolCall {
                        function: OllamaToolFunction {
                            name: tc.function.name.clone(),
                            arguments: tc.function.arguments.clone(),
                        },
                    })
                    .collect()
            }),
        });
    }

    messages
}

#[async_trait]
impl AiProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
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

        let body = OllamaChatRequest {
            model: model.to_string(),
            messages: build_ollama_messages(request),
            stream: true,
            tools: request.tools.clone(),
        };

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url()))
            .json(&body)
            .send()
            .await
            .map_err(LlmError::Network)?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(match status.as_u16() {
                400 => LlmError::InvalidRequest(format!("{status}: {text}")),
                _ => LlmError::ProviderUnavailable(format!("Ollama {status}: {text}")),
            }
            .into());
        }

        sender(StreamEvent::State {
            state: "typing".to_string(),
        })
        .map_err(AppError::Streaming)?;

        let mut full_response = String::new();
        let mut collected_tool_calls: Vec<ToolCall> = Vec::new();
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

                if let Ok(parsed) = serde_json::from_str::<OllamaStreamLine>(&data) {
                    if let Some(msg) = &parsed.message {
                        // Check for tool calls
                        if let Some(tool_calls) = &msg.tool_calls {
                            for tc in tool_calls {
                                collected_tool_calls.push(ToolCall {
                                    id: format!("toolu_{}", uuid::Uuid::new_v4().as_simple()),
                                    function: ToolFunction {
                                        name: tc.function.name.clone(),
                                        arguments: tc.function.arguments.clone(),
                                    },
                                });
                            }
                        }

                        // Stream text deltas
                        if !msg.content.is_empty() {
                            full_response.push_str(&msg.content);
                            let _ = sender(StreamEvent::Delta {
                                delta: msg.content.clone(),
                            });
                        }
                    }
                }
            }
        }

        // If the LLM returned tool calls, return those instead of text
        if !collected_tool_calls.is_empty() {
            Ok(ChatStreamResult::ToolCalls(collected_tool_calls))
        } else {
            Ok(ChatStreamResult::Text(full_response))
        }
    }
}
