//! Status bar data: keep-awake, and how much memory Axelstra uses (this app and its webview, the
//! daemon, every terminal and agent below the daemon) plus the dev-server ports they listen on.

use axelstra_core::procs;
use axelstra_daemon::ports;
use serde::Serialize;
use std::sync::Mutex;

#[derive(Serialize, Default)]
pub struct Usage {
    /// bytes: this process and its children (the webview)
    app: u64,
    /// bytes: the daemon process itself
    daemon: u64,
    /// bytes: everything the daemon started (terminals, agents, what they run)
    sessions: u64,
    ports: Vec<ports::Port>,
}

#[tauri::command]
pub async fn mem_usage() -> Usage {
    tauri::async_runtime::spawn_blocking(measure).await.unwrap_or_default()
}

fn measure() -> Usage {
    let me = std::process::id();
    let daemon = daemon_pid();
    let app_set = procs::with_descendants(&[me]);
    let app = sum(app_set.iter().copied());
    if daemon == 0 {
        return Usage { app, ..Default::default() };
    }
    let below: Vec<u32> = procs::with_descendants(&[daemon]).into_iter().filter(|p| *p != daemon && !app_set.contains(p)).collect();
    Usage {
        app,
        daemon: sum([daemon]),
        sessions: sum(below),
        ports: ports::listening(&[daemon]).unwrap_or_default(),
    }
}

fn sum(pids: impl IntoIterator<Item = u32>) -> u64 {
    let pids: Vec<u32> = pids.into_iter().collect();
    imp::memory(&pids)
}

fn daemon_pid() -> u32 {
    imp::daemon_pid()
}

#[cfg(windows)]
mod imp {
    use std::os::windows::io::AsRawHandle;
    use std::sync::atomic::{AtomicU32, Ordering};
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    /// Working set of one process, `None` when it is gone or not readable.
    fn working_set(pid: u32) -> Option<u64> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            let ok = GetProcessMemoryInfo(h, &mut c, c.cb);
            CloseHandle(h);
            (ok != 0).then_some(c.WorkingSetSize as u64)
        }
    }

    pub fn memory(pids: &[u32]) -> u64 {
        pids.iter().filter_map(|p| working_set(*p)).sum()
    }

    static DAEMON: AtomicU32 = AtomicU32::new(0);

    /// The daemon's pid, read off its named pipe (cached while that process lives).
    pub fn daemon_pid() -> u32 {
        let cached = DAEMON.load(Ordering::Relaxed);
        if cached != 0 && working_set(cached).is_some() {
            return cached;
        }
        let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).open(axelstra_daemon::proto::pipe_name()) else { return 0 };
        let mut pid = 0u32;
        let ok = unsafe { GetNamedPipeServerProcessId(f.as_raw_handle() as _, &mut pid) };
        drop(f);
        let pid = if ok != 0 { pid } else { 0 };
        DAEMON.store(pid, Ordering::Relaxed);
        pid
    }
}

#[cfg(unix)]
mod imp {
    use std::process::Command;

    /// (pid, resident KiB, command line) of every process.
    fn table() -> Vec<(u32, u64, String)> {
        let Ok(o) = Command::new("ps").args(["-A", "-o", "pid=,rss=,command="]).output() else { return Vec::new() };
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|l| {
                let l = l.trim_start();
                let (pid, rest) = l.split_once(char::is_whitespace)?;
                let rest = rest.trim_start();
                let (rss, cmd) = rest.split_once(char::is_whitespace)?;
                Some((pid.parse().ok()?, rss.parse().ok()?, cmd.trim().to_string()))
            })
            .collect()
    }

    pub fn memory(pids: &[u32]) -> u64 {
        let t = table();
        pids.iter().filter_map(|p| t.iter().find(|r| r.0 == *p)).map(|r| r.1 * 1024).sum()
    }

    /// The process started as `axelstra daemon` (by this user; ps lists the command line).
    pub fn daemon_pid() -> u32 {
        table()
            .into_iter()
            .find(|(_, _, c)| c.ends_with(" daemon") && c.contains("axelstra"))
            .map(|r| r.0)
            .unwrap_or(0)
    }
}

/// Prevent the computer from sleeping while the app is open. One holder at a time.
static AWAKE: Mutex<Option<std::sync::mpsc::Sender<()>>> = Mutex::new(None);

#[cfg(target_os = "macos")]
static CAFFEINATE: Mutex<Option<std::process::Child>> = Mutex::new(None);

#[tauri::command]
pub fn keep_awake(on: bool) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
        let mut g = AWAKE.lock().unwrap();
        // dropping the sender releases the thread, which clears the request on its way out
        *g = None;
        if on {
            let (tx, rx) = std::sync::mpsc::channel::<()>();
            // the execution state belongs to the thread that set it, so a thread of its own holds it
            std::thread::spawn(move || {
                unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
                let _ = rx.recv();
                unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
            });
            *g = Some(tx);
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = &AWAKE;
        let mut g = CAFFEINATE.lock().unwrap();
        if let Some(mut c) = g.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if on {
            // -w: caffeinate also ends by itself when this app does
            *g = std::process::Command::new("caffeinate").args(["-i", "-w", &std::process::id().to_string()]).spawn().ok();
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (on, &AWAKE);
    }
}
