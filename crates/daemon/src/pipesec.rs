//! Access control of the daemon's named pipe (Windows): without an explicit DACL the pipe takes the default one,
//! which lets other users of the machine read it. This one grants full access to the current user and SYSTEM only.

use anyhow::{bail, Result};
use std::ffi::c_void;
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW};
use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER};
use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};

/// Security descriptor kept alive for as long as pipe instances are created from it.
pub struct PipeSecurity {
    sd: *mut c_void,
}

// the descriptor is immutable after creation
unsafe impl Send for PipeSecurity {}
unsafe impl Sync for PipeSecurity {}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        unsafe { LocalFree(self.sd) };
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The user SID of an access token, in string form ("S-1-5-21-...").
fn token_sid(token: HANDLE) -> Result<String> {
    unsafe {
        let mut len = 0u32;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
        let mut buf = vec![0u8; len as usize];
        if GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len) == 0 {
            bail!("GetTokenInformation failed: {}", std::io::Error::last_os_error());
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut s) == 0 {
            bail!("ConvertSidToStringSid failed: {}", std::io::Error::last_os_error());
        }
        let mut n = 0;
        while *s.add(n) != 0 {
            n += 1;
        }
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(s, n));
        LocalFree(s.cast());
        Ok(sid)
    }
}

/// The current user's SID in string form ("S-1-5-21-...").
pub fn current_user_sid() -> Result<String> {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            bail!("OpenProcessToken failed: {}", std::io::Error::last_os_error());
        }
        let sid = token_sid(token);
        CloseHandle(token);
        sid
    }
}

/// Fails unless the process serving the pipe behind `pipe` (an open client end) runs as the current user. Called before the
/// token is sent: another local user (or a process squatting the name) must not be handed the secret.
pub fn verify_server_owner(pipe: &std::fs::File) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    let pipe = pipe.as_raw_handle() as HANDLE;
    unsafe {
        let mut pid = 0u32;
        if GetNamedPipeServerProcessId(pipe, &mut pid) == 0 {
            bail!("could not identify the pipe server: {}", std::io::Error::last_os_error());
        }
        let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if proc.is_null() {
            bail!("could not inspect the pipe server (pid {pid}): {}", std::io::Error::last_os_error());
        }
        let mut token: HANDLE = std::ptr::null_mut();
        let opened = OpenProcessToken(proc, TOKEN_QUERY, &mut token);
        let err = std::io::Error::last_os_error();
        CloseHandle(proc);
        if opened == 0 {
            bail!("could not read the pipe server's identity (pid {pid}): {err}");
        }
        let theirs = token_sid(token);
        CloseHandle(token);
        let (theirs, mine) = (theirs?, current_user_sid()?);
        if theirs != mine {
            return Err(axelstra_core::coded!(
                "pipe.foreign_owner",
                "the pipe is served by another user ({sid}); refusing to send the token",
                sid = theirs
            ));
        }
        Ok(())
    }
}

/// The executable of the process serving the pipe behind `pipe` (an open client end), or `None` when the OS does not say.
pub fn server_exe(pipe: &std::fs::File) -> Option<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Threading::QueryFullProcessImageNameW;
    unsafe {
        let mut pid = 0u32;
        if GetNamedPipeServerProcessId(pipe.as_raw_handle() as HANDLE, &mut pid) == 0 {
            return None;
        }
        let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if proc.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 32768];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(proc, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(proc);
        (ok != 0).then(|| std::path::PathBuf::from(std::ffi::OsString::from_wide(&buf[..len as usize])))
    }
}

/// Protected DACL (no inherited entries): full access for the user with this SID and for SYSTEM.
pub fn sddl_for(sid: &str) -> String {
    format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)")
}

impl PipeSecurity {
    pub fn current_user() -> Result<PipeSecurity> {
        let sddl = wide(&sddl_for(&current_user_sid()?));
        let mut sd: *mut c_void = std::ptr::null_mut();
        // SDDL_REVISION_1
        let ok = unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut sd, std::ptr::null_mut()) };
        if ok == 0 {
            bail!("could not build the pipe security descriptor: {}", std::io::Error::last_os_error());
        }
        Ok(PipeSecurity { sd })
    }

    /// For `ServerOptions::create_with_security_attributes_raw`; valid while `self` lives.
    pub fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: self.sd, bInheritHandle: 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_descriptor_names_only_the_user_and_system() {
        let sid = current_user_sid().unwrap();
        assert!(sid.starts_with("S-1-5-"), "{sid}");
        let sddl = sddl_for(&sid);
        assert_eq!(sddl, format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)"));
        assert!(!sddl.contains("WD") && !sddl.contains(";;;AU") && !sddl.contains(";;;BU"));
        let sec = PipeSecurity::current_user().unwrap();
        assert!(!sec.attributes().lpSecurityDescriptor.is_null());
    }
}
