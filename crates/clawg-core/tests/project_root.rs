use std::fs;
use std::path::{Path, PathBuf};

use clawg_core::memory::identity::{origin_key, resolve_project_root, strip_verbatim};
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
