//! The routes that back up, import and undo an import of memories, with the import jobs.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use clawg_core::memory::backup::Reason;
use clawg_core::memory::chunker::ChatChunker;
use clawg_core::memory::clock::TimestampMs;
use clawg_core::memory::embedder::Embedder;
use clawg_core::memory::identity::Disk;
use clawg_core::memory::portability::timing::{Step, Timing};
use clawg_core::memory::portability::{
    ImportControl, Imported, ImportError, ImportInput, ImportOptions, ImportReport, ImportTimings,
    MapProject, MapTarget, MAX_IMPORT_BYTES, UndoError, UndoReport,
};
use clawg_core::memory::portability;
use clawg_core::metrics::EventKind;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

use crate::daemon::imports::{Busy, Cancel, End, JobView, Meta};

use super::{ApiError, ApiResult, AppCtx, embed_failure, micros, ready_embedder};

#[derive(Debug, Serialize)]
pub struct BackupBody {
    pub path: String,
}

pub(super) async fn backup(State(app): State<Arc<AppCtx>>) -> ApiResult<Json<BackupBody>> {
    let path = app.runtime.snapshot(Reason::Manual).await.map_err(ApiError::internal)?;
    Ok(Json(BackupBody { path: path.display().to_string() }))
}

/// The body of `POST /v1/import`: an export as text, which JSON escaping
/// makes longer than the file it came from.
pub(super) const IMPORT_BODY_LIMIT: usize = 2 * MAX_IMPORT_BYTES;

#[derive(Debug, Deserialize)]
pub struct ImportRequest {
    /// The export, as `clawg export` wrote it, or as another tool did.
    pub text: String,
    /// The tool the export comes from: `clawg` when missing, `engram`, or
    /// `claude-code`, whose memory is sent as the document its directory
    /// reads into.
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub dry_run: bool,
    /// The agent every row that has one is stored under instead.
    #[serde(default)]
    pub agent: Option<String>,
    /// Exported project roots and where they are here, for a clawg export:
    /// a root, or `{"root", "path"}` with the path the user named.
    #[serde(default)]
    pub map_roots: Vec<(String, MapTarget)>,
    /// Project names of another tool and where their notes go here: a root,
    /// `{"root", "path"}`, or `{"project"}`, an id or the name of one live
    /// project. A daemon from before takes only a root, and refuses the
    /// objects instead of storing the notes in no project.
    #[serde(default)]
    pub project_map: Vec<(String, MapTarget)>,
}

/// The project each `{"project"}` of `map` names, by its live id, refused
/// as a move refuses its target: none is a 404, two a 409
/// `project_ambiguous`, a deleted one a 409 `project_deleted`.
async fn resolve_map(app: &AppCtx, map: Vec<(String, MapTarget)>) -> ApiResult<Vec<(String, MapTarget)>> {
    let mut resolved = Vec::with_capacity(map.len());
    for (name, target) in map {
        let target = match target {
            MapTarget::Project { project } => match portability::map_project(&app.runtime.store, &project).await {
                Ok(MapProject::Live(id)) => MapTarget::Project { project: id },
                Ok(MapProject::Unknown) => return Err(ApiError::not_found(format!("no project {project:?}"))),
                Ok(MapProject::Ambiguous(ids)) => {
                    return Err(ApiError {
                        detail: Some(format!("{project:?} is the name of {}: name one of them by its id", ids.join(", "))),
                        ..ApiError::conflict("project_ambiguous")
                    })
                }
                Err(e) => return Err(import_error(e)),
            },
            target => target,
        };
        resolved.push((name, target));
    }
    Ok(resolved)
}

fn import_error(e: ImportError) -> ApiError {
    match e {
        ImportError::Format(e) => ApiError::bad_request(e.to_string()),
        ImportError::NoEmbedder => ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "embedder_not_ready"),
        ImportError::Embed(e) => embed_failure(e),
        ImportError::ProjectDeleted { .. } => ApiError { detail: Some(e.to_string()), ..ApiError::conflict("project_deleted") },
        e @ ImportError::Store(_) => ApiError::internal(e),
    }
}

/// What `POST /v1/import` takes of the request besides the export itself.
struct ImportFields {
    from: Option<String>,
    dry_run: bool,
    agent: Option<String>,
    map_roots: Vec<(String, MapTarget)>,
    project_map: Vec<(String, MapTarget)>,
}

/// The tool an export comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Clawg,
    Engram,
    /// Claude Code's auto memory, a directory rather than a file.
    ClaudeCode,
}

impl Source {
    fn name(self) -> &'static str {
        match self {
            Source::Clawg => "clawg",
            Source::Engram => "engram",
            Source::ClaudeCode => portability::claude_code::SOURCE,
        }
    }
}

/// The fields of an import request once checked: where the export comes from
/// and what to do with it.
struct Asked {
    source: Source,
    options: ImportOptions,
}

fn too_large(len: usize) -> ApiError {
    ApiError::bad_request(format!("the file is {len} bytes; at most {MAX_IMPORT_BYTES} are imported"))
}

/// Everything about an import request that needs neither the export nor the
/// embedder: the agent, the maps, and the source they go with.
async fn checked_fields(app: &AppCtx, fields: ImportFields) -> ApiResult<Asked> {
    let ImportFields { from, dry_run, agent, map_roots, project_map } = fields;
    let agent = match agent.as_deref().map(str::trim) {
        Some("") => return Err(ApiError::bad_request("agent must not be blank")),
        agent => agent.map(str::to_string),
    };
    if map_roots.iter().any(|(_, target)| target.root().is_none()) {
        return Err(ApiError::bad_request("map_roots maps a root to a root; a project of the file goes by its remote, id or name"));
    }
    let project_map = resolve_map(app, project_map).await?;
    let source = match from.as_deref() {
        None | Some("clawg") => {
            if !project_map.is_empty() {
                return Err(ApiError::bad_request(
                    "project_map maps the projects of another tool's export; name the tool in from",
                ));
            }
            Source::Clawg
        }
        Some(tool @ ("engram" | "claude-code")) => {
            if !map_roots.is_empty() {
                return Err(ApiError::bad_request(
                    "map_roots maps the roots of a clawg export; another tool's projects take project_map",
                ));
            }
            if tool == "engram" { Source::Engram } else { Source::ClaudeCode }
        }
        Some(other) => {
            return Err(ApiError::bad_request(format!("unknown source {other:?}; expected clawg, engram or claude-code")))
        }
    };
    Ok(Asked { source, options: ImportOptions { dry_run, agent, map_roots, project_map } })
}

/// The export `text` as `source` writes it, parsed once.
fn parse_input(source: Source, text: &str) -> ApiResult<ImportInput> {
    match source {
        Source::Clawg => {
            let (_, parsed) = portability::format::parse(text).map_err(|e| ApiError::bad_request(e.to_string()))?;
            Ok(ImportInput::Clawg(parsed))
        }
        Source::Engram => {
            portability::engram::read(text).map(ImportInput::Foreign).map_err(|e| ApiError::bad_request(e.to_string()))
        }
        Source::ClaudeCode => {
            portability::claude_code::read(text).map(ImportInput::Foreign).map_err(|e| ApiError::bad_request(e.to_string()))
        }
    }
}

/// Another tool's export is split by the model's tokenizer as a save of each
/// note's text, so without it nothing is imported, not even a dry run: cut by
/// bytes, the pieces would be others.
fn tokenizer_for(app: &AppCtx, input: &ImportInput) -> ApiResult<()> {
    if matches!(input, ImportInput::Foreign(_)) && app.runtime.tokenizer.get().is_none() {
        return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "tokenizer_not_ready"));
    }
    Ok(())
}

/// The embedder a real import writes with; a dry run needs none.
fn embedder_for(app: &AppCtx, dry_run: bool) -> ApiResult<Option<Arc<dyn Embedder>>> {
    if dry_run {
        return Ok(None);
    }
    ready_embedder(app).map(Some)
}

/// Imports an export. The file is read before the embedder is asked for, so
/// a file this binary cannot read is a 400 even while the model loads, and a
/// dry run needs no model at all.
pub(super) async fn import_memories(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<ImportRequest>,
) -> ApiResult<Json<ImportReport>> {
    let ImportRequest { text, from, dry_run, agent, map_roots, project_map } = body;
    let mut timings = ImportTimings::new(dry_run).with_memory(crate::daemon::process::private_bytes);
    let read = Instant::now();
    if text.len() > MAX_IMPORT_BYTES {
        return Err(too_large(text.len()));
    }
    let Asked { source, options } = checked_fields(&app, ImportFields { from, dry_run, agent, map_roots, project_map }).await?;
    let input = parse_input(source, &text)?;
    tokenizer_for(&app, &input)?;
    timings.record(Timing { elapsed: read.elapsed(), ..Timing::new(Step::Read) });
    let embedder = embedder_for(&app, dry_run)?;
    let backups = clawg_core::paths::backups_dir(&app.runtime.base_dir);
    let store = &app.runtime.store;
    let chunker = Arc::new(ChatChunker::from_cell(app.runtime.tokenizer.clone()));
    let imported = portability::import_parsed(
        store,
        &app.runtime.imports,
        embedder.as_deref(),
        chunker,
        &backups,
        input,
        &options,
        Arc::new(Disk),
        &mut timings,
        &ImportControl::default(),
    )
    .await;
    match imported {
        Ok(Imported::Done(report)) => {
            timings.finish();
            record_import(&app, source, &report, read);
            Ok(Json(report))
        }
        // Nothing here holds the control, so nothing can raise a stop.
        Ok(Imported::Stopped(_)) => Err(ApiError::internal("the import was stopped")),
        Err(e) => Err(import_error(e)),
    }
}

/// The `import` event of an import that ended, `started` being when its
/// request began.
fn record_import(app: &AppCtx, source: Source, report: &ImportReport, started: Instant) {
    let detail = [
        ("source", source.name().into()),
        ("dry_run", report.dry_run.into()),
        ("ms", micros(started.elapsed()).div_ceil(1_000).into()),
        ("rows", report.rows.into()),
        ("inserted", report.inserted.into()),
        ("present", report.present.into()),
    ];
    app.runtime.metrics.event(EventKind::Import, &detail);
}

/// The body of `POST /v1/imports`: that of `POST /v1/import` with the path of
/// the export where its text would be. Strict, as a move's, so a misspelt field
/// imports nothing; it is read from the bytes because the `Json` extractor
/// answers an unknown field with a 422 in plain text.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportJobRequest {
    /// An absolute path to the export, a regular file this daemon can read;
    /// for `claude-code`, the memory directory it reads.
    pub path: String,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub map_roots: Vec<(String, MapTarget)>,
    #[serde(default)]
    pub project_map: Vec<(String, MapTarget)>,
}

/// The export at `path` as `source` writes it. The text is read and parsed in
/// one call, so it lives no longer than that: the job keeps what was parsed.
fn read_export(path: &std::path::Path, source: Source) -> ApiResult<ImportInput> {
    use std::io::Read;
    if source == Source::ClaudeCode {
        return read_claude_code(path);
    }
    let unreadable = |e: std::io::Error| match e.kind() {
        std::io::ErrorKind::NotFound => ApiError::bad_request("no such file"),
        _ => ApiError::bad_request(format!("the file cannot be read: {e}")),
    };
    // Looked at before it is opened, so a named pipe is never waited on.
    let meta = std::fs::metadata(path).map_err(unreadable)?;
    if !meta.is_file() {
        return Err(ApiError::bad_request("not a regular file"));
    }
    if meta.len() > MAX_IMPORT_BYTES as u64 {
        return Err(too_large(usize::try_from(meta.len()).unwrap_or(usize::MAX)));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(meta.len()).unwrap_or(0));
    // One more byte than the limit, so a file that grew since it was measured is refused too.
    std::fs::File::open(path)
        .and_then(|file| file.take(MAX_IMPORT_BYTES as u64 + 1).read_to_end(&mut bytes))
        .map_err(unreadable)?;
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(too_large(bytes.len()));
    }
    let text = String::from_utf8(bytes).map_err(|_| ApiError::bad_request("the file is not UTF-8 text"))?;
    parse_input(source, &text)
}

/// Claude Code's memory under the directory `path`, read whole, every text
/// counted against the limit of a file.
fn read_claude_code(path: &std::path::Path) -> ApiResult<ImportInput> {
    use portability::claude_code::{collect, from_document, CollectError};
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ApiError::bad_request("no such directory")),
        Ok(meta) if !meta.is_dir() => return Err(ApiError::bad_request("not a directory")),
        _ => {}
    }
    match collect(path, MAX_IMPORT_BYTES) {
        Ok(document) => Ok(ImportInput::Foreign(from_document(document))),
        Err(CollectError::NotADirectory { .. }) => Err(ApiError::bad_request("not a directory")),
        Err(e) => Err(ApiError::bad_request(e.to_string())),
    }
}

/// What a job runs with once it is accepted.
struct Job {
    input: ImportInput,
    options: ImportOptions,
    source: Source,
    embedder: Option<Arc<dyn Embedder>>,
    timings: ImportTimings,
    started: Instant,
    control: ImportControl,
}

/// Imports the export at a path as a job: answers `202` with the job once the
/// file is read and checked, and imports in the background. The request is
/// refused, whatever else is wrong with it, before a job exists.
pub(super) async fn start_import_job(State(app): State<Arc<AppCtx>>, body: Bytes) -> ApiResult<Response> {
    let request = serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let view = accept_job(app, request, ImportControl::default()).await?;
    Ok((StatusCode::ACCEPTED, Json(&*view)).into_response())
}

/// The checks of `POST /v1/import` in its order, with the slot taken after
/// what only reads the request and before the file is opened, so two requests
/// never read a file at once.
async fn accept_job(app: Arc<AppCtx>, request: ImportJobRequest, control: ImportControl) -> ApiResult<Arc<JobView>> {
    let started = Instant::now();
    let ImportJobRequest { path, from, dry_run, agent, map_roots, project_map } = request;
    let path = std::path::PathBuf::from(path);
    if !path.is_absolute() {
        return Err(ApiError::bad_request("path must be absolute"));
    }
    let Asked { source, options } = checked_fields(&app, ImportFields { from, dry_run, agent, map_roots, project_map }).await?;
    let reservation = app.imports.reserve().map_err(job_refused)?;
    let input = tokio::task::spawn_blocking(move || read_export(&path, source)).await.map_err(ApiError::internal)??;
    tokenizer_for(&app, &input)?;
    let mut timings = ImportTimings::new(dry_run).with_memory(crate::daemon::process::private_bytes);
    timings.record(Timing { elapsed: started.elapsed(), ..Timing::new(Step::Read) });
    let embedder = embedder_for(&app, dry_run)?;
    // A model let go for being idle loads while the job plans and decides.
    if let Some(embedder) = &embedder {
        embedder.wake();
    }

    let id = reservation.id();
    let meta = Meta { dry_run, from: source.name(), started_at_ms: TimestampMs::now().as_millis_i64() };
    let job = Job { input, options, source, embedder, timings, started, control: control.clone() };
    let task_app = app.clone();
    let view = reservation
        .run(meta, control, move || tokio::spawn(run_job(task_app, id, job)).abort_handle())
        .map_err(job_refused)?;
    log::info!("import job {id} accepted: {} from {}", if dry_run { "dry run" } else { "import" }, source.name());
    Ok(view)
}

/// What a request for a job is told when the slot is not free: the job that
/// holds it, or that the daemon is going away.
fn job_refused(busy: Busy) -> ApiError {
    match busy {
        Busy::Running(job_id) => ApiError {
            detail: Some(format!("import job {job_id} is running; try again when it ends")),
            job_id: Some(job_id),
            ..ApiError::conflict("import_running")
        },
        Busy::Closing => ApiError {
            detail: Some("the daemon is shutting down; try again when it is back".into()),
            retryable: Some(true),
            ..ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "shutting_down")
        },
    }
}

/// Ends job `id` as failed if its task goes away before it says how the job
/// ended, so a panic does not leave the slot held for good.
struct Ending {
    app: Arc<AppCtx>,
    id: u64,
    end: Option<End>,
}

impl Drop for Ending {
    fn drop(&mut self) {
        let end = self.end.take().unwrap_or(End::Failed { error: "the import task ended without a result".into(), detail: None });
        self.app.imports.finish(self.id, end, TimestampMs::now().as_millis_i64());
    }
}

async fn run_job(app: Arc<AppCtx>, id: u64, job: Job) {
    let mut ending = Ending { app: app.clone(), id, end: None };
    ending.end = Some(import_as_job(&app, id, job).await);
}

async fn import_as_job(app: &AppCtx, id: u64, mut job: Job) -> End {
    let backups = clawg_core::paths::backups_dir(&app.runtime.base_dir);
    let chunker = Arc::new(ChatChunker::from_cell(app.runtime.tokenizer.clone()));
    let imported = portability::import_parsed(
        &app.runtime.store,
        &app.runtime.imports,
        job.embedder.as_deref(),
        chunker,
        &backups,
        job.input,
        &job.options,
        Arc::new(Disk),
        &mut job.timings,
        &job.control,
    )
    .await;
    match imported {
        Ok(Imported::Done(report)) => {
            job.timings.finish();
            record_import(app, job.source, &report, job.started);
            log::info!("import job {id} ended: done");
            End::Done(report)
        }
        Ok(Imported::Stopped(stopped)) => {
            log::info!("import job {id} ended: stopped in {}", stopped.step.name());
            End::Stopped(stopped)
        }
        Err(e) => {
            let failed = import_error(e);
            log::warn!("import job {id} ended: failed with {}", failed.message);
            End::Failed { error: failed.message, detail: failed.detail }
        }
    }
}

/// The state of import job `id`, which is the job that runs or the last that ended.
pub(super) async fn import_job(State(app): State<Arc<AppCtx>>, Path(id): Path<u64>) -> ApiResult<Response> {
    let view = app.imports.view(id).ok_or_else(|| ApiError::not_found(format!("no import job {id}")))?;
    Ok(Json(&*view).into_response())
}

/// Asks job `id` to stop at the next safe point of its step: `202` with the
/// job, which says it is stopping. A job that ended is answered as it is, `200`.
pub(super) async fn cancel_import_job(State(app): State<Arc<AppCtx>>, Path(id): Path<u64>) -> ApiResult<Response> {
    match app.imports.cancel(id) {
        Some(Cancel::Asked(view)) => Ok((StatusCode::ACCEPTED, Json(&*view)).into_response()),
        Some(Cancel::Over(view)) => Ok(Json(&*view).into_response()),
        None => Err(ApiError::not_found(format!("no import job {id}"))),
    }
}

/// The body of `POST /v1/import/undo`. Strict, as a move's: a misspelt
/// field must not undo an import as if it were not there.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UndoImportRequest {
    pub import_id: i64,
    #[serde(default)]
    pub dry_run: bool,
}

/// Takes an import back for the user, keeping what changed since. Needs no
/// embedder: it only removes rows and gives back links and states.
pub(super) async fn undo_import(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<UndoImportRequest>,
) -> ApiResult<Json<UndoReport>> {
    let backups = clawg_core::paths::backups_dir(&app.runtime.base_dir);
    match portability::undo_import(&app.runtime.store, &app.runtime.imports, &backups, body.import_id, body.dry_run).await {
        Ok(report) => Ok(Json(report)),
        Err(e @ UndoError::UnknownImport(_)) => Err(ApiError::not_found(e.to_string())),
        Err(e @ UndoError::Running(_)) => Err(ApiError::conflict(e.to_string())),
        Err(e @ UndoError::Store(_)) => Err(ApiError::internal(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawg_core::memory::embedder::GatedEmbedder;
    use std::time::Duration;

    use super::super::test_support::{count, daemon_embedding_with, daemon_for_jobs};

    #[test]
    fn too_large_names_the_size_and_the_limit() {
        let refused = too_large(MAX_IMPORT_BYTES + 1);
        assert_eq!(refused.status, StatusCode::BAD_REQUEST);
        assert_eq!(refused.message, format!("the file is {} bytes; at most {MAX_IMPORT_BYTES} are imported", MAX_IMPORT_BYTES + 1));
    }

    #[test]
    fn parse_input_reads_each_source() {
        let clawg = serde_json::json!({
            "kind": "header", "format": "clawg-memories", "version": 1, "exported_at": 0, "clawg": "0.0.0",
            "filter": {"project": null, "agent": null, "include_deleted": false},
        })
        .to_string();
        let engram = serde_json::json!({ "version": "0.2.0", "exported_at": "2025-03-01 10:00:00", "sessions": [], "observations": [], "prompts": [] }).to_string();

        let claude_code = serde_json::json!({ "projects": [{ "name": "-work-app", "entries": [
            { "name": "port.md", "text": "---\nname: Port\n---\n27380", "modified_ms": 1 }
        ] }] })
        .to_string();

        assert!(matches!(parse_input(Source::Clawg, &clawg), Ok(ImportInput::Clawg(_))));
        assert!(matches!(parse_input(Source::Engram, &engram), Ok(ImportInput::Foreign(_))));
        match parse_input(Source::ClaudeCode, &claude_code) {
            Ok(ImportInput::Foreign(export)) => assert_eq!((export.source, export.notes.len()), ("claude-code", 1)),
            Ok(_) => panic!("a clawg export"),
            Err(e) => panic!("{}", e.message),
        }
        for (source, text) in [(Source::Clawg, &engram), (Source::Engram, &clawg), (Source::ClaudeCode, &clawg)] {
            let refused = parse_input(source, text).err().expect("a file of the other tool is refused");
            assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{source:?}");
        }
    }

    #[test]
    fn a_source_is_named_as_the_import_event_logs_it() {
        assert_eq!(Source::Clawg.name(), "clawg");
        assert_eq!(Source::Engram.name(), "engram");
        assert_eq!(Source::ClaudeCode.name(), "claude-code");
    }

    #[test]
    fn a_claude_code_import_reads_a_directory_and_refuses_a_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let memory = dir.path().join("-work-app").join("memory");
        std::fs::create_dir_all(&memory).expect("memory dir");
        std::fs::write(memory.join("port.md"), "---\nname: Port\ntype: reference\n---\n27380").expect("a topic file");
        match read_export(dir.path(), Source::ClaudeCode) {
            Ok(ImportInput::Foreign(export)) => {
                let origins: Vec<&str> = export.notes.iter().map(|n| n.origin.as_str()).collect();
                assert_eq!(origins, ["claude-code:-work-app/port.md"]);
            }
            Ok(_) => panic!("a clawg export"),
            Err(e) => panic!("{}", e.message),
        }
        let refused = |path: &std::path::Path| read_export(path, Source::ClaudeCode).err().expect("refused");
        let file = refused(&memory.join("port.md"));
        assert_eq!((file.status, file.message.as_str()), (StatusCode::BAD_REQUEST, "not a directory"));
        let missing = refused(&dir.path().join("missing"));
        assert_eq!(missing.message, "no such directory");
        let other = tempfile::tempdir().expect("tempdir");
        let none = refused(other.path());
        assert_eq!(none.status, StatusCode::BAD_REQUEST);
        assert!(none.message.contains("holds no Claude Code memory"), "{}", none.message);
        assert!(read_export(&memory.join("port.md"), Source::Engram).is_err(), "engram reads a file of its own");
    }

    /// The rows an import writes in one batch.
    const WRITE_BATCH: usize = 256;

    /// A clawg export of `rows` rows.
    fn rows_file(rows: usize) -> String {
        let header = serde_json::json!({
            "kind": "header", "format": "clawg-memories", "version": 1, "exported_at": 0, "clawg": "0.0.0",
            "filter": {"project": null, "agent": null, "include_deleted": false},
        });
        let row = |id: usize| {
            serde_json::json!({
                "kind": "memory", "id": id, "scope": "agent", "agent_id": "a", "content": format!("the row {id} of the file"),
                "memory_type": "semantic_fact", "source": "explicit", "created_at": 1_000 + id,
                "created_by_type": "agent", "last_modified_at": 1_000 + id, "last_accessed_at": 9_000,
                "access_count": 1, "importance": 5.0, "tier": 2, "status": "active",
            })
        };
        std::iter::once(header).chain((1..=rows).map(row)).map(|line| format!("{line}\n")).collect()
    }

    /// An export of another tool with `notes` notes of 700 words each, which
    /// the daemon splits into pieces.
    fn long_notes_file(notes: usize) -> String {
        let observation = |k: usize| {
            // Words of its own, so no piece of a note is another note's.
            let words: Vec<String> = (0..700).map(|i| format!("n{k}w{i}")).collect();
            serde_json::json!({
                "id": k, "sync_id": format!("n{k}"), "session_id": "s1", "type": "discovery", "title": format!("Note {k}"),
                "content": words.join(" "), "project": null, "scope": "project", "revision_count": 1,
                "duplicate_count": 1, "created_at": "2025-01-01 10:00:00", "updated_at": "2025-01-01 10:00:00",
            })
        };
        let observations: Vec<_> = (0..notes).map(observation).collect();
        serde_json::json!({ "version": "0.2.0", "exported_at": "2025-03-01 10:00:00", "sessions": [], "observations": observations, "prompts": [] })
            .to_string()
    }

    /// Accepts a job over `text` with `control` and answers its number.
    async fn accepted(app: &Arc<AppCtx>, dir: &std::path::Path, text: &str, fields: serde_json::Value, control: ImportControl) -> u64 {
        let path = dir.join("export.jsonl");
        std::fs::write(&path, text).expect("an export file");
        let mut body = fields;
        body["path"] = serde_json::json!(path);
        let request: ImportJobRequest = serde_json::from_value(body).expect("a request");
        let accepted = accept_job(app.clone(), request, control).await.expect("the job is accepted");
        serde_json::to_value(&*accepted).expect("a view")["job_id"].as_u64().expect("a job id")
    }

    /// The state of job `id` as `GET /v1/imports/{id}` shows it.
    fn job_state(app: &AppCtx, id: u64) -> serde_json::Value {
        serde_json::to_value(&*app.imports.view(id).expect("the job")).expect("a view")
    }

    /// Accepts a job over `text` with `control`, waits for it to end and
    /// answers its state as `GET /v1/imports/{id}` shows it.
    async fn job_over(app: &Arc<AppCtx>, dir: &std::path::Path, text: &str, fields: serde_json::Value, control: ImportControl) -> serde_json::Value {
        let id = accepted(app, dir, text, fields, control).await;
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                let view = job_state(app, id);
                if ["done", "failed", "cancelled", "interrupted"].contains(&view["phase"].as_str().unwrap_or("")) {
                    return view;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the job ended")
    }

    /// A control that is asked to stop as `step` begins.
    fn cancelled_in(step: Step) -> ImportControl {
        let control = ImportControl::default();
        let raise = control.clone();
        control.on_step(move |entered| {
            if entered == step {
                raise.stop(portability::StopCause::Cancel);
            }
        });
        control
    }

    /// How many notes have rows, and how many different numbers of rows a
    /// note has: one, when every note was written whole.
    async fn notes_stored(app: &AppCtx) -> (i64, i64) {
        app.runtime
            .store
            .conn_for_test()
            .call(|c| {
                c.query_row(
                    "SELECT COUNT(DISTINCT created_by_ref), COUNT(DISTINCT n) FROM (SELECT created_by_ref, COUNT(*) AS n FROM memory GROUP BY created_by_ref)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await
            .expect("rows by note")
    }

    #[tokio::test]
    async fn a_job_stopped_in_writing_counts_exactly_the_committed_batches() {
        let (app, dir) = daemon_for_jobs().await;
        let job = job_over(&app, dir.path(), &rows_file(WRITE_BATCH + 1), serde_json::json!({}), cancelled_in(Step::Write)).await;

        assert_eq!((job["phase"].as_str(), job["stopped_in"].as_str()), (Some("cancelled"), Some("write")), "{job}");
        let report = &job["report"];
        assert_eq!((report["inserted"].clone(), report["rows"].clone()), (serde_json::json!(WRITE_BATCH), serde_json::json!(WRITE_BATCH + 1)), "{job}");
        assert_eq!((job["rows_written"].clone(), job["notes_written"].clone()), (serde_json::json!(WRITE_BATCH), serde_json::json!(WRITE_BATCH)), "{job}");
        assert_eq!((job["rows"].clone(), job["notes"].clone()), (serde_json::json!(WRITE_BATCH + 1), serde_json::json!(WRITE_BATCH + 1)), "{job}");
        assert_eq!(job["import_id"], report["import_id"], "{job}");
        assert_eq!(count(&app, "SELECT COUNT(*) FROM memory").await, WRITE_BATCH as i64);
    }

    #[tokio::test]
    async fn a_job_stopped_in_writing_leaves_whole_notes_and_frees_its_number() {
        let (app, dir) = daemon_for_jobs().await;
        app.runtime.tokenizer.set(clawg_core::memory::chunker::word_tokenizer()).ok();
        let fields = serde_json::json!({ "from": "engram" });
        let job = job_over(&app, dir.path(), &long_notes_file(40), fields, cancelled_in(Step::Write)).await;

        assert_eq!((job["phase"].as_str(), job["stopped_in"].as_str()), (Some("cancelled"), Some("write")), "{job}");
        let (written, notes) = (job["notes_written"].as_i64().unwrap(), job["notes"].as_i64().unwrap());
        assert!(0 < written && written < notes, "a batch of whole notes was written and not all of them: {job}");
        let (stored, sizes) = notes_stored(&app).await;
        assert_eq!((stored, sizes), (written, 1), "every note written has all its rows: {job}");
        assert_eq!(count(&app, "SELECT COUNT(*) FROM memory").await, job["rows_written"].as_i64().unwrap());
        assert_eq!(count(&app, "SELECT COUNT(*) FROM memory_import WHERE finished_at IS NULL").await, 1, "the journal stays open");

        let backups = clawg_core::paths::backups_dir(&app.runtime.base_dir);
        let undone = portability::undo_import(&app.runtime.store, &app.runtime.imports, &backups, job["import_id"].as_i64().unwrap(), false).await;
        assert!(undone.is_ok(), "its number is free: {:?}", undone.err());
    }

    #[tokio::test]
    async fn importing_again_finishes_a_stopped_job() {
        let (app, dir) = daemon_for_jobs().await;
        let text = rows_file(WRITE_BATCH + 1);
        let stopped = job_over(&app, dir.path(), &text, serde_json::json!({}), cancelled_in(Step::Write)).await;
        assert_eq!(stopped["phase"], "cancelled", "{stopped}");

        let again = job_over(&app, dir.path(), &text, serde_json::json!({}), ImportControl::default()).await;

        assert_eq!(again["phase"], "done", "{again}");
        assert_eq!((again["report"]["present"].clone(), again["report"]["inserted"].clone()), (serde_json::json!(WRITE_BATCH), serde_json::json!(1)), "{again}");
        assert_eq!(count(&app, "SELECT COUNT(*) FROM memory").await, WRITE_BATCH as i64 + 1);
    }

    #[tokio::test]
    async fn undoing_a_stopped_job_works() {
        let (app, dir) = daemon_for_jobs().await;
        let stopped = job_over(&app, dir.path(), &rows_file(WRITE_BATCH + 1), serde_json::json!({}), cancelled_in(Step::Write)).await;
        let import_id = stopped["import_id"].as_i64().expect("a number");

        let backups = clawg_core::paths::backups_dir(&app.runtime.base_dir);
        let undone = portability::undo_import(&app.runtime.store, &app.runtime.imports, &backups, import_id, false).await.expect("an undo, not a refusal to touch a running import");

        assert_eq!(undone.removed.len(), WRITE_BATCH);
        assert_eq!(count(&app, "SELECT COUNT(*) FROM memory").await, 0);
    }

    #[tokio::test]
    async fn a_job_stopped_before_its_first_write_has_no_report() {
        let (app, dir) = daemon_for_jobs().await;
        let job = job_over(&app, dir.path(), &rows_file(3), serde_json::json!({}), cancelled_in(Step::Embed)).await;

        assert_eq!((job["phase"].as_str(), job["stopped_in"].as_str()), (Some("cancelled"), Some("embed")), "{job}");
        assert!(job.get("report").is_none() && job.get("import_id").is_none(), "{job}");
        assert_eq!((job["rows"].clone(), job["rows_embedded"].clone()), (serde_json::json!(3), serde_json::json!(0)), "{job}");
        assert_eq!(count(&app, "SELECT COUNT(*) FROM memory_import").await, 0);
    }

    /// A control whose job 1 is cancelled through the registry, as a `DELETE`
    /// does, as `step` begins.
    fn deleted_in(app: &Arc<AppCtx>, step: Step) -> ImportControl {
        let (control, app) = (ImportControl::default(), app.clone());
        control.on_step(move |entered| {
            if entered == step {
                assert!(matches!(app.imports.cancel(1), Some(Cancel::Asked(_))), "the job is active");
            }
        });
        control
    }

    #[tokio::test]
    async fn a_delete_during_links_ends_done() {
        let (app, dir) = daemon_for_jobs().await;
        let job = job_over(&app, dir.path(), &rows_file(3), serde_json::json!({}), deleted_in(&app, Step::Links)).await;

        assert_eq!(job["phase"], "done", "{job}");
        assert!(job.get("stopped_in").is_none() && job.get("stopping").is_none(), "{job}");
        assert_eq!((job["report"]["inserted"].clone(), job["rows_written"].clone()), (serde_json::json!(3), serde_json::json!(3)), "{job}");
        assert_eq!(count(&app, "SELECT COUNT(*) FROM memory").await, 3);
        assert_eq!(count(&app, "SELECT COUNT(*) FROM memory_import WHERE finished_at IS NOT NULL").await, 1, "the links were written");
    }

    #[tokio::test]
    async fn a_delete_during_snapshot_ends_cancelled_with_the_snapshot_and_no_rows() {
        let (app, dir) = daemon_for_jobs().await;
        let job = job_over(&app, dir.path(), &rows_file(3), serde_json::json!({}), deleted_in(&app, Step::Snapshot)).await;

        assert_eq!((job["phase"].as_str(), job["stopped_in"].as_str()), (Some("cancelled"), Some("snapshot")), "{job}");
        assert!(job.get("report").is_none() && job.get("import_id").is_none(), "{job}");
        assert_eq!(count(&app, "SELECT COUNT(*) FROM memory").await, 0);
        assert_eq!(count(&app, "SELECT COUNT(*) FROM memory_import").await, 0);
        let backups = clawg_core::paths::backups_dir(&app.runtime.base_dir);
        let snapshots: Vec<_> = std::fs::read_dir(&backups).expect("the backups directory").flatten().map(|e| e.file_name()).collect();
        assert!(snapshots.iter().any(|name| name.to_string_lossy().ends_with("-pre-import.db")), "{snapshots:?}");
    }

    const GRACE: Duration = crate::daemon::serve::HTTP_GRACE;

    #[tokio::test]
    async fn shutdown_stops_a_job_in_embedding_within_the_grace() {
        let (embedder, gate) = GatedEmbedder::new();
        let (app, dir) = daemon_embedding_with(Some(Arc::new(embedder))).await;
        let id = accepted(&app, dir.path(), &rows_file(3), serde_json::json!({}), ImportControl::default()).await;
        gate.entered().await;

        app.imports.close();

        tokio::time::timeout(GRACE, app.imports.settled()).await.expect("the job ends within the grace");
        let job = job_state(&app, id);
        assert_eq!((job["phase"].as_str(), job["stopped_in"].as_str()), (Some("interrupted"), Some("embed")), "{job}");
        assert!(job.get("abandoned").is_none() && job.get("report").is_none(), "{job}");
        assert_eq!(count(&app, "SELECT COUNT(*) FROM memory").await, 0);
        assert_eq!(count(&app, "SELECT COUNT(*) FROM memory_import").await, 0);
    }

    #[tokio::test]
    async fn shutdown_stops_a_job_in_writing_between_whole_notes() {
        let (app, dir) = daemon_for_jobs().await;
        app.runtime.tokenizer.set(clawg_core::memory::chunker::word_tokenizer()).ok();
        let control = ImportControl::default();
        control.on_step({
            let app = app.clone();
            move |step| {
                if step == Step::Write {
                    app.imports.close();
                }
            }
        });
        let id = accepted(&app, dir.path(), &long_notes_file(40), serde_json::json!({ "from": "engram" }), control).await;

        tokio::time::timeout(GRACE, app.imports.settled()).await.expect("the job ends within the grace");
        let job = job_state(&app, id);
        assert_eq!((job["phase"].as_str(), job["stopped_in"].as_str()), (Some("interrupted"), Some("write")), "{job}");
        assert!(job.get("abandoned").is_none(), "{job}");
        let (written, notes) = (job["notes_written"].as_i64().unwrap(), job["notes"].as_i64().unwrap());
        assert!(0 < written && written < notes, "a batch of whole notes was written and not all of them: {job}");
        assert_eq!(notes_stored(&app).await, (written, 1), "every note written has all its rows: {job}");
        assert!(job["report"]["inserted"].as_i64().unwrap() > 0, "{job}");
    }

    // The clock is paused so the grace runs out without the test waiting for
    // it; the connection's thread is a real one, which the clock does not see.
    #[tokio::test(start_paused = true)]
    async fn an_abandoned_job_sends_nothing_to_the_connection() {
        let (embedder, gate) = GatedEmbedder::new();
        let (app, dir) = daemon_embedding_with(Some(Arc::new(embedder))).await;
        let (release, held) = std::sync::mpsc::channel::<()>();
        let held = std::sync::Mutex::new(Some(held));
        let control = ImportControl::default();
        let queue = app.runtime.store.clone();
        control.on_step(move |step| {
            if step != Step::Snapshot {
                return;
            }
            if let Some(held) = held.lock().unwrap().take() {
                // The snapshot queues behind this call, which holds the
                // connection until the test lets go; polling it once sends it.
                let hold = move |_: &mut rusqlite::Connection| {
                    let _ = held.recv();
                    Ok::<_, rusqlite::Error>(())
                };
                let mut call = Box::pin(queue.conn_for_test().call(hold));
                let _ = std::future::Future::poll(call.as_mut(), &mut std::task::Context::from_waker(std::task::Waker::noop()));
            }
        });
        let id = accepted(&app, dir.path(), &rows_file(3), serde_json::json!({}), control.clone()).await;
        gate.entered().await;
        gate.release(1);
        while control.progress().step != Some(Step::Snapshot) {
            tokio::task::yield_now().await;
        }

        // No stop is raised, so only the abort keeps this job from writing:
        // one asked to stop would end by itself after the snapshot.
        assert!(tokio::time::timeout(GRACE, app.imports.settled()).await.is_err(), "the held connection keeps the job past the grace");
        app.imports.abandon(TimestampMs::now().as_millis_i64());

        let job = job_state(&app, id);
        assert_eq!((job["phase"].as_str(), job["stopped_in"].as_str(), job["abandoned"].as_bool()), (Some("interrupted"), Some("snapshot"), Some(true)), "{job}");
        release.send(()).unwrap();

        // The snapshot already sent ran before this call and answered before
        // it did, so a job that was not cut off has been woken by now and
        // sends its first batch while the test yields.
        count(&app, "SELECT COUNT(*) FROM memory").await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert_eq!(count(&app, "SELECT COUNT(*) FROM memory").await, 0);
        assert_eq!(count(&app, "SELECT COUNT(*) FROM memory_import").await, 0);
        assert_eq!(job_state(&app, id)["phase"], "interrupted", "the task ending changes nothing");
    }

    #[tokio::test]
    async fn a_job_import_during_shutdown_is_refused_with_a_retryable_503() {
        let (app, dir) = daemon_for_jobs().await;
        app.imports.close();
        let path = dir.path().join("export.jsonl");
        std::fs::write(&path, rows_file(1)).expect("an export file");
        let request: ImportJobRequest = serde_json::from_value(serde_json::json!({ "path": path })).expect("a request");

        let refused = accept_job(app.clone(), request, ImportControl::default()).await.expect_err("refused");

        assert_eq!((refused.status, refused.message.as_str(), refused.retryable), (StatusCode::SERVICE_UNAVAILABLE, "shutting_down", Some(true)));
        assert!(refused.job_id.is_none());
    }
}
