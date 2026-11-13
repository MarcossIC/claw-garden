//! Saving: what a deliberate save must be before it is accepted, the outbox
//! a save waits in, the worker that drains it, the insert of a row, and the
//! spill file an earlier version left.

pub mod outbox;
pub mod save;
pub mod spill;
pub mod store_insert;
pub mod worker;
