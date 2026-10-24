//! `clawg secrets` and `clawg purge` — what they print, kept apart from the
//! commands so the wording is under test without a store. Each sentence comes
//! in the user's language; ids, kinds and commands do not change.

use clawg_core::memory::redact::SecretKind;
use clawg_core::memory::secrets::{MaskedReasons, Purged, Scanned};
use clawg_core::memory::store_lock::StoreHolder;
use clawg_core::memory::MemoryError;
use clawg_core::preferences::Lang;

/// One line per row and per verdict reason that holds a secret, then a line
/// on what to do with each. Only ids, kinds and whether the row is deleted:
/// the secret itself never reaches the terminal, where scrollback and logs
/// would keep another copy.
pub fn report(lang: Lang, scanned: &Scanned) -> String {
    let kinds = |kinds: &[SecretKind]| kinds.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", ");
    let (memory, pair, deleted, live) = match lang {
        Lang::En => ("memory", "pair", "deleted", "live"),
        Lang::Es => ("memoria", "par", "borrada", "viva"),
    };
    let rows: Vec<[String; 3]> = scanned
        .memories
        .iter()
        .map(|f| {
            let state = if f.deleted { deleted } else { live };
            [f.id.to_string(), kinds(&f.kinds), state.to_string()]
        })
        .collect();
    let id_width = rows.iter().map(|r| r[0].len()).max().unwrap_or(0);
    let kinds_width = rows.iter().map(|r| r[1].len()).max().unwrap_or(0);

    let mut lines: Vec<String> = rows
        .iter()
        .map(|[id, kinds, state]| format!("{memory} {id:>id_width$}  {kinds:<kinds_width$}  {state}"))
        .collect();
    lines.extend(scanned.pairs.iter().map(|p| format!("{pair} {} {}  {}", p.new_id, p.old_id, kinds(&p.kinds))));
    match (scanned.memories.as_slice(), lang) {
        ([], _) => {}
        ([one], Lang::En) => lines.push(format!(
            "1 memory holds a secret. To remove it for good, run `clawg stop`, then `clawg purge {} --yes`.",
            one.id
        )),
        ([one], Lang::Es) => lines.push(format!(
            "1 memoria guarda un secreto. Para borrarla para siempre, ejecuta `clawg stop` y después `clawg purge {} --yes`.",
            one.id
        )),
        (many, Lang::En) => lines.push(format!(
            "{} memories hold a secret. To remove one for good, run `clawg stop`, then `clawg purge <ID> --yes`.",
            many.len()
        )),
        (many, Lang::Es) => lines.push(format!(
            "{} memorias guardan un secreto. Para borrar una para siempre, ejecuta `clawg stop` y después `clawg purge <ID> --yes`.",
            many.len()
        )),
    }
    match (scanned.pairs.len(), lang) {
        (0, _) => {}
        (1, Lang::En) => lines.push(
            "1 verdict reason holds a secret. To mask it, run `clawg stop`, then `clawg secrets --mask-reasons --yes`."
                .to_string(),
        ),
        (1, Lang::Es) => lines.push(
            "1 razón de veredicto guarda un secreto. Para enmascararla, ejecuta `clawg stop` y después \
             `clawg secrets --mask-reasons --yes`."
                .to_string(),
        ),
        (n, Lang::En) => lines.push(format!(
            "{n} verdict reasons hold a secret. To mask them, run `clawg stop`, then `clawg secrets --mask-reasons --yes`."
        )),
        (n, Lang::Es) => lines.push(format!(
            "{n} razones de veredicto guardan un secreto. Para enmascararlas, ejecuta `clawg stop` y después \
             `clawg secrets --mask-reasons --yes`."
        )),
    }
    if lines.is_empty() {
        return match lang {
            Lang::En => "no stored memory holds a secret of a known format",
            Lang::Es => "ninguna memoria guardada tiene un secreto de un formato conocido",
        }
        .to_string();
    }
    lines.join("\n")
}

/// A purge without `--yes` stops here, before anything is opened.
pub fn confirm(lang: Lang, id: i64, yes: bool) -> Result<(), String> {
    if yes {
        return Ok(());
    }
    Err(match lang {
        Lang::En => format!(
            "clawg purge removes memory #{id} for good: its text, its vector and its index terms, \
             with no backup of it. Run it again with --yes to do it"
        ),
        Lang::Es => format!(
            "clawg purge borra la memoria #{id} para siempre: su texto, su vector y sus términos del índice, \
             sin ningún backup de ella. Ejecútalo de nuevo con --yes para hacerlo"
        ),
    })
}

/// `--mask-reasons` without `--yes` stops here, before anything is opened.
pub fn confirm_mask_reasons(lang: Lang, yes: bool) -> Result<(), String> {
    if yes {
        return Ok(());
    }
    Err(match lang {
        Lang::En => {
            "clawg secrets --mask-reasons replaces the secrets of every verdict reason with their \
             placeholders for good, with no backup of them. Run it again with --yes to do it"
        }
        Lang::Es => {
            "clawg secrets --mask-reasons reemplaza para siempre los secretos de cada razón de veredicto por sus \
             marcadores, sin ningún backup de ellos. Ejecútalo de nuevo con --yes para hacerlo"
        }
    }
    .to_string())
}

/// The lines printed after the reasons were masked: each pair by its rows,
/// then the backups that still hold one as it was.
pub fn mask_report(lang: Lang, masked: &MaskedReasons) -> String {
    if masked.pairs.is_empty() {
        return match lang {
            Lang::En => "no verdict reason holds a secret of a known format",
            Lang::Es => "ninguna razón de veredicto tiene un secreto de un formato conocido",
        }
        .to_string();
    }
    let mut lines: Vec<String> = masked
        .pairs
        .iter()
        .map(|p| {
            let kinds = p.kinds.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", ");
            match lang {
                Lang::En => format!("masked the reason of pair {} {} ({kinds})", p.new_id, p.old_id),
                Lang::Es => format!("se enmascaró la razón del par {} {} ({kinds})", p.new_id, p.old_id),
            }
        })
        .collect();
    lines.extend(backups_lines(lang, &masked.backups));
    lines.join("\n")
}

fn backups_lines(lang: Lang, backups: &[std::path::PathBuf]) -> Vec<String> {
    if backups.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![match lang {
        Lang::En => "these backups still hold it; delete them if it must be gone everywhere:",
        Lang::Es => "estos backups todavía la guardan; bórralos si tiene que desaparecer de todas partes:",
    }
    .to_string()];
    lines.extend(backups.iter().map(|path| format!("  {}", path.display())));
    lines
}

/// The lines printed after a purge, one per group that is not empty. Backups
/// and workspace files that still hold the row are named and left alone:
/// the backups are the way back from everything else, and a workspace's
/// hand-written part is the user's.
pub fn purge_report(lang: Lang, id: i64, purged: &Purged) -> String {
    let ids = |ids: &[i64]| ids.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(", ");
    let mut lines = vec![match lang {
        Lang::En => format!("purged memory #{id}"),
        Lang::Es => format!("se purgó la memoria #{id}"),
    }];
    if !purged.revived.is_empty() {
        let revived = ids(&purged.revived);
        lines.push(match lang {
            Lang::En => format!("back as the current version of its topic: {revived}"),
            Lang::Es => format!("vuelve como la versión actual de su tema: {revived}"),
        });
    }
    if !purged.hidden.is_empty() {
        let hidden = ids(&purged.hidden);
        lines.push(match lang {
            Lang::En => format!("still replaced, by a verdict: {hidden}"),
            Lang::Es => format!("sigue reemplazada, por un veredicto: {hidden}"),
        });
    }
    if !purged.pieces.is_empty() {
        let pieces = ids(&purged.pieces);
        lines.push(match lang {
            Lang::En => format!(
                "the other pieces of its save share text with it at their edges: {pieces}; \
                 purge them too if the whole save must go"
            ),
            Lang::Es => format!(
                "las otras piezas de su save comparten texto con ella en sus bordes: {pieces}; \
                 púrgalas también si tiene que irse el save entero"
            ),
        });
    }
    if !purged.outbox.is_empty() {
        let removed: Vec<String> =
            purged.outbox.iter().map(|o| format!("{} ({}, {})", o.operation_id, o.agent_id, o.state)).collect();
        let removed = removed.join(", ");
        lines.push(match lang {
            Lang::En => format!("removed from the ingest outbox: {removed}"),
            Lang::Es => format!("se quitó del outbox de ingesta: {removed}"),
        });
    }
    lines.extend(backups_lines(lang, &purged.backups));
    if !purged.workspaces.is_empty() {
        let (title, auto, outside, by_hand) = match lang {
            Lang::En => (
                "these workspace files still hold it:",
                "AUTO section, rewritten by the agent's next recall_context in that project",
                "outside the AUTO section: edit it by hand",
                "and outside it: edit it by hand",
            ),
            Lang::Es => (
                "estos archivos de workspace todavía la guardan:",
                "sección AUTO, que reescribe el próximo recall_context del agente en ese proyecto",
                "fuera de la sección AUTO: edítalo a mano",
                "y fuera de ella: edítalo a mano",
            ),
        };
        lines.push(title.to_string());
        for w in &purged.workspaces {
            let section = match (w.holding.auto, w.holding.user) {
                (true, true) => format!("{auto}; {by_hand}"),
                (true, false) => auto.to_string(),
                _ => outside.to_string(),
            };
            lines.push(format!("  {} ({section})", w.path.display()));
        }
    }
    lines.join("\n")
}

/// A rewrite of the store that the daemon can stand in the way of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rewrite {
    Purge,
    MaskReasons,
}

/// A rewrite refused because the daemon holds the store says how to free
/// it, and then to do the rewrite. The refusal itself is shown as it came.
pub fn explain(lang: Lang, err: MemoryError, rewrite: Rewrite) -> String {
    match &err {
        MemoryError::StoreInUse { owner: Some(o) } if o.holder == StoreHolder::Daemon => match (lang, rewrite) {
            (Lang::En, Rewrite::Purge) => format!("{err}; run `clawg stop` first, then purge"),
            (Lang::En, Rewrite::MaskReasons) => format!("{err}; run `clawg stop` first, then mask the reasons"),
            (Lang::Es, Rewrite::Purge) => format!("{err}; ejecuta primero `clawg stop` y después purga"),
            (Lang::Es, Rewrite::MaskReasons) => format!("{err}; ejecuta primero `clawg stop` y después enmascara las razones"),
        },
        _ => err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawg_core::preferences::Lang;
    use clawg_core::memory::secrets::{Flagged, FlaggedPair};

    #[test]
    fn the_secrets_report_lists_rows_and_never_their_content() {
        let flagged = [
            Flagged { id: 7, kinds: vec![SecretKind::AwsAccessKey], deleted: false },
            Flagged { id: 1234, kinds: vec![SecretKind::GithubToken, SecretKind::Jwt], deleted: true },
        ];
        let scanned = |memories: &[Flagged]| Scanned { memories: memories.to_vec(), pairs: vec![] };
        assert_eq!(
            report(Lang::En, &scanned(&flagged)),
            "memory    7  aws_access_key     live\n\
             memory 1234  github_token, jwt  deleted\n\
             2 memories hold a secret. To remove one for good, run `clawg stop`, then `clawg purge <ID> --yes`."
        );
        assert_eq!(
            report(Lang::En, &scanned(&flagged[..1])),
            "memory 7  aws_access_key  live\n\
             1 memory holds a secret. To remove it for good, run `clawg stop`, then `clawg purge 7 --yes`."
        );
        assert_eq!(report(Lang::En, &scanned(&[])), "no stored memory holds a secret of a known format");
    }

    #[test]
    fn the_secrets_report_lists_pairs_and_how_to_mask_them() {
        let pairs = vec![
            FlaggedPair { new_id: 12, old_id: 9, kinds: vec![SecretKind::Credential] },
            FlaggedPair { new_id: 130, old_id: 4, kinds: vec![SecretKind::GithubToken, SecretKind::Jwt] },
        ];
        let memories = vec![Flagged { id: 7, kinds: vec![SecretKind::AwsAccessKey], deleted: false }];
        assert_eq!(
            report(Lang::En, &Scanned { memories: memories.clone(), pairs: pairs.clone() }),
            "memory 7  aws_access_key  live\n\
             pair 12 9  credential\n\
             pair 130 4  github_token, jwt\n\
             1 memory holds a secret. To remove it for good, run `clawg stop`, then `clawg purge 7 --yes`.\n\
             2 verdict reasons hold a secret. To mask them, run `clawg stop`, then `clawg secrets --mask-reasons --yes`."
        );
        assert_eq!(
            report(Lang::En, &Scanned { memories: vec![], pairs: pairs[..1].to_vec() }),
            "pair 12 9  credential\n\
             1 verdict reason holds a secret. To mask it, run `clawg stop`, then `clawg secrets --mask-reasons --yes`."
        );
    }

    #[test]
    fn mask_reasons_needs_yes() {
        let refused = confirm_mask_reasons(Lang::En, false).unwrap_err();
        assert!(refused.contains("for good") && refused.contains("--yes"), "{refused}");
        assert_eq!(confirm_mask_reasons(Lang::En, true), Ok(()));
    }

    #[test]
    fn the_mask_report_names_pairs_and_backups() {
        use std::path::PathBuf;
        assert_eq!(mask_report(Lang::En, &MaskedReasons::default()), "no verdict reason holds a secret of a known format");
        let masked = MaskedReasons {
            pairs: vec![FlaggedPair { new_id: 12, old_id: 9, kinds: vec![SecretKind::Credential] }],
            backups: vec![PathBuf::from("/b/memory-x-manual.db")],
        };
        assert_eq!(
            mask_report(Lang::En, &masked),
            "masked the reason of pair 12 9 (credential)\n\
             these backups still hold it; delete them if it must be gone everywhere:\n  \
             /b/memory-x-manual.db"
        );
    }

    #[test]
    fn purge_needs_yes_before_it_opens_anything() {
        let refused = confirm(Lang::En, 57, false).unwrap_err();
        assert!(refused.contains("memory #57 for good"), "{refused}");
        assert!(refused.contains("--yes"), "{refused}");
        assert_eq!(confirm(Lang::En, 57, true), Ok(()));
    }

    #[test]
    fn a_purge_refused_by_the_daemon_says_to_stop_it() {
        use clawg_core::memory::store_lock::StoreOwner;
        let err = MemoryError::StoreInUse {
            owner: Some(StoreOwner { holder: StoreHolder::Daemon, pid: 7, started_at_ms: 1 }),
        };
        let msg = explain(Lang::En, err, Rewrite::Purge);
        assert!(msg.contains("the clawg daemon (pid 7)") && msg.contains("run `clawg stop` first, then purge"), "{msg}");
        assert_eq!(explain(Lang::En, MemoryError::Internal("boom".into()), Rewrite::Purge), "internal memory error: boom");
    }

    #[test]
    fn the_purge_report_names_what_came_back_and_what_went() {
        use clawg_core::memory::secrets::{OutboxRemoved, WorkspaceHolding};
        use clawg_core::memory::workspace::Holding;
        use std::path::PathBuf;
        let workspace =
            |path: &str, auto, user| WorkspaceHolding { path: PathBuf::from(path), holding: Holding { auto, user } };
        let purged = Purged {
            revived: vec![12],
            hidden: vec![8, 9],
            pieces: vec![58, 59],
            outbox: vec![OutboxRemoved {
                operation_id: "op-1".into(),
                agent_id: "agent-s".into(),
                state: "failed".into(),
            }],
            backups: vec![PathBuf::from("/b/memory-x-manual.db")],
            workspaces: vec![
                workspace("/w/a/p/workspace.md", true, false),
                workspace("/w/a/q/workspace.md", false, true),
                workspace("/w/b/p/workspace.md", true, true),
            ],
        };
        assert_eq!(
            purge_report(Lang::En, 57, &purged),
            "purged memory #57\n\
             back as the current version of its topic: #12\n\
             still replaced, by a verdict: #8, #9\n\
             the other pieces of its save share text with it at their edges: #58, #59; \
             purge them too if the whole save must go\n\
             removed from the ingest outbox: op-1 (agent-s, failed)\n\
             these backups still hold it; delete them if it must be gone everywhere:\n  \
             /b/memory-x-manual.db\n\
             these workspace files still hold it:\n  \
             /w/a/p/workspace.md (AUTO section, rewritten by the agent's next recall_context in that project)\n  \
             /w/a/q/workspace.md (outside the AUTO section: edit it by hand)\n  \
             /w/b/p/workspace.md (AUTO section, rewritten by the agent's next recall_context in that project; \
             and outside it: edit it by hand)"
        );
    }

    #[test]
    fn the_purge_report_names_backups_to_delete_by_hand() {
        use std::path::PathBuf;
        assert_eq!(purge_report(Lang::En, 57, &Purged { revived: vec![], hidden: vec![], pieces: vec![], outbox: vec![], backups: vec![], workspaces: vec![] }), "purged memory #57");
        let held = Purged {
            revived: vec![],
            hidden: vec![],
            pieces: vec![],
            outbox: vec![],
            backups: vec![PathBuf::from("/b/memory-x-manual.db"), PathBuf::from("/b/memory-y-pre-restore.db")],
            workspaces: vec![],
        };
        assert_eq!(
            purge_report(Lang::En, 57, &held),
            "purged memory #57\n\
             these backups still hold it; delete them if it must be gone everywhere:\n  \
             /b/memory-x-manual.db\n  \
             /b/memory-y-pre-restore.db"
        );
    }

    #[test]
    fn the_secrets_report_speaks_spanish() {
        let flagged = [
            Flagged { id: 7, kinds: vec![SecretKind::AwsAccessKey], deleted: false },
            Flagged { id: 1234, kinds: vec![SecretKind::GithubToken], deleted: true },
        ];
        let pairs = vec![FlaggedPair { new_id: 12, old_id: 9, kinds: vec![SecretKind::Credential] }];
        assert_eq!(
            report(Lang::Es, &Scanned { memories: flagged[..1].to_vec(), pairs: pairs.clone() }),
            "memoria 7  aws_access_key  viva\n\
             par 12 9  credential\n\
             1 memoria guarda un secreto. Para borrarla para siempre, ejecuta `clawg stop` y después `clawg purge 7 --yes`.\n\
             1 razón de veredicto guarda un secreto. Para enmascararla, ejecuta `clawg stop` y después `clawg secrets --mask-reasons --yes`."
        );
        let two_pairs = vec![pairs[0].clone(), FlaggedPair { new_id: 13, old_id: 9, kinds: vec![SecretKind::Jwt] }];
        assert_eq!(
            report(Lang::Es, &Scanned { memories: flagged.to_vec(), pairs: two_pairs }),
            "memoria    7  aws_access_key  viva\n\
             memoria 1234  github_token    borrada\n\
             par 12 9  credential\n\
             par 13 9  jwt\n\
             2 memorias guardan un secreto. Para borrar una para siempre, ejecuta `clawg stop` y después `clawg purge <ID> --yes`.\n\
             2 razones de veredicto guardan un secreto. Para enmascararlas, ejecuta `clawg stop` y después `clawg secrets --mask-reasons --yes`."
        );
        assert_eq!(
            report(Lang::Es, &Scanned::default()),
            "ninguna memoria guardada tiene un secreto de un formato conocido"
        );
    }

    #[test]
    fn the_purge_report_speaks_spanish() {
        use clawg_core::memory::secrets::{OutboxRemoved, WorkspaceHolding};
        use clawg_core::memory::workspace::Holding;
        use std::path::PathBuf;
        let purged = Purged {
            revived: vec![3],
            hidden: vec![8],
            pieces: vec![58],
            outbox: vec![OutboxRemoved { operation_id: "op-1".into(), agent_id: "a".into(), state: "failed".into() }],
            backups: vec![PathBuf::from("/b/m.db")],
            workspaces: vec![WorkspaceHolding {
                path: PathBuf::from("/w/workspace.md"),
                holding: Holding { auto: true, user: true },
            }],
        };
        assert_eq!(
            purge_report(Lang::Es, 7, &purged),
            "se purgó la memoria #7\n\
             vuelve como la versión actual de su tema: #3\n\
             sigue reemplazada, por un veredicto: #8\n\
             las otras piezas de su save comparten texto con ella en sus bordes: #58; \
             púrgalas también si tiene que irse el save entero\n\
             se quitó del outbox de ingesta: op-1 (a, failed)\n\
             estos backups todavía la guardan; bórralos si tiene que desaparecer de todas partes:\n  \
             /b/m.db\n\
             estos archivos de workspace todavía la guardan:\n  \
             /w/workspace.md (sección AUTO, que reescribe el próximo recall_context del agente en ese proyecto; \
             y fuera de ella: edítalo a mano)"
        );
        let refused = confirm(Lang::Es, 57, false).unwrap_err();
        assert!(refused.contains("la memoria #57 para siempre") && refused.contains("--yes"), "{refused}");
        let refused = confirm_mask_reasons(Lang::Es, false).unwrap_err();
        assert!(refused.contains("para siempre") && refused.contains("--yes"), "{refused}");
        assert_eq!(mask_report(Lang::Es, &MaskedReasons::default()), "ninguna razón de veredicto tiene un secreto de un formato conocido");
        let err = MemoryError::StoreInUse {
            owner: Some(clawg_core::memory::store_lock::StoreOwner { holder: StoreHolder::Daemon, pid: 7, started_at_ms: 1 }),
        };
        let msg = explain(Lang::Es, err, Rewrite::MaskReasons);
        assert!(msg.contains("the clawg daemon (pid 7)"), "the cause goes as it is: {msg}");
        assert!(msg.contains("ejecuta primero `clawg stop` y después enmascara las razones"), "{msg}");
    }
}
