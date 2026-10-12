use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("Authentication failed: {0}")]
    Authentication(String),

    #[error("Rate limit exceeded. Retry after {retry_after_secs}s")]
    RateLimit { retry_after_secs: u64 },

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Provider unavailable: {0}")]
    ProviderUnavailable(String),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Provider error: {0}")]
    Provider(String),

    #[error("Streaming error: {0}")]
    Streaming(String),

    #[error(transparent)]
    Llm(#[from] LlmError),

    #[error(transparent)]
    Memory(#[from] crate::memory::MemoryError),
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryError;

    #[test]
    fn from_memory_error_to_app_error() {
        let mem_err = MemoryError::VecExtensionRegistration("x".into());
        let app_err: AppError = mem_err.into();
        assert!(
            matches!(app_err, AppError::Memory(_)),
            "Expected AppError::Memory, got {:?}",
            app_err
        );
    }
}
