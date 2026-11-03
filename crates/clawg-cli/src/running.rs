//! The daemon a command finds running.
//!
//! The commands that read or change the store through the daemon (`import`,
//! `undelete`, `outbox`, `projects`, `memories`) never start one: reading
//! must not launch a process the user did not ask for, and a daemon nobody
//! asked for would also hold the store lock. So a missing daemon is a failure
//! that says how to start it, in the command's language.

use std::path::Path;

use clawg_core::paths::Profile;
use clawg_core::preferences::Lang;

use crate::client::{probe_running, ClientError, DaemonClient, Probe};
use crate::error::CliError;
use crate::memories::explain_unavailable;

/// A daemon that answered, with the language and the profile its command
/// speaks of it in.
#[derive(Debug)]
pub struct Running {
    pub client: DaemonClient,
    pub lang: Lang,
    pub profile: Profile,
}

impl Running {
    /// The daemon of `profile` published in `base_dir`, when it answers;
    /// none is started.
    pub async fn find(profile: Profile, base_dir: &Path, lang: Lang) -> Result<Self, CliError> {
        match probe_running(base_dir, false).await {
            Ok(Probe::Running(client)) => Ok(Self { client, lang, profile }),
            Ok(Probe::NeedsStart) => Err(unavailable(lang, profile, ClientError::NotRunning)),
            Err(e) => Err(unavailable(lang, profile, e)),
        }
    }

    /// A request to this daemon that failed, as the command says it.
    pub fn unavailable(&self, err: ClientError) -> CliError {
        unavailable(self.lang, self.profile, err)
    }
}

fn unavailable(lang: Lang, profile: Profile, err: ClientError) -> CliError {
    CliError::Failed(explain_unavailable(lang, &err, profile))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tests::{publish, start_daemon};

    #[tokio::test]
    async fn a_missing_daemon_says_how_to_start_it_in_both_languages() {
        let dir = tempfile::tempdir().expect("tempdir");
        for (lang, said) in [
            (Lang::En, "no clawg daemon is running for profile dev; start it with 'clawg serve --profile dev'"),
            (Lang::Es, "no hay ningún daemon de clawg corriendo para el perfil dev; arráncalo con 'clawg serve --profile dev'"),
        ] {
            let err = Running::find(Profile::Dev, dir.path(), lang).await.expect_err("no daemon was published");
            assert_eq!(err.to_string(), said);
        }
    }

    #[tokio::test]
    async fn a_published_daemon_is_found() {
        let d = start_daemon().await;
        publish(&d);

        let running = Running::find(Profile::Dev, d.dir.path(), Lang::Es).await.expect("the daemon answers");

        assert_eq!(running.client.health().await.expect("health").status, "ok");
        assert_eq!((running.lang, running.profile), (Lang::Es, Profile::Dev));
        let lost = running.unavailable(ClientError::NotRunning).to_string();
        assert!(lost.starts_with("no hay ningún daemon de clawg corriendo"), "{lost}");
    }
}
