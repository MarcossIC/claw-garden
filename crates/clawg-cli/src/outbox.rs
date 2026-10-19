//! `clawg outbox`: retry or discard a save the daemon gave up on.
//!
//! The daemon owns the store and the ingest queue a retried save goes back
//! to, so both go through it; without a daemon nothing is opened.

use crate::cli::OutboxAction;
use crate::client::{ClientError, DaemonClient, OutboxDiscarded, OutboxRetried};

/// What `clawg outbox retry` prints.
pub fn retried_report(r: &OutboxRetried) -> String {
    let next = if r.queued {
        "the daemon is storing it"
    } else {
        "the daemon's queue is full, so it is stored on the daemon's next start"
    };
    format!("operation {} (agent {}) is pending again; {next}", r.operation_id, r.agent_id)
}

/// What `clawg outbox discard` prints.
pub fn discarded_report(d: &OutboxDiscarded) -> String {
    let head = format!("discarded operation {} (agent {})", d.operation_id, d.agent_id);
    if d.deleted.is_empty() {
        return format!("{head}; it had stored nothing");
    }
    let ids: Vec<String> = d.deleted.iter().map(|id| format!("#{id}")).collect();
    format!(
        "{head} and deleted the memories it had stored: {} (`clawg undelete <ID>` brings one back)",
        ids.join(", ")
    )
}

/// Runs `action` through the daemon, unscoped unless it names an agent: the
/// user's own command reaches every agent's saves, as `clawg undelete` does.
pub async fn run(client: &DaemonClient, action: &OutboxAction) -> Result<String, ClientError> {
    let target = action.target();
    let agent = target.agent.as_deref();
    match action {
        OutboxAction::Retry(_) => client.outbox_retry(&target.operation_id, agent).await.map(|r| retried_report(&r)),
        OutboxAction::Discard(_) => client.outbox_discard(&target.operation_id, agent).await.map(|d| discarded_report(&d)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbox_reports_say_what_was_done() {
        let retried = OutboxRetried { operation_id: "op-1".into(), agent_id: "a".into(), queued: true };
        assert_eq!(retried_report(&retried), "operation op-1 (agent a) is pending again; the daemon is storing it");
        let waiting = OutboxRetried { queued: false, ..retried };
        assert_eq!(
            retried_report(&waiting),
            "operation op-1 (agent a) is pending again; the daemon's queue is full, so it is stored on the daemon's next start"
        );

        let nothing = OutboxDiscarded { operation_id: "op-1".into(), agent_id: "a".into(), deleted: vec![] };
        assert_eq!(discarded_report(&nothing), "discarded operation op-1 (agent a); it had stored nothing");
        let pieces = OutboxDiscarded { deleted: vec![4, 5], ..nothing };
        assert_eq!(
            discarded_report(&pieces),
            "discarded operation op-1 (agent a) and deleted the memories it had stored: #4, #5 \
             (`clawg undelete <ID>` brings one back)"
        );
    }
}
