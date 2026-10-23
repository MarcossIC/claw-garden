//! `clawg undelete`: bring back a memory that was deleted.
//!
//! Restoring embeds the row again, so it goes through the running daemon and
//! its loaded model; without a daemon nothing is opened.

use clawg_core::preferences::Lang;

use crate::client::{ClientError, DaemonClient, Restored};

/// What `clawg undelete` prints for a restored row. The excerpts are the
/// rows' own text, in whatever language they were saved.
pub fn report(lang: Lang, r: &Restored) -> String {
    let key = match (r.topic_key.as_deref(), lang) {
        (Some(k), Lang::En) => format!(" (topic {k})"),
        (Some(k), Lang::Es) => format!(" (tema {k})"),
        (None, _) => String::new(),
    };
    let id = r.id;
    let mut out = match (r.superseded_by, lang) {
        (Some(by), Lang::En) if r.status == "superseded" && r.superseded_by_deleted => format!(
            "restored memory #{id}{key}; it is still superseded by #{by}, which is deleted too: \
             `clawg undelete {by}` brings it back"
        ),
        (Some(by), Lang::Es) if r.status == "superseded" && r.superseded_by_deleted => format!(
            "se restauró la memoria #{id}{key}; sigue reemplazada por #{by}, que también está borrada: \
             `clawg undelete {by}` la trae de vuelta"
        ),
        (Some(by), Lang::En) if r.status == "superseded" => {
            format!("restored memory #{id}{key}; it is still superseded by #{by}, so searches leave it out")
        }
        (Some(by), Lang::Es) if r.status == "superseded" => {
            format!("se restauró la memoria #{id}{key}; sigue reemplazada por #{by}, así que las búsquedas la dejan afuera")
        }
        (_, Lang::En) => format!("restored memory #{id}{key}"),
        (_, Lang::Es) => format!("se restauró la memoria #{id}{key}"),
    };
    if !r.replaced.is_empty() {
        let named: Vec<String> = r.replaced.iter().map(|m| format!("#{} \"{}\"", m.id, m.excerpt)).collect();
        let named = named.join(", ");
        out.push_str(&match lang {
            Lang::En => format!("\nsuperseded again: {named}"),
            Lang::Es => format!("\nreemplazadas de nuevo: {named}"),
        });
    }
    for m in &r.related {
        out.push_str(&match (m.escalated, lang) {
            (false, Lang::En) => format!("\nclose to #{} \"{}\", saved while it was deleted", m.id, m.excerpt),
            (true, Lang::En) => format!("\nclose to #{} \"{}\", saved while it was deleted (put to the user)", m.id, m.excerpt),
            (false, Lang::Es) => format!("\ncerca de #{} \"{}\", guardada mientras estaba borrada", m.id, m.excerpt),
            (true, Lang::Es) => {
                format!("\ncerca de #{} \"{}\", guardada mientras estaba borrada (se le pregunta a la persona)", m.id, m.excerpt)
            }
        });
    }
    out
}

/// Restores the `memory` row `id`, unscoped: the user's own command reaches
/// every agent's rows, as `clawg purge` does.
pub async fn run(lang: Lang, client: &DaemonClient, id: i64) -> Result<String, ClientError> {
    client.restore("memory", id, None).await.map(|r| report(lang, &r))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tests::{publish, save_and_wait, start_daemon};
    use crate::client::{ListArgs, RelatedMemory, ReplacedMemory};

    #[test]
    fn report_names_the_row_and_a_replacement() {
        let active = Restored { id: 4, status: "active".into(), ..Restored::default() };
        assert_eq!(report(Lang::En, &active), "restored memory #4");
        let replaced = Restored { id: 4, status: "superseded".into(), superseded_by: Some(9), ..Restored::default() };
        assert_eq!(report(Lang::En, &replaced), "restored memory #4; it is still superseded by #9, so searches leave it out");
        let gone = Restored { superseded_by_deleted: true, ..replaced };
        assert_eq!(
            report(Lang::En, &gone),
            "restored memory #4; it is still superseded by #9, which is deleted too: `clawg undelete 9` brings it back"
        );
        let full = Restored {
            topic_key: Some("storage/engine".into()),
            replaced: vec![ReplacedMemory { id: 2, excerpt: "Storage engine is SQLite".into() }],
            related: vec![RelatedMemory { id: 7, escalated: true, excerpt: "storage engine is postgres".into() }],
            ..active
        };
        assert_eq!(
            report(Lang::En, &full),
            "restored memory #4 (topic storage/engine)\n\
             superseded again: #2 \"Storage engine is SQLite\"\n\
             close to #7 \"storage engine is postgres\", saved while it was deleted (put to the user)"
        );
    }

    #[test]
    fn the_undelete_report_speaks_spanish() {
        let replaced = Restored {
            id: 4,
            status: "superseded".into(),
            superseded_by: Some(9),
            topic_key: Some("storage/engine".into()),
            ..Restored::default()
        };
        assert_eq!(
            report(Lang::Es, &replaced),
            "se restauró la memoria #4 (tema storage/engine); sigue reemplazada por #9, así que las búsquedas la dejan afuera"
        );
        let full = Restored {
            id: 4,
            status: "active".into(),
            replaced: vec![ReplacedMemory { id: 2, excerpt: "Storage engine is SQLite".into() }],
            related: vec![RelatedMemory { id: 7, escalated: true, excerpt: "storage engine is postgres".into() }],
            ..Restored::default()
        };
        assert_eq!(
            report(Lang::Es, &full),
            "se restauró la memoria #4\n\
             reemplazadas de nuevo: #2 \"Storage engine is SQLite\"\n\
             cerca de #7 \"storage engine is postgres\", guardada mientras estaba borrada (se le pregunta a la persona)"
        );
        let gone = Restored { superseded_by_deleted: true, topic_key: None, ..replaced };
        assert_eq!(
            report(Lang::Es, &gone),
            "se restauró la memoria #4; sigue reemplazada por #9, que también está borrada: `clawg undelete 9` la trae de vuelta"
        );
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

        assert_eq!(run(Lang::En, &c, id).await.expect("undelete"), format!("restored memory #{id}"));

        assert_eq!(c.list(ListArgs::default()).await.expect("list")[0].id, id);
        let err = run(Lang::En, &c, id).await.expect_err("nothing left to restore");
        assert!(matches!(err, ClientError::Daemon { status: 404, .. }), "{err:?}");
    }
}
