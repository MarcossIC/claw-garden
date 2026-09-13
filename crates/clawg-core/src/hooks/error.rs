use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("pattern inválido '{raw}': {reason}")]
    InvalidPattern { raw: String, reason: String },

    #[error("Hook [{event}][{index}]: {source}")]
    HookCompile {
        event: String,
        index: usize,
        #[source]
        source: Box<ConfigError>,
    },
}
