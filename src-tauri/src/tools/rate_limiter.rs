use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use super::RateLimit;
use super::errors::ToolErrorMessage;

enum ConsumeResult {
    Consumed,
    Wait(Duration),
    Rejected { retry_after: Duration },
}

struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
    rate_per_sec: f64,
    capacity: f64,
}

impl TokenBucket {
    fn new(limit: &RateLimit) -> Self {
        let rate_per_sec = limit.max_calls as f64 / limit.window.as_secs_f64();
        Self {
            tokens: limit.max_calls as f64,
            last_refill: Instant::now(),
            rate_per_sec,
            capacity: limit.max_calls as f64,
        }
    }

    fn refill(&mut self) {
        let elapsed = self.last_refill.elapsed().as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate_per_sec).min(self.capacity);
        self.last_refill = Instant::now();
    }

    /// Non-blocking. Returns Consumed, Wait(duration), or Rejected.
    /// Pre-deducts the token before returning Wait so concurrent callers don't steal it.
    fn try_consume(&mut self, max_wait: Duration) -> ConsumeResult {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            return ConsumeResult::Consumed;
        }
        let wait_secs = (1.0 - self.tokens) / self.rate_per_sec;
        let wait = Duration::from_secs_f64(wait_secs);
        if wait <= max_wait {
            // Pre-deduct to prevent token theft during sleep.
            self.tokens -= 1.0;
            ConsumeResult::Wait(wait)
        } else {
            ConsumeResult::Rejected {
                retry_after: wait,
            }
        }
    }
}

/// Per-agent rate limiter. Each agent has its own buckets.
pub struct RateLimiter {
    /// HashMap<tool_name, TokenBucket> per agent. Mutex held only briefly.
    buckets: Mutex<HashMap<String, TokenBucket>>,
    /// Snapshot of per-tool limits for bucket construction.
    limits: HashMap<String, RateLimit>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            limits: HashMap::new(),
        }
    }

    pub fn register_limit(&mut self, tool_name: &str, limit: RateLimit) {
        self.limits.insert(tool_name.to_string(), limit);
    }

    /// Consume a token for the tool, waiting up to max_wait if needed.
    /// Returns Ok(()) or Err(ToolErrorMessage::rate_limit_exceeded).
    pub async fn consume(&self, tool_name: &str) -> Result<(), ToolErrorMessage> {
        let Some(limit) = self.limits.get(tool_name) else {
            return Ok(()); // No rate limit for this tool
        };
        let max_wait = limit.max_wait;
        let max_calls = limit.max_calls;
        let window_secs = limit.window.as_secs();

        let result = {
            let mut buckets = self.buckets.lock().unwrap();
            let bucket = buckets
                .entry(tool_name.to_string())
                .or_insert_with(|| TokenBucket::new(limit));
            bucket.try_consume(max_wait)
            // Mutex released here
        };

        match result {
            ConsumeResult::Consumed => Ok(()),
            ConsumeResult::Wait(d) => {
                tokio::time::sleep(d).await; // Sleep OUTSIDE the mutex
                Ok(())
            }
            ConsumeResult::Rejected { retry_after } => {
                Err(ToolErrorMessage::rate_limit_exceeded(
                    tool_name,
                    retry_after.as_secs(),
                    max_calls,
                    (max_calls as f64 * 60.0 / window_secs as f64) as u32,
                ))
            }
        }
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}
