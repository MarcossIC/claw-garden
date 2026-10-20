//! The update's contact with the world: the release server, the files beside
//! the installed binary, and the daemon's processes.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use clawg_core::build_info;
use clawg_core::memory::backup;
use clawg_core::memory::store_lock::StoreLock;
use clawg_core::paths::{backups_dir, Profile};

use crate::client::{spawn_serve_at, ClientError, DaemonClient, Health};
use crate::daemon::lifecycle::{clear_state, read_state};
use crate::process::{Exit, Watched};
use crate::stop::{stop_in, STOP_TIMEOUT};

use super::client::ReleaseClient;
use super::plan::{
    judge, new_pre_migration, store_needs_restore, Answer, Effects, Observation, Running, Step, StepError,
    StoreRestore, Undo, Verdict,
};
use super::release::Release;
use super::report::{install_dir_access, NewDaemon};
use super::version::Version;

/// How often a starting daemon is looked at.
const WATCH_INTERVAL: Duration = Duration::from_millis(200);
/// A daemon that holds the store is reported on after this long, then at
/// every [`PROGRESS_EVERY`].
const PROGRESS_AFTER: Duration = Duration::from_secs(5);
const PROGRESS_EVERY: Duration = Duration::from_secs(10);
/// How many lines of the daemon's log a failure quotes.
const LOG_LINES: usize = 8;
/// How long a rename is tried again while another process holds the file.
const RENAME_PATIENCE: Duration = Duration::from_secs(10);
const RENAME_RETRY_EVERY: Duration = Duration::from_millis(100);
/// How long the downloaded binary has to print its version, with
/// `update.lock` held meanwhile. A starting value, not measured.
const SMOKE_TIMEOUT: Duration = Duration::from_secs(30);
const SMOKE_POLL: Duration = Duration::from_millis(50);

/// The files an update moves.
pub struct Paths {
    pub exe: PathBuf,
    /// Where the downloaded binary waits, beside `exe`.
    pub staged: PathBuf,
    /// Where the installed binary is set aside.
    pub aside: PathBuf,
    pub base_dir: PathBuf,
}

/// What the update installs and how it trusts it.
pub struct Candidate {
    pub release: Release,
    pub triple: &'static str,
    pub keys: Vec<String>,
    pub client: ReleaseClient,
}

/// The installation `clawg update` works on and what it has done to it.
pub struct Installation {
    paths: Paths,
    profile: Profile,
    candidate: Candidate,
    /// The version of the binary this process is, the one an undo restarts.
    installed: Version,
    /// The daemon that was running when the update began.
    old: Option<Running>,
    child: Option<Child>,
    /// The daemon that accepted the shutdown and had not ended in time.
    stopping: Option<u32>,
    backups_before: Option<Vec<String>>,
    set_aside: bool,
    new_daemon: Option<NewDaemon>,
}

impl Installation {
    pub fn new(paths: Paths, profile: Profile, candidate: Candidate, installed: Version, old: Option<Running>) -> Self {
        Self {
            paths,
            profile,
            candidate,
            installed,
            old,
            child: None,
            stopping: None,
            backups_before: None,
            set_aside: false,
            new_daemon: None,
        }
    }

    /// Where the previous binary was set aside, if it was.
    pub fn set_aside_at(&self) -> Option<&Path> {
        self.set_aside.then_some(self.paths.aside.as_path())
    }

    /// The daemon of the new version that is running, if one was started.
    pub fn new_daemon(&self) -> Option<&NewDaemon> {
        self.new_daemon.as_ref()
    }

    /// The daemon that answers for the profile now, whoever started it.
    pub async fn answering(&self) -> Option<Answer> {
        ask_health(&self.paths.base_dir).await.map(|h| Answer { pid: h.pid, version: h.version })
    }

    /// The snapshot the daemon started by the update wrote before migrating.
    pub fn pre_migration(&self) -> Option<PathBuf> {
        let before = self.backups_before.as_ref()?;
        let name = new_pre_migration(before, &self.backup_names())?;
        Some(backups_dir(&self.paths.base_dir).join(name))
    }

    fn backup_names(&self) -> Vec<String> {
        std::fs::read_dir(backups_dir(&self.paths.base_dir))
            .map(|entries| {
                entries.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect()
            })
            .unwrap_or_default()
    }

    async fn stage(&self) -> Result<(), String> {
        let dir = self.paths.staged.parent().unwrap_or(Path::new("."));
        // Creating the file is the permission check, and it comes before any
        // request: a directory the user cannot write to costs nothing.
        let created = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.paths.staged)
            .map(drop)
            .map_err(|e| e.kind());
        install_dir_access(dir, created)?;
        std::fs::remove_file(&self.paths.staged).map_err(|e| format!("cannot remove {}: {e}", self.paths.staged.display()))?;

        let keys: Vec<&str> = self.candidate.keys.iter().map(String::as_str).collect();
        self.candidate.client
            .fetch_binary(&self.candidate.release, self.candidate.triple, &keys, &self.paths.staged)
            .await
            .map_err(|e| e.to_string())?;
        make_executable(&self.paths.staged)
    }

    /// The downloaded binary must run and say it is the version the release
    /// announced.
    async fn smoke(&self) -> Result<(), String> {
        let expected = format!("clawg {}", self.candidate.release.version);
        // A virus scanner reading the file just written can keep Windows from
        // creating the process for a moment.
        let attempts = if cfg!(windows) { 3 } else { 1 };
        let mut why = String::new();
        for attempt in 1..=attempts {
            match output_within(Command::new(&self.paths.staged).arg("--version"), SMOKE_TIMEOUT).await {
                Ok(Some(out)) => {
                    let said = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if out.status.success() && said == expected {
                        return Ok(());
                    }
                    return Err(format!("the downloaded binary says {said:?} instead of {expected:?}"));
                }
                Ok(None) => {
                    return Err(format!(
                        "the downloaded binary did not print its version within {} s",
                        SMOKE_TIMEOUT.as_secs()
                    ));
                }
                Err(e) => why = e.to_string(),
            }
            if attempt < attempts {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
        Err(format!("cannot run the downloaded binary: {why}"))
    }

    /// Starts the binary at `exe` as the profile's daemon on `port` and waits
    /// until a daemon of `version` answers. When a daemon of another version
    /// got the store first (an editor's bridge started it between the stop and
    /// this start), it is stopped and the start is tried once more.
    async fn start_and_wait(&mut self, exe: &Path, version: &str, port: Option<u16>) -> Result<Health, String> {
        for attempt in 1..=2 {
            let child = spawn_serve_at(exe, &self.paths.base_dir, self.profile, port).map_err(|e| e.to_string())?;
            println!("starting the daemon (pid {})", child.id());
            self.child = Some(child);
            match self.wait_for(version).await {
                Ok(health) => return Ok(health),
                Err(Waited::Other(other)) if attempt == 1 => {
                    println!(
                        "a daemon {} (pid {}) got the store first; stopping it to start the right one",
                        other.version, other.pid
                    );
                    // The child that lost may still be starting: ended first, it
                    // cannot take the store the stop frees, out of reach of any undo.
                    self.end_child()?;
                    stop_in(&self.paths.base_dir, STOP_TIMEOUT).await.map_err(|e| e.to_string())?;
                }
                Err(Waited::Other(other)) => {
                    return Err(format!(
                        "a daemon {} (pid {}) keeps taking the store, started by something that was open \
                         before the update; run `clawg stop` and then any clawg command",
                        other.version, other.pid
                    ));
                }
                Err(Waited::Failed(why)) => return Err(why),
            }
        }
        Err("the daemon did not start".to_string())
    }

    async fn wait_for(&mut self, version: &str) -> Result<Health, Waited> {
        let started = Instant::now();
        let mut last_report: Option<Instant> = None;
        loop {
            let child = self.child.as_mut().expect("a daemon was started");
            let pid = child.id();
            let exited = child.try_wait().ok().flatten();
            let health = ask_health(&self.paths.base_dir).await;
            let seen = Observation {
                exited: exited.is_some(),
                store_owner: StoreLock::recorded_owner(&self.paths.base_dir).map(|o| o.pid),
                answering: health.as_ref().map(|h| Answer { pid: h.pid, version: h.version.clone() }),
            };
            let elapsed = started.elapsed();
            match judge(&seen, elapsed, pid, version) {
                Verdict::Ready => return Ok(health.expect("a daemon answered")),
                Verdict::Exited => {
                    let status = exited.map(|s| s.to_string()).unwrap_or_default();
                    return Err(Waited::Failed(format!(
                        "the daemon ended ({status}) before it answered\n{}",
                        self.log_tail()
                    )));
                }
                Verdict::TimedOut => {
                    return Err(Waited::Failed(format!(
                        "the daemon did not answer within {} s\n{}",
                        elapsed.as_secs(),
                        self.log_tail()
                    )));
                }
                Verdict::Other { pid, version } => return Err(Waited::Other(Answer { pid, version })),
                Verdict::Wait => {
                    let holds_store = seen.store_owner == Some(pid);
                    let due = last_report.map_or(elapsed >= PROGRESS_AFTER, |at| at.elapsed() >= PROGRESS_EVERY);
                    if holds_store && due {
                        last_report = Some(Instant::now());
                        let snapshot = self
                            .pre_migration()
                            .map(|p| format!(" (pre-migration snapshot {})", p.display()))
                            .unwrap_or_default();
                        println!("the daemon is preparing the store{snapshot}, {} s", elapsed.as_secs());
                    }
                    tokio::time::sleep(WATCH_INTERVAL).await;
                }
            }
        }
    }

    /// Ends the process the update started, if any, and waits for it.
    fn end_child(&mut self) -> Result<(), String> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        let pid = child.id();
        // By its own handle, never by name: a clawg the user's editor runs
        // must survive.
        let _ = child.kill();
        child.wait().map_err(|e| format!("cannot wait for the daemon (pid {pid}) to end: {e}"))?;
        if read_state(&self.paths.base_dir).is_some_and(|s| s.pid == pid) {
            clear_state(&self.paths.base_dir);
        }
        Ok(())
    }

    fn log_tail(&self) -> String {
        let log = std::fs::read_to_string(self.paths.base_dir.join("logs").join("serve.log")).unwrap_or_default();
        let tail = tail_lines(&log, LOG_LINES);
        if tail.is_empty() {
            String::new()
        } else {
            format!("last lines of the daemon's log:\n{tail}")
        }
    }
}

/// The `--port` of a daemon an update starts, the new one or the old one
/// again. None when the daemon that ran published its socket: it starts as an
/// autostart does, watched over the socket, and TCP stays only where someone
/// asked for it again. A daemon that served TCP alone (its socket path did not
/// fit, or a clawg older than the socket) keeps its port, since without
/// `--port` the new one would refuse to start; so does any daemon when this
/// binary is stamped `tcp_only`.
fn serve_port(old_port: Option<u16>, socket: bool, tcp_only: bool) -> Option<u16> {
    if socket && !tcp_only {
        None
    } else {
        old_port
    }
}

/// Waits for the daemon whose shutdown ran out of time to end. Started before
/// then, the old daemon would lose the store to it, and the daemon still
/// answering with the same version would pass for the one just started.
async fn wait_for_end(pid: u32, timeout: Duration) -> Result<(), String> {
    let Some(watched) = Watched::open(pid) else {
        return Ok(());
    };
    match tokio::task::spawn_blocking(move || watched.wait(timeout)).await {
        Ok(Exit::Exited) => Ok(()),
        Ok(Exit::TimedOut) | Err(_) => Err(format!(
            "the daemon (pid {pid}) was asked to stop and was still running {} s later",
            timeout.as_secs()
        )),
    }
}

/// Why waiting for a daemon ended without one.
enum Waited {
    Other(Answer),
    Failed(String),
}

/// Runs `command` and waits at most `timeout` for it to end; one still
/// running then is ended and waited for, and `None` comes back. Its stdout is
/// read once it has ended, which fits a line of `--version`: a process that
/// fills the pipe first is taken for one that hangs.
async fn output_within(command: &mut Command, timeout: Duration) -> std::io::Result<Option<Output>> {
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            let mut stdout = Vec::new();
            if let Some(mut pipe) = child.stdout.take() {
                pipe.read_to_end(&mut stdout)?;
            }
            return Ok(Some(Output { status, stdout, stderr: Vec::new() }));
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            child.wait()?;
            return Ok(None);
        }
        tokio::time::sleep(SMOKE_POLL).await;
    }
}

async fn ask_health(base_dir: &Path) -> Option<Health> {
    DaemonClient::from_data_dir(base_dir)?.health().await.ok()
}

/// The last `count` non-empty lines of `text`.
fn tail_lines(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(count)..].join("\n")
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let failed = |e: std::io::Error| format!("cannot make {} executable: {e}", path.display());
    let file = std::fs::File::open(path).map_err(failed)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o755)).map_err(failed)?;
    // The mode belongs to the file, not to the directory synced after the swap.
    file.sync_all().map_err(failed)
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

async fn rename(from: &Path, to: &Path) -> Result<(), String> {
    rename_within(from, to, RENAME_PATIENCE).await
}

async fn rename_within(from: &Path, to: &Path, patience: Duration) -> Result<(), String> {
    let started = Instant::now();
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            // A virus scanner reading a binary just written or run keeps it
            // open without sharing deletion, and Windows refuses to rename it
            // until the scan ends.
            Err(e) if held_by_another_process(&e) && started.elapsed() < patience => {
                tokio::time::sleep(RENAME_RETRY_EVERY).await;
            }
            Err(e) => return Err(format!("cannot rename {} to {}: {e}", from.display(), to.display())),
        }
    }
}

#[cfg(windows)]
fn held_by_another_process(e: &std::io::Error) -> bool {
    e.raw_os_error() == Some(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32)
}

#[cfg(not(windows))]
fn held_by_another_process(_: &std::io::Error) -> bool {
    false
}

impl Effects for Installation {
    async fn run(&mut self, step: Step) -> Result<(), StepError> {
        match step {
            Step::Stage => Ok(self.stage().await?),
            Step::Smoke => Ok(self.smoke().await?),
            Step::StopDaemon => match stop_in(&self.paths.base_dir, STOP_TIMEOUT).await {
                Ok(pid) => {
                    println!("stopped the daemon (pid {pid})");
                    Ok(())
                }
                Err(e @ ClientError::StopTimeout { pid, .. }) => {
                    self.stopping = Some(pid);
                    Err(StepError { cause: e.to_string(), under_way: true })
                }
                Err(e) => Err(e.to_string().into()),
            },
            Step::SetAside => {
                rename(&self.paths.exe, &self.paths.aside).await?;
                self.set_aside = true;
                Ok(())
            }
            Step::PutInPlace => {
                rename(&self.paths.staged, &self.paths.exe).await?;
                // Until the directory is synced a power cut can keep the first
                // rename and lose the second, leaving no binary under the name.
                let dir = self.paths.exe.parent().unwrap_or(Path::new("."));
                Ok(backup::sync_dir(dir).map_err(|e| format!("cannot sync {}: {e}", dir.display()))?)
            }
            Step::StartDaemon => {
                let old = self.old.as_ref().ok_or_else(|| "there was no daemon to start".to_string())?;
                let port = serve_port(old.port, old.socket, build_info::tcp_only());
                self.backups_before = Some(self.backup_names());
                let (exe, version) = (self.paths.exe.clone(), self.candidate.release.version.to_string());
                let health = self.start_and_wait(&exe, &version, port).await?;
                self.new_daemon = Some(NewDaemon { pid: health.pid, embedder: health.embedder });
                Ok(())
            }
        }
    }

    async fn undo(&mut self, undo: Undo) -> Result<(), String> {
        match undo {
            Undo::RemoveStaged => match std::fs::remove_file(&self.paths.staged) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    Err(format!("cannot remove {}: {e}", self.paths.staged.display()))
                }
                _ => Ok(()),
            },
            Undo::StartOldDaemon => {
                let old = self.old.clone().ok_or("there was no daemon to start again")?;
                if let Some(pid) = self.stopping.take() {
                    wait_for_end(pid, STOP_TIMEOUT).await?;
                }
                let exe = self.paths.exe.clone();
                // The daemon that was running may report another version than
                // the binary put back (one started before the file was replaced).
                let version = self.installed.to_string();
                let port = serve_port(old.port, old.socket, build_info::tcp_only());
                self.start_and_wait(&exe, &version, port).await.map(drop)
            }
            Undo::RestoreAside => {
                rename(&self.paths.aside, &self.paths.exe).await?;
                self.set_aside = false;
                Ok(())
            }
            Undo::TakeBackNew => rename(&self.paths.exe, &self.paths.staged).await,
            Undo::EndNewDaemon => {
                self.end_child()?;
                self.new_daemon = None;
                Ok(())
            }
        }
    }

    async fn restore_store(&mut self) -> Result<StoreRestore, String> {
        // This process is still the previous binary, so the probe asks the
        // question that matters: can the binary being put back open the store.
        let probe = backup::probe(&self.paths.base_dir);
        if !store_needs_restore(&probe.state) {
            return Ok(StoreRestore::NotNeeded);
        }
        let from = self
            .pre_migration()
            .ok_or("the store was migrated, but the snapshot the new daemon wrote before migrating was not found")?;
        let restored = backup::restore(&self.paths.base_dir, &from).map_err(|e| e.to_string())?;
        Ok(StoreRestore::Restored { from, kept_as: restored.previous })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_new_daemon_gets_a_port_only_where_tcp_is_the_only_transport() {
        assert_eq!(serve_port(Some(40123), true, false), None, "the socket is the way in");
        assert_eq!(serve_port(None, true, false), None);
        assert_eq!(serve_port(Some(40123), false, false), Some(40123), "it served TCP alone");
        assert_eq!(serve_port(Some(40123), true, true), Some(40123), "as an older updater does");
        assert_eq!(serve_port(None, false, false), None, "the profile's port then");
    }

    /// A process of the system that runs for half a minute and prints little.
    fn long_running() -> Command {
        if cfg!(windows) {
            let mut command = Command::new("ping");
            command.args(["-n", "30", "127.0.0.1"]);
            command
        } else {
            let mut command = Command::new("sleep");
            command.arg("30");
            command
        }
    }

    #[tokio::test]
    async fn a_process_still_running_at_its_deadline_is_ended() {
        let started = Instant::now();

        let out = output_within(&mut long_running(), Duration::from_millis(300)).await.expect("it started");

        assert!(out.is_none(), "no output from a process that did not end in time");
        assert!(started.elapsed() < Duration::from_secs(10), "the wait ended at the deadline, not with the process");
    }

    #[tokio::test]
    async fn a_process_that_ends_in_time_gives_its_output() {
        let mut command = if cfg!(windows) { Command::new("cmd") } else { Command::new("sh") };
        command.args([if cfg!(windows) { "/C" } else { "-c" }, "echo smoke"]);

        let out = output_within(&mut command, Duration::from_secs(30)).await.expect("it started").expect("it ended");

        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "smoke");
    }

    #[test]
    fn the_log_tail_keeps_the_last_lines_without_blanks() {
        let log = "one\n\ntwo\nthree\n   \nfour\n";

        assert_eq!(tail_lines(log, 2), "three\nfour");
        assert_eq!(tail_lines(log, 10), "one\ntwo\nthree\nfour");
        assert_eq!(tail_lines("", 3), "");
    }

    /// Opens `path` the way a virus scanner can: without letting anyone
    /// rename it until the handle is dropped.
    #[cfg(windows)]
    fn hold(path: &Path) -> std::fs::File {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().read(true).share_mode(0).open(path).expect("hold the file")
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_rename_waits_for_a_file_another_process_holds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (from, to) = (dir.path().join("clawg.exe"), dir.path().join("clawg.exe.old"));
        std::fs::write(&from, b"binary").expect("write");
        let held = hold(&from);
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(held);
        });

        rename(&from, &to).await.expect("renamed once the file is let go");

        release.join().expect("released");
        assert_eq!(std::fs::read(&to).expect("read the renamed file"), b"binary");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_rename_gives_up_on_a_file_held_past_its_patience() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (from, to) = (dir.path().join("clawg.exe"), dir.path().join("clawg.exe.old"));
        std::fs::write(&from, b"binary").expect("write");
        let _held = hold(&from);

        let started = Instant::now();
        let err = rename_within(&from, &to, Duration::from_millis(300)).await.expect_err("still held");

        assert!(err.contains("cannot rename"), "{err}");
        assert!(started.elapsed() >= Duration::from_millis(250), "the rename was tried again");
        assert!(from.exists() && !to.exists());
    }

    #[tokio::test]
    async fn a_rename_that_cannot_work_is_not_tried_again() {
        let dir = tempfile::tempdir().expect("tempdir");

        let started = Instant::now();
        let err = rename(&dir.path().join("missing"), &dir.path().join("other")).await.expect_err("nothing to rename");

        assert!(err.contains("cannot rename"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(1), "refused at once, not waited on");
    }
}
