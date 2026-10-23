//! Each check as a pure function from facts to findings: no filesystem, no
//! store, no clock. [`super::run`] gathers the facts.

use std::path::Path;

use clawg_core::memory::backup::{parse_snapshot_time, ProbeState, SchemaPlan, StoreProbe};
use clawg_core::memory::bootstrap::is_pooling_upgrade;
use clawg_core::memory::db::{EmbeddingIdentity, StoredContract};
use clawg_core::memory::derive_project_id;
use clawg_core::memory::inspect::{AgentRows, Invariants, OutboxFacts, ProjectRow};
use clawg_core::memory::store_lock::StoreOwner;
use clawg_core::paths::Profile;
use serde_json::json;

use super::finding::{Finding, Severity};
use super::Analysis;
use crate::daemon::socket::SocketError;
use crate::mcp::server::FALLBACK_AGENT_ID;

/// Days the newest snapshot may age before `backup.age` warns. A starting
/// value, not measured: a month of memories is what goes unwarned.
pub const BACKUP_MAX_AGE_DAYS: i64 = 30;

const DAY_MS: i64 = 86_400_000;

/// A word a shell takes as one argument: as it is when it holds nothing a
/// shell reads, in single quotes otherwise.
pub fn shell_word(word: &str) -> String {
    let plain = !word.is_empty()
        && word.bytes().all(|b| b.is_ascii_alphanumeric() || b"/._-:@+=,".contains(&b));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

fn with_profile(command: &str, profile: Profile) -> String {
    format!("{command} --profile {}", profile.as_str())
}

pub fn path(analysis: &Analysis) -> Vec<Finding> {
    let installs: Vec<&Path> = analysis.installs.iter().map(|i| i.path.as_path()).collect();
    let evidence = json!({ "winner": analysis.winner, "installs": installs });
    let mut found = Vec::new();
    if analysis.shadowed {
        let winner = analysis.winner.as_deref().map(|w| w.display().to_string()).unwrap_or_default();
        found.push(
            Finding::new("path", Severity::Error, "shadowed", format!("the PATH runs {winner}, not this binary"))
                .evidence(evidence.clone()),
        );
    }
    if analysis.duplicates {
        found.push(
            Finding::new(
                "path",
                Severity::Error,
                "duplicate",
                format!("{} copies of clawg are on the PATH; a terminal runs only the first", installs.len()),
            )
            .evidence(evidence.clone()),
        );
    }
    if found.is_empty() {
        found.push(Finding::ok("path", "the PATH resolves to this binary, or to no clawg at all").evidence(evidence));
    }
    found
}

/// What the data directory is on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirState {
    Missing,
    /// `mode` holds the permission bits on Unix and is `None` elsewhere.
    Present { mode: Option<u32> },
}

pub fn data_dir(dir: &Path, state: DirState) -> Vec<Finding> {
    let evidence = json!({ "path": dir });
    match state {
        DirState::Missing => vec![Finding::new(
            "data_dir",
            Severity::Info,
            "missing",
            "the data directory does not exist yet: nothing has run in this profile",
        )
        .evidence(evidence)],
        DirState::Present { mode: Some(mode) } if mode & 0o077 != 0 => vec![Finding::new(
            "data_dir",
            Severity::Warn,
            "open_mode",
            format!("the data directory is {:04o}, so other users of this machine may read it", mode & 0o7777),
        )
        .evidence(json!({ "path": dir, "mode": format!("{:04o}", mode & 0o7777) }))
        .step(format!("chmod 700 {}", shell_word(&dir.to_string_lossy())))],
        DirState::Present { .. } => vec![Finding::ok("data_dir", "the data directory is private to its owner").evidence(evidence)],
    }
}

pub fn socket(path: &Path, checked: &Result<(), SocketError>) -> Vec<Finding> {
    match checked {
        Ok(()) => vec![Finding::ok("socket", "the socket path fits").evidence(json!({ "path": path }))],
        Err(SocketError::TooLong { path, len, limit }) => vec![Finding::new(
            "socket",
            Severity::Error,
            "path_unusable",
            format!(
                "the socket path is {len} bytes and must be shorter than {limit}: the daemon does not start \
                 without --port, and a bridge starts it without one"
            ),
        )
        .evidence(json!({ "path": path, "len": len, "limit": limit }))],
        Err(e) => vec![Finding::new("socket", Severity::Error, "path_unusable", e.to_string())
            .evidence(json!({ "path": path }))],
    }
}

pub fn store_owner(owner: Option<&StoreOwner>) -> Vec<Finding> {
    match owner {
        None => vec![Finding::ok("store.owner", "no clawg holds the store")],
        Some(o) => vec![Finding::new(
            "store.owner",
            Severity::Info,
            "held",
            format!("{} holds the store (pid {}); the store was read beside it", o.holder, o.pid),
        )
        .evidence(json!({ "holder": o.holder, "pid": o.pid }))],
    }
}

pub fn schema(probe: &StoreProbe, profile: Profile) -> Vec<Finding> {
    let evidence = json!({ "db_version": probe.db_version, "binary_version": probe.binary_version });
    let finding = match &probe.state {
        ProbeState::Missing => Finding::new(
            "store.schema",
            Severity::Info,
            "missing",
            "there is no memory.db yet: the daemon creates it on its first start",
        ),
        ProbeState::Unreadable(error) => {
            Finding::new("store.schema", Severity::Error, "unreadable", format!("memory.db cannot be read: {error}"))
                .evidence(json!({ "error": error }))
        }
        ProbeState::Schema(SchemaPlan::Fresh) => Finding::new(
            "store.schema",
            Severity::Info,
            "fresh",
            "memory.db has no migration applied: the daemon sets it up on its next start",
        ),
        ProbeState::Schema(SchemaPlan::UpToDate) => {
            Finding::ok("store.schema", "memory.db is at the schema of this clawg").evidence(evidence)
        }
        ProbeState::Schema(SchemaPlan::Pending { from, to }) => Finding::new(
            "store.schema",
            Severity::Info,
            "pending",
            format!("memory.db is at V{from}; this clawg migrates it to V{to} on its next start, after a pre-migration snapshot"),
        )
        .evidence(evidence)
        .changing_step(with_profile("clawg serve", profile)),
        ProbeState::Schema(SchemaPlan::Newer { db, binary }) => Finding::new(
            "store.schema",
            Severity::Error,
            "newer",
            format!("memory.db is at V{db}, newer than the V{binary} this clawg knows: this clawg refuses it untouched"),
        )
        .evidence(evidence),
        ProbeState::Schema(SchemaPlan::Diverged { version }) => Finding::new(
            "store.schema",
            Severity::Error,
            "diverged",
            format!("the migration V{version} of memory.db does not match this clawg: it refuses the store untouched"),
        )
        .evidence(evidence),
    };
    vec![finding]
}

pub fn contract(stored: Option<&StoredContract>, expected: &EmbeddingIdentity) -> Vec<Finding> {
    let expected_json = json!({
        "model_id": expected.model_id,
        "dim": expected.dim,
        "query_prefix": expected.query_prefix,
        "passage_prefix": expected.passage_prefix,
    });
    let Some(stored) = stored else {
        return vec![Finding::new(
            "store.contract",
            Severity::Info,
            "not_recorded",
            "no embedding contract is recorded yet: the first model load writes it",
        )
        .evidence(json!({ "expected": expected_json }))];
    };
    let evidence = json!({
        "stored": {
            "model_id": stored.model_id,
            "dim": stored.dim,
            "query_prefix": stored.query_prefix,
            "passage_prefix": stored.passage_prefix,
        },
        "expected": expected_json,
    });
    let same = stored.model_id == expected.model_id
        && stored.dim == expected.dim as i64
        && stored.query_prefix == expected.query_prefix
        && stored.passage_prefix == expected.passage_prefix;
    let finding = if same {
        Finding::ok("store.contract", "the stored vectors were made by this clawg's embedder")
    } else if is_pooling_upgrade(stored, expected) {
        Finding::new(
            "store.contract",
            Severity::Info,
            "pooling_upgrade",
            "the stored vectors were pooled by CLS: the next start re-embeds them by mean, after a pre-reembed snapshot",
        )
    } else {
        Finding::new(
            "store.contract",
            Severity::Error,
            "mismatch",
            format!(
                "the stored vectors were made by {} ({} dimensions), not by this clawg's {}: memory stays off",
                stored.model_id, stored.dim, expected.model_id
            ),
        )
    };
    vec![finding.evidence(evidence)]
}

/// `names` are the files of `<data>/backups`; a name that is not a snapshot's
/// does not count. Only a store that holds rows has something to lose.
pub fn backups(names: &[String], has_rows: bool, now_ms: i64, profile: Profile) -> Vec<Finding> {
    let newest = names.iter().filter_map(|n| parse_snapshot_time(n).map(|t| (t, n))).max();
    let count = names.iter().filter(|n| parse_snapshot_time(n).is_some()).count();
    let step = with_profile("clawg backup", profile);
    let finding = match newest {
        _ if !has_rows => Finding::ok("backup.age", "the store holds no row to lose"),
        None => Finding::new("backup.age", Severity::Warn, "none", "the store holds rows and there is no snapshot of it")
            .step(step),
        Some((taken, name)) => {
            let age_days = (now_ms - taken).div_euclid(DAY_MS);
            let evidence = json!({ "newest": name, "age_days": age_days, "snapshots": count });
            if age_days > BACKUP_MAX_AGE_DAYS {
                Finding::new(
                    "backup.age",
                    Severity::Warn,
                    "stale",
                    format!("the newest snapshot is {age_days} days old"),
                )
                .evidence(evidence)
                .step(step)
            } else {
                Finding::ok("backup.age", format!("the newest snapshot is {age_days} days old")).evidence(evidence)
            }
        }
    };
    vec![finding]
}

/// `pending` is the spill file an older clawg left, `rejected` the file of
/// its lines that did not parse with how many lines it holds.
pub fn spill(pending: Option<&Path>, rejected: Option<(&Path, usize)>) -> Vec<Finding> {
    let mut found = Vec::new();
    if let Some(path) = pending {
        found.push(
            Finding::new(
                "spill",
                Severity::Info,
                "pending",
                "an older clawg left saves in a spill file: the next start moves them to the outbox",
            )
            .evidence(json!({ "path": path })),
        );
    }
    if let Some((path, lines)) = rejected {
        found.push(
            Finding::new(
                "spill",
                Severity::Warn,
                "rejected",
                format!(
                    "{lines} lines of saves an older clawg could not read are kept aside; nothing deletes them, \
                     and only a person can tell what they were"
                ),
            )
            .evidence(json!({ "path": path, "lines": lines })),
        );
    }
    if found.is_empty() {
        found.push(Finding::ok("spill", "no spill file is left"));
    }
    found
}

/// `error` is why `sharing.json` could not be used, as the user is shown it.
pub fn sharing(error: Option<&str>, profile: Profile) -> Vec<Finding> {
    match error {
        None => vec![Finding::ok("sharing", "the sharing policy is readable")],
        Some(error) => vec![Finding::new(
            "sharing",
            Severity::Warn,
            "invalid",
            format!("{error}; nothing is shared between agents until it is fixed"),
        )
        .evidence(json!({ "error": error }))
        .step(with_profile("clawg sharing", profile))],
    }
}

/// What `preferences.json` is, as `doctor` reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreferencesState {
    Missing,
    Readable,
    /// Why it cannot be used, as the user is shown it.
    Invalid(String),
}

/// Only a file that exists is checked: without one, clawg behaves as it did
/// before preferences, and saying so in every report would be noise.
pub fn preferences(state: &PreferencesState, path: &Path) -> Vec<Finding> {
    match state {
        PreferencesState::Missing => Vec::new(),
        PreferencesState::Readable => vec![Finding::ok("preferences", "the preferences are readable")],
        PreferencesState::Invalid(error) => vec![Finding::new(
            "preferences",
            Severity::Warn,
            "invalid",
            format!("{error}; clawg goes on without a language preference until it is fixed or removed"),
        )
        .evidence(json!({ "path": path, "error": error }))],
    }
}

/// What the user's `CLAUDE.md` holds of clawg's protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockState {
    Missing,
    /// The file could not be read, or has a marker without its pair.
    Unreadable(String),
    /// The block from its first marker to the end of its last.
    Installed(String),
}

/// Whether the installed block is the one `clawg setup claude-code` would
/// write now (`wanted`), line endings aside. A block that differs may hold
/// another language's rules or an older clawg's protocol; the remedy is the
/// same.
pub fn protocol_block(state: &BlockState, wanted: &str, path: &Path, profile: Profile) -> Vec<Finding> {
    let setup = with_profile("clawg setup claude-code", profile);
    let finding = match state {
        BlockState::Installed(text) if text.replace("\r\n", "\n") == wanted.replace("\r\n", "\n") => {
            Finding::ok("protocol_block", "the protocol block of CLAUDE.md is the one clawg setup writes")
        }
        BlockState::Installed(_) => Finding::new(
            "protocol_block",
            Severity::Warn,
            "stale",
            "the protocol block of CLAUDE.md is not the one clawg setup writes for the preferred language, \
             so the agent may read other rules there than in the server's instructions",
        )
        .step(setup),
        BlockState::Missing => Finding::new(
            "protocol_block",
            Severity::Info,
            "not_installed",
            "CLAUDE.md has no clawg protocol block; the agent reads the preferred language only in the server's instructions",
        )
        .step(setup),
        BlockState::Unreadable(error) => {
            Finding::new("protocol_block", Severity::Warn, "unreadable", format!("cannot check the protocol block: {error}"))
        }
    };
    vec![finding.evidence(json!({ "path": path }))]
}

/// Pending rows with a daemon running are being stored; without one, they
/// wait for the next start.
pub fn outbox(facts: &OutboxFacts, daemon_running: bool, profile: Profile) -> Vec<Finding> {
    let mut found: Vec<Finding> = facts
        .failed
        .iter()
        .map(|f| {
            Finding::new(
                "outbox",
                Severity::Warn,
                "failed",
                format!("the save {} of {} failed after {} attempts and is not stored", f.operation_id, f.agent_id, f.attempts),
            )
            .evidence(json!({ "operation_id": f.operation_id, "agent_id": f.agent_id, "attempts": f.attempts }))
            .changing_step(with_profile(
                &format!("clawg outbox retry {} --agent {}", shell_word(&f.operation_id), shell_word(&f.agent_id)),
                profile,
            ))
        })
        .collect();
    if facts.pending > 0 && !daemon_running {
        found.push(
            Finding::new(
                "outbox",
                Severity::Info,
                "pending",
                format!("{} accepted saves wait in the outbox for the next start", facts.pending),
            )
            .evidence(json!({ "pending": facts.pending }))
            .step(with_profile("clawg serve", profile)),
        );
    }
    if found.is_empty() {
        found.push(Finding::ok("outbox", "no save is failed or waiting").evidence(json!({ "pending": facts.pending })));
    }
    found
}

pub fn agents(rows: &[AgentRows]) -> Vec<Finding> {
    let listed: Vec<_> =
        rows.iter().map(|r| json!({ "agent_id": r.agent_id, "live": r.live, "total": r.total })).collect();
    let mut found = vec![Finding::ok("agents", format!("{} agents hold rows", rows.iter().filter(|r| r.agent_id.is_some()).count()))
        .evidence(json!({ "agents": listed }))];
    if let Some(fallback) = rows.iter().find(|r| r.agent_id.as_deref() == Some(FALLBACK_AGENT_ID) && r.live > 0) {
        found.push(
            Finding::new(
                "agents",
                Severity::Warn,
                "fallback_agent",
                format!(
                    "{} live memories are under `{FALLBACK_AGENT_ID}`, the id a bridge takes when its client gives \
                     neither --agent nor its own name; every agent reads and changes them, but the workspace and the \
                     recent saves of the client's own agent leave them out",
                    fallback.live
                ),
            )
            .evidence(json!({ "agent_id": FALLBACK_AGENT_ID, "live": fallback.live })),
        );
    }
    found
}

pub fn projects(rows: &[ProjectRow]) -> Vec<Finding> {
    let mut found: Vec<Finding> = rows
        .iter()
        .filter_map(|p| {
            let derived = derive_project_id(Path::new(&p.root_path));
            (derived != p.id).then(|| {
                Finding::new(
                    "projects",
                    Severity::Warn,
                    "id_mismatch",
                    format!("the project {} is recorded for a root whose id is {derived}", p.id),
                )
                .evidence(json!({ "project_id": p.id, "root_path": p.root_path, "derived": derived }))
            })
        })
        .collect();
    if found.is_empty() {
        found.push(Finding::ok("projects", "every project id matches its root").evidence(json!({ "projects": rows.len() })));
    }
    found
}

/// One check per rule of `memory`, `ok` when nothing breaks it.
pub fn invariants(inv: &Invariants) -> Vec<Finding> {
    let mut found = Vec::new();
    let mut check = |id: &'static str, healthy: &str, broken: Vec<Finding>| {
        if broken.is_empty() {
            found.push(Finding::ok(id, healthy));
        } else {
            found.extend(broken);
        }
    };
    check(
        "memory.topic",
        "each topic key has one live save",
        inv.topic_versions
            .iter()
            .map(|t| {
                Finding::new(
                    "memory.topic",
                    Severity::Warn,
                    "several_live_saves",
                    format!(
                        "{} live saves hold one topic key of a project, where each should have replaced the one before",
                        t.saves.len()
                    ),
                )
                .evidence(json!({ "agents": t.agents, "project_id": t.project_id, "saves": t.saves }))
            })
            .collect(),
    );
    check(
        "memory.superseded_by",
        "every superseded row names the row that replaced it",
        inv.dangling_superseded
            .iter()
            .map(|d| {
                let evidence = json!({ "id": d.id, "superseded_by": d.superseded_by, "cause": d.cause });
                if d.cause.as_deref() == Some("verdict") {
                    Finding::new(
                        "memory.superseded_by",
                        Severity::Info,
                        "dangling_verdict",
                        format!("#{} lost a verdict to a row that was purged since, and stays replaced", d.id),
                    )
                    .evidence(evidence)
                } else {
                    Finding::new(
                        "memory.superseded_by",
                        Severity::Warn,
                        "dangling",
                        format!("#{} is superseded by a row that does not exist, so no version of its topic comes back for it", d.id),
                    )
                    .evidence(evidence)
                }
            })
            .collect(),
    );
    check(
        "memory.revived_from",
        "every revived row names the delete that brought it back",
        inv.dangling_revived
            .iter()
            .map(|d| {
                Finding::new(
                    "memory.revived_from",
                    Severity::Info,
                    "dangling",
                    format!("#{} was brought back by the delete of #{}, which was purged since", d.id, d.revived_from),
                )
                .evidence(json!({ "id": d.id, "revived_from": d.revived_from }))
            })
            .collect(),
    );
    check(
        "memory.content_hash",
        "no two live rows of one project hold the same text",
        inv.hash_duplicates
            .iter()
            .map(|h| {
                Finding::new(
                    "memory.content_hash",
                    Severity::Warn,
                    "live_duplicates",
                    format!("{} live rows of one project hold the same text", h.ids.len()),
                )
                .evidence(json!({ "agents": h.agents, "project_id": h.project_id, "ids": h.ids }))
            })
            .collect(),
    );
    check(
        "memory.pieces",
        "no save keeps the opening of a purged piece",
        inv.purged_headers
            .iter()
            .map(|p| {
                Finding::new(
                    "memory.pieces",
                    Severity::Warn,
                    "purged_header",
                    format!(
                        "the save {} lost its first piece #{} to a purge by an older clawg, and its other pieces \
                         still keep that piece's opening in memory.db",
                        p.save_id, p.first_piece
                    ),
                )
                .evidence(json!({ "save_id": p.save_id, "first_piece": p.first_piece }))
            })
            .collect(),
    );
    check(
        "memory.disputed",
        "every disputed row is disputed against a live row",
        inv.lonely_disputes
            .iter()
            .map(|id| {
                Finding::new(
                    "memory.disputed",
                    Severity::Info,
                    "no_live_rival",
                    format!("#{id} is disputed against no live row; the daily maintenance pass sets it back to active"),
                )
                .evidence(json!({ "id": id }))
            })
            .collect(),
    );
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawg_core::memory::backup::{snapshot_name, Reason};
    use clawg_core::memory::inspect::{
        DanglingRevived, DanglingSuperseded, FailedSave, HashDuplicates, PurgedHeader, TopicVersions,
    };
    use clawg_core::memory::store_lock::StoreHolder;

    const P: Profile = Profile::Dev;

    fn kinds(found: &[Finding]) -> Vec<(&'static str, Severity, &'static str)> {
        found.iter().map(|f| (f.check_id, f.severity, f.reason_code)).collect()
    }

    #[test]
    fn shell_words_are_quoted_only_when_a_shell_would_read_them() {
        assert_eq!(shell_word("op-1"), "op-1");
        assert_eq!(shell_word("/home/u/.clawg-dev"), "/home/u/.clawg-dev");
        assert_eq!(shell_word("a b"), "'a b'");
        assert_eq!(shell_word("it's;rm"), r"'it'\''s;rm'");
        assert_eq!(shell_word(""), "''");
    }

    #[test]
    fn data_dir_findings() {
        let dir = Path::new("/home/u/.clawg-dev");
        assert_eq!(kinds(&data_dir(dir, DirState::Missing)), [("data_dir", Severity::Info, "missing")]);
        assert_eq!(kinds(&data_dir(dir, DirState::Present { mode: Some(0o40700) })), [("data_dir", Severity::Ok, "healthy")]);
        assert_eq!(kinds(&data_dir(dir, DirState::Present { mode: None })), [("data_dir", Severity::Ok, "healthy")]);
        let open = data_dir(dir, DirState::Present { mode: Some(0o40755) });
        assert_eq!(kinds(&open), [("data_dir", Severity::Warn, "open_mode")]);
        assert_eq!(open[0].evidence["mode"], "0755");
        assert_eq!(open[0].safe_next_step.as_deref(), Some("chmod 700 /home/u/.clawg-dev"));
    }

    #[test]
    fn socket_findings() {
        let path = Path::new("/tmp/clawg.sock");
        assert_eq!(kinds(&socket(path, &Ok(()))), [("socket", Severity::Ok, "healthy")]);
        let long = socket(path, &Err(SocketError::TooLong { path: path.into(), len: 120, limit: 108 }));
        assert_eq!(kinds(&long), [("socket", Severity::Error, "path_unusable")]);
        assert_eq!((long[0].evidence["len"].as_u64(), long[0].evidence["limit"].as_u64()), (Some(120), Some(108)));
    }

    #[test]
    fn owner_findings() {
        assert_eq!(kinds(&store_owner(None)), [("store.owner", Severity::Ok, "healthy")]);
        let owner = StoreOwner { holder: StoreHolder::Daemon, pid: 42, started_at_ms: 0 };
        let held = store_owner(Some(&owner));
        assert_eq!(kinds(&held), [("store.owner", Severity::Info, "held")]);
        assert_eq!((held[0].evidence["holder"].as_str(), held[0].evidence["pid"].as_u64()), (Some("daemon"), Some(42)));
    }

    #[test]
    fn schema_findings_follow_the_plan() {
        let at = |state| StoreProbe { state, db_version: Some(10), binary_version: 17 };
        let cases = [
            (ProbeState::Missing, Severity::Info, "missing"),
            (ProbeState::Unreadable("not a database".into()), Severity::Error, "unreadable"),
            (ProbeState::Schema(SchemaPlan::Fresh), Severity::Info, "fresh"),
            (ProbeState::Schema(SchemaPlan::UpToDate), Severity::Ok, "healthy"),
            (ProbeState::Schema(SchemaPlan::Pending { from: 10, to: 17 }), Severity::Info, "pending"),
            (ProbeState::Schema(SchemaPlan::Newer { db: 20, binary: 17 }), Severity::Error, "newer"),
            (ProbeState::Schema(SchemaPlan::Diverged { version: 9 }), Severity::Error, "diverged"),
        ];
        for (state, severity, reason) in cases {
            assert_eq!(kinds(&schema(&at(state), P)), [("store.schema", severity, reason)]);
        }
        let pending = schema(&at(ProbeState::Schema(SchemaPlan::Pending { from: 10, to: 17 })), P);
        assert_eq!(pending[0].safe_next_step.as_deref(), Some("clawg serve --profile dev"));
        assert!(pending[0].requires_confirmation, "a start migrates the store");
    }

    #[test]
    fn contract_findings() {
        let expected = clawg_core::memory::embedder::expected_contract();
        let stored = |model_id: &str, dim: i64, prefix: &str| StoredContract {
            model_id: model_id.into(),
            dim,
            query_prefix: prefix.into(),
            passage_prefix: prefix.into(),
        };
        assert_eq!(kinds(&contract(None, &expected)), [("store.contract", Severity::Info, "not_recorded")]);
        let same = stored(&expected.model_id, expected.dim as i64, &expected.query_prefix);
        assert_eq!(kinds(&contract(Some(&same), &expected)), [("store.contract", Severity::Ok, "healthy")]);
        let cls = stored(clawg_core::memory::embedder::LEGACY_CLS_MODEL_ID, expected.dim as i64, &expected.query_prefix);
        assert_eq!(kinds(&contract(Some(&cls), &expected)), [("store.contract", Severity::Info, "pooling_upgrade")]);
        for other in [
            stored("other-model", expected.dim as i64, &expected.query_prefix),
            stored(&expected.model_id, 768, &expected.query_prefix),
            stored(&expected.model_id, expected.dim as i64, "passage: "),
        ] {
            let found = contract(Some(&other), &expected);
            assert_eq!(kinds(&found), [("store.contract", Severity::Error, "mismatch")]);
            assert_eq!(found[0].evidence["stored"]["model_id"], other.model_id);
        }
    }

    #[test]
    fn backup_findings() {
        let now = 1_791_827_476_123;
        let ago = |days: i64, reason| snapshot_name(now - days * DAY_MS, reason);
        let fresh = vec![ago(40, Reason::PreMigration), ago(2, Reason::Manual), "notes.txt".to_string()];
        let found = backups(&fresh, true, now, P);
        assert_eq!(kinds(&found), [("backup.age", Severity::Ok, "healthy")]);
        assert_eq!((found[0].evidence["age_days"].as_i64(), found[0].evidence["snapshots"].as_u64()), (Some(2), Some(2)));

        let stale = backups(&[ago(90, Reason::Manual)], true, now, P);
        assert_eq!(kinds(&stale), [("backup.age", Severity::Warn, "stale")]);
        assert_eq!(stale[0].evidence["age_days"], 90);
        assert_eq!(stale[0].safe_next_step.as_deref(), Some("clawg backup --profile dev"));
        assert!(!stale[0].requires_confirmation);

        assert_eq!(kinds(&backups(&["notes.txt".into()], true, now, P)), [("backup.age", Severity::Warn, "none")]);
        assert_eq!(kinds(&backups(&[], false, now, P)), [("backup.age", Severity::Ok, "healthy")]);
    }

    #[test]
    fn spill_findings() {
        assert_eq!(kinds(&spill(None, None)), [("spill", Severity::Ok, "healthy")]);
        let both = spill(Some(Path::new("/d/ingest-pending.jsonl")), Some((Path::new("/d/ingest-pending.rejected.jsonl"), 3)));
        assert_eq!(kinds(&both), [("spill", Severity::Info, "pending"), ("spill", Severity::Warn, "rejected")]);
        assert_eq!(both[1].evidence["lines"], 3);
    }

    #[test]
    fn sharing_findings() {
        assert_eq!(kinds(&sharing(None, P)), [("sharing", Severity::Ok, "healthy")]);
        let bad = sharing(Some("sharing.json has version 2; this clawg reads version 1"), P);
        assert_eq!(kinds(&bad), [("sharing", Severity::Warn, "invalid")]);
        assert_eq!(bad[0].safe_next_step.as_deref(), Some("clawg sharing --profile dev"));
    }

    #[test]
    fn preferences_findings() {
        let path = Path::new("/d/preferences.json");
        assert!(preferences(&PreferencesState::Missing, path).is_empty(), "no file, no finding");
        assert_eq!(kinds(&preferences(&PreferencesState::Readable, path)), [("preferences", Severity::Ok, "healthy")]);
        let bad = preferences(&PreferencesState::Invalid("preferences.json has version 2; this clawg reads version 1".into()), path);
        assert_eq!(kinds(&bad), [("preferences", Severity::Warn, "invalid")]);
        assert!(bad[0].message.contains("version 2") && bad[0].message.contains("without a language preference"), "{}", bad[0].message);
        assert_eq!(bad[0].safe_next_step, None, "the file is the user's to fix");
        assert_eq!(bad[0].evidence["path"], "/d/preferences.json");
    }

    #[test]
    fn protocol_block_findings() {
        let path = Path::new("/home/u/.claude/CLAUDE.md");
        let wanted = "<!-- BEGIN -->\nrules\n<!-- END -->";
        let ok = protocol_block(&BlockState::Installed(wanted.replace('\n', "\r\n")), wanted, path, P);
        assert_eq!(kinds(&ok), [("protocol_block", Severity::Ok, "healthy")]);
        let stale = protocol_block(&BlockState::Installed("<!-- BEGIN -->\nold\n<!-- END -->".into()), wanted, path, P);
        assert_eq!(kinds(&stale), [("protocol_block", Severity::Warn, "stale")]);
        assert_eq!(stale[0].safe_next_step.as_deref(), Some("clawg setup claude-code --profile dev"));
        let missing = protocol_block(&BlockState::Missing, wanted, path, P);
        assert_eq!(kinds(&missing), [("protocol_block", Severity::Info, "not_installed")]);
        assert_eq!(missing[0].safe_next_step.as_deref(), Some("clawg setup claude-code --profile dev"));
        let unreadable = protocol_block(&BlockState::Unreadable("a lone marker".into()), wanted, path, P);
        assert_eq!(kinds(&unreadable), [("protocol_block", Severity::Warn, "unreadable")]);
        assert_eq!(unreadable[0].evidence["path"], "/home/u/.claude/CLAUDE.md");
    }

    #[test]
    fn outbox_findings() {
        let facts = OutboxFacts {
            pending: 2,
            failed: vec![FailedSave { operation_id: "op 1".into(), agent_id: "claude-code".into(), attempts: 3 }],
        };
        let found = outbox(&facts, false, P);
        assert_eq!(kinds(&found), [("outbox", Severity::Warn, "failed"), ("outbox", Severity::Info, "pending")]);
        assert_eq!(
            found[0].safe_next_step.as_deref(),
            Some("clawg outbox retry 'op 1' --agent claude-code --profile dev")
        );
        assert!(found[0].requires_confirmation);
        assert_eq!(kinds(&outbox(&OutboxFacts { pending: 2, failed: vec![] }, true, P)), [("outbox", Severity::Ok, "healthy")]);
        assert_eq!(kinds(&outbox(&OutboxFacts::default(), false, P)), [("outbox", Severity::Ok, "healthy")]);
    }

    #[test]
    fn agent_findings() {
        let rows = |mcp_live| {
            vec![
                AgentRows { agent_id: None, live: 1, total: 1 },
                AgentRows { agent_id: Some("claude-code".into()), live: 5, total: 7 },
                AgentRows { agent_id: Some(FALLBACK_AGENT_ID.into()), live: mcp_live, total: 2 },
            ]
        };
        let clean = agents(&rows(0));
        assert_eq!(kinds(&clean), [("agents", Severity::Ok, "healthy")]);
        assert_eq!(clean[0].evidence["agents"][1], json!({ "agent_id": "claude-code", "live": 5, "total": 7 }));
        let split = agents(&rows(2));
        assert_eq!(kinds(&split), [("agents", Severity::Ok, "healthy"), ("agents", Severity::Warn, "fallback_agent")]);
        assert_eq!(split[1].evidence["live"], 2);
    }

    #[test]
    fn project_findings() {
        let root = if cfg!(windows) { r"C:\work\proj" } else { "/work/proj" };
        let good = ProjectRow { id: derive_project_id(Path::new(root)), root_path: root.into() };
        assert_eq!(kinds(&projects(&[good.clone()])), [("projects", Severity::Ok, "healthy")]);
        let bad = ProjectRow { id: "proj-0000".into(), root_path: root.into() };
        let found = projects(&[good, bad]);
        assert_eq!(kinds(&found), [("projects", Severity::Warn, "id_mismatch")]);
        assert_eq!(found[0].evidence["project_id"], "proj-0000");
    }

    #[test]
    fn invariant_findings() {
        let healthy = invariants(&Invariants::default());
        assert_eq!(
            kinds(&healthy),
            [
                ("memory.topic", Severity::Ok, "healthy"),
                ("memory.superseded_by", Severity::Ok, "healthy"),
                ("memory.revived_from", Severity::Ok, "healthy"),
                ("memory.content_hash", Severity::Ok, "healthy"),
                ("memory.pieces", Severity::Ok, "healthy"),
                ("memory.disputed", Severity::Ok, "healthy"),
            ]
        );
        let broken = Invariants {
            topic_versions: vec![TopicVersions { agents: vec![Some("a".into()), None], project_id: None, saves: vec![1, 4] }],
            dangling_superseded: vec![
                DanglingSuperseded { id: 2, superseded_by: Some(9), cause: Some("revision".into()) },
                DanglingSuperseded { id: 3, superseded_by: Some(9), cause: Some("verdict".into()) },
                DanglingSuperseded { id: 5, superseded_by: None, cause: None },
            ],
            dangling_revived: vec![DanglingRevived { id: 6, revived_from: 8 }],
            hash_duplicates: vec![HashDuplicates { agents: vec![Some("a".into())], project_id: None, ids: vec![10, 11] }],
            purged_headers: vec![PurgedHeader { save_id: 3, first_piece: 40 }],
            lonely_disputes: vec![12],
        };
        let found = invariants(&broken);
        assert_eq!(
            kinds(&found),
            [
                ("memory.topic", Severity::Warn, "several_live_saves"),
                ("memory.superseded_by", Severity::Warn, "dangling"),
                ("memory.superseded_by", Severity::Info, "dangling_verdict"),
                ("memory.superseded_by", Severity::Warn, "dangling"),
                ("memory.revived_from", Severity::Info, "dangling"),
                ("memory.content_hash", Severity::Warn, "live_duplicates"),
                ("memory.pieces", Severity::Warn, "purged_header"),
                ("memory.disputed", Severity::Info, "no_live_rival"),
            ]
        );
        assert_eq!(found[0].evidence["saves"], json!([1, 4]));
        assert_eq!(found[0].evidence["agents"], json!(["a", null]), "the authors, a row no agent saved as null");
        assert_eq!(found[6].evidence["save_id"], 3);
    }
}
