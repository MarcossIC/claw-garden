//! Where each project of an export goes here: by the map, by a remote, by
//! its id, by its name and by its path, in that order, else a new one.
//!
//! What decides is pure, over the projects of the file, the registry read
//! once per pass of the plan and what the plan gave each of their folders,
//! so a dry run decides as the import does.

use std::collections::{BTreeSet, HashMap, HashSet};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::memory::merge::MAX_MERGE_HOPS;
use crate::memory::portability::format::{FolderLine, ProjectLine, SubfolderLine};
use crate::memory::project::RootPlan;

/// The lines of one id of a file: one project of the source store.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FileProject {
    pub id: String,
    pub name: Option<String>,
    /// Every folder its lines name, each path once, in the order of the file.
    pub folders: Vec<FolderLine>,
    pub remotes: Vec<String>,
    pub merged_into: Option<String>,
    pub subfolders: Vec<SubfolderLine>,
    /// Whether a `memory` line of the file is in it.
    pub rows: bool,
}

impl FileProject {
    /// Its remotes in the order of the file: its own, then those of its folders.
    pub(crate) fn all_remotes(&self) -> Vec<&str> {
        let mut seen = HashSet::new();
        self.remotes
            .iter()
            .map(String::as_str)
            .chain(self.folders.iter().filter_map(|f| f.remote.as_deref()))
            .filter(|r| seen.insert(*r))
            .collect()
    }

    /// The fingerprints the file gives it.
    pub(crate) fn prints(&self) -> Prints {
        Prints {
            remotes: self.all_remotes().into_iter().map(str::to_owned).collect(),
            commits: self.folders.iter().filter_map(|f| f.root_commit.clone()).collect(),
        }
    }
}

/// The projects of a file, one per id in the order the ids first appear;
/// `with_rows` are the ids its `memory` lines name.
pub(crate) fn file_projects<'a>(lines: impl IntoIterator<Item = &'a ProjectLine>, with_rows: &BTreeSet<&str>) -> Vec<FileProject> {
    let mut projects: Vec<FileProject> = Vec::new();
    for line in lines {
        let at = match projects.iter().position(|p| p.id == line.id) {
            Some(at) => at,
            None => {
                projects.push(FileProject { id: line.id.clone(), rows: with_rows.contains(line.id.as_str()), ..Default::default() });
                projects.len() - 1
            }
        };
        let p = &mut projects[at];
        if p.name.is_none() {
            p.name.clone_from(&line.name);
        }
        if p.merged_into.is_none() {
            p.merged_into.clone_from(&line.merged_into);
        }
        let folders = line.root_path.iter().map(|root| FolderLine { root_path: root.clone(), ..Default::default() });
        for folder in line.folders.iter().cloned().chain(folders) {
            match p.folders.iter_mut().find(|f| f.root_path == folder.root_path) {
                Some(known) => {
                    if known.remote.is_none() {
                        known.remote = folder.remote;
                    }
                    if known.root_commit.is_none() {
                        known.root_commit = folder.root_commit;
                    }
                }
                None => p.folders.push(folder),
            }
        }
        for remote in &line.remotes {
            if !p.remotes.contains(remote) {
                p.remotes.push(remote.clone());
            }
        }
        for declared in &line.subfolders {
            if !p.subfolders.iter().any(|s| s.subpath == declared.subpath) {
                p.subfolders.push(declared.clone());
            }
        }
    }
    projects
}

/// The remotes and first commits a project is known by.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Prints {
    pub remotes: BTreeSet<String>,
    pub commits: BTreeSet<String>,
}

impl Prints {
    pub(crate) fn is_empty(&self) -> bool {
        self.remotes.is_empty() && self.commits.is_empty()
    }

    fn extend(&mut self, other: &Prints) {
        self.remotes.extend(other.remotes.iter().cloned());
        self.commits.extend(other.commits.iter().cloned());
    }

    /// Whether these are another repository's: both know a remote and share
    /// none, or, when one of them knows none, both know a first commit and
    /// share none. The remote decides first, as a registration's `fit`.
    pub(crate) fn contradicts(&self, other: &Prints) -> bool {
        if !self.remotes.is_empty() && !other.remotes.is_empty() {
            return self.remotes.is_disjoint(&other.remotes);
        }
        !self.commits.is_empty() && !other.commits.is_empty() && self.commits.is_disjoint(&other.commits)
    }
}

/// A project of this store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HereProject {
    pub name: String,
    pub merged_into: Option<String>,
    pub deleted: bool,
}

/// A folder of this store, with the project it leads to as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HereFolder {
    pub id: String,
    pub root_path: String,
    pub project: String,
    pub remote: Option<String>,
    pub root_commit: Option<String>,
}

/// The registry of this store, as one pass of the plan read it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Registry {
    pub projects: HashMap<String, HereProject>,
    pub folders: Vec<HereFolder>,
    /// Remote and the project its clones share, as written.
    pub remotes: Vec<(String, String)>,
    /// Repository, subpath and project of each declaration, as written.
    pub subfolders: Vec<(String, String, String)>,
}

/// Reads the registry: four small tables.
pub(crate) fn registry_in(c: &Connection) -> rusqlite::Result<Registry> {
    let mut stmt = c.prepare("SELECT id, name, merged_into, deleted_at IS NOT NULL FROM project")?;
    let projects = stmt
        .query_map([], |r| Ok((r.get(0)?, HereProject { name: r.get(1)?, merged_into: r.get(2)?, deleted: r.get(3)? })))?
        .collect::<rusqlite::Result<_>>()?;
    let mut stmt = c.prepare("SELECT remote, project_id FROM project_remote ORDER BY created_at, remote")?;
    let remotes: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut stmt = c.prepare("SELECT id, root_path, project_id, remote, root_commit FROM project_path ORDER BY created_at, id")?;
    let folders = stmt
        .query_map([], |r| {
            let (id, project, remote): (String, Option<String>, Option<String>) = (r.get(0)?, r.get(2)?, r.get(3)?);
            // The one a registration recorded, else the one the clones of its
            // remote share, else its own id.
            let by_remote = || remote.as_ref().and_then(|k| remotes.iter().find(|(b, _)| b == k)).map(|(_, p)| p.clone());
            let project = project.or_else(by_remote).unwrap_or_else(|| id.clone());
            Ok(HereFolder { id, root_path: r.get(1)?, project, remote, root_commit: r.get(4)? })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut stmt = c.prepare("SELECT parent_id, subpath, project_id FROM project_subfolder ORDER BY created_at, parent_id, subpath")?;
    let subfolders = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
    Ok(Registry { projects, folders, remotes, subfolders })
}

impl Registry {
    /// The live project `id` leads to through `merged_into`, as `canonical_in`.
    pub(crate) fn canonical(&self, id: &str) -> String {
        let mut at = id.to_owned();
        let mut seen = HashSet::from([at.clone()]);
        for _ in 0..=MAX_MERGE_HOPS {
            match self.projects.get(&at).and_then(|p| p.merged_into.clone()) {
                Some(next) if self.projects.contains_key(&next) && seen.insert(next.clone()) => at = next,
                Some(_) => return id.to_owned(),
                None => return at,
            }
        }
        id.to_owned()
    }

    /// Whether `id` is taken: a project or a folder has it.
    pub(crate) fn taken(&self, id: &str) -> bool {
        self.projects.contains_key(id) || self.folders.iter().any(|f| f.id == id)
    }

    /// The fingerprints of the live project `project`: the remotes bound to
    /// it and the remote and first commit of each folder that leads to it.
    pub(crate) fn prints(&self, project: &str) -> Prints {
        let mut prints = Prints::default();
        for (remote, owner) in &self.remotes {
            if self.canonical(owner) == project {
                prints.remotes.insert(remote.clone());
            }
        }
        for folder in self.folders.iter().filter(|f| self.canonical(&f.project) == project) {
            prints.remotes.extend(folder.remote.clone());
            prints.commits.extend(folder.root_commit.clone());
        }
        prints
    }

    /// The live projects that are neither tombstones nor deleted, with their names.
    pub(crate) fn live(&self) -> impl Iterator<Item = (&str, &str)> {
        self.projects.iter().filter(|(_, p)| p.merged_into.is_none() && !p.deleted).map(|(id, p)| (id.as_str(), p.name.as_str()))
    }
}

/// A folder of a project of the file, as the plan found it here.
#[derive(Debug, Clone)]
pub(crate) struct FolderHere<'a> {
    pub line: &'a FolderLine,
    /// Whether `--map-root` named it.
    pub mapped: bool,
    pub plan: &'a RootPlan,
    /// Whether the path exists on this machine.
    pub present: bool,
}

impl FolderHere<'_> {
    /// What the checkout there tells, when the path exists here.
    fn checkout(&self) -> Prints {
        let mut prints = Prints::default();
        if self.present {
            prints.remotes.extend(self.plan.remote.clone());
            prints.commits.extend(self.plan.root_commit.clone());
        }
        prints
    }
}

/// The step of the order that decided where a project of the file went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum By {
    Map,
    Remote,
    Id,
    Name,
    Path,
    New,
}

/// Where a project of the file goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Placed {
    /// A project of this store, or the one a registration gives a folder.
    Here(String, By),
    /// A project the import creates.
    New,
    /// Nowhere: a line without rows that nothing here takes.
    Nowhere,
}

/// The fingerprints of `project` with what the checkouts of its folders here
/// tell, those that do not contradict the file: a file from before
/// fingerprints says nothing the disk does not.
pub(crate) fn seen_prints(project: &FileProject, folders: &[FolderHere<'_>]) -> Prints {
    let file = project.prints();
    let mut prints = file.clone();
    for folder in folders {
        let here = folder.checkout();
        if !here.contradicts(&file) {
            prints.extend(&here);
        }
    }
    prints
}

/// Where `project` goes: the map, a remote bound here, its id, its name,
/// one of its paths, else a new project; `folders` are its folders as the
/// plan found them here. A tombstone of the file only goes by its id, and a
/// line without rows creates nothing.
pub(crate) fn place(project: &FileProject, folders: &[FolderHere<'_>], registry: &Registry) -> Placed {
    let tombstone = project.merged_into.is_some();
    if !tombstone {
        if let Some(mapped) = folders.iter().find(|f| f.mapped) {
            return Placed::Here(registry.canonical(&mapped.plan.project_id), By::Map);
        }
    }
    let prints = seen_prints(project, folders);
    if !tombstone {
        let bound = |remote: &str| registry.remotes.iter().find(|(r, _)| r == remote).map(|(_, p)| p.clone());
        let mut remotes: Vec<&str> = project.all_remotes();
        remotes.extend(prints.remotes.iter().map(String::as_str).filter(|r| !remotes.contains(r)).collect::<Vec<_>>());
        if let Some(owner) = remotes.into_iter().find_map(bound) {
            return Placed::Here(registry.canonical(&owner), By::Remote);
        }
    }
    if registry.projects.contains_key(&project.id) {
        let here = registry.canonical(&project.id);
        if !prints.contradicts(&registry.prints(&here)) {
            return Placed::Here(here, By::Id);
        }
    }
    if tombstone {
        return Placed::Nowhere;
    }
    if let Some(name) = project.name.as_deref().filter(|_| prints.is_empty()) {
        let wanted = name.to_lowercase();
        let named: Vec<&str> = registry.live().filter(|(_, n)| n.to_lowercase() == wanted).map(|(id, _)| id).collect();
        if let [one] = named.as_slice() {
            if registry.prints(one).is_empty() {
                return Placed::Here((*one).to_owned(), By::Name);
            }
        }
    }
    for folder in folders {
        let recorded = registry.folders.iter().any(|f| f.root_path == folder.line.root_path);
        if !(recorded || folder.present) {
            continue;
        }
        let here = registry.canonical(&folder.plan.project_id);
        let mut theirs = registry.prints(&here);
        theirs.extend(&folder.checkout());
        if !prints.contradicts(&theirs) {
            return Placed::Here(here, By::Path);
        }
    }
    match project.rows {
        true => Placed::New,
        false => Placed::Nowhere,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str) -> ProjectLine {
        ProjectLine { id: id.into(), ..Default::default() }
    }

    fn folder(root: &str, remote: Option<&str>, commit: Option<&str>) -> FolderLine {
        FolderLine { root_path: root.into(), remote: remote.map(Into::into), root_commit: commit.map(Into::into) }
    }

    fn plan(project: &str, remote: Option<&str>, commit: Option<&str>) -> RootPlan {
        RootPlan {
            root_id: project.into(),
            project_id: project.into(),
            remote: remote.map(Into::into),
            root_commit: commit.map(Into::into),
            git_stamp: None,
        }
    }

    #[test]
    fn lines_of_one_id_are_one_project() {
        let lines = vec![
            ProjectLine { root_path: Some("/work/app".into()), ..line("app-1a2b") },
            ProjectLine { root_path: Some("/review/app".into()), ..line("app-1a2b") },
            ProjectLine {
                name: Some("App".into()),
                folders: vec![folder("/work/app", Some("github.com/acme/app"), Some("c1"))],
                remotes: vec!["github.com/acme/app".into()],
                ..line("app-1a2b")
            },
            ProjectLine { name: Some("Ideas".into()), ..line("ideas-3c4d") },
        ];
        let projects = file_projects(&lines, &BTreeSet::from(["app-1a2b"]));
        assert_eq!(projects.len(), 2);
        let app = &projects[0];
        assert_eq!((app.id.as_str(), app.name.as_deref(), app.rows), ("app-1a2b", Some("App"), true));
        assert_eq!(
            app.folders,
            vec![folder("/work/app", Some("github.com/acme/app"), Some("c1")), folder("/review/app", None, None)],
            "a path once, with what any of its lines knows"
        );
        assert_eq!(app.remotes, vec!["github.com/acme/app".to_owned()]);
        assert!(!projects[1].rows && projects[1].folders.is_empty());
    }

    /// A registry with `app` (folder /work/app, remote acme/app, commit c1),
    /// the tombstone `old` merged into it, `ideas` without fingerprints,
    /// `api` at /work/api with remote acme/api-b, `dup` twice by name and
    /// the deleted `gone`.
    fn registry() -> Registry {
        let project = |name: &str, merged_into: Option<&str>, deleted: bool| HereProject {
            name: name.into(),
            merged_into: merged_into.map(Into::into),
            deleted,
        };
        let folder = |id: &str, root: &str, remote: Option<&str>, commit: Option<&str>| HereFolder {
            id: id.into(),
            root_path: root.into(),
            project: id.into(),
            remote: remote.map(Into::into),
            root_commit: commit.map(Into::into),
        };
        Registry {
            projects: HashMap::from([
                ("app".into(), project("App", None, false)),
                ("old".into(), project("old", Some("app"), false)),
                ("ideas".into(), project("Ideas de producto", None, false)),
                ("api".into(), project("api", None, false)),
                ("dup-1".into(), project("dup", None, false)),
                ("dup-2".into(), project("Dup", None, false)),
                ("gone".into(), project("gone", None, true)),
            ]),
            folders: vec![
                folder("app", "/work/app", Some("github.com/acme/app"), Some("c1")),
                folder("api", "/work/api", Some("github.com/acme/api-b"), None),
            ],
            remotes: vec![("github.com/acme/app".into(), "app".into()), ("github.com/acme/api-b".into(), "api".into())],
            subfolders: Vec::new(),
        }
    }

    fn placed(project: FileProject, folders: &[(FolderLine, RootPlan, bool, bool)]) -> Placed {
        let here: Vec<FolderHere<'_>> = folders
            .iter()
            .zip(&project.folders)
            .map(|((_, plan, mapped, present), line)| FolderHere { line, mapped: *mapped, plan, present: *present })
            .collect();
        place(&project, &here, &registry())
    }

    fn file(id: &str, name: Option<&str>, folders: Vec<FolderLine>, remotes: &[&str]) -> FileProject {
        FileProject {
            id: id.into(),
            name: name.map(Into::into),
            folders,
            remotes: remotes.iter().map(|r| r.to_string()).collect(),
            rows: true,
            ..Default::default()
        }
    }

    #[test]
    fn a_project_of_the_file_goes_by_map_remote_id_name_then_path() {
        let elsewhere = folder("/far/app", Some("github.com/acme/app"), Some("c1"));
        let far = plan("app-far0", None, None);

        let mapped = file("x-0000", None, vec![elsewhere.clone()], &[]);
        let to = plan("mapped-1111", None, None);
        assert_eq!(placed(mapped, &[(elsewhere.clone(), to, true, false)]), Placed::Here("mapped-1111".into(), By::Map));

        let by_remote = file("x-0000", Some("App"), vec![elsewhere.clone()], &["github.com/acme/app"]);
        assert_eq!(placed(by_remote, &[(elsewhere.clone(), far.clone(), false, false)]), Placed::Here("app".into(), By::Remote));

        let by_id = file("old", None, vec![folder("/far/old", None, None)], &[]);
        assert_eq!(placed(by_id, &[(elsewhere.clone(), far.clone(), false, false)]), Placed::Here("app".into(), By::Id), "past the merge");

        let contradicted = file("api", Some("api"), vec![folder("/work/api", Some("github.com/acme/api-a"), None)], &[]);
        let at_api = plan("api", Some("github.com/acme/api-b"), None);
        assert_eq!(placed(contradicted, &[(elsewhere.clone(), at_api, false, true)]), Placed::New, "the same path, another repository");

        let by_name = file("ideas-9999", Some("ideas de PRODUCTO"), Vec::new(), &[]);
        assert_eq!(placed(by_name, &[]), Placed::Here("ideas".into(), By::Name));

        let printed = file("ideas-9999", Some("Ideas de producto"), vec![folder("/far/ideas", None, Some("c9"))], &[]);
        assert_eq!(placed(printed, &[(elsewhere.clone(), far.clone(), false, false)]), Placed::New, "fingerprints on one side only");

        let twice = file("dup-9999", Some("dup"), Vec::new(), &[]);
        assert_eq!(placed(twice, &[]), Placed::New, "two projects have the name");

        let deleted = file("gone-9999", Some("gone"), Vec::new(), &[]);
        assert_eq!(placed(deleted, &[]), Placed::New, "a deleted project's name is free");

        let deleted_by_id = file("gone", Some("gone"), Vec::new(), &[]);
        assert_eq!(placed(deleted_by_id, &[]), Placed::Here("gone".into(), By::Id));

        let by_path = file("app-7777", None, vec![folder("/work/app", None, None)], &[]);
        let at_app = plan("app", None, Some("c1"));
        assert_eq!(placed(by_path, &[(elsewhere.clone(), at_app, false, true)]), Placed::Here("app".into(), By::Path));

        let present = file("new-0000", None, vec![folder("/work/new", None, None)], &[]);
        let fresh = plan("new-5555", Some("github.com/acme/new"), None);
        assert_eq!(placed(present, &[(elsewhere.clone(), fresh, false, true)]), Placed::Here("new-5555".into(), By::Path));

        let rowless = FileProject { rows: false, ..file("lib-0000", Some("lib"), Vec::new(), &[]) };
        assert_eq!(placed(rowless, &[]), Placed::Nowhere, "a line without rows creates nothing");

        let tombstone = FileProject { merged_into: Some("app".into()), rows: false, ..file("old-ffff", Some("ideas de producto"), Vec::new(), &["github.com/acme/app"]) };
        assert_eq!(placed(tombstone, &[]), Placed::Nowhere, "a tombstone only goes by its id");
        let known_tombstone = FileProject { merged_into: Some("x".into()), rows: false, ..file("old", None, Vec::new(), &[]) };
        assert_eq!(placed(known_tombstone, &[]), Placed::Here("app".into(), By::Id));
    }
}
