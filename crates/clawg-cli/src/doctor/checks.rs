//! Each check as a pure function from facts to findings: no filesystem, no
//! store, no clock. [`super::run`] gathers the facts.
//!
//! Every check writes its `message` in the user's language, one `match` per
//! sentence; the rest of a finding is the same in every language, so a script
//! reads it alike whatever the user chose.

use std::path::Path;

use clawg_core::memory::backup::{parse_snapshot_time, ProbeState, SchemaPlan, StoreProbe};
use clawg_core::memory::bootstrap::is_pooling_upgrade;
use clawg_core::memory::db::{EmbeddingIdentity, StoredContract};
use clawg_core::memory::derive_project_id;
use clawg_core::memory::identity::is_candidate_for;
use clawg_core::memory::inspect::{
    AgentRows, FolderPrint, IdRows, Invariants, OutboxFacts, ProjectRow, RegistryRow, RemoteRow,
};
use clawg_core::memory::merge::MAX_MERGE_HOPS;
use clawg_core::memory::store_lock::StoreOwner;
use clawg_core::paths::Profile;
use clawg_core::preferences::Lang;
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

pub fn path(lang: Lang, analysis: &Analysis) -> Vec<Finding> {
    let installs: Vec<&Path> = analysis.installs.iter().map(|i| i.path.as_path()).collect();
    let evidence = json!({ "winner": analysis.winner, "installs": installs });
    let mut found = Vec::new();
    if analysis.shadowed {
        let winner = analysis.winner.as_deref().map(|w| w.display().to_string()).unwrap_or_default();
        let message = match lang {
            Lang::En => format!("the PATH runs {winner}, not this binary"),
            Lang::Es => format!("el PATH ejecuta {winner}, no este binario"),
        };
        found.push(Finding::new("path", Severity::Error, "shadowed", message).evidence(evidence.clone()));
    }
    if analysis.duplicates {
        let message = match lang {
            Lang::En => format!("{} copies of clawg are on the PATH; a terminal runs only the first", installs.len()),
            Lang::Es => format!("hay {} copias de clawg en el PATH; una terminal ejecuta sólo la primera", installs.len()),
        };
        found.push(Finding::new("path", Severity::Error, "duplicate", message).evidence(evidence.clone()));
    }
    if found.is_empty() {
        let message = match lang {
            Lang::En => "the PATH resolves to this binary, or to no clawg at all",
            Lang::Es => "el PATH lleva a este binario, o a ningún clawg",
        };
        found.push(Finding::ok("path", message).evidence(evidence));
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

pub fn data_dir(lang: Lang, dir: &Path, state: DirState) -> Vec<Finding> {
    let evidence = json!({ "path": dir });
    match state {
        DirState::Missing => {
            let message = match lang {
                Lang::En => "the data directory does not exist yet: nothing has run in this profile",
                Lang::Es => "el directorio de datos todavía no existe: nada corrió en este perfil",
            };
            vec![Finding::new("data_dir", Severity::Info, "missing", message).evidence(evidence)]
        }
        DirState::Present { mode: Some(mode) } if mode & 0o077 != 0 => {
            let mode = format!("{:04o}", mode & 0o7777);
            let message = match lang {
                Lang::En => format!("the data directory is {mode}, so other users of this machine may read it"),
                Lang::Es => format!("el directorio de datos tiene el modo {mode}, así que otros usuarios de esta máquina pueden leerlo"),
            };
            vec![Finding::new("data_dir", Severity::Warn, "open_mode", message)
                .evidence(json!({ "path": dir, "mode": mode }))
                .step(format!("chmod 700 {}", shell_word(&dir.to_string_lossy())))]
        }
        DirState::Present { .. } => {
            let message = match lang {
                Lang::En => "the data directory is private to its owner",
                Lang::Es => "el directorio de datos es privado de su dueño",
            };
            vec![Finding::ok("data_dir", message).evidence(evidence)]
        }
    }
}

pub fn socket(lang: Lang, path: &Path, checked: &Result<(), SocketError>) -> Vec<Finding> {
    match checked {
        Ok(()) => {
            let message = match lang {
                Lang::En => "the socket path fits",
                Lang::Es => "la ruta del socket entra",
            };
            vec![Finding::ok("socket", message).evidence(json!({ "path": path }))]
        }
        Err(SocketError::TooLong { path, len, limit }) => {
            let message = match lang {
                Lang::En => format!(
                    "the socket path is {len} bytes and must be shorter than {limit}: the daemon does not start \
                     without --port, and a bridge starts it without one"
                ),
                Lang::Es => format!(
                    "la ruta del socket mide {len} bytes y tiene que medir menos de {limit}: el daemon no arranca \
                     sin --port, y un puente lo arranca sin él"
                ),
            };
            vec![Finding::new("socket", Severity::Error, "path_unusable", message)
                .evidence(json!({ "path": path, "len": len, "limit": limit }))]
        }
        Err(e) => vec![Finding::new("socket", Severity::Error, "path_unusable", e.to_string())
            .evidence(json!({ "path": path }))],
    }
}

pub fn store_owner(lang: Lang, owner: Option<&StoreOwner>) -> Vec<Finding> {
    match owner {
        None => {
            let message = match lang {
                Lang::En => "no clawg holds the store",
                Lang::Es => "ningún clawg tiene el store",
            };
            vec![Finding::ok("store.owner", message)]
        }
        Some(o) => {
            let message = match lang {
                Lang::En => format!("{} holds the store (pid {}); the store was read beside it", o.holder, o.pid),
                Lang::Es => format!("{} tiene el store (pid {}); el store se leyó a su lado", o.holder, o.pid),
            };
            vec![Finding::new("store.owner", Severity::Info, "held", message)
                .evidence(json!({ "holder": o.holder, "pid": o.pid }))]
        }
    }
}

pub fn schema(lang: Lang, probe: &StoreProbe, profile: Profile) -> Vec<Finding> {
    let evidence = json!({ "db_version": probe.db_version, "binary_version": probe.binary_version });
    let finding = match &probe.state {
        ProbeState::Missing => {
            let message = match lang {
                Lang::En => "there is no memory.db yet: the daemon creates it on its first start",
                Lang::Es => "todavía no hay memory.db: el daemon lo crea la primera vez que arranca",
            };
            Finding::new("store.schema", Severity::Info, "missing", message)
        }
        ProbeState::Unreadable(error) => {
            let message = match lang {
                Lang::En => format!("memory.db cannot be read: {error}"),
                Lang::Es => format!("no se puede leer memory.db: {error}"),
            };
            Finding::new("store.schema", Severity::Error, "unreadable", message).evidence(json!({ "error": error }))
        }
        ProbeState::Schema(SchemaPlan::Fresh) => {
            let message = match lang {
                Lang::En => "memory.db has no migration applied: the daemon sets it up on its next start",
                Lang::Es => "memory.db no tiene ninguna migración aplicada: el daemon lo prepara la próxima vez que arranque",
            };
            Finding::new("store.schema", Severity::Info, "fresh", message)
        }
        ProbeState::Schema(SchemaPlan::UpToDate) => {
            let message = match lang {
                Lang::En => "memory.db is at the schema of this clawg",
                Lang::Es => "memory.db está en el esquema de este clawg",
            };
            Finding::ok("store.schema", message).evidence(evidence)
        }
        ProbeState::Schema(SchemaPlan::Pending { from, to }) => {
            let message = match lang {
                Lang::En => format!(
                    "memory.db is at V{from}; this clawg migrates it to V{to} on its next start, after a pre-migration snapshot"
                ),
                Lang::Es => format!(
                    "memory.db está en V{from}; este clawg lo migra a V{to} la próxima vez que arranque, después de un \
                     snapshot pre-migration"
                ),
            };
            Finding::new("store.schema", Severity::Info, "pending", message)
                .evidence(evidence)
                .changing_step(with_profile("clawg serve", profile))
        }
        ProbeState::Schema(SchemaPlan::Newer { db, binary }) => {
            let message = match lang {
                Lang::En => format!("memory.db is at V{db}, newer than the V{binary} this clawg knows: this clawg refuses it untouched"),
                Lang::Es => format!(
                    "memory.db está en V{db}, más nueva que la V{binary} que conoce este clawg: este clawg lo rechaza sin tocarlo"
                ),
            };
            Finding::new("store.schema", Severity::Error, "newer", message).evidence(evidence)
        }
        ProbeState::Schema(SchemaPlan::Diverged { version }) => {
            let message = match lang {
                Lang::En => format!("the migration V{version} of memory.db does not match this clawg: it refuses the store untouched"),
                Lang::Es => format!(
                    "la migración V{version} de memory.db no coincide con la de este clawg: rechaza el store sin tocarlo"
                ),
            };
            Finding::new("store.schema", Severity::Error, "diverged", message).evidence(evidence)
        }
    };
    vec![finding]
}

pub fn contract(lang: Lang, stored: Option<&StoredContract>, expected: &EmbeddingIdentity) -> Vec<Finding> {
    let expected_json = json!({
        "model_id": expected.model_id,
        "dim": expected.dim,
        "query_prefix": expected.query_prefix,
        "passage_prefix": expected.passage_prefix,
    });
    let Some(stored) = stored else {
        let message = match lang {
            Lang::En => "no embedding contract is recorded yet: the first model load writes it",
            Lang::Es => "todavía no hay un contrato de embeddings registrado: lo escribe la primera carga del modelo",
        };
        return vec![Finding::new("store.contract", Severity::Info, "not_recorded", message)
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
        let message = match lang {
            Lang::En => "the stored vectors were made by this clawg's embedder",
            Lang::Es => "los vectores guardados los hizo el embedder de este clawg",
        };
        Finding::ok("store.contract", message)
    } else if is_pooling_upgrade(stored, expected) {
        let message = match lang {
            Lang::En => "the stored vectors were pooled by CLS: the next start re-embeds them by mean, after a pre-reembed snapshot",
            Lang::Es => {
                "los vectores guardados se agruparon por CLS: el próximo arranque los vuelve a calcular por media, \
                 después de un snapshot pre-reembed"
            }
        };
        Finding::new("store.contract", Severity::Info, "pooling_upgrade", message)
    } else {
        let message = match lang {
            Lang::En => format!(
                "the stored vectors were made by {} ({} dimensions), not by this clawg's {}: memory stays off",
                stored.model_id, stored.dim, expected.model_id
            ),
            Lang::Es => format!(
                "los vectores guardados los hizo {} ({} dimensiones), no el {} de este clawg: la memoria queda apagada",
                stored.model_id, stored.dim, expected.model_id
            ),
        };
        Finding::new("store.contract", Severity::Error, "mismatch", message)
    };
    vec![finding.evidence(evidence)]
}

/// `names` are the files of `<data>/backups`; a name that is not a snapshot's
/// does not count. Only a store that holds rows has something to lose.
pub fn backups(lang: Lang, names: &[String], has_rows: bool, now_ms: i64, profile: Profile) -> Vec<Finding> {
    let newest = names.iter().filter_map(|n| parse_snapshot_time(n).map(|t| (t, n))).max();
    let count = names.iter().filter(|n| parse_snapshot_time(n).is_some()).count();
    let step = with_profile("clawg backup", profile);
    let finding = match newest {
        _ if !has_rows => {
            let message = match lang {
                Lang::En => "the store holds no row to lose",
                Lang::Es => "el store no tiene ninguna fila que perder",
            };
            Finding::ok("backup.age", message)
        }
        None => {
            let message = match lang {
                Lang::En => "the store holds rows and there is no snapshot of it",
                Lang::Es => "el store tiene filas y no hay ningún snapshot de él",
            };
            Finding::new("backup.age", Severity::Warn, "none", message).step(step)
        }
        Some((taken, name)) => {
            let age_days = (now_ms - taken).div_euclid(DAY_MS);
            let evidence = json!({ "newest": name, "age_days": age_days, "snapshots": count });
            let message = match lang {
                Lang::En => format!("the newest snapshot is {age_days} days old"),
                Lang::Es if age_days == 1 => "el snapshot más nuevo tiene 1 día".to_string(),
                Lang::Es => format!("el snapshot más nuevo tiene {age_days} días"),
            };
            if age_days > BACKUP_MAX_AGE_DAYS {
                Finding::new("backup.age", Severity::Warn, "stale", message).evidence(evidence).step(step)
            } else {
                Finding::ok("backup.age", message).evidence(evidence)
            }
        }
    };
    vec![finding]
}

/// `pending` is the spill file an older clawg left, `rejected` the file of
/// its lines that did not parse with how many lines it holds.
pub fn spill(lang: Lang, pending: Option<&Path>, rejected: Option<(&Path, usize)>) -> Vec<Finding> {
    let mut found = Vec::new();
    if let Some(path) = pending {
        let message = match lang {
            Lang::En => "an older clawg left saves in a spill file: the next start moves them to the outbox",
            Lang::Es => "un clawg anterior dejó saves en un archivo de desborde: el próximo arranque los pasa al outbox",
        };
        found.push(Finding::new("spill", Severity::Info, "pending", message).evidence(json!({ "path": path })));
    }
    if let Some((path, lines)) = rejected {
        let message = match lang {
            Lang::En => format!(
                "{lines} lines of saves an older clawg could not read are kept aside; nothing deletes them, \
                 and only a person can tell what they were"
            ),
            Lang::Es if lines == 1 => "1 línea de saves que un clawg anterior no pudo leer se guarda aparte; nada la \
                                       borra, y sólo una persona puede decir qué era"
                .to_string(),
            Lang::Es => format!(
                "{lines} líneas de saves que un clawg anterior no pudo leer se guardan aparte; nada las borra, \
                 y sólo una persona puede decir qué eran"
            ),
        };
        found.push(
            Finding::new("spill", Severity::Warn, "rejected", message).evidence(json!({ "path": path, "lines": lines })),
        );
    }
    if found.is_empty() {
        let message = match lang {
            Lang::En => "no spill file is left",
            Lang::Es => "no queda ningún archivo de desborde",
        };
        found.push(Finding::ok("spill", message));
    }
    found
}

/// `error` is why `sharing.json` could not be used, as the user is shown it.
pub fn sharing(lang: Lang, error: Option<&str>, profile: Profile) -> Vec<Finding> {
    match error {
        None => {
            let message = match lang {
                Lang::En => "the sharing policy is readable",
                Lang::Es => "la política de compartir se puede leer",
            };
            vec![Finding::ok("sharing", message)]
        }
        Some(error) => {
            let message = match lang {
                Lang::En => format!("{error}; nothing is shared between agents until it is fixed"),
                Lang::Es => format!("{error}; los agentes no comparten nada hasta que se arregle"),
            };
            vec![Finding::new("sharing", Severity::Warn, "invalid", message)
                .evidence(json!({ "error": error }))
                .step(with_profile("clawg sharing", profile))]
        }
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
pub fn preferences(lang: Lang, state: &PreferencesState, path: &Path) -> Vec<Finding> {
    match state {
        PreferencesState::Missing => Vec::new(),
        PreferencesState::Readable => {
            let message = match lang {
                Lang::En => "the preferences are readable",
                Lang::Es => "las preferencias se pueden leer",
            };
            vec![Finding::ok("preferences", message)]
        }
        PreferencesState::Invalid(error) => {
            let message = match lang {
                Lang::En => format!("{error}; clawg goes on without a language preference until it is fixed or removed"),
                Lang::Es => format!("{error}; clawg sigue sin preferencia de idioma hasta que se arregle o se borre"),
            };
            vec![Finding::new("preferences", Severity::Warn, "invalid", message)
                .evidence(json!({ "path": path, "error": error }))]
        }
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
pub fn protocol_block(lang: Lang, state: &BlockState, wanted: &str, path: &Path, profile: Profile) -> Vec<Finding> {
    let setup = with_profile("clawg setup claude-code", profile);
    let finding = match state {
        BlockState::Installed(text) if text.replace("\r\n", "\n") == wanted.replace("\r\n", "\n") => {
            let message = match lang {
                Lang::En => "the protocol block of CLAUDE.md is the one clawg setup writes",
                Lang::Es => "el bloque del protocolo de CLAUDE.md es el que escribe clawg setup",
            };
            Finding::ok("protocol_block", message)
        }
        BlockState::Installed(_) => {
            let message = match lang {
                Lang::En => {
                    "the protocol block of CLAUDE.md is not the one clawg setup writes for the preferred language, \
                     so the agent may read other rules there than in the server's instructions"
                }
                Lang::Es => {
                    "el bloque del protocolo de CLAUDE.md no es el que escribe clawg setup para el idioma preferido, \
                     así que el agente puede leer ahí otras reglas que en las instrucciones del servidor"
                }
            };
            Finding::new("protocol_block", Severity::Warn, "stale", message).step(setup)
        }
        BlockState::Missing => {
            let message = match lang {
                Lang::En => {
                    "CLAUDE.md has no clawg protocol block; the agent reads the preferred language only in the server's instructions"
                }
                Lang::Es => {
                    "CLAUDE.md no tiene el bloque del protocolo de clawg; el agente lee el idioma preferido sólo en las \
                     instrucciones del servidor"
                }
            };
            Finding::new("protocol_block", Severity::Info, "not_installed", message).step(setup)
        }
        BlockState::Unreadable(error) => {
            let message = match lang {
                Lang::En => format!("cannot check the protocol block: {error}"),
                Lang::Es => format!("no se puede revisar el bloque del protocolo: {error}"),
            };
            Finding::new("protocol_block", Severity::Warn, "unreadable", message)
        }
    };
    vec![finding.evidence(json!({ "path": path }))]
}

/// Pending rows with a daemon running are being stored; without one, they
/// wait for the next start.
pub fn outbox(lang: Lang, facts: &OutboxFacts, daemon_running: bool, profile: Profile) -> Vec<Finding> {
    let mut found: Vec<Finding> = facts
        .failed
        .iter()
        .map(|f| {
            let message = match lang {
                Lang::En => format!(
                    "the save {} of {} failed after {} attempts and is not stored",
                    f.operation_id, f.agent_id, f.attempts
                ),
                Lang::Es => format!(
                    "el save {} de {} falló después de {} intentos y no está guardado",
                    f.operation_id, f.agent_id, f.attempts
                ),
            };
            Finding::new("outbox", Severity::Warn, "failed", message)
                .evidence(json!({ "operation_id": f.operation_id, "agent_id": f.agent_id, "attempts": f.attempts }))
                .changing_step(with_profile(
                    &format!("clawg outbox retry {} --agent {}", shell_word(&f.operation_id), shell_word(&f.agent_id)),
                    profile,
                ))
        })
        .collect();
    if facts.pending > 0 && !daemon_running {
        let message = match lang {
            Lang::En => format!("{} accepted saves wait in the outbox for the next start", facts.pending),
            Lang::Es if facts.pending == 1 => "1 save aceptado espera en el outbox al próximo arranque".to_string(),
            Lang::Es => format!("{} saves aceptados esperan en el outbox al próximo arranque", facts.pending),
        };
        found.push(
            Finding::new("outbox", Severity::Info, "pending", message)
                .evidence(json!({ "pending": facts.pending }))
                .step(with_profile("clawg serve", profile)),
        );
    }
    if found.is_empty() {
        let message = match lang {
            Lang::En => "no save is failed or waiting",
            Lang::Es => "ningún save falló ni espera",
        };
        found.push(Finding::ok("outbox", message).evidence(json!({ "pending": facts.pending })));
    }
    found
}

pub fn agents(lang: Lang, rows: &[AgentRows]) -> Vec<Finding> {
    let listed: Vec<_> =
        rows.iter().map(|r| json!({ "agent_id": r.agent_id, "live": r.live, "total": r.total })).collect();
    let named = rows.iter().filter(|r| r.agent_id.is_some()).count();
    let message = match lang {
        Lang::En => format!("{named} agents hold rows"),
        Lang::Es if named == 1 => "1 agente tiene filas".to_string(),
        Lang::Es => format!("{named} agentes tienen filas"),
    };
    let mut found = vec![Finding::ok("agents", message).evidence(json!({ "agents": listed }))];
    if let Some(fallback) = rows.iter().find(|r| r.agent_id.as_deref() == Some(FALLBACK_AGENT_ID) && r.live > 0) {
        let message = match lang {
            Lang::En => format!(
                "{} live memories are under `{FALLBACK_AGENT_ID}`, the id a bridge takes when its client gives \
                 neither --agent nor its own name; every agent reads and changes them, but the workspace and the \
                 recent saves of the client's own agent leave them out",
                fallback.live
            ),
            Lang::Es => format!(
                "{} memorias vivas están bajo `{FALLBACK_AGENT_ID}`, el id que toma un puente cuando su cliente no da \
                 --agent ni su propio nombre; todos los agentes las leen y las cambian, pero el workspace y los saves \
                 recientes del agente del cliente las dejan afuera",
                fallback.live
            ),
        };
        found.push(
            Finding::new("agents", Severity::Warn, "fallback_agent", message)
                .evidence(json!({ "agent_id": FALLBACK_AGENT_ID, "live": fallback.live })),
        );
    }
    found
}

pub fn projects(lang: Lang, rows: &[ProjectRow]) -> Vec<Finding> {
    let mut found: Vec<Finding> = rows
        .iter()
        .filter_map(|p| {
            let root = Path::new(&p.root_path);
            let derived = derive_project_id(root);
            (!is_candidate_for(root, &p.id)).then(|| {
                let message = match lang {
                    Lang::En => format!("the project {} is recorded for a root whose id is {derived}", p.id),
                    Lang::Es => format!("el proyecto {} está registrado para una raíz cuyo id es {derived}", p.id),
                };
                Finding::new("projects", Severity::Warn, "id_mismatch", message)
                    .evidence(json!({ "project_id": p.id, "root_path": p.root_path, "derived": derived }))
            })
        })
        .collect();
    if found.is_empty() {
        let message = match lang {
            Lang::En => "every project id is one its root may be given",
            Lang::Es => "el id de cada proyecto es uno que su raíz puede recibir",
        };
        found.push(Finding::ok("projects", message).evidence(json!({ "projects": rows.len() })));
    }
    found
}

/// A project bound to more than one remote, each root that uses it named
/// with the remote it has now (`origins`, beside its row): a repository
/// cloned where another one was joined the other's project before a row
/// served only the remote bound to it, and a renamed remote leaves the same.
/// Telling them apart takes the user, so nothing is proposed.
pub fn project_remotes(lang: Lang, remotes: &[RemoteRow], origins: &[(ProjectRow, Option<String>)]) -> Vec<Finding> {
    let mut by_project: std::collections::BTreeMap<&str, Vec<&str>> = std::collections::BTreeMap::new();
    for r in remotes {
        by_project.entry(&r.project_id).or_default().push(&r.remote);
    }
    let mut found = Vec::new();
    for (project_id, mut bound) in by_project.into_iter().filter(|(_, bound)| bound.len() > 1) {
        bound.sort_unstable();
        let roots: Vec<_> = origins
            .iter()
            .filter(|(row, origin)| row.id == project_id || origin.as_deref().is_some_and(|o| bound.contains(&o)))
            .map(|(row, origin)| json!({ "root_id": row.id, "root_path": row.root_path, "origin": origin }))
            .collect();
        let message = match lang {
            Lang::En => format!(
                "the project {project_id} is bound to {} remotes: a repository cloned where another one was may have \
                 joined it, or its remote was renamed",
                bound.len()
            ),
            Lang::Es => format!(
                "el proyecto {project_id} está atado a {} remotos: un repositorio clonado donde estaba otro pudo \
                 haberse unido a él, o se renombró su remoto",
                bound.len()
            ),
        };
        found.push(
            Finding::new("project_remotes", Severity::Warn, "several_remotes", message)
                .evidence(json!({ "project_id": project_id, "remotes": bound, "roots": roots })),
        );
    }
    if found.is_empty() {
        let message = match lang {
            Lang::En => "no project is bound to more than one remote",
            Lang::Es => "ningún proyecto está atado a más de un remoto",
        };
        found.push(Finding::ok("project_remotes", message).evidence(json!({ "remotes": remotes.len() })));
    }
    found
}

/// The live project `id` leads to through the merges of `registry`; `None`
/// when `id` is no project or its chain ends in none (a cycle, one too long,
/// an id `project` does not have).
fn live_of<'a>(registry: &'a [RegistryRow], id: &str) -> Option<&'a RegistryRow> {
    let mut at = registry.iter().find(|p| p.id == id)?;
    for _ in 0..=MAX_MERGE_HOPS {
        match &at.merged_into {
            None => return Some(at),
            Some(into) => at = registry.iter().find(|p| p.id == *into)?,
        }
    }
    None
}

/// Two live projects that may be one: with the same name, in any case, or
/// with folders of one first commit that no pair of remotes tells apart (a
/// fork has a remote of its own). A pair someone answered is not one. The
/// step merges the newer into the older, the one that usually has the
/// history, as a dry run.
pub fn project_duplicates(
    lang: Lang,
    registry: &[RegistryRow],
    folders: &[FolderPrint],
    answered: &[(String, String)],
    profile: Profile,
) -> Vec<Finding> {
    let live: Vec<&RegistryRow> = registry.iter().filter(|p| p.merged_into.is_none()).collect();
    let answered = |a: &str, b: &str| {
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        answered.iter().any(|(x, y)| x == a && y == b)
    };
    let ordered = |a: &'_ RegistryRow, b: &'_ RegistryRow| if (a.created_at, &a.id) <= (b.created_at, &b.id) { (a.clone(), b.clone()) } else { (b.clone(), a.clone()) };
    let mut pairs: Vec<(&'static str, RegistryRow, RegistryRow, Option<&str>)> = Vec::new();
    for (i, a) in live.iter().enumerate() {
        for b in &live[i + 1..] {
            if a.name.to_lowercase() == b.name.to_lowercase() && !answered(&a.id, &b.id) {
                let (older, newer) = ordered(a, b);
                pairs.push(("same_name", older, newer, None));
            }
        }
    }
    let mut by_commit: std::collections::BTreeMap<&str, Vec<(&RegistryRow, Option<&str>)>> = std::collections::BTreeMap::new();
    for f in folders {
        let (Some(commit), Some(project)) = (f.root_commit.as_deref(), live_of(registry, f.project_id.as_deref().unwrap_or(&f.root_id)))
        else {
            continue;
        };
        by_commit.entry(commit).or_default().push((project, f.remote.as_deref()));
    }
    for (commit, sharing) in by_commit {
        let mut seen: Vec<(String, String)> = Vec::new();
        for (i, (a, a_remote)) in sharing.iter().enumerate() {
            for (b, b_remote) in &sharing[i + 1..] {
                let told_apart = matches!((a_remote, b_remote), (Some(x), Some(y)) if x != y);
                if a.id == b.id || told_apart || answered(&a.id, &b.id) {
                    continue;
                }
                let (older, newer) = ordered(a, b);
                if seen.contains(&(older.id.clone(), newer.id.clone())) {
                    continue;
                }
                seen.push((older.id.clone(), newer.id.clone()));
                pairs.push(("same_first_commit", older, newer, Some(commit)));
            }
        }
    }
    let mut found = Vec::new();
    for (reason, older, newer, commit) in pairs {
        let (o, n, name) = (&older.id, &newer.id, &older.name);
        let message = match (reason, lang) {
            ("same_name", Lang::En) => format!(
                "projects {o} and {n} are both named \"{name}\": they may be one project, and either can be merged into the other"
            ),
            ("same_name", Lang::Es) => format!(
                "los proyectos {o} y {n} se llaman los dos \"{name}\": pueden ser uno solo, y cualquiera de los dos se puede unir en el otro"
            ),
            (_, Lang::En) => format!(
                "projects {o} and {n} have folders with the same first commit and no remote that tells them apart: \
                 they may be one project, and either can be merged into the other"
            ),
            (_, Lang::Es) => format!(
                "los proyectos {o} y {n} tienen carpetas con el mismo primer commit y ningún remoto que los distinga: \
                 pueden ser uno solo, y cualquiera de los dos se puede unir en el otro"
            ),
        };
        let mut evidence = json!({ "project_ids": [o, n], "names": [older.name, newer.name] });
        if let Some(commit) = commit {
            evidence["root_commit"] = json!(commit);
        }
        found.push(
            Finding::new("project_duplicates", Severity::Warn, reason, message)
                .evidence(evidence)
                .step(with_profile(&format!("clawg projects merge {n} --into {o} --dry-run"), profile)),
        );
    }
    if found.is_empty() {
        let message = match lang {
            Lang::En => "no two projects look like one",
            Lang::Es => "ningún par de proyectos parece ser uno solo",
        };
        found.push(Finding::ok("project_duplicates", message).evidence(json!({ "projects": live.len() })));
    }
    found
}

/// The ids rows are saved under that `project` does not have: V27's triggers
/// adopt every id `memory` is written under, but not one of the other two
/// tables.
pub fn project_orphans(lang: Lang, registry: &[RegistryRow], rows: &[IdRows]) -> Vec<Finding> {
    let mut found = Vec::new();
    for r in rows.iter().filter(|r| !registry.iter().any(|p| p.id == r.project_id)) {
        let (id, n) = (&r.project_id, r.memories + r.trajectories + r.skills);
        let message = match (n == 1, lang) {
            (true, Lang::En) => format!("1 row is saved under {id}, which is no project"),
            (false, Lang::En) => format!("{n} rows are saved under {id}, which is no project"),
            (true, Lang::Es) => format!("1 fila está guardada bajo {id}, que no es un proyecto"),
            (false, Lang::Es) => format!("{n} filas están guardadas bajo {id}, que no es un proyecto"),
        };
        found.push(Finding::new("project_orphans", Severity::Warn, "no_project", message).evidence(json!({
            "project_id": id,
            "memories": r.memories,
            "trajectories": r.trajectories,
            "skills": r.skills,
        })));
    }
    if found.is_empty() {
        let message = match lang {
            Lang::En => "every id rows are saved under is a project",
            Lang::Es => "cada id con filas guardadas es un proyecto",
        };
        found.push(Finding::ok("project_orphans", message).evidence(json!({ "ids": rows.len() })));
    }
    found
}

/// Rows still under a project merged into another, which a merge again of
/// the same two moves, and a merged project whose chain of merges ends in no
/// live project, which nothing repairs on its own.
pub fn project_merges(lang: Lang, registry: &[RegistryRow], rows: &[IdRows], profile: Profile) -> Vec<Finding> {
    let mut found = Vec::new();
    for r in rows {
        let Some(tombstone) = registry.iter().find(|p| p.id == r.project_id && p.merged_into.is_some()) else {
            continue;
        };
        let Some(live) = live_of(registry, &tombstone.id) else {
            continue;
        };
        let (id, into, n) = (&tombstone.id, &live.id, r.memories + r.trajectories + r.skills);
        let message = match (n == 1, lang) {
            (true, Lang::En) => format!("1 row is still under {id}, which was merged into {into}"),
            (false, Lang::En) => format!("{n} rows are still under {id}, which was merged into {into}"),
            (true, Lang::Es) => format!("1 fila sigue bajo {id}, que se unió en {into}"),
            (false, Lang::Es) => format!("{n} filas siguen bajo {id}, que se unió en {into}"),
        };
        found.push(
            Finding::new("project_merges", Severity::Warn, "rows_under_merged", message)
                .evidence(json!({
                    "project_id": id,
                    "merged_into": into,
                    "memories": r.memories,
                    "trajectories": r.trajectories,
                    "skills": r.skills,
                }))
                .step(with_profile(&format!("clawg projects merge {id} --into {into} --dry-run"), profile)),
        );
    }
    for tombstone in registry.iter().filter(|p| p.merged_into.is_some() && live_of(registry, &p.id).is_none()) {
        let (id, into) = (&tombstone.id, tombstone.merged_into.as_deref().unwrap_or_default());
        let message = match lang {
            Lang::En => format!("project {id} was merged into {into}, which leads to no live project"),
            Lang::Es => format!("el proyecto {id} se unió en {into}, que no lleva a ningún proyecto vivo"),
        };
        found.push(
            Finding::new("project_merges", Severity::Warn, "broken_merge", message)
                .evidence(json!({ "project_id": id, "merged_into": into })),
        );
    }
    if found.is_empty() {
        let message = match lang {
            Lang::En => "no row is under a merged project, and every merge ends in a live one",
            Lang::Es => "ninguna fila quedó bajo un proyecto unido, y cada unión termina en uno vivo",
        };
        let merged = registry.iter().filter(|p| p.merged_into.is_some()).count();
        found.push(Finding::ok("project_merges", message).evidence(json!({ "merged": merged })));
    }
    found
}

/// One check per rule of `memory`, `ok` when nothing breaks it.
pub fn invariants(lang: Lang, inv: &Invariants) -> Vec<Finding> {
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
        match lang {
            Lang::En => "each topic key has one live save",
            Lang::Es => "cada clave de tema tiene un solo save vivo",
        },
        inv.topic_versions
            .iter()
            .map(|t| {
                let message = match lang {
                    Lang::En => format!(
                        "{} live saves hold one topic key of a project, where each should have replaced the one before",
                        t.saves.len()
                    ),
                    Lang::Es => format!(
                        "{} saves vivos tienen una misma clave de tema de un proyecto, donde cada uno tendría que \
                         haber reemplazado al anterior",
                        t.saves.len()
                    ),
                };
                Finding::new("memory.topic", Severity::Warn, "several_live_saves", message)
                    .evidence(json!({ "agents": t.agents, "project_id": t.project_id, "saves": t.saves }))
            })
            .collect(),
    );
    check(
        "memory.superseded_by",
        match lang {
            Lang::En => "every superseded row names the row that replaced it",
            Lang::Es => "cada fila reemplazada nombra la fila que la reemplazó",
        },
        inv.dangling_superseded
            .iter()
            .map(|d| {
                let evidence = json!({ "id": d.id, "superseded_by": d.superseded_by, "cause": d.cause });
                if d.cause.as_deref() == Some("verdict") {
                    let message = match lang {
                        Lang::En => format!("#{} lost a verdict to a row that was purged since, and stays replaced", d.id),
                        Lang::Es => format!(
                            "#{} perdió un veredicto frente a una fila que después se purgó, y sigue reemplazada",
                            d.id
                        ),
                    };
                    Finding::new("memory.superseded_by", Severity::Info, "dangling_verdict", message).evidence(evidence)
                } else {
                    let message = match lang {
                        Lang::En => format!(
                            "#{} is superseded by a row that does not exist, so no version of its topic comes back for it",
                            d.id
                        ),
                        Lang::Es => format!(
                            "#{} está reemplazada por una fila que no existe, así que ninguna versión de su tema vuelve \
                             por ella",
                            d.id
                        ),
                    };
                    Finding::new("memory.superseded_by", Severity::Warn, "dangling", message).evidence(evidence)
                }
            })
            .collect(),
    );
    check(
        "memory.revived_from",
        match lang {
            Lang::En => "every revived row names the delete that brought it back",
            Lang::Es => "cada fila revivida nombra el borrado que la trajo de vuelta",
        },
        inv.dangling_revived
            .iter()
            .map(|d| {
                let message = match lang {
                    Lang::En => format!("#{} was brought back by the delete of #{}, which was purged since", d.id, d.revived_from),
                    Lang::Es => format!(
                        "#{} volvió por el borrado de #{}, que después se purgó",
                        d.id, d.revived_from
                    ),
                };
                Finding::new("memory.revived_from", Severity::Info, "dangling", message)
                    .evidence(json!({ "id": d.id, "revived_from": d.revived_from }))
            })
            .collect(),
    );
    check(
        "memory.content_hash",
        match lang {
            Lang::En => "no two live rows of one project hold the same text",
            Lang::Es => "no hay dos filas vivas de un proyecto con el mismo texto",
        },
        inv.hash_duplicates
            .iter()
            .map(|h| {
                let message = match lang {
                    Lang::En => format!("{} live rows of one project hold the same text", h.ids.len()),
                    Lang::Es => format!("{} filas vivas de un proyecto tienen el mismo texto", h.ids.len()),
                };
                Finding::new("memory.content_hash", Severity::Warn, "live_duplicates", message)
                    .evidence(json!({ "agents": h.agents, "project_id": h.project_id, "ids": h.ids }))
            })
            .collect(),
    );
    check(
        "memory.pieces",
        match lang {
            Lang::En => "no save keeps the opening of a purged piece",
            Lang::Es => "ningún save guarda el comienzo de una pieza purgada",
        },
        inv.purged_headers
            .iter()
            .map(|p| {
                let message = match lang {
                    Lang::En => format!(
                        "the save {} lost its first piece #{} to a purge by an older clawg, and its other pieces \
                         still keep that piece's opening in memory.db",
                        p.save_id, p.first_piece
                    ),
                    Lang::Es => format!(
                        "el save {} perdió su primera pieza #{} en un purge de un clawg anterior, y sus otras piezas \
                         todavía guardan el comienzo de esa pieza en memory.db",
                        p.save_id, p.first_piece
                    ),
                };
                Finding::new("memory.pieces", Severity::Warn, "purged_header", message)
                    .evidence(json!({ "save_id": p.save_id, "first_piece": p.first_piece }))
            })
            .collect(),
    );
    check(
        "memory.disputed",
        match lang {
            Lang::En => "every disputed row is disputed against a live row",
            Lang::Es => "cada fila en disputa está en disputa con una fila viva",
        },
        inv.lonely_disputes
            .iter()
            .map(|id| {
                let message = match lang {
                    Lang::En => format!("#{id} is disputed against no live row; the daily maintenance pass sets it back to active"),
                    Lang::Es => format!(
                        "#{id} está en disputa sin ninguna fila viva enfrente; la pasada diaria de mantenimiento la vuelve a activa"
                    ),
                };
                Finding::new("memory.disputed", Severity::Info, "no_live_rival", message).evidence(json!({ "id": id }))
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
    use clawg_core::memory::identity::candidate_id;
    use clawg_core::memory::store_lock::StoreHolder;

    const P: Profile = Profile::Dev;
    const L: Lang = Lang::En;

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
        assert_eq!(kinds(&data_dir(L, dir, DirState::Missing)), [("data_dir", Severity::Info, "missing")]);
        assert_eq!(kinds(&data_dir(L, dir, DirState::Present { mode: Some(0o40700) })), [("data_dir", Severity::Ok, "healthy")]);
        assert_eq!(kinds(&data_dir(L, dir, DirState::Present { mode: None })), [("data_dir", Severity::Ok, "healthy")]);
        let open = data_dir(L, dir, DirState::Present { mode: Some(0o40755) });
        assert_eq!(kinds(&open), [("data_dir", Severity::Warn, "open_mode")]);
        assert_eq!(open[0].evidence["mode"], "0755");
        assert_eq!(open[0].safe_next_step.as_deref(), Some("chmod 700 /home/u/.clawg-dev"));
    }

    #[test]
    fn socket_findings() {
        let path = Path::new("/tmp/clawg.sock");
        assert_eq!(kinds(&socket(L, path, &Ok(()))), [("socket", Severity::Ok, "healthy")]);
        let long = socket(L, path, &Err(SocketError::TooLong { path: path.into(), len: 120, limit: 108 }));
        assert_eq!(kinds(&long), [("socket", Severity::Error, "path_unusable")]);
        assert_eq!((long[0].evidence["len"].as_u64(), long[0].evidence["limit"].as_u64()), (Some(120), Some(108)));
    }

    #[test]
    fn owner_findings() {
        assert_eq!(kinds(&store_owner(L, None)), [("store.owner", Severity::Ok, "healthy")]);
        let owner = StoreOwner { holder: StoreHolder::Daemon, pid: 42, started_at_ms: 0 };
        let held = store_owner(L, Some(&owner));
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
            assert_eq!(kinds(&schema(L, &at(state), P)), [("store.schema", severity, reason)]);
        }
        let pending = schema(L, &at(ProbeState::Schema(SchemaPlan::Pending { from: 10, to: 17 })), P);
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
        assert_eq!(kinds(&contract(L, None, &expected)), [("store.contract", Severity::Info, "not_recorded")]);
        let same = stored(&expected.model_id, expected.dim as i64, &expected.query_prefix);
        assert_eq!(kinds(&contract(L, Some(&same), &expected)), [("store.contract", Severity::Ok, "healthy")]);
        let cls = stored(clawg_core::memory::embedder::LEGACY_CLS_MODEL_ID, expected.dim as i64, &expected.query_prefix);
        assert_eq!(kinds(&contract(L, Some(&cls), &expected)), [("store.contract", Severity::Info, "pooling_upgrade")]);
        for other in [
            stored("other-model", expected.dim as i64, &expected.query_prefix),
            stored(&expected.model_id, 768, &expected.query_prefix),
            stored(&expected.model_id, expected.dim as i64, "passage: "),
        ] {
            let found = contract(L, Some(&other), &expected);
            assert_eq!(kinds(&found), [("store.contract", Severity::Error, "mismatch")]);
            assert_eq!(found[0].evidence["stored"]["model_id"], other.model_id);
        }
    }

    #[test]
    fn backup_findings() {
        let now = 1_791_827_476_123;
        let ago = |days: i64, reason| snapshot_name(now - days * DAY_MS, reason);
        let fresh = vec![ago(40, Reason::PreMigration), ago(2, Reason::Manual), "notes.txt".to_string()];
        let found = backups(L, &fresh, true, now, P);
        assert_eq!(kinds(&found), [("backup.age", Severity::Ok, "healthy")]);
        assert_eq!((found[0].evidence["age_days"].as_i64(), found[0].evidence["snapshots"].as_u64()), (Some(2), Some(2)));

        let stale = backups(L, &[ago(90, Reason::Manual)], true, now, P);
        assert_eq!(kinds(&stale), [("backup.age", Severity::Warn, "stale")]);
        assert_eq!(stale[0].evidence["age_days"], 90);
        assert_eq!(stale[0].safe_next_step.as_deref(), Some("clawg backup --profile dev"));
        assert!(!stale[0].requires_confirmation);

        assert_eq!(kinds(&backups(L, &["notes.txt".into()], true, now, P)), [("backup.age", Severity::Warn, "none")]);
        assert_eq!(kinds(&backups(L, &[], false, now, P)), [("backup.age", Severity::Ok, "healthy")]);
    }

    #[test]
    fn spill_findings() {
        assert_eq!(kinds(&spill(L, None, None)), [("spill", Severity::Ok, "healthy")]);
        let both = spill(L, Some(Path::new("/d/ingest-pending.jsonl")), Some((Path::new("/d/ingest-pending.rejected.jsonl"), 3)));
        assert_eq!(kinds(&both), [("spill", Severity::Info, "pending"), ("spill", Severity::Warn, "rejected")]);
        assert_eq!(both[1].evidence["lines"], 3);
    }

    #[test]
    fn sharing_findings() {
        assert_eq!(kinds(&sharing(L, None, P)), [("sharing", Severity::Ok, "healthy")]);
        let bad = sharing(L, Some("sharing.json has version 2; this clawg reads version 1"), P);
        assert_eq!(kinds(&bad), [("sharing", Severity::Warn, "invalid")]);
        assert_eq!(bad[0].safe_next_step.as_deref(), Some("clawg sharing --profile dev"));
    }

    #[test]
    fn preferences_findings() {
        let path = Path::new("/d/preferences.json");
        assert!(preferences(L, &PreferencesState::Missing, path).is_empty(), "no file, no finding");
        assert_eq!(kinds(&preferences(L, &PreferencesState::Readable, path)), [("preferences", Severity::Ok, "healthy")]);
        let bad = preferences(L, &PreferencesState::Invalid("preferences.json has version 2; this clawg reads version 1".into()), path);
        assert_eq!(kinds(&bad), [("preferences", Severity::Warn, "invalid")]);
        assert!(bad[0].message.contains("version 2") && bad[0].message.contains("without a language preference"), "{}", bad[0].message);
        assert_eq!(bad[0].safe_next_step, None, "the file is the user's to fix");
        assert_eq!(bad[0].evidence["path"], "/d/preferences.json");
    }

    #[test]
    fn protocol_block_findings() {
        let path = Path::new("/home/u/.claude/CLAUDE.md");
        let wanted = "<!-- BEGIN -->\nrules\n<!-- END -->";
        let ok = protocol_block(L, &BlockState::Installed(wanted.replace('\n', "\r\n")), wanted, path, P);
        assert_eq!(kinds(&ok), [("protocol_block", Severity::Ok, "healthy")]);
        let stale = protocol_block(L, &BlockState::Installed("<!-- BEGIN -->\nold\n<!-- END -->".into()), wanted, path, P);
        assert_eq!(kinds(&stale), [("protocol_block", Severity::Warn, "stale")]);
        assert_eq!(stale[0].safe_next_step.as_deref(), Some("clawg setup claude-code --profile dev"));
        let missing = protocol_block(L, &BlockState::Missing, wanted, path, P);
        assert_eq!(kinds(&missing), [("protocol_block", Severity::Info, "not_installed")]);
        assert_eq!(missing[0].safe_next_step.as_deref(), Some("clawg setup claude-code --profile dev"));
        let unreadable = protocol_block(L, &BlockState::Unreadable("a lone marker".into()), wanted, path, P);
        assert_eq!(kinds(&unreadable), [("protocol_block", Severity::Warn, "unreadable")]);
        assert_eq!(unreadable[0].evidence["path"], "/home/u/.claude/CLAUDE.md");
    }

    #[test]
    fn outbox_findings() {
        let facts = OutboxFacts {
            pending: 2,
            failed: vec![FailedSave { operation_id: "op 1".into(), agent_id: "claude-code".into(), attempts: 3 }],
        };
        let found = outbox(L, &facts, false, P);
        assert_eq!(kinds(&found), [("outbox", Severity::Warn, "failed"), ("outbox", Severity::Info, "pending")]);
        assert_eq!(
            found[0].safe_next_step.as_deref(),
            Some("clawg outbox retry 'op 1' --agent claude-code --profile dev")
        );
        assert!(found[0].requires_confirmation);
        assert_eq!(kinds(&outbox(L, &OutboxFacts { pending: 2, failed: vec![] }, true, P)), [("outbox", Severity::Ok, "healthy")]);
        assert_eq!(kinds(&outbox(L, &OutboxFacts::default(), false, P)), [("outbox", Severity::Ok, "healthy")]);
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
        let clean = agents(L, &rows(0));
        assert_eq!(kinds(&clean), [("agents", Severity::Ok, "healthy")]);
        assert_eq!(clean[0].evidence["agents"][1], json!({ "agent_id": "claude-code", "live": 5, "total": 7 }));
        let split = agents(L, &rows(2));
        assert_eq!(kinds(&split), [("agents", Severity::Ok, "healthy"), ("agents", Severity::Warn, "fallback_agent")]);
        assert_eq!(split[1].evidence["live"], 2);
    }

    #[test]
    fn project_findings() {
        let root = if cfg!(windows) { r"C:\work\proj" } else { "/work/proj" };
        let good = ProjectRow { id: derive_project_id(Path::new(root)), root_path: root.into() };
        assert_eq!(kinds(&projects(L, &[good.clone()])), [("projects", Severity::Ok, "healthy")]);
        let bad = ProjectRow { id: "proj-0000".into(), root_path: root.into() };
        let found = projects(L, &[good, bad]);
        assert_eq!(kinds(&found), [("projects", Severity::Warn, "id_mismatch")]);
        assert_eq!(found[0].evidence["project_id"], "proj-0000");
    }

    #[test]
    fn a_project_id_assigned_to_its_root_is_ok() {
        let root = if cfg!(windows) { r"C:\work\proj" } else { "/work/proj" };
        let assigned = ProjectRow { id: candidate_id(Path::new(root), 3), root_path: root.into() };
        assert_eq!(kinds(&projects(L, &[assigned])), [("projects", Severity::Ok, "healthy")]);
    }

    #[test]
    fn project_remote_findings() {
        let remote = |remote: &str, project_id: &str| RemoteRow { remote: remote.into(), project_id: project_id.into() };
        let root = |id: &str, root_path: &str, origin: Option<&str>| {
            (ProjectRow { id: id.into(), root_path: root_path.into() }, origin.map(str::to_string))
        };
        let healthy = [remote("github.com/acme/a", "app-1a2b"), remote("github.com/acme/w", "w-3c4d")];
        let roots = [root("app-1a2b", "/work/app", Some("github.com/acme/a")), root("w-3c4d", "/else/w", None)];
        let ok = project_remotes(L, &healthy, &roots);
        assert_eq!(kinds(&ok), [("project_remotes", Severity::Ok, "healthy")]);
        assert_eq!(ok[0].evidence["remotes"], 2);

        let fused = [remote("github.com/acme/b", "app-1a2b"), remote("github.com/acme/a", "app-1a2b"), remote("github.com/acme/w", "w-3c4d")];
        let roots = [
            root("app-1a2b", "/work/app", Some("github.com/acme/b")),
            root("app-5e6f", "/review/a", Some("github.com/acme/a")),
            root("w-3c4d", "/else/w", Some("github.com/acme/w")),
        ];
        let found = project_remotes(L, &fused, &roots);
        assert_eq!(kinds(&found), [("project_remotes", Severity::Warn, "several_remotes")]);
        assert_eq!(
            found[0].evidence,
            json!({
                "project_id": "app-1a2b",
                "remotes": ["github.com/acme/a", "github.com/acme/b"],
                "roots": [
                    { "root_id": "app-1a2b", "root_path": "/work/app", "origin": "github.com/acme/b" },
                    { "root_id": "app-5e6f", "root_path": "/review/a", "origin": "github.com/acme/a" },
                ],
            })
        );
        assert_eq!(found[0].safe_next_step, None);
    }

    #[test]
    fn invariant_findings() {
        let healthy = invariants(L, &Invariants::default());
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
        let found = invariants(L, &broken);
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

    fn registered(rows: &[(&str, &str, i64, Option<&str>)]) -> Vec<RegistryRow> {
        rows.iter()
            .map(|(id, name, created_at, merged_into)| RegistryRow {
                id: id.to_string(),
                name: name.to_string(),
                created_at: *created_at,
                merged_into: merged_into.map(str::to_string),
            })
            .collect()
    }

    fn print(root_id: &str, project_id: &str, remote: Option<&str>, root_commit: &str) -> FolderPrint {
        FolderPrint {
            root_id: root_id.into(),
            project_id: Some(project_id.into()),
            remote: remote.map(str::to_string),
            root_commit: Some(root_commit.into()),
        }
    }

    fn id_rows(project_id: &str, memories: i64, trajectories: i64) -> IdRows {
        IdRows { project_id: project_id.into(), memories, trajectories, skills: 0 }
    }

    /// Two that share a name, two whose folders share a first commit, two
    /// forks and a tombstone of one of them.
    fn lookalikes() -> (Vec<RegistryRow>, Vec<FolderPrint>) {
        let registry = registered(&[
            ("api-1a2b", "App", 1, None),
            ("app-3c4d", "app", 5, None),
            ("lib-5e6f", "lib", 2, None),
            ("lib2-7a8b", "lib2", 6, None),
            ("forka-0001", "forka", 3, None),
            ("forkb-0002", "forkb", 4, None),
            ("old-0003", "old", 0, Some("lib-5e6f")),
        ]);
        let prints = vec![
            print("lib-5e6f", "lib-5e6f", Some("github.com/acme/lib"), "c1"),
            print("lib2-7a8b", "lib2-7a8b", None, "c1"),
            print("old-0003", "old-0003", None, "c1"),
            print("forka-0001", "forka-0001", Some("github.com/acme/fork"), "c2"),
            print("forkb-0002", "forkb-0002", Some("github.com/other/fork"), "c2"),
        ];
        (registry, prints)
    }

    #[test]
    fn the_duplicates_are_named_by_name_and_by_first_commit() {
        let (registry, prints) = lookalikes();

        let found = project_duplicates(L, &registry, &prints, &[], P);

        let named: Vec<(&str, &serde_json::Value, Option<&str>)> =
            found.iter().map(|f| (f.reason_code, &f.evidence["project_ids"], f.safe_next_step.as_deref())).collect();
        assert_eq!(
            named,
            vec![
                (
                    "same_name",
                    &json!(["api-1a2b", "app-3c4d"]),
                    Some("clawg projects merge app-3c4d --into api-1a2b --dry-run --profile dev")
                ),
                (
                    "same_first_commit",
                    &json!(["lib-5e6f", "lib2-7a8b"]),
                    Some("clawg projects merge lib2-7a8b --into lib-5e6f --dry-run --profile dev")
                ),
            ],
            "a fork, with a remote of its own, is not one; a tombstone is its target: {found:#?}"
        );
        assert!(found.iter().all(|f| f.severity == Severity::Warn && !f.requires_confirmation));
        assert_eq!(found[1].evidence["root_commit"], "c1");
        assert_eq!(project_duplicates(L, &registered(&[("a-0001", "a", 1, None)]), &[], &[], P)[0].reason_code, "healthy");
    }

    #[test]
    fn a_pair_kept_apart_is_no_duplicate() {
        let (registry, prints) = lookalikes();
        let answered = [("lib-5e6f".to_string(), "lib2-7a8b".to_string())];

        let found = project_duplicates(L, &registry, &prints, &answered, P);

        let reasons: Vec<&str> = found.iter().map(|f| f.reason_code).collect();
        assert_eq!(reasons, vec!["same_name"]);
    }

    #[test]
    fn an_id_without_a_project_is_named() {
        let registry = registered(&[("web-3c4d", "web", 1, None)]);

        let found = project_orphans(L, &registry, &[id_rows("orphan-9f9f", 0, 1), id_rows("web-3c4d", 2, 0)]);

        assert_eq!(found.len(), 1);
        assert_eq!((found[0].reason_code, found[0].safe_next_step.as_deref()), ("no_project", None));
        assert_eq!(found[0].evidence, json!({ "project_id": "orphan-9f9f", "memories": 0, "trajectories": 1, "skills": 0 }));
        assert_eq!(project_orphans(L, &registry, &[id_rows("web-3c4d", 2, 0)])[0].reason_code, "healthy");
    }

    #[test]
    fn rows_under_a_merged_project_and_a_broken_chain_are_named() {
        let registry = registered(&[
            ("web-3c4d", "web", 1, None),
            ("app-1a2b", "app", 2, Some("web-3c4d")),
            ("loop-0001", "loop", 3, Some("loop-0002")),
            ("loop-0002", "loop", 4, Some("loop-0001")),
            ("lost-0003", "lost", 5, Some("gone-0004")),
        ]);

        let found = project_merges(L, &registry, &[id_rows("app-1a2b", 1, 0), id_rows("web-3c4d", 3, 0)], P);

        let named: Vec<(&str, &serde_json::Value)> = found.iter().map(|f| (f.reason_code, &f.evidence["project_id"])).collect();
        assert_eq!(
            named,
            vec![
                ("rows_under_merged", &json!("app-1a2b")),
                ("broken_merge", &json!("loop-0001")),
                ("broken_merge", &json!("loop-0002")),
                ("broken_merge", &json!("lost-0003")),
            ]
        );
        assert_eq!(found[0].evidence["merged_into"], "web-3c4d");
        assert_eq!(found[0].safe_next_step.as_deref(), Some("clawg projects merge app-1a2b --into web-3c4d --dry-run --profile dev"));
        assert_eq!(found[3].safe_next_step, None);
        assert_eq!(project_merges(L, &registry[..2], &[id_rows("web-3c4d", 3, 0)], P)[0].reason_code, "healthy");
    }

    /// Every finding a check can give, for the facts that give it.
    fn every_finding(lang: Lang) -> Vec<Finding> {
        let analysis = |shadowed, duplicates| Analysis {
            installs: vec![super::super::Install { path: "/old/clawg".into(), is_current: false }],
            winner: Some("/old/clawg".into()),
            shadowed,
            duplicates,
        };
        let dir = Path::new("/home/u/.clawg-dev");
        let sock = Path::new("/tmp/clawg.sock");
        let probe = |state| StoreProbe { state, db_version: Some(10), binary_version: 17 };
        let expected = clawg_core::memory::embedder::expected_contract();
        let stored = |model_id: &str| StoredContract {
            model_id: model_id.into(),
            dim: expected.dim as i64,
            query_prefix: expected.query_prefix.clone(),
            passage_prefix: expected.passage_prefix.clone(),
        };
        let now = 1_791_827_476_123;
        let snapshot = |days: i64| clawg_core::memory::backup::snapshot_name(now - days * DAY_MS, clawg_core::memory::backup::Reason::Manual);
        let path = Path::new("/d/preferences.json");
        let owner = StoreOwner { holder: StoreHolder::Daemon, pid: 42, started_at_ms: 0 };
        let failed = OutboxFacts {
            pending: 2,
            failed: vec![FailedSave { operation_id: "op".into(), agent_id: "a".into(), attempts: 3 }],
        };
        let agent_rows = [AgentRows { agent_id: Some(FALLBACK_AGENT_ID.into()), live: 2, total: 2 }];
        let root = if cfg!(windows) { r"C:\work\proj" } else { "/work/proj" };
        let broken = Invariants {
            topic_versions: vec![TopicVersions { agents: vec![None], project_id: None, saves: vec![1, 4] }],
            dangling_superseded: vec![
                DanglingSuperseded { id: 2, superseded_by: Some(9), cause: Some("revision".into()) },
                DanglingSuperseded { id: 3, superseded_by: Some(9), cause: Some("verdict".into()) },
            ],
            dangling_revived: vec![DanglingRevived { id: 6, revived_from: 8 }],
            hash_duplicates: vec![HashDuplicates { agents: vec![None], project_id: None, ids: vec![10, 11] }],
            purged_headers: vec![PurgedHeader { save_id: 3, first_piece: 40 }],
            lonely_disputes: vec![12],
        };
        let mut all = Vec::new();
        all.extend(path_check(lang, &analysis(true, true)));
        all.extend(path_check(lang, &analysis(false, false)));
        all.extend(data_dir(lang, dir, DirState::Missing));
        all.extend(data_dir(lang, dir, DirState::Present { mode: Some(0o40755) }));
        all.extend(data_dir(lang, dir, DirState::Present { mode: Some(0o40700) }));
        all.extend(socket(lang, sock, &Ok(())));
        all.extend(socket(lang, sock, &Err(SocketError::TooLong { path: sock.into(), len: 120, limit: 108 })));
        all.extend(store_owner(lang, None));
        all.extend(store_owner(lang, Some(&owner)));
        for state in [
            ProbeState::Missing,
            ProbeState::Unreadable("not a database".into()),
            ProbeState::Schema(SchemaPlan::Fresh),
            ProbeState::Schema(SchemaPlan::UpToDate),
            ProbeState::Schema(SchemaPlan::Pending { from: 10, to: 17 }),
            ProbeState::Schema(SchemaPlan::Newer { db: 20, binary: 17 }),
            ProbeState::Schema(SchemaPlan::Diverged { version: 9 }),
        ] {
            all.extend(schema(lang, &probe(state), P));
        }
        all.extend(contract(lang, None, &expected));
        all.extend(contract(lang, Some(&stored(&expected.model_id)), &expected));
        all.extend(contract(lang, Some(&stored(clawg_core::memory::embedder::LEGACY_CLS_MODEL_ID)), &expected));
        all.extend(contract(lang, Some(&stored("other-model")), &expected));
        all.extend(backups(lang, &[], false, now, P));
        all.extend(backups(lang, &[], true, now, P));
        all.extend(backups(lang, &[snapshot(1)], true, now, P));
        all.extend(backups(lang, &[snapshot(90)], true, now, P));
        all.extend(spill(lang, None, None));
        all.extend(spill(lang, Some(Path::new("/d/spill")), Some((Path::new("/d/rejected"), 1))));
        all.extend(spill(lang, None, Some((Path::new("/d/rejected"), 3))));
        all.extend(sharing(lang, None, P));
        all.extend(sharing(lang, Some("sharing.json is not valid"), P));
        all.extend(preferences(lang, &PreferencesState::Readable, path));
        all.extend(preferences(lang, &PreferencesState::Invalid("preferences.json is not valid".into()), path));
        let wanted = "<!-- BEGIN -->\nrules\n<!-- END -->";
        for state in [
            BlockState::Installed(wanted.into()),
            BlockState::Installed("old".into()),
            BlockState::Missing,
            BlockState::Unreadable("a lone marker".into()),
        ] {
            all.extend(protocol_block(lang, &state, wanted, path, P));
        }
        all.extend(outbox(lang, &failed, false, P));
        all.extend(outbox(lang, &OutboxFacts { pending: 1, failed: vec![] }, false, P));
        all.extend(outbox(lang, &OutboxFacts::default(), false, P));
        all.extend(agents(lang, &agent_rows));
        all.extend(agents(lang, &[]));
        all.extend(projects(lang, &[ProjectRow { id: "proj-0000".into(), root_path: root.into() }]));
        all.extend(projects(lang, &[]));
        let fused = [
            RemoteRow { remote: "github.com/acme/a".into(), project_id: "proj-0000".into() },
            RemoteRow { remote: "github.com/acme/b".into(), project_id: "proj-0000".into() },
        ];
        all.extend(project_remotes(lang, &fused, &[(ProjectRow { id: "proj-0000".into(), root_path: root.into() }, None)]));
        all.extend(project_remotes(lang, &[], &[]));
        all.extend(invariants(lang, &broken));
        all.extend(invariants(lang, &Invariants::default()));
        let (lookalike, prints) = lookalikes();
        all.extend(project_duplicates(lang, &lookalike, &prints, &[], P));
        all.extend(project_duplicates(lang, &[], &[], &[], P));
        let registry = registered(&[("web-3c4d", "web", 1, None), ("app-1a2b", "app", 2, Some("web-3c4d")), ("lost-0003", "lost", 5, Some("gone-0004"))]);
        all.extend(project_orphans(lang, &registry, &[id_rows("orphan-9f9f", 1, 0), id_rows("other-9f9e", 2, 0)]));
        all.extend(project_orphans(lang, &registry, &[]));
        all.extend(project_merges(lang, &registry, &[id_rows("app-1a2b", 1, 0), id_rows("app-1a2b", 2, 0)], P));
        all.extend(project_merges(lang, &registry[..1], &[], P));
        all
    }

    fn path_check(lang: Lang, analysis: &Analysis) -> Vec<Finding> {
        path(lang, analysis)
    }

    #[test]
    fn the_findings_are_the_same_in_both_languages_but_their_message() {
        let (en, es) = (every_finding(Lang::En), every_finding(Lang::Es));
        assert_eq!(en.len(), es.len());
        for (en, es) in en.iter().zip(&es) {
            assert_ne!(en.message, es.message, "{} {} has one message in both languages", en.check_id, en.reason_code);
            assert_eq!(Finding { message: String::new(), ..en.clone() }, Finding { message: String::new(), ..es.clone() });
        }
    }

    #[test]
    fn a_finding_in_spanish_agrees_with_its_number() {
        let es = every_finding(Lang::Es);
        let messages: Vec<&str> = es.iter().map(|f| f.message.as_str()).collect();
        for expected in [
            "el snapshot más nuevo tiene 1 día",
            "el snapshot más nuevo tiene 90 días",
            "1 línea de saves que un clawg anterior no pudo leer se guarda aparte",
            "3 líneas de saves que un clawg anterior no pudo leer se guardan aparte",
            "1 save aceptado espera en el outbox",
            "2 saves aceptados esperan en el outbox",
            "1 agente tiene filas",
            "0 agentes tienen filas",
            "1 fila está guardada bajo orphan-9f9f",
            "2 filas están guardadas bajo other-9f9e",
            "1 fila sigue bajo app-1a2b",
            "2 filas siguen bajo app-1a2b",
        ] {
            assert!(messages.iter().any(|m| m.starts_with(expected)), "no message starts with {expected:?}: {messages:#?}");
        }
    }
}
