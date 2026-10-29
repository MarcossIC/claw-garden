//! One load, one process: the parent starts `load-eval child`, lets it go
//! once it is accounted for, and reads the line it prints.

use std::io::{BufRead, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use clawg_core::eval_api::{load_once, LoadArm};
use serde::{Deserialize, Serialize};

use crate::plan::Arm;

/// What a child prints: its timing and the counters it reads from itself.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ChildLine {
    pub verify_ms: u64,
    pub session_ms: u64,
    pub rss_peak_bytes: Option<u64>,
    pub private_bytes: Option<u64>,
    pub private_peak_bytes: Option<u64>,
}

/// One load as the parent saw it.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Load {
    /// The peak the gate decides on: the job's peak commit on Windows,
    /// `VmHWM` elsewhere.
    pub peak_bytes: u64,
    #[serde(flatten)]
    pub child: ChildLine,
}

fn core_arm(arm: Arm) -> LoadArm {
    match arm {
        Arm::Vec => LoadArm::Vec,
        Arm::File => LoadArm::File,
        Arm::View => LoadArm::View,
    }
}

/// The body of `load-eval child`: wait for `go`, load once, print one line.
pub fn child(arm: Arm, model_dir: &Path) -> Result<(), String> {
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).map_err(|e| e.to_string())?;
    if line.trim() != "go" {
        return Err("expected `go` on stdin".into());
    }
    let timing = load_once(model_dir, core_arm(arm)).map_err(|e| e.to_string())?;
    let sample = clawg_cli::daemon::process::sample();
    let out = ChildLine {
        verify_ms: timing.verify_ms,
        session_ms: timing.session_ms,
        rss_peak_bytes: sample.rss_peak_bytes,
        private_bytes: sample.private_bytes,
        private_peak_bytes: sample.private_peak_bytes,
    };
    println!("{}", serde_json::to_string(&out).map_err(|e| e.to_string())?);
    Ok(())
}

/// Runs one child for `arm` and measures it.
pub fn measure(arm: Arm, model_dir: &Path) -> Result<Load, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = Command::new(exe)
        .args(["child", "--arm", arm.name(), "--model-dir"])
        .arg(model_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("cannot start the child: {e}"))?;

    // The child waits on stdin, so it is in the job before it loads anything.
    #[cfg(windows)]
    let job = {
        let job = job::Job::new()?;
        job.assign(&child)?;
        job
    };

    let mut stdin = child.stdin.take().ok_or("the child has no stdin")?;
    stdin.write_all(b"go\n").map_err(|e| e.to_string())?;
    drop(stdin);
    let stdout = child.stdout.take().ok_or("the child has no stdout")?;
    let mut line = String::new();
    std::io::BufReader::new(stdout).read_line(&mut line).map_err(|e| e.to_string())?;
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("the child ended with {status}"));
    }
    let parsed: ChildLine = serde_json::from_str(line.trim()).map_err(|e| format!("the child's line is not JSON: {e}"))?;

    #[cfg(windows)]
    let peak_bytes = job.peak_process_bytes()?;
    #[cfg(not(windows))]
    let peak_bytes = parsed.rss_peak_bytes.ok_or("the child read no VmHWM")?;

    Ok(Load { peak_bytes, child: parsed })
}

#[cfg(windows)]
mod job {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, QueryInformationJobObject,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    /// A job that kills what is in it when closed and remembers the peak
    /// commit of its processes.
    pub struct Job(HANDLE);

    const INFO_SIZE: u32 = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;

    impl Job {
        pub fn new() -> Result<Job, String> {
            // SAFETY: no attributes and no name; the handle is closed in `Drop`.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err(format!("CreateJobObjectW: {}", std::io::Error::last_os_error()));
            }
            let job = Job(handle);
            // SAFETY: a zeroed struct is the documented starting point.
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: `info` is a live struct of exactly `INFO_SIZE` bytes.
            let ok = unsafe {
                SetInformationJobObject(handle, JobObjectExtendedLimitInformation, &info as *const _ as *const c_void, INFO_SIZE)
            };
            if ok == 0 {
                return Err(format!("SetInformationJobObject: {}", std::io::Error::last_os_error()));
            }
            Ok(job)
        }

        pub fn assign(&self, child: &std::process::Child) -> Result<(), String> {
            // SAFETY: both handles are open for the call.
            let ok = unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) };
            if ok == 0 {
                return Err(format!("AssignProcessToJobObject: {}", std::io::Error::last_os_error()));
            }
            Ok(())
        }

        /// The most any process of the job committed.
        pub fn peak_process_bytes(&self) -> Result<u64, String> {
            // SAFETY: a zeroed struct that the call fills; its size is passed.
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            let ok = unsafe {
                QueryInformationJobObject(
                    self.0,
                    JobObjectExtendedLimitInformation,
                    &mut info as *mut _ as *mut c_void,
                    INFO_SIZE,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(format!("QueryInformationJobObject: {}", std::io::Error::last_os_error()));
            }
            Ok(info.PeakProcessMemoryUsed as u64)
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle is ours and closed once.
            unsafe { CloseHandle(self.0) };
        }
    }
}
