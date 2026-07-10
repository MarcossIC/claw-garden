use std::sync::Mutex;
use std::time::{Duration, Instant};

const DEFAULT_FAILURE_THRESHOLD: u32 = 5;
const DEFAULT_RESET_TIMEOUT_SECS: u64 = 30;
const DEFAULT_DECAY_MULTIPLIER: f64 = 2.0;
const DEFAULT_MAX_RESET_TIMEOUT_SECS: u64 = 30 * 60; // 30 min
const DEFAULT_SUCCESS_THRESHOLD: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub enum CircuitState {
    Closed { consecutive_failures: u32 },
    Open {
        opened_at: Instant,
        reset_timeout: Duration,
        decay_count: u32,
    },
    HalfOpen {
        probe_in_flight: bool,
        decay_count: u32,
    },
}

pub struct CircuitBreakerPolicy {
    pub failure_threshold: u32,
    pub reset_timeout: Duration,
    pub decay_multiplier: f64,
    pub max_reset_timeout: Duration,
    pub success_threshold: u32,
}

impl Default for CircuitBreakerPolicy {
    fn default() -> Self {
        Self {
            failure_threshold: DEFAULT_FAILURE_THRESHOLD,
            reset_timeout: Duration::from_secs(DEFAULT_RESET_TIMEOUT_SECS),
            decay_multiplier: DEFAULT_DECAY_MULTIPLIER,
            max_reset_timeout: Duration::from_secs(DEFAULT_MAX_RESET_TIMEOUT_SECS),
            success_threshold: DEFAULT_SUCCESS_THRESHOLD,
        }
    }
}

pub enum BreakerCheck {
    Closed,
    HalfOpen,
    Open { retry_after: Duration },
}

pub struct CircuitBreaker {
    pub server_name: String,
    state: Mutex<CircuitState>,
    policy: CircuitBreakerPolicy,
}

impl CircuitBreaker {
    pub fn new(server_name: impl Into<String>) -> Self {
        Self::with_policy(server_name, CircuitBreakerPolicy::default())
    }

    pub fn with_policy(server_name: impl Into<String>, policy: CircuitBreakerPolicy) -> Self {
        Self {
            server_name: server_name.into(),
            state: Mutex::new(CircuitState::Closed { consecutive_failures: 0 }),
            policy,
        }
    }

    /// Check the circuit before attempting a call.
    pub fn check(&self) -> BreakerCheck {
        let mut state = self.state.lock().unwrap();

        match &*state {
            CircuitState::Closed { .. } => BreakerCheck::Closed,

            CircuitState::Open { opened_at, reset_timeout, decay_count } => {
                let elapsed = opened_at.elapsed();
                if elapsed >= *reset_timeout {
                    // Transition to HalfOpen
                    let dc = *decay_count;
                    *state = CircuitState::HalfOpen {
                        probe_in_flight: false,
                        decay_count: dc,
                    };
                    BreakerCheck::HalfOpen
                } else {
                    let remaining = *reset_timeout - elapsed;
                    BreakerCheck::Open { retry_after: remaining }
                }
            }

            CircuitState::HalfOpen { probe_in_flight, decay_count } => {
                if *probe_in_flight {
                    // Another probe is in flight — report as Open temporarily
                    BreakerCheck::Open {
                        retry_after: Duration::from_secs(5),
                    }
                } else {
                    // Mark probe in flight
                    let dc = *decay_count;
                    *state = CircuitState::HalfOpen {
                        probe_in_flight: true,
                        decay_count: dc,
                    };
                    BreakerCheck::HalfOpen
                }
            }
        }
    }

    /// Record a successful call.
    pub fn record_success(&self) {
        let mut state = self.state.lock().unwrap();
        match &*state {
            CircuitState::HalfOpen { .. } | CircuitState::Open { .. } => {
                // Recover: back to closed with clean slate
                *state = CircuitState::Closed { consecutive_failures: 0 };
            }
            CircuitState::Closed { .. } => {
                // Reset failure count on success
                *state = CircuitState::Closed { consecutive_failures: 0 };
            }
        }
    }

    /// Record a transient failure (Timeout, ConnectionError).
    /// Non-transient errors (NotFound, InvalidInput) should NOT call this.
    pub fn record_failure(&self) {
        let mut state = self.state.lock().unwrap();
        match &*state {
            CircuitState::Closed { consecutive_failures } => {
                let new_failures = consecutive_failures + 1;
                if new_failures >= self.policy.failure_threshold {
                    *state = CircuitState::Open {
                        opened_at: Instant::now(),
                        reset_timeout: self.policy.reset_timeout,
                        decay_count: 0,
                    };
                } else {
                    *state = CircuitState::Closed { consecutive_failures: new_failures };
                }
            }
            CircuitState::HalfOpen { decay_count, .. } => {
                // Probe failed → reopen with decay
                let new_decay = decay_count + 1;
                let new_timeout = Duration::from_secs_f64(
                    (self.policy.reset_timeout.as_secs_f64()
                        * self.policy.decay_multiplier.powi(new_decay as i32))
                    .min(self.policy.max_reset_timeout.as_secs_f64()),
                );
                *state = CircuitState::Open {
                    opened_at: Instant::now(),
                    reset_timeout: new_timeout,
                    decay_count: new_decay,
                };
            }
            CircuitState::Open { .. } => {
                // Already open — nothing to do
            }
        }
    }

    pub fn is_open(&self) -> bool {
        matches!(*self.state.lock().unwrap(), CircuitState::Open { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_after_threshold_failures() {
        let cb = CircuitBreaker::new("test");
        for _ in 0..DEFAULT_FAILURE_THRESHOLD {
            cb.record_failure();
        }
        assert!(cb.is_open());
    }

    #[test]
    fn recovers_after_success() {
        let cb = CircuitBreaker::new("test");
        for _ in 0..DEFAULT_FAILURE_THRESHOLD {
            cb.record_failure();
        }
        assert!(cb.is_open());
        cb.record_success();
        assert!(!cb.is_open());
    }

    #[test]
    fn stays_closed_below_threshold() {
        let cb = CircuitBreaker::new("test");
        for _ in 0..(DEFAULT_FAILURE_THRESHOLD - 1) {
            cb.record_failure();
        }
        assert!(!cb.is_open());
    }
}
