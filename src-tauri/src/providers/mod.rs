pub mod anthropic;
pub mod ollama;
pub mod openrouter;
pub mod sse;

use async_trait::async_trait;

use crate::models::{AppError, ChatRequest, ChatStreamResult, StreamEvent};

pub type StreamSender = Box<dyn Fn(StreamEvent) -> Result<(), String> + Send + Sync>;

#[async_trait]
pub trait AiProvider: Send + Sync {
    fn name(&self) -> &str;

    /// Streams chat completion, sending delta events via sender.
    /// Returns Text(full_response) or ToolCalls(vec) if the LLM wants to use tools.
    async fn chat_stream(
        &self,
        request: &ChatRequest,
        sender: StreamSender,
    ) -> Result<ChatStreamResult, AppError>;
}
