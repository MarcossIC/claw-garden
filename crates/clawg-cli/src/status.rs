//! The JSON `clawg status` prints.
//!
//! Assembled by a pure function so the shape is under test without a daemon:
//! the command itself only resolves a profile, does two lookups and prints.

use std::path::Path;

use clawg_core::paths::Profile;
use serde_json::{json, Value};

use crate::client::Health;
use crate::daemon::lifecycle::DaemonState;

/// Builds the status document.
///
/// `running` is derived from `health`, never from `state`: a `daemon.json` file
/// outlives the process that wrote it (a crash leaves one behind), so treating
/// its presence as liveness is exactly the mistake that sends the next client
/// to a dead port. The stale record is still reported, because seeing it is how
/// a user understands what happened.
pub fn status_report(
    profile: Profile,
    base_dir: &Path,
    state: Option<&DaemonState>,
    health: Option<&Health>,
) -> Value {
    json!({
        "running": health.is_some(),
        "profile": profile.as_str(),
        "data_dir": base_dir.display().to_string(),
        // `null` rather than an omitted key in both slots: a consumer that has
        // to check for a missing field and for a null one has two ways to be
        // wrong instead of one.
        "daemon": state.map(|s| serde_json::to_value(s).unwrap_or(Value::Null)),
        "health": health.map(|h| serde_json::to_value(h).unwrap_or(Value::Null)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> DaemonState {
        DaemonState {
            pid: 4242,
            port: 27381,
            profile: "dev".into(),
            version: "0.1.0".into(),
            started_at_ms: 1_700_000_000_000,
        }
    }

    fn health() -> Health {
        Health {
            status: "ok".into(),
            version: "0.1.0".into(),
            profile: "dev".into(),
            pid: 4242,
            embedder: "ready".into(),
            uptime_s: 12,
        }
    }

    #[test]
    fn a_live_daemon_reports_running_with_both_records() {
        let v = status_report(
            Profile::Dev,
            Path::new("/home/u/.clawg-dev"),
            Some(&state()),
            Some(&health()),
        );
        assert_eq!(v["running"], true);
        assert_eq!(v["profile"], "dev");
        assert!(v["data_dir"].as_str().is_some_and(|d| d.contains(".clawg-dev")));
        assert_eq!(v["daemon"]["pid"], 4242);
        assert_eq!(v["daemon"]["port"], 27381);
        assert_eq!(v["health"]["embedder"], "ready");
    }

    /// The case the whole function exists for: a `daemon.json` left behind by a
    /// crash must not read as a running daemon.
    #[test]
    fn a_stale_state_file_without_health_is_not_running() {
        let v = status_report(Profile::Dev, Path::new("/x"), Some(&state()), None);
        assert_eq!(v["running"], false);
        assert_eq!(
            v["daemon"]["pid"], 4242,
            "the stale record must still be shown — it is the evidence"
        );
        assert!(v["health"].is_null());
    }

    #[test]
    fn nothing_at_all_reports_not_running_with_null_records() {
        let v = status_report(Profile::Prod, Path::new("/x"), None, None);
        assert_eq!(v["running"], false);
        assert_eq!(v["profile"], "prod");
        assert!(v["daemon"].is_null());
        assert!(v["health"].is_null());
    }

    #[test]
    fn the_report_always_carries_every_key() {
        for (s, h) in [
            (Some(state()), Some(health())),
            (Some(state()), None),
            (None, None),
        ] {
            let v = status_report(Profile::Dev, Path::new("/x"), s.as_ref(), h.as_ref());
            for key in ["running", "profile", "data_dir", "daemon", "health"] {
                assert!(v.get(key).is_some(), "missing {key} in {v}");
            }
        }
    }

    #[test]
    fn the_report_serializes_as_an_object() {
        let v = status_report(Profile::Dev, Path::new("/x"), None, None);
        assert!(v.is_object());
        let _ = json!({}); // keeps the import honest
    }
}
