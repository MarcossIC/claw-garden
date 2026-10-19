//! `clawg secrets` and `clawg purge` — what they print, kept apart from the
//! commands so the wording is under test without a store.

use clawg_core::memory::redact::SecretKind;
use clawg_core::memory::secrets::{MaskedReasons, Purged, Scanned};
use clawg_core::memory::store_lock::StoreHolder;
use clawg_core::memory::MemoryError;

/// One line per row and per verdict reason that holds a secret, then a line
/// on what to do with each. Only ids, kinds and whether the row is deleted:
/// the secret itself never reaches the terminal, where scrollback and logs
/// would keep another copy.
pub fn report(scanned: &Scanned) -> String {
    let kinds = |kinds: &[SecretKind]| kinds.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", ");
    let rows: Vec<[String; 3]> = scanned
        .memories
        .iter()
        .map(|f| {
            let state = if f.deleted { "deleted" } else { "live" };
            [f.id.to_string(), kinds(&f.kinds), state.to_string()]
        })
        .collect();
    let id_width = rows.iter().map(|r| r[0].len()).max().unwrap_or(0);
    let kinds_width = rows.iter().map(|r| r[1].len()).max().unwrap_or(0);

    let mut lines: Vec<String> = rows
        .iter()
        .map(|[id, kinds, state]| format!("memory {id:>id_width$}  {kinds:<kinds_width$}  {state}"))
        .collect();
    lines.extend(scanned.pairs.iter().map(|p| format!("pair {} {}  {}", p.new_id, p.old_id, kinds(&p.kinds))));
    match scanned.memories.as_slice() {
        [] => {}
        [one] => lines.push(format!(
            "1 memory holds a secret. To remove it for good, run `clawg stop`, then `clawg purge {} --yes`.",
            one.id
        )),
        many => lines.push(format!(
            "{} memories hold a secret. To remove one for good, run `clawg stop`, then `clawg purge <ID> --yes`.",
            many.len()
        )),
    }
    match scanned.pairs.len() {
        0 => {}
        1 => lines.push(
            "1 verdict reason holds a secret. To mask it, run `clawg stop`, then `clawg secrets --mask-reasons --yes`."
                .to_string(),
        ),
        n => lines.push(format!(
            "{n} verdict reasons hold a secret. To mask them, run `clawg stop`, then `clawg secrets --mask-reasons --yes`."
        )),
    }
    if lines.is_empty() {
        return "no stored memory holds a secret of a known format".to_string();
    }
    lines.join("\n")
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

/// `--mask-reasons` without `--yes` stops here, before anything is opened.
pub fn confirm_mask_reasons(yes: bool) -> Result<(), String> {
    if yes {
        return Ok(());
    }
    Err("clawg secrets --mask-reasons replaces the secrets of every verdict reason with their \
         placeholders for good, with no backup of them. Run it again with --yes to do it"
        .to_string())
}

/// The lines printed after the reasons were masked: each pair by its rows,
/// then the backups that still hold one as it was.
pub fn mask_report(masked: &MaskedReasons) -> String {
    if masked.pairs.is_empty() {
        return "no verdict reason holds a secret of a known format".to_string();
    }
    let mut lines: Vec<String> = masked
        .pairs
        .iter()
        .map(|p| {
            let kinds: Vec<&str> = p.kinds.iter().map(|k| k.as_str()).collect();
            format!("masked the reason of pair {} {} ({})", p.new_id, p.old_id, kinds.join(", "))
        })
        .collect();
    lines.extend(backups_lines(&masked.backups));
    lines.join("\n")
}

fn backups_lines(backups: &[std::path::PathBuf]) -> Vec<String> {
    if backups.is_empty() {
        return Vec::new();
    }
    let mut lines = vec!["these backups still hold it; delete them if it must be gone everywhere:".to_string()];
    lines.extend(backups.iter().map(|path| format!("  {}", path.display())));
    lines
}

/// The lines printed after a purge. Backups that still hold the row are named
/// and left alone: they are the way back from everything else.
pub fn purge_report(id: i64, purged: &Purged) -> String {
    let mut lines = vec![format!("purged memory #{id}")];
    lines.extend(backups_lines(&purged.backups));
    lines.join("\n")
}

/// A rewrite refused because the daemon holds the store says how to free
/// it, and then to `then`.
pub fn explain(err: MemoryError, then: &str) -> String {
    match &err {
        MemoryError::StoreInUse { owner: Some(o) } if o.holder == StoreHolder::Daemon => {
            format!("{err}; run `clawg stop` first, then {then}")
        }
        _ => err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawg_core::memory::secrets::{Flagged, FlaggedPair};

    #[test]
    fn the_secrets_report_lists_rows_and_never_their_content() {
        let flagged = [
            Flagged { id: 7, kinds: vec![SecretKind::AwsAccessKey], deleted: false },
            Flagged { id: 1234, kinds: vec![SecretKind::GithubToken, SecretKind::Jwt], deleted: true },
        ];
        let scanned = |memories: &[Flagged]| Scanned { memories: memories.to_vec(), pairs: vec![] };
        assert_eq!(
            report(&scanned(&flagged)),
            "memory    7  aws_access_key     live\n\
             memory 1234  github_token, jwt  deleted\n\
             2 memories hold a secret. To remove one for good, run `clawg stop`, then `clawg purge <ID> --yes`."
        );
        assert_eq!(
            report(&scanned(&flagged[..1])),
            "memory 7  aws_access_key  live\n\
             1 memory holds a secret. To remove it for good, run `clawg stop`, then `clawg purge 7 --yes`."
        );
        assert_eq!(report(&scanned(&[])), "no stored memory holds a secret of a known format");
    }

    #[test]
    fn the_secrets_report_lists_pairs_and_how_to_mask_them() {
        let pairs = vec![
            FlaggedPair { new_id: 12, old_id: 9, kinds: vec![SecretKind::Credential] },
            FlaggedPair { new_id: 130, old_id: 4, kinds: vec![SecretKind::GithubToken, SecretKind::Jwt] },
        ];
        let memories = vec![Flagged { id: 7, kinds: vec![SecretKind::AwsAccessKey], deleted: false }];
        assert_eq!(
            report(&Scanned { memories: memories.clone(), pairs: pairs.clone() }),
            "memory 7  aws_access_key  live\n\
             pair 12 9  credential\n\
             pair 130 4  github_token, jwt\n\
             1 memory holds a secret. To remove it for good, run `clawg stop`, then `clawg purge 7 --yes`.\n\
             2 verdict reasons hold a secret. To mask them, run `clawg stop`, then `clawg secrets --mask-reasons --yes`."
        );
        assert_eq!(
            report(&Scanned { memories: vec![], pairs: pairs[..1].to_vec() }),
            "pair 12 9  credential\n\
             1 verdict reason holds a secret. To mask it, run `clawg stop`, then `clawg secrets --mask-reasons --yes`."
        );
    }

    #[test]
    fn mask_reasons_needs_yes() {
        let refused = confirm_mask_reasons(false).unwrap_err();
        assert!(refused.contains("for good") && refused.contains("--yes"), "{refused}");
        assert_eq!(confirm_mask_reasons(true), Ok(()));
    }

    #[test]
    fn the_mask_report_names_pairs_and_backups() {
        use std::path::PathBuf;
        assert_eq!(mask_report(&MaskedReasons::default()), "no verdict reason holds a secret of a known format");
        let masked = MaskedReasons {
            pairs: vec![FlaggedPair { new_id: 12, old_id: 9, kinds: vec![SecretKind::Credential] }],
            backups: vec![PathBuf::from("/b/memory-x-manual.db")],
        };
        assert_eq!(
            mask_report(&masked),
            "masked the reason of pair 12 9 (credential)\n\
             these backups still hold it; delete them if it must be gone everywhere:\n  \
             /b/memory-x-manual.db"
        );
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
        let msg = explain(err, "purge");
        assert!(msg.contains("the clawg daemon (pid 7)") && msg.contains("run `clawg stop` first, then purge"), "{msg}");
        assert_eq!(explain(MemoryError::Internal("boom".into()), "purge"), "internal memory error: boom");
    }

    #[test]
    fn the_purge_report_names_backups_to_delete_by_hand() {
        use std::path::PathBuf;
        assert_eq!(purge_report(57, &Purged { pieces: vec![], outbox: vec![], backups: vec![] }), "purged memory #57");
        let held = Purged {
            pieces: vec![],
            outbox: vec![],
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
