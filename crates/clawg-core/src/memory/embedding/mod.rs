//! From text to vectors: the model's files, the embedder and its idle unload,
//! and the cuts that fit a text to the embedder's window.

pub mod chunker;
pub mod embedder;
pub mod idle_embedder;
pub(crate) mod model_download;
