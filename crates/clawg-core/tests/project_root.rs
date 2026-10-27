use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use clawg_core::memory::identity::{
    git_stamp, origin_key, resolve_project_root, root_commit, strip_verbatim, Disk, Ground, RootCommit,
};
use clawg_core::memory::{derive_project_id, ProjectIdentity};

/// A temporary directory by its canonical name, so expected roots compare
/// equal on systems where the temp dir sits behind a link. Without the
/// `\\?\` prefix, as `resolve_project_root` returns it on Windows.
fn canonical_tempdir() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = strip_verbatim(&fs::canonicalize(dir.path()).expect("canonicalize tempdir"));
    (dir, path)
}

fn git_repo(at: &Path) {
    fs::create_dir_all(at.join(".git")).expect("create .git");
}

#[test]
fn every_way_of_naming_a_repository_resolves_to_its_root() {
    let (_tmp, base) = canonical_tempdir();
    let repo = base.join("repo");
    git_repo(&repo);
    fs::create_dir_all(repo.join("src").join("lib")).expect("create src/lib");

    for spelling in [
        repo.clone(),
        PathBuf::from(format!("{}{}", repo.display(), std::path::MAIN_SEPARATOR)),
        repo.join("src").join("lib"),
        repo.join("src").join(".."),
    ] {
        assert_eq!(resolve_project_root(&spelling), repo, "{}", spelling.display());
    }
}

#[test]
fn a_worktree_resolves_to_the_repository_it_was_added_from() {
    let (_tmp, base) = canonical_tempdir();
    let repo = base.join("repo");
    git_repo(&repo);
    let admin = repo.join(".git").join("worktrees").join("wt");
    fs::create_dir_all(&admin).expect("create worktree admin dir");
    fs::write(admin.join("commondir"), "../..\n").expect("write commondir");
    let wt = base.join("wt");
    fs::create_dir_all(&wt).expect("create worktree");
    fs::write(wt.join(".git"), format!("gitdir: {}\n", admin.display())).expect("write .git file");

    assert_eq!(resolve_project_root(&wt), repo);
}

#[test]
fn a_submodule_is_its_own_project() {
    let (_tmp, base) = canonical_tempdir();
    let repo = base.join("repo");
    git_repo(&repo);
    fs::create_dir_all(repo.join(".git").join("modules").join("sub")).expect("create module dir");
    let sub = repo.join("sub");
    fs::create_dir_all(&sub).expect("create submodule");
    fs::write(sub.join(".git"), "gitdir: ../.git/modules/sub\n").expect("write .git file");

    assert_eq!(resolve_project_root(&sub), sub);
}

#[test]
fn outside_git_the_canonical_path_is_the_root() {
    let (_tmp, base) = canonical_tempdir();
    let plain = base.join("plain");
    fs::create_dir_all(&plain).expect("create plain dir");

    // The temp dir itself might sit inside a checkout; only assert when it does not.
    if resolve_project_root(&base) == base {
        let trailing = PathBuf::from(format!("{}{}", plain.display(), std::path::MAIN_SEPARATOR));
        assert_eq!(resolve_project_root(&trailing), plain);
    }
}

#[test]
fn a_path_that_does_not_exist_is_normalized_without_searching_for_git() {
    let (_tmp, base) = canonical_tempdir();
    git_repo(&base);
    let missing = base.join("gone").join(".").join("deeper").join("..");

    assert_eq!(resolve_project_root(&missing), base.join("gone"));
}

#[test]
fn the_id_of_a_canonical_root_is_the_id_it_always_had() {
    let (_tmp, base) = canonical_tempdir();
    let repo = base.join("repo");
    git_repo(&repo);

    let identity = ProjectIdentity::resolve(&repo);
    assert_eq!(identity.id, derive_project_id(&repo));
    assert_eq!(identity.legacy_id, identity.id, "nothing to move for a bridge started at the root");
}

#[test]
fn a_subdirectory_keeps_its_old_id_as_the_legacy_one() {
    let (_tmp, base) = canonical_tempdir();
    let repo = base.join("repo");
    git_repo(&repo);
    let src = repo.join("src");
    fs::create_dir_all(&src).expect("create src");

    let identity = ProjectIdentity::resolve(&src);
    assert_eq!(identity.root, repo);
    assert_eq!(identity.id, derive_project_id(&repo));
    assert_eq!(identity.legacy_id, derive_project_id(&src));
}

#[cfg(windows)]
#[test]
fn windows_spellings_of_one_directory_share_a_root() {
    let (_tmp, base) = canonical_tempdir();
    let repo = base.join("Repo");
    git_repo(&repo);

    let lower = PathBuf::from(repo.to_string_lossy().to_lowercase());
    let forward = PathBuf::from(format!("{}/", repo.to_string_lossy().replace('\\', "/")));
    for spelling in [lower, forward] {
        let root = resolve_project_root(&spelling);
        assert_eq!(root, repo, "{}", spelling.display());
        assert!(!root.to_string_lossy().starts_with(r"\\?\"));
    }
}

fn with_origin(git_dir: &Path, url: &str) {
    fs::create_dir_all(git_dir).expect("create git dir");
    fs::write(git_dir.join("config"), format!("[core]\n\tbare = false\n[remote \"origin\"]\n\turl = {url}\n"))
        .expect("write config");
}

#[test]
fn two_clones_of_one_remote_have_one_key() {
    let (_tmp, base) = canonical_tempdir();
    let (work, review, fork) = (base.join("work").join("app"), base.join("review").join("app"), base.join("fork").join("app"));
    with_origin(&work.join(".git"), "https://github.com/Acme/App.git");
    with_origin(&review.join(".git"), "git@github.com:acme/app.git");
    with_origin(&fork.join(".git"), "git@github.com:me/app.git");

    assert_eq!(origin_key(&work).as_deref(), Some("github.com/acme/app"));
    assert_eq!(origin_key(&work), origin_key(&review));
    assert_ne!(origin_key(&work), origin_key(&fork));
}

#[test]
fn the_key_of_a_worktree_and_of_a_submodule() {
    let (_tmp, base) = canonical_tempdir();
    let repo = base.join("repo");
    with_origin(&repo.join(".git"), "https://github.com/acme/repo");
    let admin = repo.join(".git").join("worktrees").join("wt");
    fs::create_dir_all(&admin).expect("create worktree admin dir");
    fs::write(admin.join("commondir"), "../..\n").expect("write commondir");
    let wt = base.join("wt");
    fs::create_dir_all(&wt).expect("create worktree");
    fs::write(wt.join(".git"), format!("gitdir: {}\n", admin.display())).expect("write .git file");

    let module = repo.join(".git").join("modules").join("sub");
    with_origin(&module, "https://github.com/acme/sub.git");
    let sub = repo.join("sub");
    fs::create_dir_all(&sub).expect("create submodule");
    fs::write(sub.join(".git"), "gitdir: ../.git/modules/sub\n").expect("write .git file");

    assert_eq!(origin_key(&wt).as_deref(), Some("github.com/acme/repo"));
    assert_eq!(origin_key(&sub).as_deref(), Some("github.com/acme/sub"));
}

#[test]
fn a_root_without_origin_has_no_key() {
    let (_tmp, base) = canonical_tempdir();
    let bare = base.join("bare");
    git_repo(&bare);
    let local = base.join("local");
    with_origin(&local.join(".git"), "/srv/git/local.git");
    let plain = base.join("plain");
    fs::create_dir_all(&plain).expect("create plain dir");

    assert_eq!(origin_key(&bare), None);
    assert_eq!(origin_key(&local), None);
    assert_eq!(origin_key(&plain), None);
}

/// Runs the real `git` in `dir` and returns what it printed, trimmed. It runs
/// apart from the caller's git: none of its `GIT_*` (a hook that started the
/// tests would point it at the repository being tested), no global or system
/// config, and a fixed author.
fn git(dir: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.as_encoded_bytes().get(..4).is_some_and(|p| p.eq_ignore_ascii_case(b"GIT_")) {
            command.env_remove(key);
        }
    }
    let null_device = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let output = command
        .current_dir(dir)
        .args(["-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", null_device)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("these tests need git on the PATH");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).expect("git prints UTF-8").trim().to_string()
}

fn commit(repo: &Path, message: &str) -> String {
    git(repo, &["commit", "--allow-empty", "-m", message]);
    git(repo, &["rev-parse", "HEAD"])
}

/// A `file://` URL, which `clone --depth` needs: a plain local path ignores the depth.
fn file_url(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    if path.starts_with('/') { format!("file://{path}") } else { format!("file:///{path}") }
}

#[test]
fn the_first_commit_is_the_root_of_the_first_parent_chain() {
    let (_tmp, base) = canonical_tempdir();
    git(&base, &["init", "repo"]);
    let repo = base.join("repo");
    let first = commit(&repo, "first");
    commit(&repo, "second");
    git(&repo, &["checkout", "--orphan", "vendor"]);
    fs::write(repo.join("vendor.txt"), "vendored\n").expect("write vendor.txt");
    git(&repo, &["add", "vendor.txt"]);
    git(&repo, &["commit", "-m", "a foreign history"]);
    git(&repo, &["checkout", "main"]);
    git(&repo, &["merge", "--no-ff", "--allow-unrelated-histories", "-m", "merge the foreign history", "vendor"]);

    assert_eq!(git(&repo, &["rev-list", "--max-parents=0", "HEAD"]).lines().count(), 2, "the merge brought a second root");
    assert_eq!(root_commit(&repo), RootCommit::Known(first.clone()));

    git(&repo, &["worktree", "add", "-b", "topic", "../wt"]);
    assert_eq!(root_commit(&base.join("wt")), RootCommit::Known(first), "a worktree reads through its .git file");
}

#[test]
fn a_shallow_or_empty_repository_has_no_first_commit_to_record() {
    let (_tmp, base) = canonical_tempdir();
    git(&base, &["init", "origin"]);
    let origin = base.join("origin");
    commit(&origin, "first");
    commit(&origin, "second");
    git(&base, &["clone", "--depth", "1", &file_url(&origin), "shallow"]);
    let shallow = base.join("shallow");
    assert!(shallow.join(".git").join("shallow").exists(), "the clone is shallow");
    git(&base, &["init", "empty"]);

    assert_eq!(root_commit(&shallow), RootCommit::Unknowable, "the commit it ends in is not the first one");
    assert_eq!(root_commit(&base.join("empty")), RootCommit::Later, "there is no commit yet");

    let broken = origin.join("vendored");
    fs::create_dir_all(broken.join(".git")).expect("create a .git that is no repository");
    assert_eq!(root_commit(&broken), RootCommit::Later, "not the first commit of the repository around it");
}

#[test]
fn the_stamp_of_a_checkout_changes_when_it_is_cloned_again() {
    let (_tmp, base) = canonical_tempdir();
    git(&base, &["init", "app"]);
    let app = base.join("app");
    commit(&app, "first");
    let recorded = Disk.checkout(&app).stamp.expect("a checkout has a stamp");

    // The old checkout is moved aside rather than deleted: a deleted one
    // frees its inode for the new clone, and a system without creation dates
    // could then give both one stamp, a limit the registration accepts.
    let moved = base.join("moved");
    fs::rename(&app, &moved).expect("move the checkout");
    assert_eq!(git_stamp(&moved).as_ref(), Some(&recorded), "a move keeps its git directory");
    assert!(!Disk.recorded_checkout_present(&app, Some(&recorded)), "nothing is there");

    git(&base, &["clone", "moved", "app"]);
    let cloned = git_stamp(&app).expect("a clone has a stamp");
    assert_ne!(cloned, recorded);
    assert!(!Disk.recorded_checkout_present(&app, Some(&recorded)), "another checkout is there");
    assert!(Disk.recorded_checkout_present(&app, Some(&cloned)));

    let plain = base.join("plain");
    fs::create_dir_all(&plain).expect("create plain dir");
    assert_eq!(Disk.checkout(&plain).stamp, None, "outside git there is no git directory");
}

#[test]
fn a_path_in_a_worktree_has_its_subpath_from_the_worktree() {
    let (_tmp, base) = canonical_tempdir();
    git(&base, &["init", "mono"]);
    let mono = base.join("mono");
    commit(&mono, "first");
    git(&mono, &["worktree", "add", "-b", "topic", "../mono-wt"]);
    let deep = base.join("mono-wt").join("apps").join("web").join("src");
    fs::create_dir_all(&deep).expect("create apps/web/src");
    fs::create_dir_all(mono.join("apps").join("web")).expect("create apps/web");

    assert_eq!(Disk.subpath(&mono, &deep).as_deref(), Some("apps/web/src"));
    assert_eq!(Disk.subpath(&mono, &mono.join("apps").join("web")).as_deref(), Some("apps/web"));
    assert_eq!(Disk.subpath(&mono, &mono).as_deref(), Some(""), "the top of the checkout");
    assert_eq!(Disk.subpath(&mono, &base.join("mono-wt")).as_deref(), Some(""), "the top of the worktree");
    assert_eq!(ProjectIdentity::resolve(&deep).path, deep, "the path the root was found from");
}

#[test]
fn a_path_that_does_not_lead_to_its_root_has_no_subpath() {
    let (_tmp, base) = canonical_tempdir();
    for repo in ["a", "b"] {
        git(&base, &["init", repo]);
    }
    let src = base.join("b").join("src");
    fs::create_dir_all(&src).expect("create b/src");
    let plain = base.join("plain");
    fs::create_dir_all(&plain).expect("create plain dir");

    assert_eq!(Disk.subpath(&base.join("a"), &src), None, "a path of another repository");
    assert_eq!(Disk.subpath(&base.join("b"), &src).as_deref(), Some("src"));
    if resolve_project_root(&base) == base {
        assert_eq!(Disk.subpath(&plain, &plain), None, "outside git there is no subpath");
    }
}
