//! Memory readings without an OS event: the sampler (the fallback when an event cannot be set up, and macOS), and the
//! parsers of what each OS prints.

use super::{MemInfo, MemSource, Stop, Wake};
use std::time::Duration;

/// Reads memory when asked and sleeps on the stop token between reads; the governor's computed deadline is its only wake.
#[derive(Default)]
pub struct Sampler;

impl Sampler {
    pub fn new() -> Sampler {
        Sampler
    }
}

impl MemSource for Sampler {
    fn snapshot(&self) -> Option<MemInfo> {
        read_now()
    }

    fn wait_change(&self, stop: &Stop, timeout: Duration) -> Wake {
        if stop.sleep(timeout) {
            Wake::Stop
        } else {
            Wake::Timeout
        }
    }

    fn has_events(&self) -> bool {
        false
    }

    fn name(&self) -> &'static str {
        "sampler"
    }
}

/// Memory now, by whatever this OS offers; `None` when it cannot be read.
pub fn read_now() -> Option<MemInfo> {
    #[cfg(windows)]
    return super::win::global_status();
    #[cfg(target_os = "linux")]
    return parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?);
    #[cfg(target_os = "macos")]
    return macos_now();
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    return None;
}

/// `/proc/meminfo`: MemTotal and MemAvailable, in kB.
#[allow(dead_code)] // used on Linux and in tests
pub fn parse_meminfo(text: &str) -> Option<MemInfo> {
    let kb = |key: &str| -> Option<u64> {
        let line = text.lines().find(|l| l.starts_with(key))?;
        line[key.len()..].trim_start_matches(':').split_whitespace().next()?.parse().ok()
    };
    let total = kb("MemTotal")?;
    let free = kb("MemAvailable").or_else(|| Some(kb("MemFree")? + kb("Cached").unwrap_or(0)))?;
    Some(MemInfo { total_mb: total / 1024, free_mb: free / 1024, commit_pct: None })
}

/// `vm_stat` output plus `hw.memsize`: free = free + inactive + speculative + purgeable pages (what the system hands
/// out without paging).
#[allow(dead_code)] // used on macOS and in tests
pub fn parse_vm_stat(text: &str, total_bytes: u64) -> Option<MemInfo> {
    let page: u64 = text.lines().next()?.split("page size of").nth(1)?.split_whitespace().next()?.parse().ok()?;
    let pages = |key: &str| -> u64 {
        text.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split(':').nth(1))
            .and_then(|v| v.trim().trim_end_matches('.').parse().ok())
            .unwrap_or(0)
    };
    let free = pages("Pages free") + pages("Pages inactive") + pages("Pages speculative") + pages("Pages purgeable");
    Some(MemInfo { total_mb: total_bytes / (1024 * 1024), free_mb: free * page / (1024 * 1024), commit_pct: None })
}

#[cfg(target_os = "macos")]
fn macos_now() -> Option<MemInfo> {
    let run = |prog: &str, args: &[&str]| {
        let o = std::process::Command::new(prog).args(args).output().ok()?;
        o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let total: u64 = run("sysctl", &["-n", "hw.memsize"])?.trim().parse().ok()?;
    parse_vm_stat(&run("vm_stat", &[])?, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meminfo_uses_memavailable() {
        let t = "MemTotal:       16384000 kB\nMemFree:          200000 kB\nMemAvailable:    8192000 kB\nCached: 5 kB\n";
        let i = parse_meminfo(t).unwrap();
        assert_eq!((i.total_mb, i.free_mb), (16000, 8000));
        // an old kernel without MemAvailable: free + cached
        let old = "MemTotal: 2048000 kB\nMemFree: 1024000 kB\nCached: 1024000 kB\n";
        assert_eq!(parse_meminfo(old).unwrap().free_mb, 2000);
        assert!(parse_meminfo("garbage").is_none());
    }

    #[test]
    fn vm_stat_counts_reclaimable_pages() {
        let t = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\nPages free:                               1000.\nPages active:                            50000.\nPages inactive:                           2000.\nPages speculative:                         500.\nPages purgeable:                           500.\n";
        let i = parse_vm_stat(t, 16 * 1024 * 1024 * 1024).unwrap();
        assert_eq!(i.total_mb, 16384);
        assert_eq!(i.free_mb, 4000 * 16384 / (1024 * 1024));
        assert!(parse_vm_stat("nothing", 1).is_none());
    }

    #[test]
    fn this_machine_reads() {
        if cfg!(any(windows, target_os = "linux", target_os = "macos")) {
            let i = read_now().expect("a reading");
            assert!(i.total_mb > 0 && i.free_mb <= i.total_mb, "{i:?}");
        }
    }

    #[test]
    fn sampler_sleeps_until_its_deadline_or_stop() {
        let (s, stop) = (Sampler::new(), Stop::default());
        assert_eq!(s.wait_change(&stop, Duration::from_millis(10)), Wake::Timeout);
        stop.trigger();
        assert_eq!(s.wait_change(&stop, Duration::from_secs(30)), Wake::Stop);
    }
}
