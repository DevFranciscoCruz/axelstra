//! Machines with two graphics adapters (integrated + discrete) sometimes run WebView2 on the weak one. At start we find
//! the adapter with the most video memory and, when it is clearly better than the others, ask Windows to run Axelstra
//! and its WebView2 on the high-performance GPU, the same switch as Settings > Display > Graphics. An existing choice
//! of the user for an executable is never overwritten. It takes effect the next time those processes start.

#[cfg(windows)]
mod imp {
    use std::path::PathBuf;
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// (name, video memory in bytes) of every display adapter the driver registered
    pub fn adapters() -> Vec<(String, u64)> {
        let mut out = vec![];
        unsafe {
            let mut class: HKEY = std::ptr::null_mut();
            let path = wide(r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}");
            if RegOpenKeyExW(HKEY_LOCAL_MACHINE, path.as_ptr(), 0, KEY_READ, &mut class) != ERROR_SUCCESS {
                return out;
            }
            for i in 0..32u32 {
                let mut name = [0u16; 16];
                let mut len = name.len() as u32;
                if RegEnumKeyExW(class, i, name.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) != ERROR_SUCCESS {
                    break;
                }
                let mut sub: HKEY = std::ptr::null_mut();
                if RegOpenKeyExW(class, name.as_ptr(), 0, KEY_READ, &mut sub) != ERROR_SUCCESS {
                    continue;
                }
                let mut mem: u64 = 0;
                let mut size = 8u32;
                let q = wide("HardwareInformation.qwMemorySize");
                let ok = RegQueryValueExW(sub, q.as_ptr(), std::ptr::null(), std::ptr::null_mut(), &mut mem as *mut u64 as *mut u8, &mut size);
                let mut buf = [0u16; 256];
                let mut bsize = (buf.len() * 2) as u32;
                let d = wide("DriverDesc");
                let dd = RegQueryValueExW(sub, d.as_ptr(), std::ptr::null(), std::ptr::null_mut(), buf.as_mut_ptr() as *mut u8, &mut bsize);
                if ok == ERROR_SUCCESS && dd == ERROR_SUCCESS {
                    let n = String::from_utf16_lossy(&buf[..(bsize as usize / 2).saturating_sub(1)]);
                    out.push((n, mem));
                }
                RegCloseKey(sub);
            }
            RegCloseKey(class);
        }
        out
    }

    fn webview_exes() -> Vec<PathBuf> {
        let mut v = vec![];
        for base in ["ProgramFiles(x86)", "ProgramFiles"] {
            let Some(root) = std::env::var_os(base) else { continue };
            let app = PathBuf::from(root).join("Microsoft").join("EdgeWebView").join("Application");
            let Ok(rd) = std::fs::read_dir(app) else { continue };
            for e in rd.flatten() {
                let exe = e.path().join("msedgewebview2.exe");
                if exe.is_file() {
                    v.push(exe);
                }
            }
        }
        v
    }

    fn already_set(key: HKEY, name: &[u16]) -> bool {
        unsafe { RegQueryValueExW(key, name.as_ptr(), std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) == ERROR_SUCCESS }
    }

    pub fn prefer_best() {
        let mut gpus = adapters();
        gpus.retain(|(n, _)| !n.contains("Basic Render"));
        gpus.sort_by_key(|(_, m)| std::cmp::Reverse(*m));
        // one adapter, or no clear winner: nothing to choose
        if gpus.len() < 2 || gpus[0].1 < gpus[1].1.saturating_mul(2) {
            return;
        }
        let mut targets = webview_exes();
        if let Ok(me) = std::env::current_exe() {
            targets.push(me);
        }
        unsafe {
            let mut key: HKEY = std::ptr::null_mut();
            let path = wide(r"Software\Microsoft\DirectX\UserGpuPreferences");
            if RegCreateKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, std::ptr::null(), 0, KEY_READ | KEY_WRITE, std::ptr::null(), &mut key, std::ptr::null_mut()) != ERROR_SUCCESS {
                return;
            }
            let data = wide("GpuPreference=2;");
            for t in targets {
                let name = wide(&t.to_string_lossy());
                if !already_set(key, &name) {
                    RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, data.as_ptr() as *const u8, (data.len() * 2) as u32);
                }
            }
            RegCloseKey(key);
        }
    }
}

/// Picks the best graphics adapter for Axelstra's own processes (Windows only).
pub fn prefer_best_gpu() {
    #[cfg(windows)]
    std::thread::spawn(imp::prefer_best);
}
