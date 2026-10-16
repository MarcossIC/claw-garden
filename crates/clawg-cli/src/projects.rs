//! `clawg projects add`: let the daemon read a project's files.
//!
//! A bridge only has its own root indexed; any other root the model names is
//! a label for memories. This command is how the user, not the model, adds
//! one. Registering goes through the running daemon, which owns the store.

use std::path::{Path, PathBuf};

use clawg_core::memory::ProjectIdentity;

use crate::client::{ClientError, DaemonClient};

#[derive(Debug, thiserror::Error)]
pub enum AddError {
    #[error("{} is not a directory", .0.display())]
    NotADirectory(PathBuf),
    #[error(transparent)]
    Client(#[from] ClientError),
}

/// Registers the project `path` is in so the daemon may read its files, and
/// says which root and id that was.
pub async fn add(client: &DaemonClient, path: &Path) -> Result<String, AddError> {
    let project = ProjectIdentity::resolve(path);
    if !project.root.is_dir() {
        return Err(AddError::NotADirectory(project.root));
    }
    let legacy: Vec<String> = Some(project.legacy_id.clone()).filter(|l| *l != project.id).into_iter().collect();
    let (id, _) = client.register_project(&project.root, &legacy, true).await?;
    Ok(format!(
        "added {} as project {id}; the daemon is indexing its files in the background",
        project.root.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tests::{publish, start_daemon};

    #[tokio::test]
    async fn adding_a_project_lets_its_docs_be_searched() {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");
        let repo = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(repo.path().join(".git")).unwrap();
        let root = ProjectIdentity::resolve(repo.path()).root;
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(root.join("README.md"), "# Other\nHow the other project ships.\n").unwrap();
        let (id, _) = c.register_project(&root, &[], false).await.expect("register as a label");
        let refused = c.search_docs(&id, "How the other project ships", None).await.expect_err("not accepted");
        assert!(matches!(refused, ClientError::Daemon { status: 403, .. }), "{refused:?}");

        let out = add(&c, &root.join("docs")).await.expect("add");

        assert_eq!(
            out,
            format!("added {} as project {id}; the daemon is indexing its files in the background", root.display())
        );
        let found = c.search_docs(&id, "How the other project ships", None).await.expect("search");
        assert_eq!(found.hits.first().map(|h| h.path.as_str()), Some("README.md"), "{:?}", found.hits);
    }

    #[tokio::test]
    async fn adding_a_path_that_is_not_a_directory_is_refused() {
        // Nothing listens here: the path is refused before the daemon is asked.
        let c = DaemonClient::new("http://127.0.0.1:1", "token", std::env::temp_dir());
        let dir = tempfile::tempdir().expect("tempdir");

        let err = add(&c, &dir.path().join("gone")).await.expect_err("missing");

        assert!(matches!(err, AddError::NotADirectory(_)), "{err:?}");
    }
}
