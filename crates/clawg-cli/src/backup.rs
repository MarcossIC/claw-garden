//! `clawg backup`: a snapshot of the store, from the daemon when one runs.

use std::path::{Path, PathBuf};

use clawg_core::memory::backup::backup_offline;

use crate::client::{probe_running, ClientError, Probe};
use crate::error::CliError;

/// Snapshots the store in `base_dir` and returns the copy's path. A running
/// daemon holds the store and its only connection, so the snapshot has to
/// come from it; with none, no daemon is started just to copy a file.
pub async fn command(base_dir: &Path) -> Result<PathBuf, CliError> {
    match probe_running(base_dir, false).await {
        Ok(Probe::Running(client)) => Ok(client.backup().await?),
        Ok(Probe::NeedsStart) | Err(ClientError::NotRunning) => Ok(backup_offline(base_dir)?),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawg_core::paths::backups_dir;

    use crate::client::tests::{publish, start_daemon};

    #[tokio::test]
    async fn a_running_daemon_takes_the_snapshot() {
        let d = start_daemon().await;
        publish(&d);

        let path = command(d.dir.path()).await.expect("a backup");

        assert!(path.starts_with(backups_dir(d.dir.path())) && path.is_file(), "{}", path.display());
    }

    #[tokio::test]
    async fn without_a_daemon_the_missing_store_is_said_as_it_came() {
        let dir = tempfile::tempdir().expect("tempdir");

        let err = command(dir.path()).await.expect_err("there is no store");

        let offline = backup_offline(dir.path()).expect_err("there is no store");
        assert!(matches!(err, CliError::Memory(_)), "{err:?}");
        assert_eq!(err.to_string(), offline.to_string());
    }
}
