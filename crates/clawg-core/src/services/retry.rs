use std::time::Duration;

use rand::Rng;

use crate::models::{AppError, LlmError};

pub struct RetryConfig {
    pub max_retries: u32,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub backoff_factor: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 1_000,
            max_delay_ms: 30_000,
            backoff_factor: 2.0,
        }
    }
}

/// Returns `true` if the error is transient and a retry may succeed.
/// Only covers provider-level errors (HTTP status, connection). Streaming
/// errors mid-flight are NOT retried to avoid duplicate content in the UI.
pub fn is_retryable(error: &AppError) -> bool {
    let AppError::Llm(e) = error else {
        return false;
    };
    matches!(
        e,
        LlmError::RateLimit { .. } | LlmError::ProviderUnavailable(_) | LlmError::Network(_)
    )
}

/// Exponential backoff with ±25% jitter to avoid thundering-herd.
pub fn compute_delay(config: &RetryConfig, attempt: u32) -> Duration {
    let base = config.initial_delay_ms as f64 * config.backoff_factor.powi(attempt as i32);
    let capped = base.min(config.max_delay_ms as f64);
    let jitter_range = capped * 0.25;
    let jitter: f64 = rand::rng().random_range(-jitter_range..=jitter_range);
    Duration::from_millis((capped + jitter).max(0.0) as u64)
}

/// Extracts the provider-supplied retry-after hint, if any.
pub fn extract_retry_after(error: &AppError) -> Option<Duration> {
    match error {
        AppError::Llm(LlmError::RateLimit { retry_after_secs }) => {
            Some(Duration::from_secs(*retry_after_secs))
        }
        _ => None,
    }
}

/// Parses "retry after N" or "retry-after: N" (seconds) from a response body.
/// Falls back to 60 seconds if no hint is found.
pub fn parse_retry_after_secs(body: &str) -> u64 {
    let lower = body.to_lowercase();
    for pattern in &["retry after ", "retry-after: "] {
        if let Some(pos) = lower.find(pattern) {
            let start = pos + pattern.len();
            let end = lower[start..]
                .find(|c: char| !c.is_ascii_digit())
                .map(|i| start + i)
                .unwrap_or(lower.len());
            if let Ok(secs) = lower[start..end].parse::<u64>() {
                return secs;
            }
        }
    }
    60
}
