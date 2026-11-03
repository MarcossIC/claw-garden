//! What a `clawg` command fails with. `main` prints it after `clawg: ` on
//! stderr and exits with 1.
//!
//! A typed error keeps its type and its own text: it is the technical cause,
//! shown as it came. A failure clawg says in words of its own is composed by
//! the command's module, in the command's language, and travels here as
//! [`CliError::Failed`]; neither `main` nor the dispatch composes a sentence.

use clawg_core::memory::portability::export::ExportError;
use clawg_core::memory::MemoryError;
use clawg_core::paths::PathsError;

use crate::cli::ProfileArgError;
use crate::client::ClientError;
use crate::daemon::serve::ServeError;
use crate::doctor::DoctorError;
use crate::import::ImportError;
use crate::mcp::McpError;
use crate::projects::AddError;
use crate::setup::SetupError;
use crate::update::UpdateError;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error(transparent)]
    Profile(#[from] ProfileArgError),
    #[error(transparent)]
    Paths(#[from] PathsError),
    #[error(transparent)]
    Memory(#[from] MemoryError),
    #[error(transparent)]
    Export(#[from] ExportError),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Import(#[from] ImportError),
    #[error(transparent)]
    Project(#[from] AddError),
    #[error(transparent)]
    Serve(#[from] ServeError),
    #[error(transparent)]
    Mcp(#[from] McpError),
    #[error(transparent)]
    Doctor(#[from] DoctorError),
    #[error(transparent)]
    Update(#[from] UpdateError),
    #[error(transparent)]
    Setup(#[from] SetupError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// A failure whose sentence the command's module already composed, in
    /// the command's language. Built by name, never from any `String`, so a
    /// typed error cannot turn into text unseen.
    #[error("{0}")]
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use crate::cli::ProfileArg;

    #[test]
    fn a_typed_error_reads_as_its_source() {
        let profile = ProfileArg { profile: Some("staging".into()) }.resolve().expect_err("not a profile");
        let client = ClientError::StopTimeout { pid: 7, timeout: Duration::from_secs(30) };
        let io = std::io::Error::other("disk full");
        let expected = [profile.to_string(), client.to_string(), io.to_string(), PathsError::NoHome.to_string()];

        let shown = [
            CliError::from(profile).to_string(),
            CliError::from(client).to_string(),
            CliError::from(io).to_string(),
            CliError::from(PathsError::NoHome).to_string(),
        ];

        assert_eq!(shown, expected);
    }

    #[test]
    fn a_composed_sentence_reads_as_it_was_written() {
        let sentence = "no hay ningún daemon de clawg corriendo para el perfil dev";
        assert_eq!(CliError::Failed(sentence.into()).to_string(), sentence);
    }
}
