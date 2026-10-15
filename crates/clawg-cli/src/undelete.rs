//! `clawg undelete`: bring back a memory that was deleted.
//!
//! Restoring embeds the row again, so it goes through the running daemon and
//! its loaded model; without a daemon nothing is opened.

use crate::client::{ClientError, DaemonClient, Restored};

/// What `clawg undelete` prints for a restored row.
pub fn report(r: &Restored) -> String {
    match r.superseded_by {
        Some(by) if r.status == "superseded" => format!(
            "restored memory #{}; it is still superseded by #{by}, so searches leave it out",
            r.id
        ),
        _ => format!("restored memory #{}", r.id),
    }
}

/// Restores the `memory` row `id`, unscoped: the user's own command reaches
/// every agent's rows, as `clawg purge` does.
pub async fn run(client: &DaemonClient, id: i64) -> Result<String, ClientError> {
    client.restore("memory", id, None).await.map(|r| report(&r))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tests::{publish, save_and_wait, start_daemon};
    use crate::client::ListArgs;

    #[test]
    fn report_names_the_row_and_a_replacement() {
        let active = Restored { id: 4, status: "active".into(), superseded_by: None };
        assert_eq!(report(&active), "restored memory #4");
        let replaced = Restored { id: 4, status: "superseded".into(), superseded_by: Some(9) };
        assert_eq!(report(&replaced), "restored memory #4; it is still superseded by #9, so searches leave it out");
    }

    #[tokio::test]
    async fn undelete_restores_through_the_daemon() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");
        save_and_wait(&c, "the release train leaves on tuesday", "agent-a", None, 1).await;
        let id = c.list(ListArgs::default()).await.expect("list")[0].id;
        c.delete("memory", id, None).await.expect("delete");
        let deleted = c.list(ListArgs { deleted: true, ..Default::default() }).await.expect("list");
        assert_eq!(deleted.iter().map(|r| r.id).collect::<Vec<_>>(), vec![id]);

        assert_eq!(run(&c, id).await.expect("undelete"), format!("restored memory #{id}"));

        assert_eq!(c.list(ListArgs::default()).await.expect("list")[0].id, id);
        let err = run(&c, id).await.expect_err("nothing left to restore");
        assert!(matches!(err, ClientError::Daemon { status: 404, .. }), "{err:?}");
    }
}
