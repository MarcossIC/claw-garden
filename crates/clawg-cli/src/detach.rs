//! What a detached daemon must not take with it from the process that started it.
//!
//! # The leak
//!
//! `std::process::Command` on Windows always calls `CreateProcess` with handle
//! inheritance on, and that copies EVERY inheritable handle of the parent into
//! the child — not only the three it hands the child as stdio. The bridge's
//! own stdin/stdout are pipes its host created inheritable (that is how the
//! bridge got them), so a daemon autostarted by `clawg mcp` used to walk away
//! holding the write end of the host's stdout pipe. The host then never saw
//! end-of-file when the bridge died: on 2026-09-29 Claude Code waited out its
//! whole connect timeout for a bridge that had exited in milliseconds, and a
//! plain `clawg mcp | cat` hung until `clawg stop` killed the daemon.
//!
//! Clearing the inherit flag on the bridge's three std handles before the
//! spawn closes that path. It does not change how the bridge itself uses
//! them, and `Command` still gives the daemon its own stdio (null stdin, the
//! log file) through handles it duplicates for exactly that purpose.
//!
//! On Unix none of this applies: the child's fds 0–2 are replaced by its
//! configured stdio, and everything std opens is close-on-exec.

use std::os::windows::io::{AsRawHandle, RawHandle};

use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT};

/// Stops the current process's stdin, stdout and stderr from being inherited
/// by any child spawned after this call.
pub fn disinherit_std_handles() {
    clear_inherit(&[
        std::io::stdin().as_raw_handle(),
        std::io::stdout().as_raw_handle(),
        std::io::stderr().as_raw_handle(),
    ]);
}

/// Clears `HANDLE_FLAG_INHERIT` on each handle. Best effort by design: a
/// handle it cannot change leaves the daemon holding a pipe it should not —
/// slower end-of-file for the host, never a failed autostart.
fn clear_inherit(handles: &[RawHandle]) {
    for &handle in handles {
        if handle.is_null() {
            continue;
        }
        // SAFETY: SetHandleInformation only reads the handle value; an invalid
        // one makes the call fail, which is ignored on purpose (see above).
        unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Foundation::GetHandleInformation;

    fn is_inheritable(handle: RawHandle) -> bool {
        let mut flags = 0u32;
        // SAFETY: `handle` is a live handle owned by the calling test, and
        // `flags` is a valid out-pointer for the duration of the call.
        let ok = unsafe { GetHandleInformation(handle, &mut flags) };
        assert_ne!(ok, 0, "GetHandleInformation failed");
        flags & HANDLE_FLAG_INHERIT != 0
    }

    #[test]
    fn an_inheritable_handle_stops_being_inheritable() {
        let file = tempfile::tempfile().expect("tempfile");
        let handle = file.as_raw_handle();
        // SAFETY: `handle` belongs to `file`, alive until the end of the test.
        let ok = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
        assert_ne!(ok, 0, "SetHandleInformation failed");
        assert!(is_inheritable(handle), "precondition: the handle starts inheritable");

        clear_inherit(&[handle]);

        assert!(!is_inheritable(handle));
    }

    #[test]
    fn a_missing_std_handle_is_skipped_rather_than_touched() {
        // A process started without a console has null std handles.
        clear_inherit(&[std::ptr::null_mut()]);
    }
}
