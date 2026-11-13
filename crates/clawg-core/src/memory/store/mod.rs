//! The SQLite store: its connection and schema, its lock, its snapshots, its
//! errors and the SQL functions it registers on the connection.

pub mod backup;
pub(crate) mod conn_probe;
pub mod db;
pub(crate) mod dedup;
pub(crate) mod error;
pub(crate) mod latency;
pub(crate) mod migrations;
pub mod store_lock;
pub(crate) mod tier;
