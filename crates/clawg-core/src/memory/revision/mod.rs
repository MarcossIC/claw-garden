//! How a save stands to the ones before it: it contradicts one, disputes it,
//! replaces it under a topic key, or is brought back when its replacement goes.

pub(crate) mod contradiction;
pub mod dispute;
pub(crate) mod revival;
pub mod topic;
