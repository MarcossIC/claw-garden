//! The memory, CPU and handles of this process, and the load of its async
//! runtime, as the metrics writer samples them once a minute.
//!
//! What a system does not give stays `None`: a missing value must not read
//! as a measured zero.

use std::time::Duration;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcessSample {
    pub rss_bytes: Option<u64>,
    pub rss_peak_bytes: Option<u64>,
    /// Memory only this process holds: `RssAnon` on Linux, `PrivateUsage`
    /// (what Task Manager shows) on Windows.
    pub private_bytes: Option<u64>,
    pub threads: Option<u64>,
    pub open_files: Option<u64>,
    pub handles: Option<u64>,
    /// User and system CPU time since the process began.
    pub cpu: Option<Duration>,
}

/// Reads this process now.
pub fn sample() -> ProcessSample {
    let mut sample = os_sample();
    sample.cpu = cpu_time();
    sample
}

/// The fields of `/proc/self/status` this module uses, in bytes and counts.
pub fn parse_proc_status(text: &str) -> ProcessSample {
    let mut sample = ProcessSample::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let mut words = value.split_whitespace();
        let Some(n) = words.next().and_then(|n| n.parse::<u64>().ok()) else { continue };
        let bytes = match words.next() {
            Some("kB") => n.saturating_mul(1024),
            _ => n,
        };
        match key {
            "VmRSS" => sample.rss_bytes = Some(bytes),
            "VmHWM" => sample.rss_peak_bytes = Some(bytes),
            "RssAnon" => sample.private_bytes = Some(bytes),
            "Threads" => sample.threads = Some(n),
            _ => {}
        }
    }
    sample
}

/// `busy` over `wall` spread on `workers`: how much of the time given the
/// share was spent working. `None` with no time or no workers to divide by.
pub fn ratio(busy: Duration, wall: Duration, workers: usize) -> Option<f64> {
    let capacity = wall.as_secs_f64() * workers as f64;
    (capacity > 0.0).then(|| busy.as_secs_f64() / capacity)
}

/// What the async runtime reports, from the metrics tokio keeps stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeSample {
    pub alive_tasks: usize,
    pub global_queue_depth: usize,
    pub workers: usize,
    /// Time every worker spent busy since the runtime began, added up.
    pub busy: Duration,
}

pub fn runtime_sample(handle: &tokio::runtime::Handle) -> RuntimeSample {
    let metrics = handle.metrics();
    let workers = metrics.num_workers();
    RuntimeSample {
        alive_tasks: metrics.num_alive_tasks(),
        global_queue_depth: metrics.global_queue_depth(),
        workers,
        busy: (0..workers).map(|i| metrics.worker_total_busy_duration(i)).sum(),
    }
}

#[cfg(target_os = "linux")]
fn os_sample() -> ProcessSample {
    let mut sample = std::fs::read_to_string("/proc/self/status").map(|t| parse_proc_status(&t)).unwrap_or_default();
    sample.open_files = std::fs::read_dir("/proc/self/fd").ok().map(|dir| dir.count() as u64);
    sample
}

#[cfg(windows)]
fn os_sample() -> ProcessSample {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};

    let mut sample = ProcessSample::default();
    // SAFETY: the pseudo handle of this process needs no closing; each call
    // writes only into the local it is given, whose size it is told.
    unsafe {
        let process = GetCurrentProcess();
        let mut counters: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        if GetProcessMemoryInfo(process, &mut counters as *mut _ as *mut PROCESS_MEMORY_COUNTERS, counters.cb) != 0 {
            sample.rss_bytes = Some(counters.WorkingSetSize as u64);
            sample.rss_peak_bytes = Some(counters.PeakWorkingSetSize as u64);
            sample.private_bytes = Some(counters.PrivateUsage as u64);
        }
        let mut handles = 0u32;
        if GetProcessHandleCount(process, &mut handles) != 0 {
            sample.handles = Some(u64::from(handles));
        }
    }
    sample
}

#[cfg(not(any(target_os = "linux", windows)))]
fn os_sample() -> ProcessSample {
    ProcessSample::default()
}

#[cfg(unix)]
fn cpu_time() -> Option<Duration> {
    // SAFETY: `getrusage` only writes the struct it is given.
    let usage = unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        (libc::getrusage(libc::RUSAGE_SELF, &mut usage) == 0).then_some(usage)
    }?;
    let of = |t: libc::timeval| Duration::from_secs(t.tv_sec as u64) + Duration::from_micros(t.tv_usec as u64);
    Some(of(usage.ru_utime) + of(usage.ru_stime))
}

#[cfg(windows)]
fn cpu_time() -> Option<Duration> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: the pseudo handle needs no closing; the call writes only the
    // four locals.
    let ok = unsafe { GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user) } != 0;
    // A FILETIME counts 100 ns intervals.
    let of = |t: FILETIME| Duration::from_nanos(((u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)) * 100);
    ok.then(|| of(kernel) + of(user))
}

#[cfg(not(any(unix, windows)))]
fn cpu_time() -> Option<Duration> {
    None
}

/// Local time minus UTC now, in seconds; `0` when the system does not say.
#[cfg(unix)]
pub fn utc_offset_s() -> i64 {
    // SAFETY: `time` with a null pointer only returns; `localtime_r` writes
    // only the struct it is given.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            0
        } else {
            tm.tm_gmtoff as i64
        }
    }
}

#[cfg(windows)]
pub fn utc_offset_s() -> i64 {
    use windows_sys::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};

    // SAFETY: the call writes only the struct it is given.
    let (zone, info) = unsafe {
        let mut info: TIME_ZONE_INFORMATION = std::mem::zeroed();
        (GetTimeZoneInformation(&mut info), info)
    };
    // 1 is standard time and 2 daylight time; anything else is no zone or
    // a failure, where only the base bias is known.
    let bias = match zone {
        1 => info.Bias + info.StandardBias,
        2 => info.Bias + info.DaylightBias,
        _ => info.Bias,
    };
    -i64::from(bias) * 60
}

#[cfg(not(any(unix, windows)))]
pub fn utc_offset_s() -> i64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_linux_status_file_is_read_in_bytes() {
        let text = "Name:\tclawg\nVmHWM:\t  204800 kB\nVmRSS:\t  102400 kB\nRssAnon:\t   51200 kB\nThreads:\t17\nVmSwap:\t0 kB\nbroken line\n";
        let sample = parse_proc_status(text);
        assert_eq!(sample.rss_bytes, Some(102_400 * 1024));
        assert_eq!(sample.rss_peak_bytes, Some(204_800 * 1024));
        assert_eq!(sample.private_bytes, Some(51_200 * 1024));
        assert_eq!(sample.threads, Some(17));
        assert_eq!(sample.handles, None, "what the file does not say stays unknown");
    }

    #[test]
    fn a_ratio_is_the_delta_over_the_wall_time() {
        assert_eq!(ratio(Duration::from_secs(30), Duration::from_secs(60), 1), Some(0.5));
        assert_eq!(ratio(Duration::from_secs(60), Duration::from_secs(60), 4), Some(0.25));
        assert_eq!(ratio(Duration::from_secs(1), Duration::ZERO, 4), None);
        assert_eq!(ratio(Duration::from_secs(1), Duration::from_secs(1), 0), None);
    }

    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn the_sample_of_this_process_has_its_memory() {
        let sample = sample();
        assert!(sample.rss_bytes.is_some_and(|b| b > 0), "{sample:?}");
        assert!(sample.rss_peak_bytes >= sample.rss_bytes, "{sample:?}");
        assert!(sample.private_bytes.is_some_and(|b| b > 0), "{sample:?}");
        assert!(sample.cpu.is_some(), "{sample:?}");
        if cfg!(windows) {
            assert!(sample.handles.is_some_and(|h| h > 0), "{sample:?}");
        } else {
            assert!(sample.threads.is_some_and(|t| t > 0), "{sample:?}");
            assert!(sample.open_files.is_some_and(|f| f > 0), "{sample:?}");
        }
    }

    #[test]
    fn the_utc_offset_is_within_a_day() {
        assert!(utc_offset_s().abs() <= 14 * 3600);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_runtime_sample_counts_its_workers() {
        let parked = tokio::spawn(std::future::pending::<()>());
        let sample = runtime_sample(&tokio::runtime::Handle::current());
        parked.abort();
        assert_eq!(sample.workers, 2);
        assert!(sample.alive_tasks >= 1, "a spawned task is alive: {sample:?}");
    }
}
