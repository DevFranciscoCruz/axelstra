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
    /// % of the whole machine's CPU used by all of the above since the last reading (None: not measurable here)
    cpu: Option<f64>,
    /// % of the busiest GPU engine used by those processes, and per adapter (None: not measurable here)
    gpu: Option<f64>,
    gpu_adapters: Vec<(String, f64)>,
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
    let below: Vec<u32> = if daemon == 0 {
        vec![]
    } else {
        procs::with_descendants(&[daemon]).into_iter().filter(|p| *p != daemon && !app_set.contains(p)).collect()
    };
    let mut all: Vec<u32> = app_set.iter().copied().collect();
    all.extend(below.iter().copied());
    if daemon != 0 {
        all.push(daemon);
    }
    let cpu = imp::cpu_percent(&all);
    let (gpu, gpu_adapters) = imp::gpu(&all);
    if daemon == 0 {
        return Usage { app, cpu, gpu, gpu_adapters, ..Default::default() };
    }
    Usage {
        app,
        daemon: sum([daemon]),
        sessions: sum(below),
        ports: ports::listening(&[daemon]).unwrap_or_default(),
        cpu,
        gpu,
        gpu_adapters,
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
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;
    use std::time::Instant;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    /// Private memory of one process, `None` when it is gone or not readable.
    fn working_set(pid: u32) -> Option<u64> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut c: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
            c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
            let ok = GetProcessMemoryInfo(h, &mut c as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS, c.cb);
            CloseHandle(h);
            // private bytes: pages shared between processes (system DLLs, webview) would otherwise be counted once per process
            (ok != 0).then_some(c.PrivateUsage as u64)
        }
    }

    pub fn memory(pids: &[u32]) -> u64 {
        pids.iter().filter_map(|p| working_set(*p)).sum()
    }

    static DAEMON: AtomicU32 = AtomicU32::new(0);

    fn cpu_time(pid: u32) -> Option<u64> {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::GetProcessTimes;
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
            let (mut c, mut e, mut k, mut u) = (z, z, z, z);
            let ok = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u);
            CloseHandle(h);
            let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
            (ok != 0).then(|| t(k) + t(u))
        }
    }

    static LAST_CPU: Mutex<Option<(Instant, HashMap<u32, u64>)>> = Mutex::new(None);

    /// CPU used by these processes since the previous call, as % of the whole machine.
    pub fn cpu_percent(pids: &[u32]) -> Option<f64> {
        let now: HashMap<u32, u64> = pids.iter().filter_map(|p| cpu_time(*p).map(|t| (*p, t))).collect();
        let at = Instant::now();
        let mut g = LAST_CPU.lock().unwrap();
        let out = g.as_ref().and_then(|(t0, before)| {
            let dt = at.duration_since(*t0).as_secs_f64();
            if dt < 0.5 {
                return None;
            }
            // only processes seen both times, so one that started or ended in between cannot skew it
            let used: u64 = now.iter().filter_map(|(p, t)| before.get(p).map(|b| t.saturating_sub(*b))).sum();
            let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;
            Some((used as f64 / 1e7 / dt / cores * 100.0).min(100.0))
        });
        *g = Some((at, now));
        out
    }

    /// (query, counter) handles of the GPU engine counters, opened on first use
    static GPU_QUERY: Mutex<Option<(isize, isize)>> = Mutex::new(None);

    /// GPU use of these processes: the busiest engine type summed over them (what Task Manager shows), plus the same per adapter.
    pub fn gpu(pids: &[u32]) -> (Option<f64>, Vec<(String, f64)>) {
        use windows_sys::Win32::System::Performance::*;
        let mut g = GPU_QUERY.lock().unwrap();
        unsafe {
            let Some((q, c)) = *g else {
                let mut q: isize = 0;
                if PdhOpenQueryW(std::ptr::null(), 0, &mut q) != 0 {
                    return (None, vec![]);
                }
                let path: Vec<u16> = r"\GPU Engine(*)\Utilization Percentage".encode_utf16().chain([0]).collect();
                let mut c: isize = 0;
                if PdhAddEnglishCounterW(q, path.as_ptr(), 0, &mut c) != 0 {
                    PdhCloseQuery(q);
                    return (None, vec![]);
                }
                PdhCollectQueryData(q);
                *g = Some((q, c));
                return (None, vec![]); // a rate needs two samples
            };
            if PdhCollectQueryData(q) != 0 {
                return (None, vec![]);
            }
            let (mut size, mut count) = (0u32, 0u32);
            PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut count, std::ptr::null_mut());
            if size == 0 {
                return (Some(0.0), vec![]);
            }
            let mut buf = vec![0u64; (size as usize).div_ceil(8)];
            let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            if PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut count, items) != 0 {
                return (None, vec![]);
            }
            let mut by_type: HashMap<String, f64> = HashMap::new();
            let mut by_gpu: HashMap<String, HashMap<String, f64>> = HashMap::new();
            for i in 0..count as usize {
                let it = &*items.add(i);
                let mut n = 0;
                while *it.szName.add(n) != 0 {
                    n += 1;
                }
                let name = String::from_utf16_lossy(std::slice::from_raw_parts(it.szName, n));
                let v = it.FmtValue.Anonymous.doubleValue;
                let Some((pid, luid, ty)) = parse_engine(&name) else { continue };
                if !pids.contains(&pid) {
                    continue;
                }
                *by_type.entry(ty.clone()).or_default() += v;
                *by_gpu.entry(luid).or_default().entry(ty).or_default() += v;
            }
            let top = |m: &HashMap<String, f64>| m.values().cloned().fold(0.0, f64::max).min(100.0);
            let names = adapter_names();
            let mut adapters: Vec<(String, f64)> = by_gpu.iter().map(|(l, m)| (names.get(l).cloned().unwrap_or_else(|| format!("GPU {l}")), top(m))).collect();
            adapters.sort_by(|a, b| b.1.total_cmp(&a.1));
            (Some(top(&by_type)), adapters)
        }
    }

    /// Adapter names by LUID ("0x00000000_0x0000D6B2", the spelling of the GPU counters), asked once from DXGI.
    fn adapter_names() -> &'static HashMap<String, String> {
        use std::sync::OnceLock;
        static NAMES: OnceLock<HashMap<String, String>> = OnceLock::new();
        NAMES.get_or_init(|| unsafe {
            use windows_sys::core::GUID;
            // windows-sys has no DXGI: the one function and the one struct needed, declared here
            #[repr(C)]
            struct Luid {
                low: u32,
                high: i32,
            }
            #[repr(C)]
            #[allow(non_snake_case)]
            struct DXGI_ADAPTER_DESC1 {
                Description: [u16; 128],
                VendorId: u32,
                DeviceId: u32,
                SubSysId: u32,
                Revision: u32,
                DedicatedVideoMemory: usize,
                DedicatedSystemMemory: usize,
                SharedSystemMemory: usize,
                AdapterLuid: Luid,
                Flags: u32,
            }
            #[link(name = "dxgi")]
            extern "system" {
                fn CreateDXGIFactory1(riid: *const GUID, pp: *mut *mut std::ffi::c_void) -> i32;
            }
            let mut out = HashMap::new();
            // IDXGIFactory1 {770aae78-f26f-4dba-a829-253c83d1b387}
            let iid = GUID { data1: 0x770a_ae78, data2: 0xf26f, data3: 0x4dba, data4: [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87] };
            let mut factory: *mut std::ffi::c_void = std::ptr::null_mut();
            if CreateDXGIFactory1(&iid, &mut factory) != 0 || factory.is_null() {
                return out;
            }
            // raw COM: the vtable is a list of function pointers
            type Fn2 = unsafe extern "system" fn(*mut std::ffi::c_void, u32, *mut *mut std::ffi::c_void) -> i32;
            type FnDesc = unsafe extern "system" fn(*mut std::ffi::c_void, *mut DXGI_ADAPTER_DESC1) -> i32;
            type FnRelease = unsafe extern "system" fn(*mut std::ffi::c_void) -> u32;
            let vt = |obj: *mut std::ffi::c_void| *(obj as *const *const usize);
            let fvt = vt(factory);
            let enum_adapters1: Fn2 = std::mem::transmute(*fvt.add(12));
            for i in 0..16 {
                let mut ad: *mut std::ffi::c_void = std::ptr::null_mut();
                if enum_adapters1(factory, i, &mut ad) != 0 || ad.is_null() {
                    break;
                }
                let avt = vt(ad);
                let get_desc1: FnDesc = std::mem::transmute(*avt.add(10));
                let mut d: DXGI_ADAPTER_DESC1 = std::mem::zeroed();
                if get_desc1(ad, &mut d) == 0 {
                    let n = d.Description.iter().position(|c| *c == 0).unwrap_or(d.Description.len());
                    let name = String::from_utf16_lossy(&d.Description[..n]);
                    out.insert(format!("0x{:08X}_0x{:08X}", d.AdapterLuid.high as u32, d.AdapterLuid.low), name);
                }
                let release: FnRelease = std::mem::transmute(*avt.add(2));
                release(ad);
            }
            let release: FnRelease = std::mem::transmute(*fvt.add(2));
            release(factory);
            out
        })
    }

    /// `pid_1234_luid_0x00000000_0x0000D6B2_phys_0_eng_0_engtype_3D` gives (1234, "0x00000000_0x0000D6B2", "3D")
    fn parse_engine(name: &str) -> Option<(u32, String, String)> {
        let rest = name.strip_prefix("pid_")?;
        let (pid, rest) = rest.split_once("_luid_")?;
        let (luid, rest) = rest.split_once("_phys_")?;
        let ty = rest.rsplit_once("_engtype_")?.1;
        Some((pid.parse().ok()?, luid.to_string(), ty.to_string()))
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn engine_names_parse() {
            let r = super::parse_engine("pid_1234_luid_0x00000000_0x0000D6B2_phys_0_eng_3_engtype_3D").unwrap();
            assert_eq!(r, (1234, "0x00000000_0x0000D6B2".to_string(), "3D".to_string()));
            assert!(super::parse_engine("junk").is_none());
        }
    }

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

    pub fn cpu_percent(_pids: &[u32]) -> Option<f64> {
        None
    }

    pub fn gpu(_pids: &[u32]) -> (Option<f64>, Vec<(String, f64)>) {
        (None, vec![])
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
