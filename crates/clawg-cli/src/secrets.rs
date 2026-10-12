//! `clawg secrets` and `clawg purge` — what they print, kept apart from the
//! commands so the wording is under test without a store.

use clawg_core::memory::secrets::{Flagged, Purged};
use clawg_core::memory::store_lock::StoreHolder;
use clawg_core::memory::MemoryError;

/// One line per row that holds a secret, then a line on what to do. Only ids,
/// kinds and whether the row is deleted: the secret itself never reaches the
/// terminal, where scrollback and logs would keep another copy.
pub fn report(flagged: &[Flagged]) -> String {
    let rows: Vec<[String; 3]> = flagged
        .iter()
        .map(|f| {
            let kinds: Vec<&str> = f.kinds.iter().map(|k| k.as_str()).collect();
            let state = if f.deleted { "deleted" } else { "live" };
            [f.id.to_string(), kinds.join(", "), state.to_string()]
        })
        .collect();
    let id_width = rows.iter().map(|r| r[0].len()).max().unwrap_or(0);
    let kinds_width = rows.iter().map(|r| r[1].len()).max().unwrap_or(0);

    let mut out = String::new();
    for [id, kinds, state] in &rows {
        out.push_str(&format!("memory {id:>id_width$}  {kinds:<kinds_width$}  {state}\n"));
    }
    match flagged {
        [] => out.push_str("no stored memory holds a secret of a known format"),
        [one] => out.push_str(&format!(
            "1 memory holds a secret. To remove it for good, run `clawg stop`, then `clawg purge {} --yes`.",
            one.id
        )),
        many => out.push_str(&format!(
            "{} memories hold a secret. To remove one for good, run `clawg stop`, then `clawg purge <ID> --yes`.",
            many.len()
        )),
    }
    out
}

/// A purge without `--yes` stops here, before anything is opened.
pub fn confirm(id: i64, yes: bool) -> Result<(), String> {
    if yes {
        return Ok(());
    }
    Err(format!(
        "clawg purge removes memory #{id} for good: its text, its vector and its index terms, \
         with no backup of it. Run it again with --yes to do it"
    ))
}

/// The lines printed after a purge. Backups that still hold the row are named
/// and left alone: they are the way back from everything else.
pub fn purge_report(id: i64, purged: &Purged) -> String {
    let mut out = format!("purged memory #{id}");
    if !purged.backups.is_empty() {
        out.push_str("\nthese backups still hold it; delete them if it must be gone everywhere:");
        for path in &purged.backups {
            out.push_str(&format!("\n  {}", path.display()));
        }
    }
    out
}

/// A purge refused because the daemon holds the store says how to free it.
pub fn explain(err: MemoryError) -> String {
    match &err {
        MemoryError::StoreInUse { owner: Some(o) } if o.holder == StoreHolder::Daemon => {
            format!("{err}; run `clawg stop` first, then purge")
        }
        _ => err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawg_core::memory::redact::SecretKind;

    #[test]
    fn the_secrets_report_lists_rows_and_never_their_content() {
        let flagged = [
            Flagged { id: 7, kinds: vec![SecretKind::AwsAccessKey], deleted: false },
            Flagged { id: 1234, kinds: vec![SecretKind::GithubToken, SecretKind::Jwt], deleted: true },
        ];
        assert_eq!(
            report(&flagged),
            "memory    7  aws_access_key     live\n\
             memory 1234  github_token, jwt  deleted\n\
             2 memories hold a secret. To remove one for good, run `clawg stop`, then `clawg purge <ID> --yes`."
        );
        assert_eq!(
            report(&flagged[..1]),
            "memory 7  aws_access_key  live\n\
             1 memory holds a secret. To remove it for good, run `clawg stop`, then `clawg purge 7 --yes`."
        );
        assert_eq!(report(&[]), "no stored memory holds a secret of a known format");
    }

    #[test]
    fn purge_needs_yes_before_it_opens_anything() {
        let refused = confirm(57, false).unwrap_err();
        assert!(refused.contains("memory #57 for good"), "{refused}");
        assert!(refused.contains("--yes"), "{refused}");
        assert_eq!(confirm(57, true), Ok(()));
    }

    #[test]
    fn a_purge_refused_by_the_daemon_says_to_stop_it() {
        use clawg_core::memory::store_lock::StoreOwner;
        let err = MemoryError::StoreInUse {
            owner: Some(StoreOwner { holder: StoreHolder::Daemon, pid: 7, started_at_ms: 1 }),
        };
        let msg = explain(err);
        assert!(msg.contains("the clawg daemon (pid 7)") && msg.contains("run `clawg stop` first, then purge"), "{msg}");
        assert_eq!(explain(MemoryError::Internal("boom".into())), "internal memory error: boom");
    }

    #[test]
    fn the_purge_report_names_backups_to_delete_by_hand() {
        use std::path::PathBuf;
        assert_eq!(purge_report(57, &Purged { backups: vec![] }), "purged memory #57");
        let held = Purged {
            backups: vec![PathBuf::from("/b/memory-x-manual.db"), PathBuf::from("/b/memory-y-pre-restore.db")],
        };
        assert_eq!(
            purge_report(57, &held),
            "purged memory #57\n\
             these backups still hold it; delete them if it must be gone everywhere:\n  \
             /b/memory-x-manual.db\n  \
             /b/memory-y-pre-restore.db"
        );
    }
}
