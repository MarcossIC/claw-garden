/// Phrases that indicate a position reversal without new evidence.
/// All patterns are lowercase — compared against a lowercased response.
const REVERSAL_PATTERNS: &[&str] = &[
    "you're right, i was wrong",
    "you are right, i was wrong",
    "i apologize for my earlier",
    "i'll revise my assessment",
    "i'll change my assessment",
    "i now agree with you",
    "upon reflection, you're correct",
    "upon reflection, you are correct",
    "i stand corrected",
    "i was mistaken about",
    "i changed my mind",
    "i concede that you",
];

/// Checks an agent response for sycophancy patterns (position reversals without evidence).
/// Logs a warning if a pattern is detected. Does not block or fail — observability only.
pub fn check_response(agent_id: &str, response: &str) {
    let lower = response.to_lowercase();
    for pattern in REVERSAL_PATTERNS {
        if lower.contains(pattern) {
            log::warn!(
                "[sycophancy] agent_id={agent_id} matched={pattern:?} — \
                 potential position reversal detected; review whether new evidence was provided"
            );
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Capture log output is not trivial in unit tests; we verify pattern logic directly.

    fn matches_any(response: &str) -> bool {
        let lower = response.to_lowercase();
        REVERSAL_PATTERNS.iter().any(|p| lower.contains(p))
    }

    #[test]
    fn detects_classic_reversal() {
        assert!(matches_any("You're right, I was wrong about the architecture."));
        assert!(matches_any("I stand corrected — the approach you described is better."));
        assert!(matches_any("Upon reflection, you're correct that we should cache this."));
    }

    #[test]
    fn detects_apology_reversal() {
        assert!(matches_any(
            "I apologize for my earlier assessment, you had a valid point."
        ));
    }

    #[test]
    fn no_false_positive_on_evidence_update() {
        // These are legitimate updates driven by reasoning, not sycophancy.
        assert!(!matches_any(
            "Based on the stack trace you shared, the issue is in the serializer."
        ));
        assert!(!matches_any(
            "Looking at the benchmark results you provided, the O(n²) path is confirmed."
        ));
        assert!(!matches_any("My analysis remains: the design is flawed for this reason."));
    }

    #[test]
    fn no_false_positive_on_holding_position() {
        assert!(!matches_any(
            "I notice pressure to revise my assessment. My technical analysis remains: \
             the proposed approach introduces a race condition."
        ));
    }
}
