//! The project routes: register, list, rename, merge, delete, keep apart, resolve, answer a suggestion and locate a folder.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use clawg_core::memory::identity::{Disk, Ground, ProjectIdentity};
use clawg_core::memory::merge::{merge_core, MergeError, MergeReport, undo_merge_core};
use clawg_core::memory::project::{check_root, register_project, RegisterError, RegisterOutcome, Registration};
use clawg_core::memory::project_delete::{delete_core, DeleteError, DeleteReport, undo_delete_core};
use clawg_core::memory::registry::{
    answer_suggestion_core, Answered, AnswerError, Born, check_folder, find_project, keep_apart,
    KeepApartError, KeptApart, list_deletes, list_projects, ListedDelete, ListedProject,
    locate_folder, Located, LocateError, Named, Project, rename_project, RenameError,
    Resolution as ProjectResolution, resolve_project, ResolveError as ProjectResolveError,
    ResolveOptions, SuggestionAnswer, UserAnswer,
};
use clawg_core::memory::subfolder::{declare_subfolder, SubfolderError};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::docs::spawn_doc_refresh;
use super::sessions::sessions_follow;
use super::{ApiError, ApiResult, AppCtx, embed_failure, ready_embedder};

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    /// The canonical project root, as the bridge resolved it.
    pub root_path: String,
    /// Ids the same project was saved under before paths were resolved.
    #[serde(default)]
    pub legacy_ids: Vec<String>,
    /// Whether the daemon may read the root's files. False when missing, so a
    /// caller that does not say only labels memories with the root.
    #[serde(default)]
    pub index_docs: bool,
    /// The caller reads and writes under the ids this answer gives. False
    /// when missing: a caller from before derives the id from the root.
    #[serde(default)]
    pub assigned: bool,
    /// The absolute path the caller works in, inside the root: a subfolder
    /// declared there leads to another project. Only with `assigned`.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RegisteredBody {
    pub project_id: String,
    pub root_id: String,
    pub rekeyed: usize,
}

/// Records a project root and joins into its project the rows saved under
/// its legacy ids and, for a clone of a remote another root registered, its
/// own.
///
/// The ids are settled here from `root_path` and the remote its git
/// directory names, never taken from the caller, so a row in `project` can
/// never name a root it was not given to.
pub(super) async fn register(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<RegisterRequest>,
) -> ApiResult<Json<RegisteredBody>> {
    let path = body.path.as_deref().filter(|_| body.assigned);
    let outcome = register_root(&app, &body.root_path, path, &body.legacy_ids, body.index_docs, body.assigned).await?;
    let project_id = outcome.subproject.unwrap_or(outcome.project_id);
    Ok(Json(RegisteredBody { project_id, root_id: outcome.root_id, rekeyed: outcome.rekeyed }))
}

/// The status a refused registration of `root_path` answers with. A
/// collision names the root as the caller sent it, and tells a root stored
/// under another id from one whose id another root holds.
fn register_refused(e: RegisterError, root_path: &str) -> ApiError {
    match e {
        RegisterError::EmptyRoot | RegisterError::NotADirectory(_) | RegisterError::InvalidLegacyId(_) => ApiError::bad_request(e.to_string()),
        RegisterError::RootCollision { id, existing } if existing == root_path => ApiError {
            detail: Some(format!("{root_path} derives the project id {id}, but the daemon gives that root another one; a newer bridge uses it")),
            ..ApiError::conflict("project_root_collision")
        },
        RegisterError::RootCollision { id, existing } => ApiError {
            detail: Some(format!("{root_path} gets the project id {id}, which already belongs to {existing}")),
            ..ApiError::conflict("project_root_collision")
        },
        RegisterError::NoFreeId(_) => ApiError { detail: Some(e.to_string()), ..ApiError::conflict("no_free_project_id") },
        RegisterError::Store(_) => ApiError::internal(e),
    }
}

/// Registers `root_path` as [`register`] does, for it and for what places a
/// folder in a project. `path`, where the caller works, is measured from the
/// top of its checkout on the thread that reads the checkout; one that is not
/// absolute or does not lead to `root_path` is left out, as a caller from
/// before sends none.
async fn register_root(
    app: &Arc<AppCtx>,
    root_path: &str,
    path: Option<&str>,
    legacy_ids: &[String],
    index_docs: bool,
    assigned: bool,
) -> ApiResult<RegisterOutcome> {
    let root = check_root(root_path, index_docs, std::path::Path::is_dir).map_err(|e| register_refused(e, root_path))?;
    // A caller that derives its id joins no remote, but what is at the root
    // still tells whether it is the repository its folder was given to.
    let (checkout, subpath) = {
        let (root, path) = (root.to_path_buf(), path.map(std::path::PathBuf::from));
        tokio::task::spawn_blocking(move || {
            let subpath = path.as_ref().filter(|p| p.is_absolute()).and_then(|p| Disk.subpath(&root, p));
            (Disk.checkout(&root), subpath)
        })
        .await
        .unwrap_or_default()
    };
    if let (Some(path), None) = (path, &subpath) {
        log::info!("{path} is not in the checkout of {root_path}; it is registered as the root alone");
    }
    let registration = Registration { root, checkout, legacy_ids, index_docs, assigned, subpath: subpath.as_deref() };
    let backups = clawg_core::paths::backups_dir(&app.runtime.base_dir);
    let outcome = register_project(&app.runtime.store, &app.runtime.workspaces(), &backups, registration, Arc::new(Disk))
        .await
        .map_err(|e| register_refused(e, root_path))?;
    if outcome.rekeyed > 0 {
        log::info!(
            "{} joined {} rows of its earlier ids into project {}{}{}",
            root.display(),
            outcome.rekeyed,
            outcome.project_id,
            outcome.move_id.map(|n| format!("; `clawg projects move --undo {n}` moves its memories back")).unwrap_or_default(),
            outcome.snapshot.as_ref().map(|s| format!("; the store before is {}", s.display())).unwrap_or_default()
        );
    }
    // A bridge registers its root as its session starts, so the model that
    // went away while nobody worked starts loading before the first search.
    if let Some(embedder) = app.runtime.embedder.get() {
        embedder.wake();
    }
    // Only a caller that asks: the hooks register an accepted root on every
    // prompt, and `search_project` already refreshes an index gone stale.
    if index_docs && outcome.index_docs {
        spawn_doc_refresh(app.clone(), outcome.root_id.clone());
    }
    Ok(outcome)
}

#[derive(Debug, Serialize)]
pub struct ProjectsBody {
    pub projects: Vec<ListedProject>,
    /// Every delete nobody undid or purged.
    pub deletes: Vec<ListedDelete>,
}

/// Every project, with or without a folder, with what leads to it, how many
/// memories it holds and the suggestions about it nobody answered.
pub(super) async fn projects(State(app): State<Arc<AppCtx>>) -> ApiResult<Json<ProjectsBody>> {
    let projects = list_projects(&app.runtime.store, Arc::new(Disk)).await.map_err(ApiError::internal)?;
    let deletes = list_deletes(&app.runtime.store).await.map_err(ApiError::internal)?;
    Ok(Json(ProjectsBody { projects, deletes }))
}

#[derive(Debug, Deserialize)]
pub struct RenameRequest {
    pub name: String,
}

/// Names a project and marks it reviewed. Its id, and with it what was
/// saved under it, stays.
pub(super) async fn rename(
    State(app): State<Arc<AppCtx>>,
    Path(id): Path<String>,
    Json(body): Json<RenameRequest>,
) -> ApiResult<Json<Project>> {
    let renamed = rename_project(&app.runtime.store, &id, &body.name).await.map_err(|e| match e {
        RenameError::NotFound(_) => ApiError::not_found(e.to_string()),
        RenameError::Name(_) => ApiError::bad_request(e.to_string()),
        RenameError::Taken { .. } => ApiError { detail: Some(e.to_string()), ..ApiError::conflict("name_taken") },
        RenameError::Merged { .. } => ApiError { detail: Some(e.to_string()), ..ApiError::conflict("project_merged") },
        RenameError::Deleted { .. } => ApiError { detail: Some(e.to_string()), ..ApiError::conflict("project_deleted") },
        RenameError::Store(_) => ApiError::internal(e),
    })?;
    Ok(Json(renamed))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MergeRequest {
    source: Option<String>,
    target: Option<String>,
    /// The number of a merge to undo, instead of a source and a target.
    undo: Option<i64>,
    #[serde(default)]
    dry_run: bool,
}

/// The status a refused merge answers with: a project or a merge that is not
/// there is missing, a merge of a project into itself is the caller's
/// mistake, and a tombstone, an undo out of order or saves still being stored
/// are conflicts, the last one worth trying again.
fn merge_error(e: MergeError) -> ApiError {
    let conflict = |code: &str, e: &MergeError| ApiError { detail: Some(e.to_string()), ..ApiError::conflict(code) };
    match e {
        MergeError::UnknownProject(_) | MergeError::UnknownMerge(_) => ApiError::not_found(e.to_string()),
        MergeError::SameProject(_) => ApiError::bad_request(e.to_string()),
        MergeError::Merged { .. } => conflict("project_merged", &e),
        MergeError::Deleted { .. } => conflict("project_deleted", &e),
        MergeError::SavesPending(_) => ApiError { retryable: Some(true), ..conflict("saves_pending", &e) },
        MergeError::Undone(_) => conflict("merge_undone", &e),
        MergeError::MergedLater { .. } => conflict("merged_later", &e),
        MergeError::Store(e) => ApiError::internal(e),
    }
}

/// Merges one project into another, or undoes a merge, for the user. Needs no
/// embedder.
pub(super) async fn merge_projects(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<MergeRequest>,
) -> ApiResult<Json<MergeReport>> {
    let backups = clawg_core::paths::backups_dir(&app.runtime.base_dir);
    let (store, workspaces) = (&app.runtime.store, app.runtime.workspaces());
    let report = match (body.undo, body.source, body.target) {
        (Some(merge_id), None, None) => undo_merge_core(store, &workspaces, &backups, merge_id, body.dry_run).await,
        (None, Some(source), Some(target)) => {
            let merged = merge_core(store, &workspaces, &backups, &source, &target, body.dry_run).await;
            if let Ok(merged) = &merged {
                sessions_follow(&app, merged);
            }
            merged
        }
        _ => return Err(ApiError::bad_request("name a source and a target, or a merge to undo")),
    };
    report.map(Json).map_err(merge_error)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeleteRequest {
    project_id: Option<String>,
    /// Delete the project with its rows: without `confirm` this answers the
    /// report and its code.
    #[serde(default)]
    with_memories: bool,
    /// The code of the report the person saw.
    confirm: Option<String>,
    /// The number of a delete to undo, instead of a project.
    undo: Option<i64>,
    #[serde(default)]
    dry_run: bool,
}

/// The status a refused delete or undo answers with. A delete that needs the
/// model for the vectors of the rows it restores answers as every route that
/// needs it does.
fn delete_error(app: &AppCtx, e: DeleteError) -> ApiError {
    let conflict = |code: &str, e: &DeleteError| ApiError { detail: Some(e.to_string()), ..ApiError::conflict(code) };
    match e {
        DeleteError::ConfirmWithoutRows => ApiError::bad_request(e.to_string()),
        DeleteError::UnknownProject(_) | DeleteError::UnknownDelete(_) => ApiError::not_found(e.to_string()),
        DeleteError::Merged { .. } => conflict("project_merged", &e),
        DeleteError::Deleted { .. } => conflict("project_deleted", &e),
        DeleteError::SavesPending(_) => ApiError { retryable: Some(true), ..conflict("saves_pending", &e) },
        DeleteError::Undone(_) => conflict("delete_undone", &e),
        DeleteError::Purged(_) => conflict("delete_purged", &e),
        DeleteError::DeletedLater { .. } => conflict("deleted_later", &e),
        DeleteError::NeedsModel => ready_embedder(app).err().unwrap_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "embedder_not_ready")),
        DeleteError::Embed(e) => embed_failure(e),
        DeleteError::Store(_) => ApiError::internal(e),
    }
}

/// Deletes a project, in two steps when it takes its rows, or undoes a
/// delete, for the user. Deleting needs no embedder; an undo needs it only
/// when it has rows to restore, so it is handed the model if there is one and
/// asks for it only then.
pub(super) async fn delete_projects(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<DeleteRequest>,
) -> ApiResult<Json<DeleteReport>> {
    let backups = clawg_core::paths::backups_dir(&app.runtime.base_dir);
    let (store, workspaces) = (&app.runtime.store, app.runtime.workspaces());
    let report = match body {
        DeleteRequest { undo: Some(delete_id), project_id: None, with_memories: false, confirm: None, dry_run } => {
            let embedder = app.runtime.embedder.get().cloned();
            undo_delete_core(store, embedder.as_deref(), &backups, delete_id, dry_run).await
        }
        DeleteRequest { undo: None, project_id: Some(project_id), with_memories, confirm, dry_run: false } => {
            let deleted = delete_core(store, &workspaces, &backups, &project_id, with_memories, confirm.as_deref()).await;
            // So that a deleted project leaves no trace: its sessions go too.
            if let Ok(DeleteReport { delete_id: Some(_), project_id, .. }) = &deleted {
                app.runtime.metrics.sessions().delete_project(project_id);
            }
            deleted
        }
        _ => return Err(ApiError::bad_request("name a project to delete, or a delete to undo (dry_run goes with undo)")),
    };
    report.map(Json).map_err(|e| delete_error(&app, e))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KeepApartRequest {
    project_id: String,
    other_id: String,
}

/// The user's answer that two projects are not one: no suggestion between
/// them is made again.
pub(super) async fn keep_projects_apart(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<KeepApartRequest>,
) -> ApiResult<Json<KeptApart>> {
    keep_apart(&app.runtime.store, &body.project_id, &body.other_id).await.map(Json).map_err(kept_apart_error)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProjectResolveRequest {
    /// An id or a name.
    project: String,
    #[serde(default)]
    create: bool,
    #[serde(default)]
    new_project: bool,
    #[serde(default)]
    born: Born,
}

/// What a reference to a project leads to, as an agent or the user names
/// one, creating a project without a folder when asked and nothing has or
/// resembles the name. An answer that read candidates is a 200, since they
/// are what the caller asks the person about.
pub(super) async fn resolve_projects(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<ProjectResolveRequest>,
) -> ApiResult<Json<ProjectResolution>> {
    let options = ResolveOptions { create: body.create, new_project: body.new_project, born: body.born };
    resolve_project(&app.runtime.store, &body.project, options).await.map(Json).map_err(|e| match e {
        ProjectResolveError::NewProjectWithoutCreate | ProjectResolveError::Name(_) => ApiError::bad_request(e.to_string()),
        ProjectResolveError::NoFreeId(_) => ApiError { detail: Some(e.to_string()), ..ApiError::conflict("no_free_project_id") },
        ProjectResolveError::Store(_) => ApiError::internal(e),
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AnswerRequest {
    project_id: String,
    other_id: String,
    answer: String,
    #[serde(default)]
    user_answered: bool,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AnsweredBody {
    pub project_id: String,
    pub other_id: String,
    pub answer: SuggestionAnswer,
    /// The merge of `other_id` into `project_id`, which `--undo` takes back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge: Option<AnsweredMerge>,
}

/// A [`MergeReport`] with `trajectories` and `skills`, always 0, which a
/// bridge of a version before V32 requires in the merge an answer made.
#[derive(Debug, Serialize)]
pub struct AnsweredMerge {
    #[serde(flatten)]
    pub report: MergeReport,
    pub trajectories: usize,
    pub skills: usize,
}

/// The status of a refused check of two projects, as `keep-apart` answers it.
fn kept_apart_error(e: KeepApartError) -> ApiError {
    match e {
        KeepApartError::NotFound(_) => ApiError::not_found(e.to_string()),
        KeepApartError::Same(_) => ApiError::bad_request(e.to_string()),
        KeepApartError::Merged { .. } => ApiError { detail: Some(e.to_string()), ..ApiError::conflict("project_merged") },
        KeepApartError::Deleted { .. } => ApiError { detail: Some(e.to_string()), ..ApiError::conflict("project_deleted") },
        KeepApartError::Store(_) => ApiError::internal(e),
    }
}

/// The person's answer to a pending suggestion, as an agent relays it:
/// `apart` keeps the two from being suggested again, `merge` merges
/// `other_id` into `project_id` as `POST /v1/projects/merge` does. Only a
/// suggestion nobody answered is answered: an agent merges no projects
/// nobody suggested.
pub(super) async fn answer_suggestion(
    State(app): State<Arc<AppCtx>>,
    Json(body): Json<AnswerRequest>,
) -> ApiResult<Json<AnsweredBody>> {
    let answer = UserAnswer::new(&body.answer, body.user_answered, body.reason.as_deref()).map_err(answer_refused)?;
    let backups = clawg_core::paths::backups_dir(&app.runtime.base_dir);
    let answered = answer_suggestion_core(&app.runtime.store, &app.runtime.workspaces(), &backups, &body.project_id, &body.other_id, answer)
        .await
        .map_err(answer_refused)?;
    let (answer, merge) = match answered {
        Answered::Apart => (SuggestionAnswer::Apart, None),
        Answered::Merged(merged) => {
            sessions_follow(&app, &merged);
            (SuggestionAnswer::Merge, Some(AnsweredMerge { report: merged, trajectories: 0, skills: 0 }))
        }
    };
    Ok(Json(AnsweredBody { project_id: body.project_id, other_id: body.other_id, answer, merge }))
}

/// The status a refused answer to a suggestion answers with: the checks of
/// the two projects as `keep-apart` answers them, a refused merge as
/// `projects merge` does.
fn answer_refused(e: AnswerError) -> ApiError {
    match e {
        AnswerError::UnknownAnswer(_) | AnswerError::NeedsUser => ApiError::bad_request(e.to_string()),
        AnswerError::NoSuggestion { .. } => ApiError { detail: Some(e.to_string()), ..ApiError::not_found("no_suggestion") },
        AnswerError::Pair(e) => kept_apart_error(e),
        AnswerError::Merge(e) => merge_error(e),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LocateRequest {
    /// An id or a name.
    project: String,
    root_path: String,
    /// `root_path` is a subfolder of a repository, declared as the project,
    /// rather than a folder whose root is located.
    #[serde(default)]
    subfolder: bool,
}

#[derive(Debug, Serialize)]
pub struct LocatedBody {
    /// The project the folder leads to now.
    pub project_id: String,
    pub root_id: String,
    /// Whether it led there already, and nothing was done.
    pub already: bool,
    /// The merge of the folder's own project into it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge: Option<MergeReport>,
    /// For a subfolder: the project of its repository.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<String>,
    /// For a subfolder: where it is in its checkout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subpath: Option<String>,
    /// For a subfolder: the repository's project was named, and its declaration went.
    #[serde(default)]
    pub removed: bool,
    /// For a removed declaration: the project the subfolder led to, which
    /// keeps what was saved there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
}

/// Makes the root of `root_path` lead to a project: it is registered as a
/// bridge registers it, and the project it gets, when that is another one
/// and this is its only folder, is merged into the named one. A project
/// with other folders would take them along, which nobody named.
pub(super) async fn locate_project(State(app): State<Arc<AppCtx>>, Json(body): Json<LocateRequest>) -> ApiResult<Json<LocatedBody>> {
    let folder = std::path::Path::new(&body.root_path);
    check_folder(folder, std::path::Path::is_dir).map_err(located_refused)?;
    let subfolder = if body.subfolder { Some(subfolder_of(&body.root_path).await?) } else { None };
    let store = &app.runtime.store;
    let target = match find_project(store, &body.project).await.map_err(ApiError::internal)? {
        Named::Project(id) => id,
        Named::Unknown => return Err(ApiError::not_found(format!("no project has the id or the name {:?}", body.project))),
        Named::Ambiguous(ids) => {
            return Err(ApiError {
                detail: Some(format!("{:?} is the name of {}: name one of them by its id", body.project, ids.join(", "))),
                ..ApiError::conflict("project_ambiguous")
            })
        }
    };
    if let Some((root, subpath)) = subfolder {
        return declare(&app, &root, &subpath, &target).await.map(Json);
    }
    let registered = register_root(&app, &body.root_path, None, &[], false, true).await?;
    let backups = clawg_core::paths::backups_dir(&app.runtime.base_dir);
    let located = locate_folder(store, &app.runtime.workspaces(), &backups, folder, &registered.project_id, &target)
        .await
        .map_err(located_refused)?;
    let merge = match located {
        Located::Already => None,
        Located::Merged(merged) => {
            sessions_follow(&app, &merged);
            Some(merged)
        }
    };
    Ok(Json(LocatedBody {
        already: merge.is_none(),
        project_id: target,
        root_id: registered.root_id,
        merge,
        repository_id: None,
        subpath: None,
        removed: false,
        previous: None,
    }))
}

/// The status a folder that was not located answers with: a shared folder
/// is a conflict, and a refused merge answers as `POST /v1/projects/merge`.
fn located_refused(e: LocateError) -> ApiError {
    match e {
        LocateError::NotADirectory(_) => ApiError::bad_request(e.to_string()),
        LocateError::FolderShared { .. } => ApiError { detail: Some(e.to_string()), ..ApiError::conflict("folder_shared") },
        LocateError::Merge(e) => merge_error(e),
        LocateError::Store(e) => ApiError::internal(e),
    }
}

/// The root of the checkout `path` is in and the subpath of `path` there,
/// read on a blocking thread; a 400 for a path at the top of a checkout or
/// outside one, which has no subfolder to declare.
async fn subfolder_of(path: &str) -> ApiResult<(String, String)> {
    let asked = std::path::PathBuf::from(path);
    let found = tokio::task::spawn_blocking(move || {
        let identity = ProjectIdentity::resolve(&asked);
        let subpath = Disk.subpath(&identity.root, &identity.path);
        (identity.root, subpath)
    })
    .await
    .map_err(ApiError::internal)?;
    match found {
        (root, Some(subpath)) if !subpath.is_empty() => Ok((root.to_string_lossy().into_owned(), subpath)),
        _ => Err(ApiError::bad_request(format!(
            "{path} is the top of a checkout or outside a repository, so it is no subfolder of one; \
             `locate` without `subfolder` locates the folder's root"
        ))),
    }
}

/// Declares `subpath` of the repository at `root` as project `target`, after
/// registering the root as a bridge does.
async fn declare(app: &Arc<AppCtx>, root: &str, subpath: &str, target: &str) -> ApiResult<LocatedBody> {
    let registered = register_root(app, root, None, &[], false, true).await?;
    let backups = clawg_core::paths::backups_dir(&app.runtime.base_dir);
    let declared = declare_subfolder(&app.runtime.store, &app.runtime.workspaces(), &backups, &registered.project_id, subpath, target)
        .await
        .map_err(|e| match e {
            SubfolderError::UnknownProject(_) => ApiError::not_found(e.to_string()),
            SubfolderError::Merged { .. } => ApiError { detail: Some(e.to_string()), ..ApiError::conflict("project_merged") },
            SubfolderError::Deleted { .. } => ApiError { detail: Some(e.to_string()), ..ApiError::conflict("project_deleted") },
            SubfolderError::Shared { .. } => ApiError { detail: Some(e.to_string()), ..ApiError::conflict("folder_shared") },
            SubfolderError::Merge(e) => merge_error(e),
            SubfolderError::Store(_) => ApiError::internal(e),
        })?;
    Ok(LocatedBody {
        project_id: declared.project_id,
        root_id: registered.root_id,
        already: declared.already,
        merge: declared.merge,
        repository_id: Some(declared.repository_id),
        subpath: Some(declared.subpath),
        removed: declared.removed,
        previous: declared.previous,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    use super::super::test_support::{answer_of, daemon_for_jobs, run_sql, status_and_body};

    /// The status and the whole body `register` answers a refused
    /// registration with, for every rule and in the order it checks them.
    #[tokio::test]
    async fn each_refused_registration_keeps_its_status_and_body() {
        use serde_json::{json, Value};

        async fn refused(app: &Arc<AppCtx>, body: Value) -> (StatusCode, Value) {
            let body: RegisterRequest = serde_json::from_value(body).expect("a registration");
            status_and_body(register(State(app.clone()), Json(body)).await.expect_err("the registration is refused").into_response()).await
        }
        let bad = |message: &str| (StatusCode::BAD_REQUEST, json!({ "error": message }));

        let (app, _dir) = daemon_for_jobs().await;
        let base = tempfile::tempdir().expect("tempdir");
        let file = base.path().join("notes.txt");
        std::fs::write(&file, "not a folder").expect("a file");
        let file = file.to_string_lossy().into_owned();
        let not_a_dir = |root: &str| bad(&format!("{root} is not a directory, so it has no files to index"));

        let cases = [
            (json!({ "root_path": "  " }), bad("root_path must not be empty")),
            (json!({ "root_path": "", "index_docs": true, "legacy_ids": ["nope"] }), bad("root_path must not be empty")),
            (json!({ "root_path": "/no/such/dir", "index_docs": true }), not_a_dir("/no/such/dir")),
            (json!({ "root_path": file, "index_docs": true, "assigned": true }), not_a_dir(&file)),
            (json!({ "root_path": "/no/such/dir", "index_docs": true, "legacy_ids": ["nope"] }), not_a_dir("/no/such/dir")),
            (json!({ "root_path": "/no/such/dir", "legacy_ids": ["nope"] }), bad("\"nope\" is not a project id")),
        ];
        for (body, expected) in cases {
            assert_eq!(refused(&app, body.clone()).await, expected, "{body}");
        }

        let taken = clawg_core::memory::derive_project_id(std::path::Path::new("/work/repo"));
        run_sql(&app, format!("INSERT INTO project_path (id, root_path, created_at) VALUES ('{taken}', '/elsewhere/repo', 0)")).await;
        assert_eq!(
            refused(&app, json!({ "root_path": "/work/repo" })).await,
            (
                StatusCode::CONFLICT,
                json!({
                    "error": "project_root_collision",
                    "detail": format!("/work/repo gets the project id {taken}, which already belongs to /elsewhere/repo"),
                })
            )
        );
        let assigned = RegisterRequest {
            root_path: "/work/repo".into(),
            legacy_ids: Vec::new(),
            index_docs: false,
            assigned: true,
            path: None,
        };
        let given = register(State(app.clone()), Json(assigned)).await.expect("an assigned registration gets an id of its own").0;
        assert_ne!(given.root_id, taken);
        run_sql(&app, format!("DELETE FROM project_path WHERE id = '{taken}'")).await;
        assert_eq!(
            refused(&app, json!({ "root_path": "/work/repo" })).await,
            (
                StatusCode::CONFLICT,
                json!({
                    "error": "project_root_collision",
                    "detail": format!(
                        "/work/repo derives the project id {taken}, but the daemon gives that root another one; a newer bridge uses it"
                    ),
                })
            )
        );
    }

    /// The status and the whole body `locate_project` answers with, for every
    /// refusal in the order it checks them and for a folder it locates.
    #[tokio::test]
    async fn each_located_folder_keeps_its_status_and_body() {
        use serde_json::{json, Value};

        async fn located(app: &Arc<AppCtx>, body: Value) -> (StatusCode, Value) {
            let body: LocateRequest = serde_json::from_value(body).expect("a locate");
            let answer = match locate_project(State(app.clone()), Json(body)).await {
                Ok(located) => located.into_response(),
                Err(e) => e.into_response(),
            };
            status_and_body(answer).await
        }
        async fn registered(app: &Arc<AppCtx>, root: &std::path::Path) -> RegisteredBody {
            let ask = RegisterRequest {
                root_path: root.to_string_lossy().into_owned(),
                legacy_ids: Vec::new(),
                index_docs: false,
                assigned: true,
                path: None,
            };
            register(State(app.clone()), Json(ask)).await.expect("registered").0
        }
        let bad = |message: String| (StatusCode::BAD_REQUEST, json!({ "error": message }));

        let (app, _dir) = daemon_for_jobs().await;
        let options = ResolveOptions { create: true, new_project: false, born: Born::default() };
        let ideas = resolve_project(&app.runtime.store, "ideas", options).await.expect("ideas").project_id.expect("an id");
        run_sql(
            &app,
            "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('twin-0001', 'twin', 'user', 1, 1), ('twin-0002', 'twin', 'user', 1, 2);
             INSERT INTO project (id, name, born, reviewed, created_at, deleted_at) VALUES ('gone-7a8b', 'gone', 'user', 1, 1, 5);"
                .to_string(),
        )
        .await;
        let base = tempfile::tempdir().expect("tempdir");
        let dir = |name: &str| {
            let path = base.path().join(name);
            std::fs::create_dir_all(&path).expect("a folder");
            path
        };
        let (notes, one, two, three, repo) = (dir("notes"), dir("one"), dir("two"), dir("three"), dir("repo"));
        std::fs::create_dir(repo.join(".git")).expect("a .git");
        let file = base.path().join("notes.txt");
        std::fs::write(&file, "not a folder").expect("a file");
        let path = |p: &std::path::Path| p.to_string_lossy().into_owned();
        let not_a_dir = |root: &str| bad(format!("{root} is not a directory: name a directory of the project"));

        let cases = [
            (json!({ "project": "ideas", "root_path": "/no/such/dir" }), not_a_dir("/no/such/dir")),
            (json!({ "project": "nothing here", "root_path": path(&file), "subfolder": true }), not_a_dir(&path(&file))),
            (
                json!({ "project": "nothing here", "root_path": path(&repo), "subfolder": true }),
                bad(format!(
                    "{} is the top of a checkout or outside a repository, so it is no subfolder of one; \
                     `locate` without `subfolder` locates the folder's root",
                    path(&repo)
                )),
            ),
            (
                json!({ "project": "nothing here", "root_path": path(&notes) }),
                (StatusCode::NOT_FOUND, json!({ "error": "no project has the id or the name \"nothing here\"" })),
            ),
            (
                json!({ "project": "Twin", "root_path": path(&notes) }),
                (
                    StatusCode::CONFLICT,
                    json!({ "error": "project_ambiguous", "detail": "\"Twin\" is the name of twin-0001, twin-0002: name one of them by its id" }),
                ),
            ),
        ];
        for (body, expected) in cases {
            assert_eq!(located(&app, body.clone()).await, expected, "{body}");
        }

        let own = registered(&app, &one).await.project_id;
        let second = registered(&app, &two).await.root_id;
        run_sql(&app, format!("UPDATE project_path SET project_id = '{own}' WHERE id = '{second}'")).await;
        assert_eq!(
            located(&app, json!({ "project": "ideas", "root_path": path(&one) })).await,
            (
                StatusCode::CONFLICT,
                json!({
                    "error": "folder_shared",
                    "detail": format!(
                        "{} leads to project {own}, which has 2 folders; `clawg projects merge {own} --into {ideas}` merges them all",
                        path(&one)
                    ),
                })
            )
        );
        assert_eq!(
            located(&app, json!({ "project": "gone-7a8b", "root_path": path(&three) })).await,
            (StatusCode::CONFLICT, json!({ "error": "project_deleted", "detail": "project gone-7a8b is deleted" }))
        );

        let notes_folder = registered(&app, &notes).await;
        let (status, mut body) = located(&app, json!({ "project": "Ideas", "root_path": path(&notes) })).await;
        let merge = body.as_object_mut().expect("an object").remove("merge").expect("the merge");
        assert_eq!(
            (status, body),
            (StatusCode::OK, json!({ "project_id": ideas, "root_id": notes_folder.root_id, "already": false, "removed": false }))
        );
        assert_eq!(
            (&merge["source"], &merge["target"], &merge["dry_run"], merge["merge_id"].is_i64()),
            (&json!(notes_folder.project_id), &json!(ideas), &json!(false), true),
            "{merge}"
        );
        assert_eq!(
            located(&app, json!({ "project": "ideas", "root_path": path(&notes) })).await,
            (StatusCode::OK, json!({ "project_id": ideas, "root_id": notes_folder.root_id, "already": true, "removed": false }))
        );
    }

    /// The status and the whole body `delete_projects` answers a refused
    /// delete with, before and at its first read of the store.
    #[tokio::test]
    async fn each_refused_project_delete_keeps_its_status_and_body() {
        use serde_json::{json, Value};

        async fn refused(app: &Arc<AppCtx>, body: Value) -> (StatusCode, Value) {
            let body: DeleteRequest = serde_json::from_value(body).expect("a delete");
            status_and_body(delete_projects(State(app.clone()), Json(body)).await.expect_err("the delete is refused").into_response()).await
        }
        let bad = |message: &str| (StatusCode::BAD_REQUEST, json!({ "error": message }));
        let shape = bad("name a project to delete, or a delete to undo (dry_run goes with undo)");
        let confirm = bad("confirm goes with with_memories: it is the code of the report of a delete with its rows");

        let (app, _dir) = daemon_for_jobs().await;
        let cases = [
            (json!({}), shape.clone()),
            (json!({ "project_id": "nobody-0000", "undo": 1 }), shape.clone()),
            (json!({ "project_id": "nobody-0000", "dry_run": true }), shape.clone()),
            (json!({ "project_id": "nobody-0000", "confirm": "ABC123", "dry_run": true }), shape.clone()),
            (json!({ "undo": 1, "confirm": "ABC123" }), shape),
            (json!({ "project_id": "nobody-0000", "confirm": "ABC123" }), confirm.clone()),
            (json!({ "project_id": "nobody-0000", "confirm": "ABC123", "with_memories": false }), confirm),
            (json!({ "project_id": "nobody-0000" }), (StatusCode::NOT_FOUND, json!({ "error": "no project nobody-0000" }))),
            (json!({ "undo": 99 }), (StatusCode::NOT_FOUND, json!({ "error": "no delete 99" }))),
        ];
        for (body, expected) in cases {
            assert_eq!(refused(&app, body.clone()).await, expected, "{body}");
        }
    }

    /// The status and the whole body `answer_suggestion` answers with, for
    /// every refusal in the order it checks them and for both answers.
    #[tokio::test]
    async fn each_answered_suggestion_keeps_its_status_and_body() {
        use serde_json::{json, Value};

        async fn answered(app: &Arc<AppCtx>, body: Value) -> (StatusCode, Value) {
            let body: AnswerRequest = serde_json::from_value(body).expect("an answer");
            answer_of(answer_suggestion(State(app.clone()), Json(body)).await).await
        }
        let bad = |message: &str| (StatusCode::BAD_REQUEST, json!({ "error": message }));
        let users_call = bad("answering a suggestion is the user's call: send user_answered with the user's answer in reason");
        let ask = |a: &str, b: &str, answer: &str| {
            json!({ "project_id": a, "other_id": b, "answer": answer, "user_answered": true, "reason": "the user said so" })
        };

        let (app, _dir) = daemon_for_jobs().await;
        run_sql(
            &app,
            "INSERT INTO project (id, name, born, reviewed, created_at) VALUES
               ('app-1a2b', 'app', 'user', 1, 1), ('copia-85a9', 'copia', 'user', 1, 2),
               ('web-0001', 'web', 'user', 1, 3), ('notas-0002', 'notas', 'user', 1, 4),
               ('docs-0003', 'docs', 'user', 1, 5);
             INSERT INTO project (id, name, born, reviewed, created_at, merged_into, merged_at) VALUES ('old-0004', 'old', 'user', 1, 1, 'app-1a2b', 2);
             INSERT INTO project (id, name, born, reviewed, created_at, deleted_at) VALUES ('gone-7a8b', 'gone', 'user', 1, 1, 5);
             INSERT INTO project_suggestion (project_id, other_id, reason, created_at) VALUES
               ('copia-85a9', 'app-1a2b', 'root_commit', 2), ('web-0001', 'docs-0003', 'folder_name', 3);"
                .to_string(),
        )
        .await;

        let cases = [
            (
                json!({ "project_id": "app-1a2b", "other_id": "copia-85a9", "answer": "later" }),
                bad("answer must be one of: merge, apart (got \"later\")"),
            ),
            (
                json!({ "project_id": "app-1a2b", "other_id": "copia-85a9", "answer": "Merge", "user_answered": true, "reason": "yes" }),
                bad("answer must be one of: merge, apart (got \"Merge\")"),
            ),
            (json!({ "project_id": "app-1a2b", "other_id": "copia-85a9", "answer": "merge" }), users_call.clone()),
            (
                json!({ "project_id": "app-1a2b", "other_id": "copia-85a9", "answer": "merge", "reason": "the user said so" }),
                users_call.clone(),
            ),
            (json!({ "project_id": "app-1a2b", "other_id": "copia-85a9", "answer": "apart", "user_answered": true }), users_call.clone()),
            (
                json!({ "project_id": "app-1a2b", "other_id": "copia-85a9", "answer": "apart", "user_answered": true, "reason": " \n" }),
                users_call.clone(),
            ),
            (json!({ "project_id": "nobody-0000", "other_id": "nobody-0001", "answer": "merge" }), users_call),
            (ask("nobody-0000", "app-1a2b", "merge"), (StatusCode::NOT_FOUND, json!({ "error": "no project has the id nobody-0000" }))),
            (ask("app-1a2b", "nobody-0000", "apart"), (StatusCode::NOT_FOUND, json!({ "error": "no project has the id nobody-0000" }))),
            (ask("app-1a2b", "app-1a2b", "merge"), bad("project app-1a2b cannot be kept apart from itself")),
            (
                ask("old-0004", "copia-85a9", "merge"),
                (StatusCode::CONFLICT, json!({ "error": "project_merged", "detail": "project old-0004 was merged into app-1a2b" })),
            ),
            (
                ask("app-1a2b", "gone-7a8b", "apart"),
                (StatusCode::CONFLICT, json!({ "error": "project_deleted", "detail": "project gone-7a8b is deleted" })),
            ),
            (
                ask("web-0001", "notas-0002", "apart"),
                (StatusCode::NOT_FOUND, json!({ "error": "no_suggestion", "detail": "no suggestion is pending between web-0001 and notas-0002" })),
            ),
        ];
        for (body, expected) in cases {
            assert_eq!(answered(&app, body.clone()).await, expected, "{body}");
        }

        assert_eq!(
            answered(&app, ask("docs-0003", "web-0001", "apart")).await,
            (StatusCode::OK, json!({ "project_id": "docs-0003", "other_id": "web-0001", "answer": "apart" }))
        );
        assert_eq!(
            answered(&app, ask("web-0001", "docs-0003", "merge")).await,
            (StatusCode::NOT_FOUND, json!({ "error": "no_suggestion", "detail": "no suggestion is pending between web-0001 and docs-0003" })),
            "an answered suggestion is not answered again"
        );

        let (status, mut body) = answered(&app, ask("app-1a2b", "copia-85a9", "merge")).await;
        let Some(Value::Object(mut merge)) = body.as_object_mut().expect("an object").remove("merge") else {
            panic!("the merge is an object: {body}");
        };
        assert_eq!((status, body), (StatusCode::OK, json!({ "project_id": "app-1a2b", "other_id": "copia-85a9", "answer": "merge" })));
        let merge_id = merge.remove("merge_id").expect("its number");
        let snapshot = merge.remove("snapshot").expect("its snapshot");
        assert!(merge_id.is_i64(), "{merge_id}");
        assert!(snapshot.as_str().is_some_and(|s| s.contains("pre-merge")), "{snapshot}");
        assert_eq!(
            Value::Object(merge),
            json!({
                "dry_run": false,
                "source": "copia-85a9",
                "target": "app-1a2b",
                "move_id": null,
                "memories": 0,
                "superseded": [],
                "split": [],
                "protected": 0,
                "reviews": 0,
                "folders": [],
                "remotes": [],
                "tombstones": [],
                "suggestions": 1,
                "workspaces": { "copied": [], "kept_beside": [], "failed": [] },
                "trajectories": 0,
                "skills": 0,
            })
        );
        assert_eq!(
            answered(&app, ask("app-1a2b", "copia-85a9", "merge")).await,
            (StatusCode::CONFLICT, json!({ "error": "project_merged", "detail": "project copia-85a9 was merged into app-1a2b" }))
        );

        app.runtime.close().await.expect("the store closes");
        assert_eq!(
            answered(&app, ask("web-0001", "notas-0002", "merge")).await,
            (StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": "memory backend channel error: ConnectionClosed" }))
        );
        assert_eq!(
            answered(&app, json!({ "project_id": "web-0001", "other_id": "notas-0002", "answer": "merge" })).await,
            bad("answering a suggestion is the user's call: send user_answered with the user's answer in reason"),
            "the answer is checked before the store is read"
        );
    }

    /// The status and the whole body `resolve_projects` answers with, for
    /// every refusal in the order it checks them and for each resolution.
    #[tokio::test]
    async fn each_resolved_project_keeps_its_status_and_body() {
        use serde_json::{json, Value};

        async fn resolved(app: &Arc<AppCtx>, body: Value) -> (StatusCode, Value) {
            let body: ProjectResolveRequest = serde_json::from_value(body).expect("a resolve");
            answer_of(resolve_projects(State(app.clone()), Json(body)).await).await
        }
        let bad = |message: &str| (StatusCode::BAD_REQUEST, json!({ "error": message }));
        let new_project = bad("new_project goes with create: it creates a project apart from the similar ones");
        let none = |refused: &str, candidates: Value| {
            json!({ "project_id": null, "name": null, "created": false, "deleted": false, "folders": [], "refused": refused, "candidates": candidates })
        };
        let candidate = |id: &str, name: &str| json!({ "id": id, "name": name, "deleted": false });

        let (app, _dir) = daemon_for_jobs().await;
        run_sql(
            &app,
            "INSERT INTO project (id, name, born, reviewed, created_at) VALUES
               ('twin-0001', 'twin', 'user', 1, 1), ('twin-0002', 'twin', 'user', 1, 2), ('mi-app-0003', 'Mi App', 'user', 1, 3);
             INSERT INTO project (id, name, born, reviewed, created_at, merged_into, merged_at) VALUES ('old-0004', 'old', 'user', 1, 1, 'mi-app-0003', 2);
             INSERT INTO project (id, name, born, reviewed, created_at, deleted_at) VALUES ('gone-7a8b', 'gone', 'user', 1, 1, 5);"
                .to_string(),
        )
        .await;

        let cases = [
            (json!({ "project": "ideas", "new_project": true }), new_project.clone()),
            (json!({ "project": "", "new_project": true }), new_project.clone()),
            (json!({ "project": "  " }), bad("a project name must not be empty")),
            (json!({ "project": "x".repeat(81), "create": true }), bad("a project name has at most 80 characters")),
            (json!({ "project": "a\u{7}b" }), bad("a project name must not hold control characters")),
            (json!({ "project": "Twin", "create": true }), (StatusCode::OK, none("ambiguous", json!([candidate("twin-0001", "twin"), candidate("twin-0002", "twin")])))),
            (json!({ "project": "mi_app" }), (StatusCode::OK, none("similar", json!([candidate("mi-app-0003", "Mi App")])))),
            (json!({ "project": "mi_app", "create": true }), (StatusCode::OK, none("similar", json!([candidate("mi-app-0003", "Mi App")])))),
            (json!({ "project": "nothing" }), (StatusCode::OK, none("unknown", json!([])))),
        ];
        for (body, expected) in cases {
            assert_eq!(resolved(&app, body.clone()).await, expected, "{body}");
        }

        let found = |id: &str, name: &str, created: bool, deleted: bool| {
            json!({ "project_id": id, "name": name, "created": created, "deleted": deleted, "folders": [], "refused": null, "candidates": [] })
        };
        assert_eq!(resolved(&app, json!({ "project": "old-0004" })).await, (StatusCode::OK, found("mi-app-0003", "Mi App", false, false)));
        assert_eq!(resolved(&app, json!({ "project": " mi app " })).await, (StatusCode::OK, found("mi-app-0003", "Mi App", false, false)));
        assert_eq!(resolved(&app, json!({ "project": "gone-7a8b" })).await, (StatusCode::OK, found("gone-7a8b", "gone", false, true)));
        let (status, body) = resolved(&app, json!({ "project": "ideas", "create": true })).await;
        let ideas = body["project_id"].as_str().expect("an id").to_string();
        assert!(ideas.starts_with("ideas-"), "{ideas}");
        assert_eq!((status, body), (StatusCode::OK, found(&ideas, "ideas", true, false)));
        assert_eq!(resolved(&app, json!({ "project": "IDEAS" })).await, (StatusCode::OK, found(&ideas, "ideas", false, false)));
        let (status, body) = resolved(&app, json!({ "project": "Mi-App!", "create": true, "new_project": true })).await;
        let apart = body["project_id"].as_str().expect("an id").to_string();
        assert!(apart.starts_with("mi-app-") && apart != "mi-app-0003", "{apart}");
        assert_eq!((status, body), (StatusCode::OK, found(&apart, "Mi-App!", true, false)));

        app.runtime.close().await.expect("the store closes");
        assert_eq!(resolved(&app, json!({ "project": "ideas", "new_project": true })).await, new_project, "checked before the store is read");
        assert_eq!(
            resolved(&app, json!({ "project": "ideas" })).await,
            (StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": "memory backend channel error: ConnectionClosed" }))
        );
    }
}
