//! What Ctrl+click on a path in a terminal may do with it: show it in the file manager, open it with the system's
//! default program, or run it (programs and scripts), each only after the user confirmed in a dialog.
//!
//! The webview never decides: the daemon resolves the path again on every call (`files.resolve_path` to describe it,
//! `files.path_action` to act), so a stale dialog or a forged call cannot do more than the rules below allow.
//! The path must exist; it is canonical (symlinks followed, so what is shown is the real target). `exec` marks programs
//! and scripts. `can_open` (default program) needs a regular, non-exec file; `can_run` needs a regular exec file. Both
//! also need the real path *inside* one of the session's roots (cwd, worktree, project): a path elsewhere, or a link
//! that points outside, can only be revealed. Nothing goes through a shell and no argument is concatenated: the path is
//! one argument (`ShellExecuteW` on Windows, `open`/`xdg-open` or the file itself on Unix). Agents cannot call these
//! methods (`agent_forbidden`); every run is written to the audit log.
//!
//! In tests and E2E `AXELSTRA_TEST_OPENER_LOG=<file>` makes `perform` append `<action>\t<path>` to that file instead of
//! touching the system: nothing real is revealed, opened or run.

use crate::htmlpath::{clean, strip_verbatim};
use anyhow::Result;
use axelstra_core::coded;
use std::path::{Path, PathBuf};

pub struct PathInfo {
    /// Canonical absolute path, without the Windows `\\?\` prefix.
    pub path: String,
    pub is_dir: bool,
    pub exec: bool,
    pub inside: bool,
    pub can_open: bool,
    pub can_run: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Reveal,
    Open,
    Run,
}

impl Action {
    pub fn parse(s: &str) -> Option<Action> {
        match s {
            "reveal" => Some(Action::Reveal),
            "open" => Some(Action::Open),
            "run" => Some(Action::Run),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Action::Reveal => "reveal",
            Action::Open => "open",
            Action::Run => "run",
        }
    }
}

/// Extensions that run code (or load as code) when opened.
const EXEC_EXT: &[&str] = &[
    "exe",
    "com",
    "msi",
    "msp",
    "msix",
    "appx",
    "scr",
    "bat",
    "cmd",
    "ps1",
    "psm1",
    "psd1",
    "ps1xml",
    "vbs",
    "vbe",
    "wsf",
    "wsh",
    "hta",
    "jar",
    "dll",
    "sys",
    "ocx",
    "cpl",
    "msc",
    "lnk",
    "url",
    "pif",
    "reg",
    "gadget",
    "application",
    "appref-ms",
    "sh",
    "bash",
    "zsh",
    "fish",
    "command",
    "app",
    "run",
    "bin",
    "appimage",
    "deb",
    "rpm",
    "apk",
    "dmg",
    "pkg",
    "py",
    "pyw",
    "pl",
    "rb",
    "php",
    "lua",
    "js",
    "mjs",
    "cjs",
    "ts",
    "jse",
    "vb",
    "chm",
    "inf",
    "ws",
];

pub fn is_exec_ext(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| EXEC_EXT.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

#[cfg(unix)]
fn has_exec_bit(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.is_file() && meta.permissions().mode() & 0o111 != 0
}
#[cfg(not(unix))]
fn has_exec_bit(_: &std::fs::Metadata) -> bool {
    false
}

/// `cwd` is the session's folder (relative paths start there); `roots` are the folders that count as "inside".
pub fn resolve(raw: &str, cwd: Option<&Path>, roots: &[PathBuf]) -> Result<PathInfo> {
    let cleaned = clean(raw);
    let not_found = || coded!("files.path_not_found", "path not found: {path}", path = raw.trim());
    // a NUL cannot be part of a path; refuse before it reaches the file system
    if cleaned.is_empty() || cleaned.contains('\0') {
        return Err(not_found());
    }
    let given = Path::new(cleaned);
    let full = if given.is_absolute() || given.has_root() { given.to_path_buf() } else { cwd.ok_or_else(not_found)?.join(given) };
    let real = std::fs::canonicalize(&full).map_err(|_| not_found())?;
    let meta = std::fs::metadata(&real).map_err(|_| not_found())?;
    let is_dir = meta.is_dir();
    let is_file = meta.is_file();
    // the name as clicked counts too: a link named `x.exe` to a `.dat` is still treated as a program
    let exec = is_file && (is_exec_ext(&real) || is_exec_ext(given) || has_exec_bit(&meta));
    let inside = roots.iter().filter_map(|r| std::fs::canonicalize(r).ok()).any(|r| real.starts_with(r));
    Ok(PathInfo {
        path: strip_verbatim(&real.to_string_lossy()),
        is_dir,
        exec,
        inside,
        can_open: is_file && !exec && inside,
        can_run: is_file && exec && inside,
    })
}

/// Whether `action` is allowed on what `resolve` described.
pub fn allowed(info: &PathInfo, action: Action) -> bool {
    match action {
        Action::Reveal => true,
        Action::Open => info.can_open,
        Action::Run => info.can_run,
    }
}

/// Does it. `info.path` is the canonical path from `resolve`, never the text from the terminal.
pub fn perform(info: &PathInfo, action: Action) -> Result<()> {
    let fail = |e: String| {
        coded!("files.path_action_failed", "could not {action} {path}: {detail}", action = action.name(), path = &info.path, detail = e)
    };
    if let Some(log) = std::env::var_os("AXELSTRA_TEST_OPENER_LOG").filter(|l| !l.is_empty()) {
        use std::io::Write;
        return std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .and_then(|mut f| writeln!(f, "{}\t{}", action.name(), info.path))
            .map_err(|e| fail(e.to_string()));
    }
    sys::perform(info, action).map_err(fail)
}

#[cfg(windows)]
mod sys {
    use super::{Action, PathInfo};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::process::CommandExt;

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    pub fn perform(info: &PathInfo, action: Action) -> Result<(), String> {
        match action {
            Action::Reveal => {
                // `/select,"path"` must be one raw argument: Rust's own quoting would break explorer's parser. A file is
                // selected in its folder, a folder is opened. Explorer exits non-zero even when it worked.
                let mut c = std::process::Command::new("explorer.exe");
                if info.is_dir {
                    c.arg(&info.path);
                } else {
                    c.raw_arg(format!("/select,\"{}\"", info.path));
                }
                crate::pathopen::quiet(&mut c).spawn().map(|_| ()).map_err(|e| e.to_string())
            }
            Action::Open | Action::Run => {
                use windows_sys::Win32::UI::Shell::ShellExecuteW;
                let dir = std::path::Path::new(&info.path).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                let (verb, file, dirw) = (wide("open"), wide(&info.path), wide(&dir));
                // SAFETY: all pointers are NUL-terminated buffers that live through the call; no window, no parameters.
                let r = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), dirw.as_ptr(), 1) };
                // values above 32 mean success
                if (r as usize) > 32 {
                    Ok(())
                } else {
                    Err(format!("ShellExecute code {}", r as usize))
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod sys {
    use super::{Action, PathInfo};

    pub fn perform(info: &PathInfo, action: Action) -> Result<(), String> {
        let path = std::path::Path::new(&info.path);
        let folder = if info.is_dir { path } else { path.parent().unwrap_or(path) };
        let mut c = if cfg!(target_os = "macos") {
            let mut c = std::process::Command::new("open");
            match action {
                Action::Reveal if !info.is_dir => c.arg("-R").arg(path),
                Action::Reveal => c.arg(path),
                _ => c.arg(path),
            };
            c
        } else {
            match action {
                Action::Reveal => {
                    let mut c = std::process::Command::new("xdg-open");
                    c.arg(folder);
                    c
                }
                // a file with the exec bit is the program itself; anything else goes to the desktop's default
                Action::Run if has_exec(path) => std::process::Command::new(path),
                _ => {
                    let mut c = std::process::Command::new("xdg-open");
                    c.arg(path);
                    c
                }
            }
        };
        c.current_dir(folder).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        super::quiet(&mut c).spawn().map(|_| ()).map_err(|e| e.to_string())
    }

    fn has_exec(p: &std::path::Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
}

/// Keeps a console window from flashing on Windows; a no-op elsewhere.
pub(crate) fn quiet(c: &mut std::process::Command) -> &mut std::process::Command {
    axelstra_core::git::quiet(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("axp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::canonicalize(&d).unwrap()
    }

    fn code(r: Result<PathInfo>) -> String {
        axelstra_core::msg::find(&r.err().expect("expected an error")).map(|c| c.code.clone()).unwrap_or_default()
    }

    #[test]
    fn extensions_that_run_code_are_exec() {
        for n in ["a.exe", "A.EXE", "a.msi", "x.bat", "x.cmd", "x.ps1", "x.sh", "x.dll", "x.lnk", "x.py", "x.app"] {
            assert!(is_exec_ext(Path::new(n)), "{n}");
        }
        for n in ["a.txt", "a.png", "a.pdf", "a.md", "exe", "a.exe.txt", "Makefile", "a.zip"] {
            assert!(!is_exec_ext(Path::new(n)), "{n}");
        }
    }

    #[test]
    fn plain_file_inside_can_be_opened_not_run() {
        let d = tmp("in");
        std::fs::create_dir_all(d.join("my docs")).unwrap();
        std::fs::write(d.join("my docs/Photo One.png"), "x").unwrap();
        let roots = std::slice::from_ref(&d);
        for raw in ["my docs/Photo One.png", "./my docs/Photo One.png:3:9", "my docs/Photo One.png)."] {
            let r = resolve(raw, Some(&d), roots).unwrap_or_else(|e| panic!("{raw}: {e:#}"));
            assert!(r.path.ends_with("Photo One.png") && !r.path.starts_with(r"\\?\"), "{}", r.path);
            assert!(!r.is_dir && !r.exec && r.inside && r.can_open && !r.can_run, "{raw}");
            assert!(allowed(&r, Action::Open) && allowed(&r, Action::Reveal) && !allowed(&r, Action::Run));
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn programs_run_but_are_not_opened_and_folders_only_reveal() {
        let d = tmp("exe");
        std::fs::write(d.join("setup.exe"), "MZ").unwrap();
        std::fs::write(d.join("run.ps1"), "").unwrap();
        std::fs::create_dir_all(d.join("bundle")).unwrap();
        let roots = std::slice::from_ref(&d);
        for n in ["setup.exe", "run.ps1"] {
            let r = resolve(n, Some(&d), roots).unwrap();
            assert!(r.exec && r.inside && r.can_run && !r.can_open && !r.is_dir, "{n}");
            assert!(allowed(&r, Action::Run) && !allowed(&r, Action::Open));
        }
        let r = resolve("bundle", Some(&d), roots).unwrap();
        assert!(r.is_dir && !r.exec && r.inside && !r.can_open && !r.can_run);
        assert!(allowed(&r, Action::Reveal));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn outside_the_roots_is_reveal_only_even_for_programs() {
        let base = tmp("out");
        let (proj, evil) = (base.join("proj"), base.join("proj-evil"));
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::create_dir_all(&evil).unwrap();
        std::fs::write(evil.join("e.png"), "").unwrap();
        std::fs::write(evil.join("e.exe"), "").unwrap();
        let roots = std::slice::from_ref(&proj);
        let abs = evil.join("e.png");
        let r = resolve(&abs.to_string_lossy(), Some(&proj), roots).unwrap();
        assert!(!r.inside && !r.can_open && !r.exec);
        let r = resolve(&evil.join("e.exe").to_string_lossy(), Some(&proj), roots).unwrap();
        assert!(r.exec && !r.inside && !r.can_run && !allowed(&r, Action::Run) && allowed(&r, Action::Reveal));
        // `..` leaves the root too; a sibling with the same prefix is not inside
        let r = resolve("../proj-evil/e.exe", Some(&proj), roots).unwrap();
        assert!(!r.inside && !r.can_run);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn missing_and_empty_are_not_found() {
        let d = tmp("miss");
        let roots = std::slice::from_ref(&d);
        assert_eq!(code(resolve("nope.png", Some(&d), roots)), "files.path_not_found");
        assert_eq!(code(resolve("", Some(&d), roots)), "files.path_not_found");
        assert_eq!(code(resolve("a\0b", Some(&d), roots)), "files.path_not_found");
        assert_eq!(code(resolve("a.png", None, roots)), "files.path_not_found");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn perform_with_the_test_log_touches_nothing_real() {
        // a single test sets the variable: it is process-wide
        let d = tmp("log");
        let log = d.join("opener.log");
        std::fs::write(d.join("setup.exe"), "MZ").unwrap();
        let info = resolve("setup.exe", Some(&d), std::slice::from_ref(&d)).unwrap();
        std::env::set_var("AXELSTRA_TEST_OPENER_LOG", &log);
        perform(&info, Action::Run).unwrap();
        perform(&info, Action::Reveal).unwrap();
        std::env::remove_var("AXELSTRA_TEST_OPENER_LOG");
        let text = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines, [format!("run\t{}", info.path), format!("reveal\t{}", info.path)]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn exec_bit_and_symlink_target_count() {
        use std::os::unix::fs::PermissionsExt;
        let base = tmp("sym");
        let (proj, out) = (base.join("proj"), base.join("out"));
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(proj.join("tool"), "#!/bin/sh").unwrap();
        std::fs::set_permissions(proj.join("tool"), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(out.join("secret.exe"), "").unwrap();
        std::os::unix::fs::symlink(out.join("secret.exe"), proj.join("link.exe")).unwrap();
        let roots = std::slice::from_ref(&proj);
        assert!(resolve("tool", Some(&proj), roots).unwrap().can_run);
        // a link inside that points outside is judged by its target: not inside, so reveal only
        let r = resolve("link.exe", Some(&proj), roots).unwrap();
        assert!(r.exec && !r.inside && !r.can_run && r.path.ends_with("out/secret.exe"), "{}", r.path);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(windows)]
    #[test]
    fn windows_drive_path_with_spaces() {
        let d = tmp("win");
        std::fs::write(d.join("Axelstra_0.1.0_x64-setup.exe"), "MZ").unwrap();
        let abs = d.join("Axelstra_0.1.0_x64-setup.exe");
        let r = resolve(&strip_verbatim(&abs.to_string_lossy()), None, std::slice::from_ref(&d)).unwrap();
        assert!(r.exec && r.can_run && r.inside && !r.path.starts_with(r"\\?\"), "{}", r.path);
        assert!(resolve(&abs.to_string_lossy().replace('\\', "/"), None, &[]).unwrap().exec);
        let _ = std::fs::remove_dir_all(&d);
    }
}
