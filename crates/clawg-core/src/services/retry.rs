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
