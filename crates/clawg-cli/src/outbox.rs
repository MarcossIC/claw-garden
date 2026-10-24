//! `clawg outbox`: retry or discard a save the daemon gave up on.
//!
//! The daemon owns the store and the ingest queue a retried save goes back
//! to, so both go through it; without a daemon nothing is opened.

use clawg_core::preferences::Lang;

use crate::cli::OutboxAction;
use crate::client::{ClientError, DaemonClient, OutboxDiscarded, OutboxRetried};

/// What `clawg outbox retry` prints.
pub fn retried_report(lang: Lang, r: &OutboxRetried) -> String {
    let next = match (r.queued, lang) {
        (true, Lang::En) => "the daemon is storing it",
        (false, Lang::En) => "the daemon's queue is full, so it is stored on the daemon's next start",
        (true, Lang::Es) => "el daemon la está guardando",
        (false, Lang::Es) => "la cola del daemon está llena, así que se guarda en el próximo arranque del daemon",
    };
    match lang {
        Lang::En => format!("operation {} (agent {}) is pending again; {next}", r.operation_id, r.agent_id),
        Lang::Es => format!("la operación {} (agente {}) vuelve a estar pendiente; {next}", r.operation_id, r.agent_id),
    }
}

/// What `clawg outbox discard` prints.
pub fn discarded_report(lang: Lang, d: &OutboxDiscarded) -> String {
    let head = match lang {
        Lang::En => format!("discarded operation {} (agent {})", d.operation_id, d.agent_id),
        Lang::Es => format!("se descartó la operación {} (agente {})", d.operation_id, d.agent_id),
    };
    if d.deleted.is_empty() {
        return match lang {
            Lang::En => format!("{head}; it had stored nothing"),
            Lang::Es => format!("{head}; no había guardado nada"),
        };
    }
    let ids: Vec<String> = d.deleted.iter().map(|id| format!("#{id}")).collect();
    let ids = ids.join(", ");
    match lang {
        Lang::En => format!("{head} and deleted the memories it had stored: {ids} (`clawg undelete <ID>` brings one back)"),
        Lang::Es => format!("{head} y se borraron las memorias que había guardado: {ids} (`clawg undelete <ID>` trae una de vuelta)"),
    }
}

/// Runs `action` through the daemon, unscoped unless it names an agent: the
/// user's own command reaches every agent's saves, as `clawg undelete` does.
pub async fn run(lang: Lang, client: &DaemonClient, action: &OutboxAction) -> Result<String, ClientError> {
    let target = action.target();
    let agent = target.agent.as_deref();
    match action {
        OutboxAction::Retry(_) => client.outbox_retry(&target.operation_id, agent).await.map(|r| retried_report(lang, &r)),
        OutboxAction::Discard(_) => client.outbox_discard(&target.operation_id, agent).await.map(|d| discarded_report(lang, &d)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbox_reports_say_what_was_done() {
        let retried = OutboxRetried { operation_id: "op-1".into(), agent_id: "a".into(), queued: true };
        assert_eq!(retried_report(Lang::En, &retried), "operation op-1 (agent a) is pending again; the daemon is storing it");
        let waiting = OutboxRetried { queued: false, ..retried };
        assert_eq!(
            retried_report(Lang::En, &waiting),
            "operation op-1 (agent a) is pending again; the daemon's queue is full, so it is stored on the daemon's next start"
        );

        let nothing = OutboxDiscarded { operation_id: "op-1".into(), agent_id: "a".into(), deleted: vec![] };
        assert_eq!(discarded_report(Lang::En, &nothing), "discarded operation op-1 (agent a); it had stored nothing");
        let pieces = OutboxDiscarded { deleted: vec![4, 5], ..nothing };
        assert_eq!(
            discarded_report(Lang::En, &pieces),
            "discarded operation op-1 (agent a) and deleted the memories it had stored: #4, #5 \
             (`clawg undelete <ID>` brings one back)"
        );
    }

    #[test]
    fn the_outbox_reports_speak_spanish() {
        let retried = OutboxRetried { operation_id: "op-1".into(), agent_id: "a".into(), queued: true };
        assert_eq!(
            retried_report(Lang::Es, &retried),
            "la operación op-1 (agente a) vuelve a estar pendiente; el daemon la está guardando"
        );
        let nothing = OutboxDiscarded { operation_id: "op-1".into(), agent_id: "a".into(), deleted: vec![] };
        assert_eq!(discarded_report(Lang::Es, &nothing), "se descartó la operación op-1 (agente a); no había guardado nada");
        let pieces = OutboxDiscarded { deleted: vec![4, 5], ..nothing };
        assert_eq!(
            discarded_report(Lang::Es, &pieces),
            "se descartó la operación op-1 (agente a) y se borraron las memorias que había guardado: #4, #5 \
             (`clawg undelete <ID>` trae una de vuelta)"
        );
    }
}
