//! macOS memory source: `host_statistics64(HOST_VM_INFO64)` and `sysctl hw.memsize`, read in-process (no `vm_stat` child).
//!
//! macOS has no free-memory notification we can wait on without libdispatch, so `has_events` is false: like the sampler,
//! the governor's computed deadline is the only wake (15 s cap in Normal, 1..10 s above it). The reading itself is cheap
//! (one Mach call), which is why this source exists instead of the `vm_stat` + `sysctl` children the sampler spawns.
//!
//! The counters, their math and the `MemInfo` conversion are pure (`VmCounts`, `mem_info`) and compile and test on every
//! OS; only the two FFI calls are `cfg(target_os = "macos")`.

use super::{MemInfo, MemSource, Stop, Wake};
use std::time::Duration;

const MIB: u64 = 1024 * 1024;

/// The fields of `vm_statistics64` the governor uses, in pages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VmCounts {
    pub free: u64,
    pub inactive: u64,
    pub speculative: u64,
    pub purgeable: u64,
    /// file-backed pages (the cache)
    pub external: u64,
    /// pages the compressor holds (physical pages used by compressed memory)
    pub compressor: u64,
}

/// Memory the system hands out without paging: free + inactive + speculative + purgeable (same rule as `parse_vm_stat`).
pub fn mem_info(c: &VmCounts, page_bytes: u64, total_bytes: u64) -> MemInfo {
    let free = (c.free + c.inactive + c.speculative + c.purgeable).saturating_mul(page_bytes) / MIB;
    let total_mb = total_bytes / MIB;
    MemInfo { total_mb, free_mb: free.min(total_mb), commit_pct: None }
}

/// File cache in MB.
#[allow(dead_code)] // used on macOS and in tests
pub fn cache_mb(c: &VmCounts, page_bytes: u64) -> u64 {
    c.external.saturating_mul(page_bytes) / MIB
}

/// Physical memory the compressor occupies, in MB.
#[allow(dead_code)] // used on macOS and in tests
pub fn compressed_mb(c: &VmCounts, page_bytes: u64) -> u64 {
    c.compressor.saturating_mul(page_bytes) / MIB
}

/// `vm_statistics64` as the kernel lays it out (`#pragma pack(4)`, 38 natural_t words).
#[allow(dead_code)] // read on macOS, size-checked everywhere
#[repr(C, packed(4))]
#[derive(Clone, Copy, Default)]
struct VmStatistics64 {
    free_count: u32,
    active_count: u32,
    inactive_count: u32,
    wire_count: u32,
    zero_fill_count: u64,
    reactivations: u64,
    pageins: u64,
    pageouts: u64,
    faults: u64,
    cow_faults: u64,
    lookups: u64,
    hits: u64,
    purges: u64,
    purgeable_count: u32,
    speculative_count: u32,
    decompressions: u64,
    compressions: u64,
    swapins: u64,
    swapouts: u64,
    compressor_page_count: u32,
    throttled_count: u32,
    external_page_count: u32,
    internal_page_count: u32,
    total_uncompressed_pages_in_compressor: u64,
}

impl VmStatistics64 {
    #[allow(dead_code)] // used on macOS and in tests
    fn counts(&self) -> VmCounts {
        VmCounts {
            free: self.free_count as u64,
            inactive: self.inactive_count as u64,
            speculative: self.speculative_count as u64,
            purgeable: self.purgeable_count as u64,
            external: self.external_page_count as u64,
            compressor: self.compressor_page_count as u64,
        }
    }
}

#[cfg(target_os = "macos")]
mod ffi {
    use super::{VmCounts, VmStatistics64};
    use std::os::raw::{c_char, c_int, c_uint, c_void};

    const HOST_VM_INFO64: c_int = 4;

    extern "C" {
        fn mach_host_self() -> c_uint;
        fn host_statistics64(host: c_uint, flavor: c_int, info: *mut c_int, count: *mut c_uint) -> c_int;
        fn sysctlbyname(name: *const c_char, old: *mut c_void, oldlen: *mut usize, new: *mut c_void, newlen: usize) -> c_int;
    }

    /// (counts, page size in bytes); `None` when the kernel refuses.
    pub fn vm_counts() -> Option<(VmCounts, u64)> {
        let mut st = VmStatistics64::default();
        let mut count = (std::mem::size_of::<VmStatistics64>() / 4) as c_uint;
        // SAFETY: `st` is a plain-old-data struct with the kernel's `vm_statistics64` layout, `count` is its size in 32-bit
        // words, and both outlive the call; the host port is the process' own send right.
        let kr = unsafe { host_statistics64(mach_host_self(), HOST_VM_INFO64, &mut st as *mut _ as *mut c_int, &mut count) };
        if kr != 0 {
            return None;
        }
        // SAFETY: sysconf takes no pointers.
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        (page > 0).then(|| (st.counts(), page as u64))
    }

    /// `hw.memsize` in bytes.
    pub fn total_bytes() -> Option<u64> {
        let mut v: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        // SAFETY: the name is NUL-terminated, `v` and `len` are valid for the call and `len` is the size of `v`.
        let rc = unsafe { sysctlbyname(c"hw.memsize".as_ptr(), &mut v as *mut _ as *mut c_void, &mut len, std::ptr::null_mut(), 0) };
        (rc == 0 && v > 0).then_some(v)
    }
}

/// Counters, page size and total now; `None` off macOS or when the kernel refuses.
#[allow(dead_code)] // used on macOS
pub fn read_counts() -> Option<(VmCounts, u64, u64)> {
    #[cfg(target_os = "macos")]
    return ffi::vm_counts().and_then(|(c, page)| Some((c, page, ffi::total_bytes()?)));
    #[cfg(not(target_os = "macos"))]
    return None;
}

/// Memory now through the Mach call, `None` off macOS or on failure.
#[allow(dead_code)] // used on macOS
pub fn read_now() -> Option<MemInfo> {
    let (c, page, total) = read_counts()?;
    Some(mem_info(&c, page, total))
}

#[allow(dead_code)] // built on macOS only
pub struct MacSource;

#[allow(dead_code)]
impl MacSource {
    /// Fails (so the caller samples instead) when the Mach call does not answer on this machine.
    pub fn new() -> Result<MacSource, String> {
        read_now().map(|_| MacSource).ok_or_else(|| "host_statistics64 unavailable".to_string())
    }
}

impl MemSource for MacSource {
    fn snapshot(&self) -> Option<MemInfo> {
        // a failed Mach call keeps the level: the sampler path (vm_stat) is not tried every few seconds
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
        "macos-host"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_matches_the_kernel_layout() {
        assert_eq!(std::mem::size_of::<VmStatistics64>(), 152);
        assert_eq!(std::mem::size_of::<VmStatistics64>() / 4, 38);
    }

    #[test]
    fn counts_convert_to_mem_info() {
        let c = VmCounts { free: 1000, inactive: 2000, speculative: 500, purgeable: 500, external: 8192, compressor: 4096 };
        let i = mem_info(&c, 16384, 16 * 1024 * MIB);
        assert_eq!((i.total_mb, i.free_mb), (16384, 4000 * 16384 / MIB));
        assert_eq!(cache_mb(&c, 16384), 128);
        assert_eq!(compressed_mb(&c, 16384), 64);
    }

    #[test]
    fn free_never_exceeds_total() {
        let c = VmCounts { free: u64::MAX / 2, ..Default::default() };
        let i = mem_info(&c, 4096, 8 * 1024 * MIB);
        assert_eq!(i.free_mb, i.total_mb);
    }

    #[test]
    fn struct_fields_map_to_counts() {
        let st = VmStatistics64 {
            free_count: 1,
            inactive_count: 2,
            speculative_count: 3,
            purgeable_count: 4,
            external_page_count: 5,
            compressor_page_count: 6,
            ..Default::default()
        };
        assert_eq!(st.counts(), VmCounts { free: 1, inactive: 2, speculative: 3, purgeable: 4, external: 5, compressor: 6 });
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_real_call_reads_this_mac() {
        let i = read_now().expect("host_statistics64");
        assert!(i.total_mb > 0 && i.free_mb <= i.total_mb, "{i:?}");
        let s = MacSource::new().unwrap();
        assert!(!s.has_events());
        assert!(s.snapshot().is_some());
    }
}
