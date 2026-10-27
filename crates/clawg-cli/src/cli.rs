//! Argument parsing for the `clawg` binary.
//!
//! Parsing is deliberately separated from execution: every type here is inert
//! data, so the whole surface is testable by feeding argv vectors to clap
//! without touching a filesystem, a port or a database.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use clawg_core::memory::sharing;
use clawg_core::paths::{Profile, ProfileError};
use clawg_core::preferences::Lang;

use crate::client::{DeleteRequest, MergeRequest, MoveRequest};

/// `clawg` — local-first memory for coding agents.
#[derive(Debug, Parser)]
#[command(name = "clawg", version = clawg_core::build_info::version(), about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// Trims an `--agent` value, refusing one with nothing left: a blank id
/// would be an agent no client could ever name again.
fn parse_agent(raw: &str) -> Result<String, String> {
    let agent = raw.trim();
    if agent.is_empty() {
        return Err("the agent id must not be blank".into());
    }
    Ok(agent.to_string())
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Command {
    /// Run the memory daemon (owns the database, the embedder and the workers).
    Serve {
        #[command(flatten)]
        profile: ProfileArg,
        /// TCP port on 127.0.0.1. `0` binds an ephemeral port, which is what
        /// tests and side-by-side dev runs want; omitted uses the profile's
        /// default port.
        #[arg(long)]
        port: Option<u16>,
    },
    /// Bridge stdio MCP to a running daemon, starting one if needed.
    Mcp {
        #[command(flatten)]
        profile: ProfileArg,
        /// Fail instead of starting a daemon when none is running.
        #[arg(long)]
        no_autostart: bool,
        /// The agent id this bridge reads and writes memories as. Without it,
        /// the name the MCP client reports is used. Pin it to keep what was
        /// saved reachable if the client stops reporting a name or changes it.
        #[arg(long, value_parser = parse_agent)]
        agent: Option<String>,
    },
    /// Print the daemon's state file and health as JSON. Exits 1 if not running.
    Status {
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Stop the running daemon and wait until its process has exited.
    Stop {
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Write a snapshot of the memory store to `<data>/backups` and print its
    /// path. Goes through the daemon when one is running.
    Backup {
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Replace the memory store with a snapshot. The daemon must be stopped;
    /// the store being replaced is snapshotted first.
    Restore {
        /// The snapshot to restore, usually one in `<data>/backups`.
        file: PathBuf,
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Write the stored memories, one row per line with their states and
    /// links, to a file another profile or machine can import, or as Markdown
    /// to read. Read-only; works with the daemon running.
    Export {
        /// The file to write. It must not exist yet; on Unix only its owner
        /// can read it, since it holds the text of every memory it takes.
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        /// Only the memories of the project this directory is in.
        #[arg(long, value_name = "PATH")]
        project: Option<PathBuf>,
        /// Only this agent's memories.
        #[arg(long, value_parser = parse_agent)]
        agent: Option<String>,
        /// Take the deleted memories too; they are imported deleted.
        #[arg(long)]
        include_deleted: bool,
        /// `jsonl` for `clawg import`, `markdown` to read.
        #[arg(long, value_enum, default_value_t = ExportFormat::Jsonl)]
        format: ExportFormat,
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Import a file `clawg export` wrote, keeping each memory's dates,
    /// accesses, state and links, or the export of another memory tool, each
    /// note split and masked as a save of its text. Needs a running daemon,
    /// which embeds each memory again; the store is snapshotted first.
    /// Importing a file twice adds nothing. `--undo N` takes import N back,
    /// keeping what was saved, edited or judged since.
    Import {
        /// The file `clawg export` wrote, or the other tool's export (for
        /// engram, `engram export --all`).
        #[arg(required_unless_present = "undo")]
        file: Option<PathBuf>,
        /// Take back import N, the number its report gave: remove for good
        /// the memories it inserted that are still as it left them, and give
        /// back what it changed in the others. The store is snapshotted
        /// first.
        #[arg(long, value_name = "N", conflicts_with_all = ["file", "from", "agent", "map_root", "project_map"])]
        undo: Option<i64>,
        /// The tool the file comes from.
        #[arg(long, value_enum, default_value_t = ImportSource::Clawg)]
        from: ImportSource,
        /// Report what would be imported, or undone, and write nothing.
        #[arg(long)]
        dry_run: bool,
        /// Store every memory that has an agent under this one instead.
        #[arg(long, value_parser = parse_agent)]
        agent: Option<String>,
        /// Store the memories of the exported project root OLD in the project
        /// the directory NEW is in. Repeat it for each root that moved. Only
        /// for a clawg export.
        #[arg(long = "map-root", value_name = "OLD=NEW", value_parser = parse_root_map)]
        map_root: Vec<RootMap>,
        /// Store the notes of the other tool's project NAME in the project
        /// the directory PATH is in. Repeat it for each project; the notes of
        /// a project not named are stored in none, and `clawg projects move
        /// --origin-project NAME` places them later.
        #[arg(long = "project-map", value_name = "NAME=PATH", value_parser = parse_project_map)]
        project_map: Vec<ProjectMap>,
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// List the stored memories and verdict reasons that hold a secret of a
    /// known format, by id and kind, never the secret. Read-only; works with
    /// the daemon running.
    Secrets {
        /// Replace the secrets of every stored verdict reason with their
        /// placeholders instead. The daemon must be stopped.
        #[arg(long)]
        mask_reasons: bool,
        /// Confirm that the reasons are to be rewritten for good.
        #[arg(long, requires = "mask_reasons")]
        yes: bool,
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Remove one stored memory for good: its text, its vector and its index
    /// terms, or, with `--project-delete N`, every row that delete N left
    /// deleted. There is no way back, so it needs `--yes`. The daemon must be
    /// stopped.
    Purge {
        /// The memory's id, as `clawg secrets` or `clawg memories` show it.
        #[arg(required_unless_present = "project_delete", conflicts_with = "project_delete")]
        id: Option<i64>,
        /// Remove for good every row of project delete N that is still
        /// deleted, as `clawg projects delete` and `clawg projects list` show
        /// its number, instead of one memory.
        #[arg(long, value_name = "N")]
        project_delete: Option<i64>,
        /// Confirm that it is to be removed for good.
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Bring back a memory that was deleted, with its id as `clawg memories
    /// list --deleted` shows it. Needs a running daemon, which embeds it again.
    Undelete {
        /// The memory's id.
        id: i64,
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Retry or discard a save the daemon gave up on, by the operation id
    /// `clawg status` names under `ingest.problems`. Needs a running daemon.
    Outbox {
        #[command(subcommand)]
        action: OutboxAction,
    },
    /// Show which agents do not read which agents' memories, or change it with
    /// `hide` and `unhide`. Every agent reads every other agent until hidden;
    /// a change counts from the next search, with the daemon running. A shared
    /// memory reaches the other agent's provider: `clawg secrets` lists the
    /// rows that hold a secret.
    Sharing {
        #[command(subcommand)]
        action: Option<SharingAction>,
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Show the user's preferences, or change one with `set` and `unset`. The
    /// language (`es` or `en`) is the one memories are saved and searched in
    /// and the agent answers in. A change reaches the Claude Code sessions
    /// opened after it; `clawg setup claude-code` rewrites the protocol block.
    Preferences {
        #[command(subcommand)]
        action: Option<PreferencesAction>,
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Report which `clawg` executables the PATH resolves to and whether this
    /// one is the one a terminal runs. Exits 1 if shadowed or duplicated.
    Doctor {
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Replace this binary with the latest release. With `--check`, only say
    /// whether there is one: exit 0 when up to date, 100 when a newer release
    /// exists, 1 when it cannot tell.
    Update {
        /// Report instead of installing; nothing is downloaded or changed.
        #[arg(long)]
        check: bool,
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// List and name the projects, let the daemon read a project's files
    /// besides the one each bridge is started in, and move memories between
    /// them. Needs a running daemon.
    Projects {
        #[command(subcommand)]
        action: ProjectsAction,
    },
    /// Read what the daemon measured of itself, kept in `<data>/metrics.db`.
    /// Read-only; works with the daemon running or stopped.
    Metrics {
        #[command(subcommand)]
        action: MetricsAction,
    },
    /// Browse what the daemon has stored. Read-only; needs a running daemon.
    Memories {
        #[command(subcommand)]
        action: MemoriesAction,
    },
    /// Answer one of the agent client's lifecycle hooks: read its JSON on
    /// stdin, print what it should add to the agent's context. Never starts a
    /// daemon, and prints nothing when there is none.
    Hook {
        event: HookEvent,
        /// The agent id the client's bridge uses (its `--agent`): the hook
        /// gets no client name, and another id would read another agent.
        #[arg(long, value_parser = parse_agent)]
        agent: String,
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Connect clawg to an agent client: register the MCP server with this
    /// binary's absolute path and a pinned agent id, install the lifecycle
    /// hooks and the memory protocol. Safe to run again; what differs from
    /// the plan is reported and left as it is.
    Setup {
        client: SetupClient,
        /// Print what would change, and change nothing.
        #[arg(long)]
        dry_run: bool,
        /// Take away what setup installed. clawg's data stays.
        #[arg(long)]
        remove: bool,
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Add or remove a directory on the user PATH (Windows). The installer
    /// runs this so `clawg` works from any new terminal.
    Path {
        #[command(subcommand)]
        action: PathAction,
    },
}

/// What `clawg export` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ExportFormat {
    /// JSON Lines, one row per line, that `clawg import` reads.
    Jsonl,
    /// A view to read, by project and type.
    Markdown,
}

/// An exported project root and the directory whose project takes its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootMap {
    pub old: String,
    pub new: PathBuf,
}

/// `OLD=NEW`, split at the first `=`. OLD is kept as written: it names a root
/// on the machine that exported, which need not exist here.
fn parse_root_map(raw: &str) -> Result<RootMap, String> {
    match raw.split_once('=') {
        Some((old, new)) if !old.is_empty() && !new.is_empty() => {
            Ok(RootMap { old: old.to_string(), new: PathBuf::from(new) })
        }
        _ => Err(format!("{raw:?} is not OLD=NEW")),
    }
}

/// The tool a file `clawg import` reads comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ImportSource {
    /// A file `clawg export` wrote.
    Clawg,
    /// The JSON `engram export` writes.
    Engram,
}

impl ImportSource {
    /// As the daemon names it.
    pub fn as_str(self) -> &'static str {
        match self {
            ImportSource::Clawg => "clawg",
            ImportSource::Engram => "engram",
        }
    }
}

/// A project name of another tool and the directory whose project takes its
/// notes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectMap {
    pub name: String,
    pub root: PathBuf,
}

/// `NAME=PATH`, split at the last `=`: a project name may hold one, a path
/// here does not.
fn parse_project_map(raw: &str) -> Result<ProjectMap, String> {
    match raw.rsplit_once('=') {
        Some((name, root)) if !name.is_empty() && !root.is_empty() => {
            Ok(ProjectMap { name: name.to_string(), root: PathBuf::from(root) })
        }
        _ => Err(format!("{raw:?} is not NAME=PATH")),
    }
}

impl Cli {
    /// Parses `args` as clap does, then refuses what no single argument
    /// tells clap: a map of the other kind of export than `--from` names.
    pub fn parse_args<I, T>(args: I) -> Result<Cli, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let cli = Cli::try_parse_from(args)?;
        if let Command::Import { from, map_root, project_map, .. } = &cli.command {
            let refused = match from {
                ImportSource::Clawg if !project_map.is_empty() => {
                    Some("--project-map maps the projects of another tool's export; name the tool with --from")
                }
                ImportSource::Engram if !map_root.is_empty() => {
                    Some("--map-root maps the roots of a clawg export; another tool's projects take --project-map")
                }
                _ => None,
            };
            if let Some(message) = refused {
                return Err(<Cli as clap::CommandFactory>::command().error(clap::error::ErrorKind::ArgumentConflict, message));
            }
        }
        Ok(cli)
    }
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum ProjectsAction {
    /// List every project, with or without a folder: its folders, its
    /// remotes, how many memories it holds and the suggestions nobody
    /// answered, marking what was created on its own and not reviewed.
    List {
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Give a project a name and mark it reviewed. Its id, and with it what
    /// was saved under it, stays.
    Rename {
        /// The project's id, as `clawg projects list` shows it.
        id: String,
        /// Its new name, at most 80 characters.
        name: String,
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Create a project without a folder, by name, through the running
    /// daemon. A name a live project has leads to that project; one close to
    /// other projects' names is refused with them, unless `--new`.
    Create {
        /// Its name, at most 80 characters.
        name: String,
        /// Create it even when other projects have a close name.
        #[arg(long)]
        new: bool,
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Make the folder PATH is in lead to PROJECT, by id or name, through the
    /// running daemon: the project the folder had, when this is its only
    /// folder, is merged into PROJECT, and `clawg projects merge --undo N`
    /// takes that back.
    Locate {
        /// The project, by id or by name.
        project: String,
        /// A directory of the folder; its root is found as for a bridge.
        path: PathBuf,
        /// PATH itself, a subfolder of a repository, leads to PROJECT: what is
        /// worked on inside it, in every clone, goes there. Naming the
        /// repository's own project takes the declaration away.
        #[arg(long)]
        subfolder: bool,
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Index the docs and code of the project PATH is in, so `search_project`
    /// can search it from a bridge started anywhere.
    Add {
        /// A directory of the project; its root is found as for a bridge.
        path: PathBuf,
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Move memories to another project, or out of every project, through the
    /// running daemon. Each one moves with its whole save and topic history; a
    /// memory whose text or topic key is live in the target stays where it is.
    Move(MoveOpts),
    /// Merge project SOURCE into TARGET through the running daemon: its
    /// memories, folders, remotes and reviews go there, and SOURCE leaves the
    /// listing while its id still leads to TARGET. `--undo N` takes merge N
    /// back. A merge waits for the saves of both projects still being stored.
    Merge(MergeOpts),
    /// Delete a project through the running daemon. An empty project is
    /// deleted at once. One with live rows is refused, with the ways to empty
    /// it; `--with-memories` reports what deleting it with them takes and a
    /// code, and `--with-memories --confirm CODE` deletes it. `--undo N`
    /// takes delete N back.
    Delete(DeleteOpts),
    /// Answer that two projects are not one: no suggestion between them is
    /// made again, and both are marked reviewed.
    KeepApart {
        /// One project's id, as `clawg projects list` shows it.
        project_id: String,
        /// The other's.
        other_id: String,
        #[command(flatten)]
        profile: ProfileArg,
    },
}

/// What `clawg projects merge` merges, or the merge it undoes: a source with
/// `--into`, or `--undo`.
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
#[command(group(clap::ArgGroup::new("merge").args(["source", "undo"]).required(true)))]
pub struct MergeOpts {
    /// The project to merge, by id, as `clawg projects list` shows it.
    #[arg(requires = "into")]
    pub source: Option<String>,
    /// The project it goes into, by id.
    #[arg(long, value_name = "TARGET", requires = "source")]
    pub into: Option<String>,
    /// Take merge N back instead, by the number the merge printed.
    #[arg(long, value_name = "N", conflicts_with_all = ["source", "into"])]
    pub undo: Option<i64>,
    /// Say what it would do, and change nothing.
    #[arg(long)]
    pub dry_run: bool,
    #[command(flatten)]
    pub profile: ProfileArg,
}

impl MergeOpts {
    pub fn request(&self) -> MergeRequest {
        MergeRequest { source: self.source.clone(), target: self.into.clone(), undo: self.undo, dry_run: self.dry_run }
    }
}

/// What `clawg projects delete` deletes, or the delete it undoes: a project,
/// or `--undo`.
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
#[command(group(clap::ArgGroup::new("delete").args(["project_id", "undo"]).required(true)))]
pub struct DeleteOpts {
    /// The project to delete, by id, as `clawg projects list` shows it.
    pub project_id: Option<String>,
    /// Delete it with its memories: without `--confirm`, report what that
    /// takes and the code to confirm it with.
    #[arg(long, requires = "project_id")]
    pub with_memories: bool,
    /// The code of the report, to confirm the delete with its memories.
    #[arg(long, value_name = "CODE", requires = "with_memories")]
    pub confirm: Option<String>,
    /// Take delete N back instead, by the number the delete printed.
    #[arg(long, value_name = "N", conflicts_with_all = ["project_id", "with_memories", "confirm"])]
    pub undo: Option<i64>,
    /// Say what the undo would do, and change nothing.
    #[arg(long, conflicts_with_all = ["project_id", "with_memories", "confirm"])]
    pub dry_run: bool,
    #[command(flatten)]
    pub profile: ProfileArg,
}

impl DeleteOpts {
    pub fn request(&self) -> DeleteRequest {
        DeleteRequest {
            project_id: self.project_id.clone(),
            with_memories: self.with_memories,
            confirm: self.confirm.clone(),
            undo: self.undo,
            dry_run: self.dry_run,
        }
    }
}

/// What `clawg projects move` moves, and where. Exactly one of `--to`,
/// `--to-project`, `--to-none` and `--undo`.
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
#[command(group(clap::ArgGroup::new("target").args(["to", "to_project", "to_none", "undo"]).required(true)))]
pub struct MoveOpts {
    /// A memory to move, by id; repeat it for several.
    #[arg(long = "id", value_name = "ID")]
    pub ids: Vec<i64>,
    /// Only memories of this project id.
    #[arg(long, value_name = "ID", conflicts_with_all = ["from_none", "here"])]
    pub from_project: Option<String>,
    /// Only memories saved in no project.
    #[arg(long, conflicts_with = "here")]
    pub from_none: bool,
    /// Only memories of the project of the current directory, as `clawg mcp`
    /// would scope them.
    #[arg(long)]
    pub here: bool,
    /// Only memories this agent saved.
    #[arg(long, value_name = "NAME")]
    pub agent: Option<String>,
    /// Only memories of this type.
    #[arg(long = "type", value_name = "T")]
    pub memory_type: Option<String>,
    /// Only memories of this source (`explicit`, `synthesis`, `imported`, ...).
    #[arg(long, value_name = "S")]
    pub source: Option<String>,
    /// Only memories saved in this host session.
    #[arg(long, value_name = "ID")]
    pub session: Option<String>,
    /// Only memories that came from this project of the tool they were
    /// imported from, by its exact name.
    #[arg(long, value_name = "NAME")]
    pub origin_project: Option<String>,
    /// Only memories whose content matches every word, as `memories search`
    /// reads them.
    #[arg(long, value_name = "WORDS")]
    pub text: Option<String>,
    /// Select deleted memories too.
    #[arg(long)]
    pub include_deleted: bool,
    /// Move to the project PATH is in, registered first as a bridge would.
    #[arg(long, value_name = "PATH")]
    pub to: Option<PathBuf>,
    /// Move to this registered project id, for a root no longer on disk.
    #[arg(long, value_name = "ID")]
    pub to_project: Option<String>,
    /// Move out of every project, where every project's searches reach them.
    #[arg(long)]
    pub to_none: bool,
    /// Move back what move N moved and is still where it left it.
    #[arg(
        long,
        value_name = "N",
        conflicts_with_all = [
            "ids", "from_project", "from_none", "here", "agent", "memory_type", "source", "session", "text",
            "origin_project", "include_deleted",
        ]
    )]
    pub undo: Option<i64>,
    /// Report what would move without moving anything.
    #[arg(long)]
    pub dry_run: bool,
    #[command(flatten)]
    pub profile: ProfileArg,
}

impl MoveOpts {
    /// The request to send, given the id of the project of the current
    /// directory (for `--here`) and of `--to`'s path, both resolved by the
    /// caller so this stays testable without a filesystem.
    pub fn request(&self, here: Option<String>, to: Option<String>) -> MoveRequest {
        MoveRequest {
            ids: self.ids.clone(),
            from_project: if self.here { here } else { self.from_project.clone() },
            from_none: self.from_none,
            agent: self.agent.clone(),
            memory_type: self.memory_type.clone(),
            source: self.source.clone(),
            session_id: self.session.clone(),
            q: self.text.clone(),
            origin_project: self.origin_project.clone(),
            include_deleted: self.include_deleted,
            to_project: to.or_else(|| self.to_project.clone()),
            to_none: self.to_none,
            undo: self.undo,
            dry_run: self.dry_run,
        }
    }
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum MetricsAction {
    /// Sum one metric by local day, by hour of the local day, by local minute
    /// or by version, and print its count, mean and percentiles (or, for a gauge, its
    /// samples, mean and extremes) as JSON.
    Report {
        /// The metric, as `http.duration` or `process.rss_bytes`.
        #[arg(long)]
        metric: String,
        /// Only the series with this label, as `route=/v1/search`. Repeat it
        /// to ask for several.
        #[arg(long = "label", value_name = "KEY=VALUE", value_parser = parse_label)]
        labels: Vec<(String, String)>,
        #[arg(long, value_enum, default_value_t = ReportBy::Version)]
        by: ReportBy,
        /// How far back, in days (`30d`) or hours (`12h`).
        #[arg(long, value_name = "AGE", value_parser = parse_since, default_value = "30d")]
        since: std::time::Duration,
        #[command(flatten)]
        profile: ProfileArg,
    },
}

/// What `clawg metrics report` groups by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ReportBy {
    /// The local date.
    Day,
    /// The hour of the local day, 00 to 23, across every day.
    Hour,
    /// The clawg version that recorded it.
    Version,
    /// The local date and minute, over the 14 days kept by minute.
    Minute,
}

/// `KEY=VALUE`, both sides trimmed and not empty.
fn parse_label(raw: &str) -> Result<(String, String), String> {
    let (key, value) = raw.split_once('=').ok_or_else(|| format!("{raw:?} is not KEY=VALUE"))?;
    let (key, value) = (key.trim(), value.trim());
    if key.is_empty() || value.is_empty() {
        return Err(format!("{raw:?} is not KEY=VALUE"));
    }
    Ok((key.to_string(), value.to_string()))
}

/// `30d` or `12h`: a whole number of days or hours.
fn parse_since(raw: &str) -> Result<std::time::Duration, String> {
    let raw = raw.trim();
    let invalid = || format!("{raw:?} is not an age such as 30d or 12h");
    let (number, unit) = raw.split_at(raw.len().saturating_sub(1));
    let n: u64 = number.parse().map_err(|_| invalid())?;
    let hours = match unit {
        "d" => n.checked_mul(24).ok_or_else(invalid)?,
        "h" => n,
        _ => return Err(invalid()),
    };
    Ok(std::time::Duration::from_secs(hours.checked_mul(3600).ok_or_else(invalid)?))
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum OutboxAction {
    /// Store it again, after the pieces it already stored.
    Retry(OutboxTarget),
    /// Drop it, deleting the memories it had stored; `clawg undelete` brings
    /// each back.
    Discard(OutboxTarget),
}

/// The failed save an `outbox` action acts on.
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct OutboxTarget {
    /// The save's operation id.
    pub operation_id: String,
    /// The agent that saved it, needed only when another agent used the same
    /// operation id.
    #[arg(long, value_parser = parse_agent)]
    pub agent: Option<String>,
    #[command(flatten)]
    pub profile: ProfileArg,
}

impl OutboxAction {
    pub fn target(&self) -> &OutboxTarget {
        match self {
            OutboxAction::Retry(t) | OutboxAction::Discard(t) => t,
        }
    }
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum SharingAction {
    /// Stop READER from reading the memories AUTHOR saves. Only that way
    /// round; `*` on either side stands for every agent.
    Hide(Exclusion),
    /// Take an exclusion away.
    Unhide(Exclusion),
}

impl Command {
    /// Whether this process will load the embedding model, and so has to set up
    /// the allocator for it before the runtime starts any thread.
    pub fn loads_the_model(&self) -> bool {
        matches!(self, Command::Serve { .. })
    }
}

impl SharingAction {
    /// The `--profile` the action was given, if any.
    pub fn profile(&self) -> &ProfileArg {
        match self {
            SharingAction::Hide(e) | SharingAction::Unhide(e) => &e.profile,
        }
    }
}

/// The `--profile` a `clawg sharing` run uses: the action's when it was given
/// after the subcommand, else the one given before it. Both spellings are
/// accepted, and an exclusion written to another profile's data directory
/// would leave the daemon sharing what the user believes is hidden.
pub fn sharing_profile<'a>(action: Option<&'a SharingAction>, own: &'a ProfileArg) -> &'a ProfileArg {
    action.map(SharingAction::profile).filter(|p| p.profile.is_some()).unwrap_or(own)
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum PreferencesAction {
    /// Set a preference: `language es` or `language en`.
    Set(SetPreference),
    /// Take a preference away, back to how clawg behaves without it.
    Unset(UnsetPreference),
}

/// What a preference is about. One for now; the next preference is another
/// value here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum PreferenceKey {
    Language,
}

#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct SetPreference {
    pub key: PreferenceKey,
    /// `es` or `en`.
    #[arg(value_parser = parse_lang)]
    pub value: Lang,
    #[command(flatten)]
    pub profile: ProfileArg,
}

#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct UnsetPreference {
    pub key: PreferenceKey,
    #[command(flatten)]
    pub profile: ProfileArg,
}

fn parse_lang(raw: &str) -> Result<Lang, String> {
    Lang::parse(raw).ok_or_else(|| format!("{raw:?} is not a language clawg knows: use es or en"))
}

impl PreferencesAction {
    /// The `--profile` the action was given, if any.
    pub fn profile(&self) -> &ProfileArg {
        match self {
            PreferencesAction::Set(s) => &s.profile,
            PreferencesAction::Unset(u) => &u.profile,
        }
    }
}

/// The `--profile` a `clawg preferences` run uses, by the rule of
/// [`sharing_profile`]: a preference written to another profile's data
/// directory would leave the user's sessions in the language they left.
pub fn preferences_profile<'a>(action: Option<&'a PreferencesAction>, own: &'a ProfileArg) -> &'a ProfileArg {
    action.map(PreferencesAction::profile).filter(|p| p.profile.is_some()).unwrap_or(own)
}

/// One directed exclusion: READER does not read what AUTHOR saved.
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct Exclusion {
    /// The agent that reads, as `memory_status` names it, or `*`.
    #[arg(value_parser = parse_agent)]
    pub reader: String,
    /// The agent whose memories it does not read, or `*`.
    #[arg(value_parser = parse_agent)]
    pub author: String,
    #[command(flatten)]
    pub profile: ProfileArg,
}

impl Exclusion {
    /// Refuses hiding an agent from itself: an agent always reads its own
    /// memories, and accepting it would tell the user something changed when
    /// nothing did. `* *` is not that: it means no agent reads another.
    pub fn check(&self, lang: Lang) -> Result<(), String> {
        if self.reader == self.author && self.reader != sharing::EVERY_AGENT {
            return Err(match lang {
                Lang::En => format!("{} always reads its own memories", self.reader),
                Lang::Es => format!("{} siempre lee sus propias memorias", self.reader),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum MemoriesAction {
    /// List stored rows, newest first.
    List(ListOpts),
    /// Find memories whose content matches the words of QUERY.
    Search(SearchOpts),
    /// Show one row in full, with its metadata.
    Show(ShowOpts),
    /// Count what is stored, grouped by type, tier, status, agent and project.
    Stats(StatsOpts),
}

impl MemoriesAction {
    /// The `--profile` the action was given, whichever action it is.
    pub fn profile(&self) -> &ProfileArg {
        match self {
            MemoriesAction::List(o) => &o.profile,
            MemoriesAction::Search(o) => &o.profile,
            MemoriesAction::Show(o) => &o.profile,
            MemoriesAction::Stats(o) => &o.profile,
        }
    }
}

/// The client lifecycle events `clawg hook` answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum HookEvent {
    SessionStart,
    UserPromptSubmit,
}

/// The agent clients `clawg setup` knows how to configure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum SetupClient {
    ClaudeCode,
}

/// Whether `args`, as the process got them, run `clawg hook`. A client reads
/// exit code 2 from a hook as "block": clap's usage error would erase the
/// user's prompt, so a hook's usage error has to exit otherwise.
pub fn is_hook_invocation<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    args.into_iter().nth(1).is_some_and(|a| a.as_ref() == "hook")
}

/// Whether `args` hold no argument after the program: what a client that does
/// not know a hook's `args` runs, which must not exit 2 either.
pub fn is_bare_invocation<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    args.into_iter().nth(1).is_none()
}

/// The three tables a row can live in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum KindArg {
    Memory,
    Episodic,
    Procedural,
}

impl KindArg {
    /// The spelling the daemon's API takes.
    pub fn as_str(self) -> &'static str {
        match self {
            KindArg::Memory => "memory",
            KindArg::Episodic => "episodic",
            KindArg::Procedural => "procedural",
        }
    }
}

/// The lifecycle states a memory can be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum StatusArg {
    Active,
    Disputed,
    Superseded,
}

impl StatusArg {
    /// The spelling the daemon's API takes.
    pub fn as_str(self) -> &'static str {
        match self {
            StatusArg::Active => "active",
            StatusArg::Disputed => "disputed",
            StatusArg::Superseded => "superseded",
        }
    }
}

/// Who or what the rows belong to.
#[derive(Debug, Clone, Default, PartialEq, Eq, clap::Args)]
pub struct Scope {
    /// Only rows of this agent.
    #[arg(long, value_name = "ID")]
    pub agent: Option<String>,
    /// Only rows of this project id.
    #[arg(long, value_name = "ID")]
    pub project: Option<String>,
    /// Only rows of the project of the current directory, as `clawg mcp` would
    /// scope them.
    #[arg(long, conflicts_with = "project")]
    pub here: bool,
}

impl Scope {
    /// The project id to filter by: the one named, or for `--here` the id of
    /// the project the current directory resolves to. Takes that id rather
    /// than resolving it so the rule is testable without a filesystem.
    pub fn project_id(&self, cwd_project: Option<String>) -> Option<String> {
        if self.here {
            return cwd_project;
        }
        self.project.clone()
    }
}

/// The filters `list` and `search` share.
#[derive(Debug, Clone, Default, PartialEq, Eq, clap::Args)]
pub struct Filters {
    #[command(flatten)]
    pub scope: Scope,
    /// Only memories of this type (memory rows only).
    #[arg(long = "type", value_name = "T")]
    pub memory_type: Option<String>,
    /// Only rows in this tier: 1 (hot), 2 (warm) or 3 (cold).
    #[arg(long, value_parser = clap::value_parser!(i64).range(1..=3))]
    pub tier: Option<i64>,
    /// Only memories in this lifecycle state.
    #[arg(long, value_enum)]
    pub status: Option<StatusArg>,
    /// Only rows saved in no project, which every project's searches reach.
    #[arg(long, conflicts_with_all = ["project", "here"])]
    pub no_project: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct ListOpts {
    /// Which table to list. Lists all three when omitted.
    #[arg(long, value_enum)]
    pub kind: Option<KindArg>,
    #[command(flatten)]
    pub filters: Filters,
    /// How many rows to show (1 to 100).
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100))]
    pub limit: u32,
    /// How many rows to skip, for paging.
    #[arg(long, default_value_t = 0)]
    pub offset: u32,
    /// Print JSON instead of a table.
    #[arg(long)]
    pub json: bool,
    /// List the deleted rows instead, most recently deleted first.
    #[arg(long)]
    pub deleted: bool,
    /// Only memories that came from this project of the tool they were
    /// imported from, by its exact name (memory rows only).
    #[arg(long, value_name = "NAME")]
    pub origin_project: Option<String>,
    #[command(flatten)]
    pub profile: ProfileArg,
}

#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct SearchOpts {
    /// Words to look for. Every word must match, as a prefix, ignoring case
    /// and accents. Only `memory` rows are searched.
    pub query: String,
    #[command(flatten)]
    pub filters: Filters,
    /// How many rows to show (1 to 100).
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100))]
    pub limit: u32,
    /// How many rows to skip, for paging.
    #[arg(long, default_value_t = 0)]
    pub offset: u32,
    /// Print JSON instead of a table.
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub profile: ProfileArg,
}

#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct ShowOpts {
    /// The row id, as printed by `list` and `search`. Ids are per table.
    pub id: i64,
    /// Which table the id belongs to.
    #[arg(long, value_enum, default_value_t = KindArg::Memory)]
    pub kind: KindArg,
    /// Print JSON instead of the detail view.
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub profile: ProfileArg,
}

#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct StatsOpts {
    #[command(flatten)]
    pub scope: Scope,
    /// Print JSON instead of text.
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub profile: ProfileArg,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum PathAction {
    /// Append the directory unless an equivalent entry is already there.
    Add {
        /// Directory to add. Defaults to the one holding this `clawg` binary.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Remove every entry naming the directory.
    Remove {
        /// Directory to remove. Defaults to the one holding this `clawg` binary.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
}

/// The `--profile` flag, shared by every subcommand.
///
/// Flattened rather than global so each subcommand's `--help` lists it where a
/// reader looks for it, and so the parsed value lives next to the other options
/// of the command it belongs to.
#[derive(Debug, Clone, Default, PartialEq, Eq, clap::Args)]
pub struct ProfileArg {
    /// Data directory profile: `dev` (~/.clawg-dev) or `prod` (~/.clawg).
    /// Defaults to `$CLAWG_PROFILE`, then to the build type.
    #[arg(long, value_name = "dev|prod")]
    pub profile: Option<String>,
}

impl ProfileArg {
    /// Resolves the flag into a [`Profile`].
    ///
    /// Routed through [`Profile::resolve`] with the build's real
    /// `debug_assertions` so the flag inherits the env var's guarantee
    /// *exactly*: a debug binary refuses `prod`. Without that, `--profile prod`
    /// would be a hole straight through the isolation `CLAWG_PROFILE` exists to
    /// enforce — a debug build writing into the production data directory.
    ///
    /// An absent flag is NOT `Profile::compiled_default()`: it falls through to
    /// [`Profile::from_env`], so `CLAWG_PROFILE` still decides when the user
    /// did not say.
    pub fn resolve(&self) -> Result<Profile, ProfileArgError> {
        match self.profile.as_deref() {
            Some(raw) => Profile::resolve(Some(raw), cfg!(debug_assertions))
                .map_err(|inner| ProfileArgError { inner, from_flag: true }),
            None => Profile::from_env()
                .map_err(|inner| ProfileArgError { inner, from_flag: false }),
        }
    }
}

/// Failure turning `--profile` (or its env fallback) into a [`Profile`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileArgError {
    inner: ProfileError,
    /// Whether the bad value came from the flag or from the environment —
    /// the message has to name the right one, or the user edits the wrong thing.
    from_flag: bool,
}

impl std::fmt::Display for ProfileArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.from_flag {
            match &self.inner {
                ProfileError::InvalidValue(raw) => write!(
                    f,
                    "invalid --profile value {raw:?}: accepted values are \"dev\" and \"prod\""
                ),
                ProfileError::ProdForbiddenInDebugBuild => write!(
                    f,
                    "--profile prod is forbidden in a debug build; use --profile dev, \
                     or run a release build to use prod"
                ),
            }
        } else {
            write!(f, "{}", self.inner)
        }
    }
}

impl std::error::Error for ProfileArgError {}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).expect("parse")
    }

    #[test]
    fn export_takes_its_filters_and_a_format() {
        assert_eq!(
            parse(&["clawg", "export", "--out", "m.jsonl"]).command,
            Command::Export {
                out: PathBuf::from("m.jsonl"),
                project: None,
                agent: None,
                include_deleted: false,
                format: ExportFormat::Jsonl,
                profile: ProfileArg { profile: None },
            }
        );
        assert_eq!(
            parse(&[
                "clawg", "export", "--out", "m.md", "--project", ".", "--agent", " codex ", "--include-deleted",
                "--format", "markdown", "--profile", "dev",
            ])
            .command,
            Command::Export {
                out: PathBuf::from("m.md"),
                project: Some(PathBuf::from(".")),
                agent: Some("codex".into()),
                include_deleted: true,
                format: ExportFormat::Markdown,
                profile: ProfileArg { profile: Some("dev".into()) },
            }
        );
        assert!(Cli::try_parse_from(["clawg", "export"]).is_err(), "--out is required");
    }

    #[test]
    fn import_takes_a_file_and_repeated_root_maps() {
        assert_eq!(
            parse(&[
                "clawg", "import", "m.jsonl", "--dry-run", "--agent", "dev", "--map-root", "/a/repo=.",
                "--map-root", "C:\\work=D:\\work",
            ])
            .command,
            Command::Import {
                file: Some(PathBuf::from("m.jsonl")),
                undo: None,
                from: ImportSource::Clawg,
                dry_run: true,
                agent: Some("dev".into()),
                map_root: vec![
                    RootMap { old: "/a/repo".into(), new: PathBuf::from(".") },
                    RootMap { old: "C:\\work".into(), new: PathBuf::from("D:\\work") },
                ],
                project_map: Vec::new(),
                profile: ProfileArg { profile: None },
            }
        );
        for bad in ["/a/repo", "=.", "/a/repo="] {
            assert!(Cli::try_parse_from(["clawg", "import", "m.jsonl", "--map-root", bad]).is_err(), "{bad}");
        }
    }

    #[test]
    fn import_takes_a_file_or_an_undo() {
        assert_eq!(
            parse(&["clawg", "import", "--undo", "4", "--dry-run"]).command,
            Command::Import {
                file: None,
                undo: Some(4),
                from: ImportSource::Clawg,
                dry_run: true,
                agent: None,
                map_root: Vec::new(),
                project_map: Vec::new(),
                profile: ProfileArg { profile: None },
            }
        );
        for refused in [
            &["clawg", "import"][..],
            &["clawg", "import", "m.jsonl", "--undo", "4"],
            &["clawg", "import", "--undo", "4", "--from", "engram"],
            &["clawg", "import", "--undo", "4", "--agent", "dev"],
            &["clawg", "import", "--undo", "4", "--map-root", "/a=."],
            &["clawg", "import", "--undo", "4", "--project-map", "a=."],
        ] {
            assert!(Cli::parse_args(refused).is_err(), "{refused:?}");
        }
    }

    #[test]
    fn import_takes_a_source_and_project_maps() {
        let args = ["clawg", "import", "--from", "engram", "e.json", "--project-map", "my=app=C:\\work\\app", "--project-map", "b=."];
        assert_eq!(
            Cli::parse_args(args).expect("parse").command,
            Command::Import {
                file: Some(PathBuf::from("e.json")),
                undo: None,
                from: ImportSource::Engram,
                dry_run: false,
                agent: None,
                map_root: Vec::new(),
                project_map: vec![
                    ProjectMap { name: "my=app".into(), root: PathBuf::from("C:\\work\\app") },
                    ProjectMap { name: "b".into(), root: PathBuf::from(".") },
                ],
                profile: ProfileArg { profile: None },
            }
        );
        let parses = |args: &[&str]| Cli::parse_args(args).map(|_| ()).map_err(|e| e.kind());
        assert_eq!(parses(&["clawg", "import", "--from", "clawg", "m.jsonl", "--map-root", "/a=."]), Ok(()));
        let conflict = Err(clap::error::ErrorKind::ArgumentConflict);
        assert_eq!(parses(&["clawg", "import", "--from", "engram", "e.json", "--map-root", "/a=."]), conflict);
        assert_eq!(parses(&["clawg", "import", "m.jsonl", "--project-map", "a=."]), conflict);
        assert_eq!(parses(&["clawg", "import", "--from", "clawg", "m.jsonl", "--project-map", "a=."]), conflict);
        assert!(parses(&["clawg", "import", "--from", "notion", "n.json"]).is_err());
        for bad in ["a", "=.", "a="] {
            assert!(parses(&["clawg", "import", "--from", "engram", "e.json", "--project-map", bad]).is_err(), "{bad}");
        }
    }

    #[test]
    fn update_takes_check_and_a_profile() {
        assert_eq!(
            parse(&["clawg", "update"]).command,
            Command::Update { check: false, profile: ProfileArg { profile: None } }
        );
        assert_eq!(
            parse(&["clawg", "update", "--check", "--profile", "dev"]).command,
            Command::Update { check: true, profile: ProfileArg { profile: Some("dev".into()) } }
        );
        assert!(!parse(&["clawg", "update"]).command.loads_the_model());
    }

    #[test]
    fn only_serve_loads_the_model() {
        assert!(parse(&["clawg", "serve"]).command.loads_the_model());
        for args in [
            &["clawg", "mcp"][..],
            &["clawg", "status"],
            &["clawg", "stop"],
            &["clawg", "backup"],
            &["clawg", "doctor"],
            &["clawg", "memories", "stats"],
        ] {
            assert!(!parse(args).command.loads_the_model(), "{args:?}");
        }
    }

    #[test]
    fn serve_defaults_have_no_profile_and_no_port() {
        let cli = parse(&["clawg", "serve"]);
        assert_eq!(
            cli.command,
            Command::Serve {
                profile: ProfileArg { profile: None },
                port: None
            }
        );
    }

    #[test]
    fn serve_accepts_profile_and_port() {
        let cli = parse(&["clawg", "serve", "--profile", "dev", "--port", "0"]);
        assert_eq!(
            cli.command,
            Command::Serve {
                profile: ProfileArg { profile: Some("dev".into()) },
                port: Some(0)
            }
        );
    }

    #[test]
    fn mcp_defaults_to_autostart_enabled() {
        let cli = parse(&["clawg", "mcp"]);
        assert_eq!(
            cli.command,
            Command::Mcp {
                profile: ProfileArg { profile: None },
                no_autostart: false,
                agent: None,
            }
        );
    }

    #[test]
    fn mcp_no_autostart_is_a_flag() {
        let cli = parse(&["clawg", "mcp", "--no-autostart"]);
        assert_eq!(
            cli.command,
            Command::Mcp {
                profile: ProfileArg { profile: None },
                no_autostart: true,
                agent: None,
            }
        );
    }

    #[test]
    fn hook_takes_an_event_an_agent_and_a_profile() {
        assert_eq!(
            parse(&["clawg", "hook", "session-start", "--agent", " claude-code ", "--profile", "dev"]).command,
            Command::Hook {
                event: HookEvent::SessionStart,
                agent: "claude-code".into(),
                profile: ProfileArg { profile: Some("dev".into()) },
            }
        );
        assert_eq!(
            parse(&["clawg", "hook", "user-prompt-submit", "--agent", "a"]).command,
            Command::Hook { event: HookEvent::UserPromptSubmit, agent: "a".into(), profile: ProfileArg { profile: None } }
        );
        assert!(Cli::try_parse_from(["clawg", "hook", "session-start"]).is_err(), "the agent is required");
        assert!(Cli::try_parse_from(["clawg", "hook", "pre-compact", "--agent", "a"]).is_err(), "only the events it answers");
    }

    #[test]
    fn setup_takes_a_client_a_profile_dry_run_and_remove() {
        assert_eq!(
            parse(&["clawg", "setup", "claude-code"]).command,
            Command::Setup { client: SetupClient::ClaudeCode, dry_run: false, remove: false, profile: ProfileArg { profile: None } }
        );
        assert_eq!(
            parse(&["clawg", "setup", "claude-code", "--dry-run", "--remove", "--profile", "dev"]).command,
            Command::Setup { client: SetupClient::ClaudeCode, dry_run: true, remove: true, profile: ProfileArg { profile: Some("dev".into()) } }
        );
        assert!(Cli::try_parse_from(["clawg", "setup"]).is_err(), "the client is required");
        assert!(Cli::try_parse_from(["clawg", "setup", "cursor"]).is_err(), "only the clients it knows");
        assert!(!parse(&["clawg", "setup", "claude-code"]).command.loads_the_model());
    }

    #[test]
    fn a_hook_usage_error_never_blocks() {
        assert!(is_hook_invocation(["clawg", "hook", "session-start"]));
        assert!(is_hook_invocation(["clawg", "hook"]));
        assert!(!is_hook_invocation(["clawg", "mcp", "hook"]));
        assert!(!is_hook_invocation(["clawg"]));
    }

    #[test]
    fn a_bare_clawg_never_blocks() {
        assert!(is_bare_invocation(["clawg"]));
        assert!(is_bare_invocation(Vec::<&str>::new()));
        assert!(!is_bare_invocation(["clawg", "status"]));
        assert!(!is_bare_invocation(["clawg", "--help"]));
    }

    #[test]
    fn mcp_takes_an_agent() {
        let cli = parse(&["clawg", "mcp", "--agent", " claude-code "]);
        assert_eq!(
            cli.command,
            Command::Mcp {
                profile: ProfileArg { profile: None },
                no_autostart: false,
                agent: Some("claude-code".into()),
            }
        );
    }

    #[test]
    fn sharing_shows_hides_and_unhides() {
        assert_eq!(
            parse(&["clawg", "sharing"]).command,
            Command::Sharing { action: None, profile: ProfileArg { profile: None } }
        );
        let e = Exclusion { reader: "cursor".into(), author: "claude-code".into(), profile: ProfileArg { profile: Some("dev".into()) } };
        assert_eq!(
            parse(&["clawg", "sharing", "hide", " cursor ", "claude-code", "--profile", "dev"]).command,
            Command::Sharing { action: Some(SharingAction::Hide(e.clone())), profile: ProfileArg { profile: None } }
        );
        assert_eq!(
            parse(&["clawg", "sharing", "unhide", "cursor", "claude-code", "--profile", "dev"]).command,
            Command::Sharing { action: Some(SharingAction::Unhide(e)), profile: ProfileArg { profile: None } }
        );
    }

    #[test]
    fn preferences_parses_set_and_unset() {
        assert_eq!(
            parse(&["clawg", "preferences"]).command,
            Command::Preferences { action: None, profile: ProfileArg { profile: None } }
        );
        let dev = ProfileArg { profile: Some("dev".into()) };
        assert_eq!(
            parse(&["clawg", "preferences", "set", "language", "es", "--profile", "dev"]).command,
            Command::Preferences {
                action: Some(PreferencesAction::Set(SetPreference {
                    key: PreferenceKey::Language,
                    value: Lang::Es,
                    profile: dev.clone()
                })),
                profile: ProfileArg { profile: None },
            }
        );
        assert_eq!(
            parse(&["clawg", "preferences", "--profile", "dev", "unset", "language"]).command,
            Command::Preferences {
                action: Some(PreferencesAction::Unset(UnsetPreference {
                    key: PreferenceKey::Language,
                    profile: ProfileArg { profile: None }
                })),
                profile: dev,
            }
        );
        for bad in [["clawg", "preferences", "set", "language", "fr"], ["clawg", "preferences", "set", "idioma", "es"]] {
            assert!(Cli::try_parse_from(bad).is_err(), "{bad:?}");
        }
        let Command::Preferences { action, profile } =
            parse(&["clawg", "preferences", "set", "language", "en", "--profile", "dev"]).command
        else {
            panic!("expected preferences");
        };
        assert_eq!(preferences_profile(action.as_ref(), &profile).profile.as_deref(), Some("dev"));
    }

    fn hide_of(args: &[&str]) -> Exclusion {
        let Command::Sharing { action: Some(SharingAction::Hide(e)), .. } = parse(args).command else {
            panic!("expected a hide");
        };
        e
    }

    #[test]
    fn sharing_rejects_a_blank_or_self_exclusion() {
        assert!(Cli::try_parse_from(["clawg", "sharing", "hide", " ", "claude-code"]).is_err());
        assert!(Cli::try_parse_from(["clawg", "sharing", "hide", "cursor"]).is_err(), "both agents are needed");
        let e = hide_of(&["clawg", "sharing", "hide", "cursor", "cursor"]);
        assert_eq!(e.check(Lang::En), Err("cursor always reads its own memories".to_string()));
    }

    #[test]
    fn sharing_takes_the_profile_from_either_side_of_the_action() {
        for args in [
            ["clawg", "sharing", "--profile", "dev", "hide", "cursor", "codex"],
            ["clawg", "sharing", "hide", "cursor", "codex", "--profile", "dev"],
        ] {
            let Command::Sharing { action, profile } = parse(&args).command else {
                panic!("expected sharing");
            };
            assert_eq!(sharing_profile(action.as_ref(), &profile).profile.as_deref(), Some("dev"), "{args:?}");
        }
        let Command::Sharing { action, profile } = parse(&["clawg", "sharing", "hide", "cursor", "codex"]).command else {
            panic!("expected sharing");
        };
        assert_eq!(sharing_profile(action.as_ref(), &profile).profile, None);
    }

    #[test]
    fn every_agent_is_accepted_on_either_side() {
        for args in [["*", "codex"], ["cursor", "*"], ["*", "*"]] {
            let e = hide_of(&["clawg", "sharing", "hide", args[0], args[1]]);
            assert_eq!(e.check(Lang::En), Ok(()), "{args:?}");
        }
    }

    #[test]
    fn mcp_rejects_a_blank_agent() {
        for blank in ["", "   "] {
            assert!(
                Cli::try_parse_from(["clawg", "mcp", "--agent", blank]).is_err(),
                "{blank:?} must not become an agent id"
            );
        }
    }

    #[test]
    fn status_and_stop_take_only_a_profile() {
        assert_eq!(
            parse(&["clawg", "status", "--profile", "dev"]).command,
            Command::Status { profile: ProfileArg { profile: Some("dev".into()) } }
        );
        assert_eq!(
            parse(&["clawg", "stop"]).command,
            Command::Stop { profile: ProfileArg { profile: None } }
        );
    }

    #[test]
    fn backup_takes_only_a_profile() {
        assert_eq!(
            parse(&["clawg", "backup"]).command,
            Command::Backup { profile: ProfileArg { profile: None } }
        );
        assert_eq!(
            parse(&["clawg", "backup", "--profile", "dev"]).command,
            Command::Backup { profile: ProfileArg { profile: Some("dev".into()) } }
        );
    }

    #[test]
    fn restore_takes_a_file_and_a_profile() {
        assert_eq!(
            parse(&["clawg", "restore", "backups/memory-x-manual.db", "--profile", "dev"]).command,
            Command::Restore {
                file: PathBuf::from("backups/memory-x-manual.db"),
                profile: ProfileArg { profile: Some("dev".into()) },
            }
        );
        assert!(Cli::try_parse_from(["clawg", "restore"]).is_err(), "the file is required");
    }

    #[test]
    fn secrets_takes_a_profile_and_masks_reasons_on_request() {
        assert_eq!(
            parse(&["clawg", "secrets"]).command,
            Command::Secrets { mask_reasons: false, yes: false, profile: ProfileArg { profile: None } }
        );
        assert_eq!(
            parse(&["clawg", "secrets", "--profile", "dev"]).command,
            Command::Secrets { mask_reasons: false, yes: false, profile: ProfileArg { profile: Some("dev".into()) } }
        );
        assert_eq!(
            parse(&["clawg", "secrets", "--mask-reasons", "--yes"]).command,
            Command::Secrets { mask_reasons: true, yes: true, profile: ProfileArg { profile: None } }
        );
        assert!(Cli::try_parse_from(["clawg", "secrets", "--yes"]).is_err(), "--yes confirms a rewrite");
    }

    #[test]
    fn metrics_report_takes_a_metric_labels_a_grouping_and_an_age() {
        assert_eq!(
            parse(&["clawg", "metrics", "report", "--metric", "http.duration"]).command,
            Command::Metrics {
                action: MetricsAction::Report {
                    metric: "http.duration".into(),
                    labels: vec![],
                    by: ReportBy::Version,
                    since: std::time::Duration::from_secs(30 * 86_400),
                    profile: ProfileArg { profile: None },
                }
            }
        );
        assert_eq!(
            parse(&[
                "clawg", "metrics", "report", "--metric", "http.duration", "--label", "route=/v1/search", "--label",
                " transport = unix ", "--by", "hour", "--since", "12h", "--profile", "dev",
            ])
            .command,
            Command::Metrics {
                action: MetricsAction::Report {
                    metric: "http.duration".into(),
                    labels: vec![("route".into(), "/v1/search".into()), ("transport".into(), "unix".into())],
                    by: ReportBy::Hour,
                    since: std::time::Duration::from_secs(12 * 3600),
                    profile: ProfileArg { profile: Some("dev".into()) },
                }
            }
        );
        let Command::Metrics { action: MetricsAction::Report { by, .. } } =
            parse(&["clawg", "metrics", "report", "--metric", "process.private_bytes", "--by", "minute"]).command
        else {
            panic!("a report")
        };
        assert_eq!(by, ReportBy::Minute);
        assert!(Cli::try_parse_from(["clawg", "metrics", "report"]).is_err(), "the metric is required");
        for bad in ["route", "=x", "route=", "a"] {
            assert!(parse_label(bad).is_err(), "{bad}");
        }
        for bad in ["30", "d", "3w", "-1d", "99999999999999999d"] {
            assert!(parse_since(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_since("1d"), Ok(std::time::Duration::from_secs(86_400)));
    }

    #[test]
    fn projects_add_takes_a_path_and_a_profile() {
        assert_eq!(
            parse(&["clawg", "projects", "add", "/work/other", "--profile", "dev"]).command,
            Command::Projects {
                action: ProjectsAction::Add {
                    path: "/work/other".into(),
                    profile: ProfileArg { profile: Some("dev".into()) }
                }
            }
        );
        assert!(Cli::try_parse_from(["clawg", "projects", "add"]).is_err(), "the path is required");
    }

    #[test]
    fn projects_list_and_rename_parse() {
        assert_eq!(
            parse(&["clawg", "projects", "list", "--profile", "dev"]).command,
            Command::Projects { action: ProjectsAction::List { profile: ProfileArg { profile: Some("dev".into()) } } }
        );
        assert_eq!(
            parse(&["clawg", "projects", "rename", "app-1a2b", "App web"]).command,
            Command::Projects {
                action: ProjectsAction::Rename { id: "app-1a2b".into(), name: "App web".into(), profile: ProfileArg::default() }
            }
        );
        assert!(Cli::try_parse_from(["clawg", "projects", "rename", "app-1a2b"]).is_err(), "the name is required");
        assert!(Cli::try_parse_from(["clawg", "projects", "list", "extra"]).is_err(), "list takes no argument");
    }

    #[test]
    fn projects_create_and_locate_parse() {
        assert_eq!(
            parse(&["clawg", "projects", "create", "ideas de producto"]).command,
            Command::Projects {
                action: ProjectsAction::Create { name: "ideas de producto".into(), new: false, profile: ProfileArg::default() }
            }
        );
        assert_eq!(
            parse(&["clawg", "projects", "create", "Mi App", "--new", "--profile", "dev"]).command,
            Command::Projects {
                action: ProjectsAction::Create { name: "Mi App".into(), new: true, profile: ProfileArg { profile: Some("dev".into()) } }
            }
        );
        assert_eq!(
            parse(&["clawg", "projects", "locate", "ideas", "/work/notes"]).command,
            Command::Projects {
                action: ProjectsAction::Locate {
                    project: "ideas".into(),
                    path: "/work/notes".into(),
                    subfolder: false,
                    profile: ProfileArg::default()
                }
            }
        );
        assert!(Cli::try_parse_from(["clawg", "projects", "create"]).is_err(), "the name is required");
        assert!(Cli::try_parse_from(["clawg", "projects", "locate", "ideas"]).is_err(), "the path is required");
    }

    #[test]
    fn projects_locate_takes_subfolder() {
        assert_eq!(
            parse(&["clawg", "projects", "locate", "web", "/work/mono/apps/web", "--subfolder"]).command,
            Command::Projects {
                action: ProjectsAction::Locate {
                    project: "web".into(),
                    path: "/work/mono/apps/web".into(),
                    subfolder: true,
                    profile: ProfileArg::default()
                }
            }
        );
    }

    #[test]
    fn projects_merge_and_keep_apart_parse() {
        let merge = |args: &[&str]| {
            let mut argv = vec!["clawg", "projects", "merge"];
            argv.extend_from_slice(args);
            match parse(&argv).command {
                Command::Projects { action: ProjectsAction::Merge(opts) } => opts,
                other => panic!("not a merge: {other:?}"),
            }
        };
        assert_eq!(
            merge(&["app-1a2b", "--into", "web-3c4d"]).request(),
            MergeRequest { source: Some("app-1a2b".into()), target: Some("web-3c4d".into()), ..MergeRequest::default() }
        );
        let dry = merge(&["app-1a2b", "--into", "web-3c4d", "--dry-run", "--profile", "dev"]);
        assert_eq!((dry.request().dry_run, dry.profile.profile.as_deref()), (true, Some("dev")));
        assert_eq!(merge(&["--undo", "3"]).request(), MergeRequest { undo: Some(3), ..MergeRequest::default() });
        let refused: [&[&str]; 5] = [
            &[],
            &["app-1a2b"],
            &["--into", "web-3c4d"],
            &["--undo", "3", "app-1a2b"],
            &["--undo", "3", "--into", "web-3c4d"],
        ];
        for args in refused {
            let mut argv = vec!["clawg", "projects", "merge"];
            argv.extend_from_slice(args);
            assert!(Cli::try_parse_from(&argv).is_err(), "{argv:?}");
        }

        assert_eq!(
            parse(&["clawg", "projects", "keep-apart", "app-1a2b", "copia-3c4d"]).command,
            Command::Projects {
                action: ProjectsAction::KeepApart {
                    project_id: "app-1a2b".into(),
                    other_id: "copia-3c4d".into(),
                    profile: ProfileArg::default()
                }
            }
        );
        assert!(Cli::try_parse_from(["clawg", "projects", "keep-apart", "app-1a2b"]).is_err(), "it takes two projects");
    }

    fn move_opts(args: &[&str]) -> MoveOpts {
        let mut argv = vec!["clawg", "projects", "move"];
        argv.extend_from_slice(args);
        match parse(&argv).command {
            Command::Projects { action: ProjectsAction::Move(opts) } => opts,
            other => panic!("not a move: {other:?}"),
        }
    }

    #[test]
    fn move_takes_a_selection_and_one_target_or_an_undo() {
        let opts = move_opts(&["--id", "3", "--id", "7", "--from-none", "--type", "semantic_fact", "--to-project", "repo-1a2b"]);
        assert_eq!(
            opts.request(None, None),
            MoveRequest {
                ids: vec![3, 7],
                from_none: true,
                memory_type: Some("semantic_fact".into()),
                to_project: Some("repo-1a2b".into()),
                ..MoveRequest::default()
            }
        );

        let opts = move_opts(&["--here", "--text", "parser cache", "--to", "/work/other", "--dry-run"]);
        assert_eq!(opts.to.as_deref(), Some(std::path::Path::new("/work/other")));
        assert_eq!(
            opts.request(Some("here-0000".into()), Some("other-1111".into())),
            MoveRequest {
                from_project: Some("here-0000".into()),
                q: Some("parser cache".into()),
                to_project: Some("other-1111".into()),
                dry_run: true,
                ..MoveRequest::default()
            }
        );

        assert_eq!(move_opts(&["--undo", "4"]).request(None, None), MoveRequest { undo: Some(4), ..MoveRequest::default() });
        assert_eq!(move_opts(&["--agent", "a", "--to-none"]).request(None, None).to_none, true);

        for bad in [
            vec!["--id", "3"],
            vec!["--id", "3", "--to-none", "--to-project", "repo-1a2b"],
            vec!["--undo", "4", "--id", "3"],
            vec!["--from-none", "--here", "--to-none"],
            vec!["--from-project", "a-0000", "--from-none", "--to-none"],
        ] {
            let mut argv = vec!["clawg", "projects", "move"];
            argv.extend(bad.iter().copied());
            assert!(Cli::try_parse_from(&argv).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn move_and_list_take_an_origin_project() {
        let opts = move_opts(&["--origin-project", "my repo", "--to-project", "repo-1a2b"]);
        assert_eq!(
            opts.request(None, None),
            MoveRequest {
                origin_project: Some("my repo".into()),
                to_project: Some("repo-1a2b".into()),
                ..MoveRequest::default()
            }
        );
        assert!(
            Cli::try_parse_from(["clawg", "projects", "move", "--origin-project", "a", "--undo", "4"]).is_err(),
            "an undo takes no selection"
        );

        let Command::Memories { action: MemoriesAction::List(opts) } =
            parse(&["clawg", "memories", "list", "--origin-project", "my repo"]).command
        else {
            panic!("not a list");
        };
        assert_eq!(opts.origin_project.as_deref(), Some("my repo"));
        assert_eq!(crate::memories::list_args(&opts, None).origin_project.as_deref(), Some("my repo"));
    }

    #[test]
    fn list_and_search_take_no_project() {
        let Command::Memories { action: MemoriesAction::List(opts) } = parse(&["clawg", "memories", "list", "--no-project"]).command
        else {
            panic!("not a list");
        };
        assert!(opts.filters.no_project);
        let Command::Memories { action: MemoriesAction::Search(opts) } =
            parse(&["clawg", "memories", "search", "cache", "--no-project"]).command
        else {
            panic!("not a search");
        };
        assert!(opts.filters.no_project);
        assert!(Cli::try_parse_from(["clawg", "memories", "list", "--no-project", "--project", "a-0000"]).is_err());
        assert!(Cli::try_parse_from(["clawg", "memories", "list", "--no-project", "--here"]).is_err());
    }

    #[test]
    fn outbox_takes_an_action_an_operation_id_and_an_agent() {
        assert_eq!(
            parse(&["clawg", "outbox", "retry", "op-1"]).command,
            Command::Outbox {
                action: OutboxAction::Retry(OutboxTarget {
                    operation_id: "op-1".into(),
                    agent: None,
                    profile: ProfileArg { profile: None },
                })
            }
        );
        assert_eq!(
            parse(&["clawg", "outbox", "discard", "op-1", "--agent", "codex", "--profile", "dev"]).command,
            Command::Outbox {
                action: OutboxAction::Discard(OutboxTarget {
                    operation_id: "op-1".into(),
                    agent: Some("codex".into()),
                    profile: ProfileArg { profile: Some("dev".into()) },
                })
            }
        );
        assert!(Cli::try_parse_from(["clawg", "outbox", "retry"]).is_err(), "the operation id is required");
        assert!(Cli::try_parse_from(["clawg", "outbox", "forget", "op-1"]).is_err());
    }

    #[test]
    fn undelete_takes_an_id_and_a_profile() {
        assert_eq!(
            parse(&["clawg", "undelete", "57", "--profile", "dev"]).command,
            Command::Undelete { id: 57, profile: ProfileArg { profile: Some("dev".into()) } }
        );
        assert_eq!(
            parse(&["clawg", "undelete", "57"]).command,
            Command::Undelete { id: 57, profile: ProfileArg { profile: None } }
        );
        assert!(Cli::try_parse_from(["clawg", "undelete"]).is_err(), "the id is required");
        assert!(Cli::try_parse_from(["clawg", "undelete", "memory"]).is_err(), "the id is a number");
    }

    #[test]
    fn purge_takes_an_id_or_a_delete() {
        let purge = |args: &[&str]| {
            let mut argv = vec!["clawg", "purge"];
            argv.extend_from_slice(args);
            parse(&argv).command
        };
        assert_eq!(
            purge(&["57", "--yes", "--profile", "dev"]),
            Command::Purge { id: Some(57), project_delete: None, yes: true, profile: ProfileArg { profile: Some("dev".into()) } }
        );
        assert_eq!(
            purge(&["57"]),
            Command::Purge { id: Some(57), project_delete: None, yes: false, profile: ProfileArg { profile: None } }
        );
        assert_eq!(
            purge(&["--project-delete", "4", "--yes"]),
            Command::Purge { id: None, project_delete: Some(4), yes: true, profile: ProfileArg { profile: None } }
        );
        assert_eq!(
            purge(&["--project-delete", "4"]),
            Command::Purge { id: None, project_delete: Some(4), yes: false, profile: ProfileArg { profile: None } },
            "without --yes it parses, and the command refuses before it opens anything"
        );
        let refused: [&[&str]; 4] = [&[], &["57", "--project-delete", "4"], &["memory"], &["--project-delete", "four"]];
        for args in refused {
            let mut argv = vec!["clawg", "purge"];
            argv.extend_from_slice(args);
            assert!(Cli::try_parse_from(&argv).is_err(), "{argv:?}");
        }
    }

    #[test]
    fn doctor_takes_only_a_profile() {
        assert_eq!(
            parse(&["clawg", "doctor"]).command,
            Command::Doctor { profile: ProfileArg { profile: None } }
        );
        assert_eq!(
            parse(&["clawg", "doctor", "--profile", "dev"]).command,
            Command::Doctor { profile: ProfileArg { profile: Some("dev".into()) } }
        );
    }

    #[test]
    fn path_add_and_remove_default_to_the_binarys_own_directory() {
        assert_eq!(
            parse(&["clawg", "path", "add"]).command,
            Command::Path { action: PathAction::Add { dir: None } }
        );
        assert_eq!(
            parse(&["clawg", "path", "remove"]).command,
            Command::Path { action: PathAction::Remove { dir: None } }
        );
    }

    #[test]
    fn path_accepts_an_explicit_directory() {
        assert_eq!(
            parse(&["clawg", "path", "add", "--dir", r"C:\Program Files\Claw Garden"]).command,
            Command::Path {
                action: PathAction::Add { dir: Some(r"C:\Program Files\Claw Garden".into()) }
            }
        );
    }

    #[test]
    fn path_without_an_action_is_a_usage_error() {
        assert!(Cli::try_parse_from(["clawg", "path"]).is_err());
    }

    #[test]
    fn a_port_above_u16_is_a_usage_error_not_a_wrap() {
        assert!(Cli::try_parse_from(["clawg", "serve", "--port", "70000"]).is_err());
    }

    #[test]
    fn an_unknown_subcommand_is_rejected() {
        assert!(Cli::try_parse_from(["clawg", "frobnicate"]).is_err());
    }

    /// `--version` is how a user and a bug report agree on which binary they
    /// are talking about, so both halves of the line have to be right.
    #[test]
    fn version_prints_the_binary_name_and_the_package_version() {
        let err = Cli::try_parse_from(["clawg", "--version"]).expect_err("--version exits");
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
        let rendered = err.to_string();
        assert!(rendered.contains("clawg"), "rendered: {rendered}");
        assert!(
            rendered.contains(env!("CARGO_PKG_VERSION")),
            "rendered: {rendered}"
        );
    }

    /// A subcommand is mandatory: a bare `clawg` has nothing to do, and doing
    /// nothing silently would read as a hang.
    #[test]
    fn a_bare_invocation_is_a_usage_error() {
        assert!(Cli::try_parse_from(["clawg"]).is_err());
    }

    // ── memories ────────────────────────────────────────────────────────────

    fn memories(args: &[&str]) -> MemoriesAction {
        let mut argv = vec!["clawg", "memories"];
        argv.extend_from_slice(args);
        match parse(&argv).command {
            Command::Memories { action } => action,
            other => panic!("expected memories, got {other:?}"),
        }
    }

    #[test]
    fn memories_list_defaults() {
        assert_eq!(
            memories(&["list"]),
            MemoriesAction::List(ListOpts {
                kind: None,
                filters: Filters::default(),
                limit: 20,
                offset: 0,
                json: false,
                deleted: false,
                origin_project: None,
                profile: ProfileArg { profile: None },
            })
        );
        match memories(&["list", "--deleted"]) {
            MemoriesAction::List(opts) => assert!(opts.deleted),
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn list_accepts_every_filter() {
        let action = memories(&[
            "list", "--kind", "episodic", "--agent", "a1", "--project", "p1", "--type",
            "semantic_fact", "--tier", "2", "--status", "disputed", "--limit", "5", "--offset",
            "10", "--json", "--profile", "dev",
        ]);
        assert_eq!(
            action,
            MemoriesAction::List(ListOpts {
                kind: Some(KindArg::Episodic),
                filters: Filters {
                    scope: Scope {
                        agent: Some("a1".into()),
                        project: Some("p1".into()),
                        here: false,
                    },
                    memory_type: Some("semantic_fact".into()),
                    tier: Some(2),
                    status: Some(StatusArg::Disputed),
                    no_project: false,
                },
                limit: 5,
                offset: 10,
                json: true,
                deleted: false,
                origin_project: None,
                profile: ProfileArg { profile: Some("dev".into()) },
            })
        );
    }

    #[test]
    fn here_is_a_flag_and_conflicts_with_project() {
        match memories(&["list", "--here"]) {
            MemoriesAction::List(opts) => assert!(opts.filters.scope.here),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn project_and_here_conflict() {
        for argv in [
            ["clawg", "memories", "list", "--project", "p", "--here"],
            ["clawg", "memories", "stats", "--here", "--project", "p"],
        ] {
            let err = Cli::try_parse_from(argv).expect_err("they name the project twice");
            assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
        }
    }

    #[test]
    fn scope_resolves_the_project_it_names_or_the_working_directory() {
        let cwd = || Some("claw-garden-1a2b".to_string());
        let named = Scope { project: Some("p1".into()), ..Scope::default() };
        assert_eq!(named.project_id(cwd()), Some("p1".to_string()));

        let here = Scope { here: true, ..Scope::default() };
        assert_eq!(here.project_id(cwd()), cwd());

        assert_eq!(Scope::default().project_id(cwd()), None, "no flag, no project");
    }

    #[test]
    fn search_requires_query() {
        assert!(Cli::try_parse_from(["clawg", "memories", "search"]).is_err());
        match memories(&["search", "deploy window", "--agent", "a1", "--limit", "3"]) {
            MemoriesAction::Search(opts) => {
                assert_eq!(opts.query, "deploy window");
                assert_eq!(opts.filters.scope.agent.as_deref(), Some("a1"));
                assert_eq!(opts.limit, 3);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn search_has_no_kind_flag() {
        assert!(Cli::try_parse_from(["clawg", "memories", "search", "x", "--kind", "memory"])
            .is_err());
    }

    #[test]
    fn show_requires_id_and_defaults_kind_memory() {
        assert!(Cli::try_parse_from(["clawg", "memories", "show"]).is_err());
        assert_eq!(
            memories(&["show", "57"]),
            MemoriesAction::Show(ShowOpts {
                id: 57,
                kind: KindArg::Memory,
                json: false,
                profile: ProfileArg { profile: None },
            })
        );
        match memories(&["show", "3", "--kind", "procedural", "--json"]) {
            MemoriesAction::Show(opts) => {
                assert_eq!((opts.id, opts.kind, opts.json), (3, KindArg::Procedural, true));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn stats_takes_a_scope_and_json() {
        assert_eq!(
            memories(&["stats", "--agent", "a1", "--json"]),
            MemoriesAction::Stats(StatsOpts {
                scope: Scope { agent: Some("a1".into()), project: None, here: false },
                json: true,
                profile: ProfileArg { profile: None },
            })
        );
    }

    #[test]
    fn tier_out_of_range_is_usage_error() {
        for tier in ["0", "4", "-1", "x"] {
            assert!(
                Cli::try_parse_from(["clawg", "memories", "list", "--tier", tier]).is_err(),
                "tier {tier} must be refused"
            );
        }
    }

    #[test]
    fn limit_outside_what_the_daemon_serves_is_usage_error() {
        for limit in ["0", "101"] {
            assert!(Cli::try_parse_from(["clawg", "memories", "list", "--limit", limit]).is_err());
        }
    }

    #[test]
    fn unknown_kind_and_status_are_usage_errors() {
        assert!(Cli::try_parse_from(["clawg", "memories", "list", "--kind", "bogus"]).is_err());
        assert!(Cli::try_parse_from(["clawg", "memories", "list", "--status", "stale"]).is_err());
    }

    #[test]
    fn every_memories_action_exposes_its_profile() {
        for argv in [
            &["list"][..],
            &["search", "q"],
            &["show", "1"],
            &["stats"],
        ] {
            let mut with_profile = argv.to_vec();
            with_profile.extend(["--profile", "dev"]);
            assert_eq!(memories(&with_profile).profile().profile.as_deref(), Some("dev"));
            assert_eq!(memories(argv).profile().profile, None);
        }
    }

    #[test]
    fn memories_without_action_is_usage_error() {
        assert!(Cli::try_parse_from(["clawg", "memories"]).is_err());
    }

    // ── ProfileArg::resolve ─────────────────────────────────────────────────

    #[test]
    fn resolve_dev_flag_is_dev_in_any_build() {
        let arg = ProfileArg { profile: Some("dev".into()) };
        assert_eq!(arg.resolve(), Ok(Profile::Dev));
    }

    #[test]
    fn resolve_is_case_and_whitespace_insensitive_like_the_env_var() {
        let arg = ProfileArg { profile: Some("  DEV ".into()) };
        assert_eq!(arg.resolve(), Ok(Profile::Dev));
    }

    /// The point of routing through `Profile::resolve`: the flag must inherit
    /// the env var's refusal, otherwise `--profile prod` on a debug build
    /// writes into the production data directory.
    #[test]
    fn resolve_prod_flag_matches_the_env_vars_debug_build_rule() {
        let arg = ProfileArg { profile: Some("prod".into()) };
        if cfg!(debug_assertions) {
            let err = arg.resolve().expect_err("debug build must refuse prod");
            let msg = err.to_string();
            assert!(msg.contains("--profile"), "message was: {msg}");
            assert!(msg.contains("debug build"), "message was: {msg}");
        } else {
            assert_eq!(arg.resolve(), Ok(Profile::Prod));
        }
    }

    #[test]
    fn resolve_invalid_flag_names_the_flag_and_the_accepted_values() {
        let arg = ProfileArg { profile: Some("staging".into()) };
        let err = arg.resolve().expect_err("staging is not a profile");
        let msg = err.to_string();
        assert!(msg.contains("--profile"), "message was: {msg}");
        assert!(msg.contains("dev"), "message was: {msg}");
        assert!(msg.contains("prod"), "message was: {msg}");
        assert!(
            !msg.contains("CLAWG_PROFILE"),
            "a flag error must not send the user to the env var: {msg}"
        );
    }

    /// An absent flag must defer to the environment, not hardcode the compiled
    /// default — otherwise `CLAWG_PROFILE` silently stops working under the CLI.
    #[test]
    fn resolve_absent_flag_matches_from_env() {
        let arg = ProfileArg { profile: None };
        assert_eq!(arg.resolve().ok(), Profile::from_env().ok());
    }

    #[test]
    fn projects_delete_parses() {
        let delete = |args: &[&str]| {
            let mut argv = vec!["clawg", "projects", "delete"];
            argv.extend_from_slice(args);
            match parse(&argv).command {
                Command::Projects { action: ProjectsAction::Delete(opts) } => opts,
                other => panic!("not a delete: {other:?}"),
            }
        };
        assert_eq!(
            delete(&["app-1a2b"]).request(),
            DeleteRequest { project_id: Some("app-1a2b".into()), ..DeleteRequest::default() }
        );
        assert_eq!(
            delete(&["app-1a2b", "--with-memories"]).request(),
            DeleteRequest { project_id: Some("app-1a2b".into()), with_memories: true, ..DeleteRequest::default() }
        );
        let confirmed = delete(&["app-1a2b", "--with-memories", "--confirm", "1a2b3c4d", "--profile", "dev"]);
        assert_eq!(
            confirmed.request(),
            DeleteRequest {
                project_id: Some("app-1a2b".into()),
                with_memories: true,
                confirm: Some("1a2b3c4d".into()),
                ..DeleteRequest::default()
            }
        );
        assert_eq!(confirmed.profile.profile.as_deref(), Some("dev"));
        assert_eq!(delete(&["--undo", "3"]).request(), DeleteRequest { undo: Some(3), ..DeleteRequest::default() });
        assert_eq!(
            delete(&["--undo", "3", "--dry-run"]).request(),
            DeleteRequest { undo: Some(3), dry_run: true, ..DeleteRequest::default() }
        );
        let refused: [&[&str]; 8] = [
            &[],
            &["app-1a2b", "--confirm", "1a2b3c4d"],
            &["--undo", "3", "app-1a2b"],
            &["--undo", "3", "--with-memories"],
            &["--undo", "3", "--confirm", "1a2b3c4d"],
            &["app-1a2b", "--dry-run"],
            &["--with-memories"],
            &["--undo", "three"],
        ];
        for args in refused {
            let mut argv = vec!["clawg", "projects", "delete"];
            argv.extend_from_slice(args);
            assert!(Cli::try_parse_from(&argv).is_err(), "{argv:?}");
        }
    }
}
