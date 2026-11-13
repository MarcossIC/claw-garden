//! What keeps a store right over time: changing one memory, drawing
//! conclusions, the maintenance passes, getting the embedder ready at start,
//! and finding and purging secrets.

pub mod admin;
pub mod bootstrap;
pub(crate) mod maintenance;
pub mod secrets;
pub mod synthesis;
