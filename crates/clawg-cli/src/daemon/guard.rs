//! Which requests may reach the daemon at all, before the bearer token is
//! looked at.
//!
//! The token keeps out whoever cannot read the data directory, but a web page
//! open in the user's browser can still send requests to `127.0.0.1:<port>`,
//! and through DNS rebinding read the answers of anything that does not ask for
//! the token. Such a request names the attacker's host, not a loopback name, or
//! carries an `Origin`; no client of this daemon does either (`DaemonClient`
//! dials `http://127.0.0.1:<port>` and sends no `Origin`). The port is not
//! checked: rebinding is stopped by the name, and the port is not known yet
//! where the router is built for an ephemeral listener.
//!
//! This is no barrier against a local process: one that can read the token
//! file sends the right `Host` too.

use std::sync::Mutex;
use std::time::{Duration, Instant};

const LOOPBACK_NAMES: [&str; 3] = ["127.0.0.1", "localhost", "[::1]"];

/// How often a refusal may reach the log, so a page that keeps trying does
/// not flood it.
pub const REFUSAL_LOG_EVERY: Duration = Duration::from_secs(60);

/// Whether a request may go on: every authority it names (its `Host` header
/// and the authority of its URI, the only one an HTTP/2 request may carry) is
/// a loopback name, it names at least one, and it carries no `Origin` at all.
pub fn admits(host: Option<&str>, uri_authority: Option<&str>, origin: Option<&str>) -> bool {
    if origin.is_some() {
        return false;
    }
    let mut named = [host, uri_authority].into_iter().flatten().peekable();
    named.peek().is_some() && named.all(is_loopback_authority)
}

fn is_loopback_authority(authority: &str) -> bool {
    let (name, port) = match authority.strip_prefix('[') {
        Some(_) => match authority.find(']') {
            Some(end) => authority.split_at(end + 1),
            None => return false,
        },
        None => authority.split_at(authority.find(':').unwrap_or(authority.len())),
    };
    let port_ok = match port.strip_prefix(':') {
        Some(digits) => !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()),
        None => port.is_empty(),
    };
    port_ok && LOOPBACK_NAMES.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// Counts refusals and says when one is due in the log.
#[derive(Debug, Default)]
pub struct RefusalLog {
    state: Mutex<RefusalState>,
}

#[derive(Debug, Default)]
struct RefusalState {
    last_logged: Option<Instant>,
    unlogged: u64,
}

impl RefusalLog {
    /// Records a refusal at `now`. Returns how many refusals the line due now
    /// reports (this one included), or `None` while the last line is younger
    /// than [`REFUSAL_LOG_EVERY`].
    pub fn refused(&self, now: Instant) -> Option<u64> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.unlogged += 1;
        let due = state
            .last_logged
            .is_none_or(|last| now.saturating_duration_since(last) >= REFUSAL_LOG_EVERY);
        if !due {
            return None;
        }
        state.last_logged = Some(now);
        Some(std::mem::take(&mut state.unlogged))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(h: &str) -> bool {
        admits(Some(h), None, None)
    }

    #[test]
    fn loopback_names_are_admitted_with_or_without_a_port() {
        for h in [
            "127.0.0.1:27380",
            "127.0.0.1",
            "localhost:27381",
            "localhost",
            "LOCALHOST",
            "LocalHost:1",
            "[::1]:27380",
            "[::1]",
        ] {
            assert!(host(h), "{h} must be admitted");
        }
    }

    #[test]
    fn a_foreign_or_lookalike_host_is_refused() {
        for h in [
            "evil.example:27380",
            "evil.example",
            "127.0.0.1.evil.example",
            "localhost.evil.example:27380",
            "localhost.",
            "127.0.0.2",
            "0.0.0.0:27380",
            "127.0.0.1:x",
            "127.0.0.1:",
            "127.0.0.1:27380:1",
            "[::1",
            "[::1]x",
            "[::2]:27380",
            "",
        ] {
            assert!(!host(h), "{h} must be refused");
        }
    }

    #[test]
    fn any_origin_is_refused() {
        for origin in ["http://evil.example", "http://127.0.0.1:27380", "null", ""] {
            assert!(
                !admits(Some("127.0.0.1:27380"), None, Some(origin)),
                "Origin {origin:?} must be refused"
            );
        }
    }

    #[test]
    fn a_request_without_an_authority_is_refused() {
        assert!(!admits(None, None, None));
    }

    /// An HTTP/2 request names its authority only in the URI.
    #[test]
    fn the_uri_authority_stands_in_for_a_missing_host() {
        assert!(admits(None, Some("127.0.0.1:27380"), None));
        assert!(!admits(None, Some("evil.example:27380"), None));
    }

    /// An absolute URI overrides `Host` for the server, so both must be ours.
    #[test]
    fn a_foreign_uri_authority_is_refused_behind_a_loopback_host() {
        assert!(!admits(Some("127.0.0.1:27380"), Some("evil.example"), None));
        assert!(!admits(Some("evil.example"), Some("127.0.0.1:27380"), None));
        assert!(admits(Some("localhost:27380"), Some("127.0.0.1:27380"), None));
    }

    #[test]
    fn the_first_refusal_is_logged_and_the_next_wait_a_minute() {
        let log = RefusalLog::default();
        let t0 = Instant::now();

        assert_eq!(log.refused(t0), Some(1));
        assert_eq!(log.refused(t0 + Duration::from_secs(1)), None);
        assert_eq!(log.refused(t0 + Duration::from_secs(59)), None);
        assert_eq!(log.refused(t0 + REFUSAL_LOG_EVERY), Some(3));
        assert_eq!(log.refused(t0 + REFUSAL_LOG_EVERY + Duration::from_secs(1)), None);
        assert_eq!(log.refused(t0 + REFUSAL_LOG_EVERY * 3), Some(2));
    }
}
