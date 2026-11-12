//! What a memory is, apart from where it is kept: when it was written, its
//! record, who stated it, who may read it, its vector, and the secrets taken
//! out of its text.

pub mod clock;
pub mod events;
pub mod provenance;
pub(crate) mod record;
pub mod redact;
pub mod sharing;
pub mod vector;
