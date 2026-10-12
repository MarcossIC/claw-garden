//! Waiting for another process — the daemon — to be gone.
//!
//! # Why `clawg stop` needs this
//!
//! Asking the daemon to shut down is not the same as the daemon being gone.
//! Until the process ends it still owns the memory store (so `clawg stop &&
//! clawg serve` loses the race and reports the store as in use), and on Windows it
//! still holds its own `clawg.exe` image, which an installer updating that
//! file cannot overwrite. Only the end of the process releases either.
//!
//! # Why the handle is opened before the shutdown request
//!
//! A pid names a process only while that process exists; once it exits the
//! number can be handed to something else. [`Watched::open`] takes the handle
//! while the daemon is known to be alive — it just answered `/health` — so the
//! wait that follows can only ever be about that process.

use std::time::Duration;

/// How a wait ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Exit {
    Exited,
    TimedOut,
}

/// A process being waited on.
pub struct Watched {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(unix)]
    pid: u32,
}

// SAFETY: a process handle is a kernel object reference, usable from any
// thread; nothing here is tied to the thread that opened it.
#[cfg(windows)]
unsafe impl Send for Watched {}

#[cfg(windows)]
impl Watched {
    /// Starts watching `pid`, or `None` when no such process exists (it has
    /// already exited, or was never there).
    pub fn open(pid: u32) -> Option<Self> {
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};
        // SYNCHRONIZE is the only right a wait needs, and the one a same-user
        // process always grants.
        // SAFETY: plain FFI call; a null result is handled below.
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        (!handle.is_null()).then_some(Self { handle })
    }

    /// Blocks until the process ends or `timeout` passes.
    pub fn wait(&self, timeout: Duration) -> Exit {
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::WaitForSingleObject;
        // A process handle is signalled when the process ends. Clamped below
        // INFINITE (u32::MAX), which would turn a long timeout into none.
        let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1).min(u32::MAX - 1);
        // SAFETY: `self.handle` is open for as long as `self` lives.
        match unsafe { WaitForSingleObject(self.handle, ms) } {
            WAIT_OBJECT_0 => Exit::Exited,
            _ => Exit::TimedOut,
        }
    }
}

#[cfg(windows)]
impl Drop for Watched {
    fn drop(&mut self) {
        // SAFETY: the handle came from OpenProcess and is closed exactly once.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.handle) };
    }
}

#[cfg(unix)]
impl Watched {
    /// Starts watching `pid`, or `None` when no such process exists.
    pub fn open(pid: u32) -> Option<Self> {
        let watched = Self { pid };
        watched.alive().then_some(watched)
    }

    /// Polls until the process ends or `timeout` passes. Unix has no handle to
    /// wait on for a process that is not our child, so this asks the kernel
    /// whether the pid still exists.
    pub fn wait(&self, timeout: Duration) -> Exit {
        let deadline = std::time::Instant::now() + timeout;
        while self.alive() {
            if std::time::Instant::now() >= deadline {
                return Exit::TimedOut;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Exit::Exited
    }

    fn alive(&self) -> bool {
        let Ok(pid) = libc::pid_t::try_from(self.pid) else {
            return false;
        };
        // SAFETY: signal 0 performs the existence check and delivers nothing.
        let rc = unsafe { libc::kill(pid, 0) };
        // EPERM means the process exists but belongs to someone else.
        rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use std::time::Instant;

    /// A process that stays alive for about half a minute unless killed.
    fn long_lived() -> Child {
        #[cfg(windows)]
        let mut cmd = {
            let mut c = Command::new("ping");
            c.args(["-n", "30", "127.0.0.1"]);
            c
        };
        #[cfg(unix)]
        let mut cmd = {
            let mut c = Command::new("sleep");
            c.arg("30");
            c
        };
        cmd.stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn a long-lived process")
    }

    #[test]
    fn a_running_process_is_still_running_when_the_timeout_passes() {
        let mut child = long_lived();
        let watched = Watched::open(child.id()).expect("the child is alive");

        let started = Instant::now();
        let outcome = watched.wait(Duration::from_millis(300));
        child.kill().expect("kill");
        let _ = child.wait();

        assert_eq!(outcome, Exit::TimedOut);
        assert!(started.elapsed() >= Duration::from_millis(250), "the wait really waited");
    }

    #[test]
    fn the_wait_ends_as_soon_as_the_process_does() {
        let mut child = long_lived();
        let watched = Watched::open(child.id()).expect("the child is alive");
        child.kill().expect("kill");
        let _ = child.wait();

        let started = Instant::now();
        assert_eq!(watched.wait(Duration::from_secs(10)), Exit::Exited);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "an ended process must not be waited out to the timeout"
        );
    }

    #[test]
    fn a_pid_that_names_no_process_cannot_be_watched() {
        // Windows pids are multiples of four, so this one is never issued;
        // on Unix it is far beyond any pid_max.
        assert!(Watched::open(4_294_967_291).is_none());
    }
}
