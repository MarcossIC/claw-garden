//! The one shape every check reports in, so a script reads them all alike.

use serde::Serialize;
use serde_json::Value;

/// How much a finding matters. Only `Error` fails `clawg doctor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Ok,
    Info,
    Warn,
    Error,
}

/// One thing a check found: what is wrong, why, the facts that show it and
/// the next step that is safe to take. The evidence holds ids, counts, paths
/// and names, never the text of a row.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    pub check_id: &'static str,
    pub severity: Severity,
    pub reason_code: &'static str,
    pub message: String,
    pub evidence: Value,
    pub safe_next_step: Option<String>,
    /// Whether the next step changes stored data.
    pub requires_confirmation: bool,
}

impl Finding {
    pub fn new(check_id: &'static str, severity: Severity, reason_code: &'static str, message: impl Into<String>) -> Self {
        Finding {
            check_id,
            severity,
            reason_code,
            message: message.into(),
            evidence: Value::Object(Default::default()),
            safe_next_step: None,
            requires_confirmation: false,
        }
    }

    /// The check ran and found nothing wrong.
    pub fn ok(check_id: &'static str, message: impl Into<String>) -> Self {
        Finding::new(check_id, Severity::Ok, "healthy", message)
    }

    pub fn evidence(mut self, evidence: Value) -> Self {
        self.evidence = evidence;
        self
    }

    /// A next step that only reads, or writes nothing the user saved.
    pub fn step(mut self, command: impl Into<String>) -> Self {
        self.safe_next_step = Some(command.into());
        self
    }

    /// A next step that changes stored data: migrates the store or stores
    /// saves again.
    pub fn changing_step(mut self, command: impl Into<String>) -> Self {
        self.safe_next_step = Some(command.into());
        self.requires_confirmation = true;
        self
    }
}

/// How many findings of each severity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub ok: usize,
    pub info: usize,
    pub warn: usize,
    pub error: usize,
}

impl Summary {
    pub fn of(findings: &[Finding]) -> Summary {
        let mut s = Summary::default();
        for f in findings {
            *match f.severity {
                Severity::Ok => &mut s.ok,
                Severity::Info => &mut s.info,
                Severity::Warn => &mut s.warn,
                Severity::Error => &mut s.error,
            } += 1;
        }
        s
    }

    /// `clawg doctor` exits 1 on an error and 0 otherwise: a warning is for
    /// the user to read, and a script that checks the exit code must not
    /// start failing over an old backup.
    pub fn fails(&self) -> bool {
        self.error > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_finding_serialises_its_envelope() {
        let f = Finding::new("backup.age", Severity::Warn, "stale", "the newest snapshot is 40 days old")
            .evidence(json!({ "age_days": 40 }))
            .step("clawg backup --profile dev");
        assert_eq!(
            serde_json::to_value(&f).unwrap(),
            json!({
                "check_id": "backup.age",
                "severity": "warn",
                "reason_code": "stale",
                "message": "the newest snapshot is 40 days old",
                "evidence": { "age_days": 40 },
                "safe_next_step": "clawg backup --profile dev",
                "requires_confirmation": false,
            })
        );
        let ok = serde_json::to_value(Finding::ok("sharing", "fine")).unwrap();
        assert_eq!((ok["reason_code"].as_str(), ok["safe_next_step"].is_null()), (Some("healthy"), true));
        assert_eq!(ok["evidence"], json!({}));
    }

    #[test]
    fn the_summary_counts_findings_by_severity() {
        let findings = [
            Finding::ok("a", ""),
            Finding::ok("b", ""),
            Finding::new("c", Severity::Info, "x", ""),
            Finding::new("d", Severity::Warn, "x", ""),
        ];
        assert_eq!(Summary::of(&findings), Summary { ok: 2, info: 1, warn: 1, error: 0 });
    }

    #[test]
    fn the_exit_code_fails_only_on_an_error() {
        let warn = [Finding::new("d", Severity::Warn, "x", ""), Finding::new("e", Severity::Info, "x", "")];
        assert!(!Summary::of(&warn).fails());
        let error = [Finding::ok("a", ""), Finding::new("f", Severity::Error, "x", "")];
        assert!(Summary::of(&error).fails());
    }
}
