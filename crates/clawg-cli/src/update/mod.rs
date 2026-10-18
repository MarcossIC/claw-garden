//! `clawg update`: what to download, whether to trust it, and how.

pub mod client;
pub mod signature;
pub mod version;
pub mod release;
pub mod eligibility;

#[cfg(test)]
mod testkit;
