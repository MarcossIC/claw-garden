//! An index of each registered project's documentation: its Markdown files and
//! manifests, split by heading, embedded and searchable by meaning and by term.
//!
//! Derived from the files on disk and rebuilt from them at will. Nothing here
//! reads or writes a memory.

pub mod corpus;
