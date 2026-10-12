//! `clawg restore` — what it prints, kept apart from the command so the
//! wording is under test without a store.

use std::path::Path;

use clawg_core::memory::backup::Restored;
use clawg_core::memory::store_lock::StoreHolder;
use clawg_core::memory::MemoryError;

/// The line printed after a restore.
pub fn report(from: &Path, restored: &Restored) -> String {
    match &restored.previous {
        Some(previous) => format!(
            "restored {}; the store it replaced is at {}",
            from.display(),
            previous.display()
        ),
        None => format!("restored {}", from.display()),
    }
}

/// A restore refused because the daemon holds the store says how to free it.
pub fn explain(err: MemoryError) -> String {
    match &err {
        MemoryError::StoreInUse { owner: Some(o) } if o.holder == StoreHolder::Daemon => {
            format!("{err}; run `clawg stop` first, then restore")
        }
        _ => err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawg_core::memory::store_lock::StoreOwner;
    use std::path::PathBuf;

    #[test]
    fn a_store_held_by_the_daemon_says_to_stop_it() {
        let err = MemoryError::StoreInUse {
            owner: Some(StoreOwner { holder: StoreHolder::Daemon, pid: 7, started_at_ms: 1 }),
        };
        let msg = explain(err);
        assert!(msg.contains("the clawg daemon (pid 7)"), "{msg}");
        assert!(msg.contains("clawg stop"), "{msg}");
    }

    #[test]
    fn other_refusals_keep_their_own_message() {
        let err = MemoryError::StoreInUse {
            owner: Some(StoreOwner { holder: StoreHolder::Eval, pid: 7, started_at_ms: 1 }),
        };
        assert!(!explain(err).contains("clawg stop"));
        assert_eq!(explain(MemoryError::Internal("boom".into())), "internal memory error: boom");
    }

    #[test]
    fn the_report_names_where_the_replaced_store_went() {
        let from = Path::new("/b/memory-x-manual.db");
        let kept = Restored { previous: Some(PathBuf::from("/b/memory-y-pre-restore.db")) };
        let msg = report(from, &kept);
        assert!(msg.contains("/b/memory-x-manual.db") && msg.contains("pre-restore"), "{msg}");
        assert_eq!(report(from, &Restored { previous: None }), "restored /b/memory-x-manual.db");
    }
}
