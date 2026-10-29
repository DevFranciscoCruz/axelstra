//! `axelstra statusline-tee`: Claude Code's status line command for sessions started by Axelstra. It saves the
//! JSON Claude passes in (context window, 5 h and weekly limits...) where the app reads it, then runs the
//! status line the user had configured themselves, so what they see in the terminal does not change.
//! It never leaves the line empty: when the user's command is missing, fails or prints nothing, a minimal line
//! built from the same JSON is shown and the reason goes to `statusline.log` in the data dir.

use anyhow::Result;
use axelstra_daemon::proto;
use serde_json::Value;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from)
}

/// The user's own `statusLine.command`: project local, project shared, then user settings (Claude's order).
fn user_command() -> Option<String> {
    let mut files = vec![];
    if let Ok(cwd) = std::env::current_dir() {
        files.push(cwd.join(".claude").join("settings.local.json"));
        files.push(cwd.join(".claude").join("settings.json"));
    }
    if let Some(h) = home() {
        files.push(h.join(".claude").join("settings.json"));
    }
    files.into_iter().find_map(|f| {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(f).ok()?).ok()?;
        v["statusLine"]["command"].as_str().map(String::from)
    })
}

fn save(input: &str) {
    let (Ok(key), Ok(dir)) = (std::env::var("AXELSTRA_KEY"), std::env::var("AXELSTRA_DIR")) else { return };
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return;
    }
    let dir = PathBuf::from(dir).join("status");
    let _ = std::fs::create_dir_all(&dir);
    // written aside and renamed, so the app never reads half a file
    let tmp = dir.join(format!("{key}.tmp"));
    if std::fs::write(&tmp, input).is_ok() {
        let _ = std::fs::rename(&tmp, dir.join(format!("{key}.json")));
    }
}

/// Why a status line fell back, appended to `statusline.log` (kept small: cut when it passes 64 KiB).
fn log(reason: &str) {
    let dir = proto::data_dir();
    let path = dir.join("statusline.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 64 * 1024) {
        let _ = std::fs::remove_file(&path);
    }
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{} {reason}", std::env::var("AXELSTRA_KEY").unwrap_or_default());
    }
}

/// `<root>\bin\bash.exe` for a Git for Windows `root`.
#[cfg_attr(not(windows), allow(dead_code))]
fn bash_in_root(root: &Path) -> Option<PathBuf> {
    let b = root.join("bin").join("bash.exe");
    b.is_file().then_some(b)
}

/// Git's bash given the path of a `git.exe` (`<root>\cmd`, `<root>\bin`, `<root>\mingw64\bin`...): the nearest
/// ancestor that has `bin\bash.exe`.
#[cfg_attr(not(windows), allow(dead_code))]
fn bash_from_git_exe(git: &Path) -> Option<PathBuf> {
    git.ancestors().skip(1).take(4).find_map(bash_in_root)
}

/// True for the folders that hold the WSL launcher `bash.exe`, never usable as Claude's shell.
#[cfg_attr(not(windows), allow(dead_code))]
fn is_wsl_stub(p: &Path) -> bool {
    let s = p.to_string_lossy().to_ascii_lowercase().replace('/', "\\");
    s.contains("\\windows\\system32") || s.contains("\\windowsapps")
}

/// Git for Windows' `bash.exe`: `GIT_INSTALL_ROOT`, the `git.exe` on PATH, a Git `bin` on PATH, the usual
/// folders, then the registry install path (read lazily). Never a `bash` from the WSL folders.
#[cfg_attr(not(windows), allow(dead_code))]
fn find_git_bash(
    path_var: &str,
    env_root: Option<&str>,
    registry_roots: impl FnOnce() -> Vec<PathBuf>,
    extra_roots: &[PathBuf],
) -> Option<PathBuf> {
    if let Some(b) = env_root.filter(|r| !r.is_empty()).and_then(|r| bash_in_root(Path::new(r))) {
        return Some(b);
    }
    for dir in std::env::split_paths(path_var).filter(|d| !is_wsl_stub(d)) {
        if let Some(b) = ["git.exe", "git.cmd"].iter().map(|n| dir.join(n)).find(|g| g.is_file()).and_then(|g| bash_from_git_exe(&g)) {
            return Some(b);
        }
        let direct = dir.join("bash.exe");
        if direct.is_file() {
            return Some(direct);
        }
    }
    // the usual folders are plain stats; the registry is read only when they found nothing
    if let Some(b) = extra_roots.iter().find_map(|r| bash_in_root(r.as_path())) {
        return Some(b);
    }
    registry_roots().iter().find_map(|r| bash_in_root(r.as_path()))
}

/// `InstallPath` of Git for Windows in `HKLM` and `HKCU`, read with the registry API (no `reg.exe`, no console host).
#[cfg(windows)]
fn registry_roots() -> Vec<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (key, value) = (wide(r"SOFTWARE\GitForWindows"), wide("InstallPath"));
    let mut out = vec![];
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let mut bytes = 0u32;
        // SAFETY: valid NUL-terminated wide strings; a null data pointer asks only for the size
        let rc = unsafe {
            RegGetValueW(hive, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut bytes)
        };
        if rc != 0 || bytes < 2 {
            continue;
        }
        let mut buf = vec![0u16; (bytes as usize).div_ceil(2)];
        // SAFETY: `buf` holds `bytes` bytes
        let rc = unsafe {
            RegGetValueW(hive, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut bytes)
        };
        if rc != 0 {
            continue;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        if len > 0 {
            out.push(PathBuf::from(std::ffi::OsString::from_wide(&buf[..len])));
        }
    }
    out
}

/// Where the last Git Bash found is remembered. It is dropped by event, not by age: when the file is gone
/// (Git moved or uninstalled) the lookup runs again and writes a new answer.
#[cfg(windows)]
fn bash_cache_file() -> PathBuf {
    proto::data_dir().join("statusline-bash.txt")
}

#[cfg(windows)]
fn git_bash() -> Option<PathBuf> {
    let cache = bash_cache_file();
    if let Some(b) = std::fs::read_to_string(&cache).ok().map(|t| PathBuf::from(t.trim())).filter(|b| b.is_file()) {
        return Some(b);
    }
    let mut extra: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|p| PathBuf::from(p).join("Git"))
        .collect();
    extra.extend(std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Programs").join("Git")));
    let path = std::env::var("PATH").unwrap_or_default();
    let found = find_git_bash(&path, std::env::var("GIT_INSTALL_ROOT").ok().as_deref(), registry_roots, &extra)?;
    let _ = std::fs::create_dir_all(proto::data_dir());
    let _ = std::fs::write(cache, found.to_string_lossy().as_bytes());
    Some(found)
}

/// A command line that is just a program on PATH and plain arguments (no quotes, variables, globs, pipes,
/// redirections...) can run without a shell in between. Returns the program file and its arguments.
fn direct_program(cmdline: &str, path_var: &str) -> Option<(PathBuf, Vec<String>)> {
    const SPECIAL: &str = "\"'`$&|;<>()*?~#!{}[]=%\\^";
    let mut words = cmdline.split_whitespace();
    let name = words.next()?;
    if cmdline.chars().any(|c| SPECIAL.contains(c) || c.is_control()) || name.contains(['/', ':']) {
        return None;
    }
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".com", ".cmd", ".bat"] } else { &[""] };
    let has_ext = cfg!(windows) && Path::new(name).extension().is_some();
    for dir in std::env::split_paths(path_var) {
        let found = if has_ext { Some(dir.join(name)) } else { exts.iter().map(|e| dir.join(format!("{name}{e}"))).find(|p| p.is_file()) };
        if let Some(f) = found.filter(|f| f.is_file() && is_runnable(f)) {
            return Some((f, words.map(String::from).collect()));
        }
    }
    None
}

#[cfg(unix)]
fn is_runnable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_runnable(_: &Path) -> bool {
    true
}

/// Minimal status line from Claude's JSON, shown when the user's own command cannot be used.
fn fallback_line(input: &str) -> String {
    let v: Value = serde_json::from_str(input).unwrap_or(Value::Null);
    let model = v["model"]["display_name"].as_str().or_else(|| v["model"]["id"].as_str()).unwrap_or("Claude");
    match v["context_window"]["used_percentage"].as_f64() {
        Some(p) => format!("{model} | ctx {p:.0}%"),
        None => model.to_string(),
    }
}

/// The shell Claude would use: Git Bash on Windows (plain `cmd` when Git is not installed), `sh` elsewhere.
fn shell_command(cmdline: &str) -> Command {
    if let Some((program, args)) = direct_program(cmdline, &std::env::var("PATH").unwrap_or_default()) {
        let mut c = Command::new(program);
        c.args(args);
        return c;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        match git_bash() {
            Some(bash) => {
                let mut c = Command::new(bash);
                c.arg("-c").arg(cmdline);
                c
            }
            None => {
                // cmd cannot run a POSIX command line, but it can run a plain program name
                let mut c = Command::new("cmd");
                c.args(["/d", "/s", "/c"]).raw_arg(format!("\"{cmdline}\""));
                c
            }
        }
    }
    #[cfg(not(windows))]
    {
        let mut c = Command::new("sh");
        c.arg("-c").arg(cmdline);
        c
    }
}

/// Runs the user's command and returns its stdout, or why it gave none.
fn run_user(cmdline: &str, input: &str) -> std::result::Result<String, String> {
    let mut shell = shell_command(cmdline);
    let mut child = axelstra_core::proc::hidden(&mut shell)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start `{cmdline}`: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| format!("`{cmdline}` failed: {e}"))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return Err(format!("`{cmdline}` exited {}: {}", out.status, stderr.trim()));
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if text.trim().is_empty() {
        return Err(format!("`{cmdline}` printed nothing: {}", stderr.trim()));
    }
    Ok(text)
}

/// What the status line shows: the user's command output, or the minimal line (with the reason logged).
fn render(cmdline: Option<&str>, input: &str) -> String {
    match cmdline.map(|c| run_user(c, input)) {
        Some(Ok(t)) => t,
        Some(Err(why)) => {
            log(&why);
            fallback_line(input)
        }
        None => fallback_line(input),
    }
}

/// Ticks closer than this with the same command and input show the previous output.
const REUSE: std::time::Duration = std::time::Duration::from_secs(1);

fn reuse_file() -> PathBuf {
    let key = std::env::var("AXELSTRA_KEY").ok().filter(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    proto::data_dir().join("statusline-last").join(format!("{}.txt", key.as_deref().unwrap_or("default")))
}

fn digest(cmdline: Option<&str>, input: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (cmdline, input).hash(&mut h);
    h.finish()
}

/// The previous successful output when it is fresh and was made from the same command and input.
fn reused(file: &Path, digest: u64) -> Option<String> {
    let age = std::fs::metadata(file).ok()?.modified().ok()?.elapsed().ok()?;
    if age >= REUSE {
        return None;
    }
    let text = std::fs::read_to_string(file).ok()?;
    let (head, body) = text.split_once('\n')?;
    (head == digest.to_string()).then(|| body.to_string())
}

fn remember(file: &Path, digest: u64, text: &str) {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, format!("{digest}\n{text}"));
}

pub fn tee() -> Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    save(&input);
    let cmd = user_command();
    let (file, d) = (reuse_file(), digest(cmd.as_deref(), &input));
    let text = match reused(&file, d) {
        Some(t) => t,
        None => {
            let t = render(cmd.as_deref(), &input);
            if cmd.is_some() {
                remember(&file, d, &t);
            }
            t
        }
    };
    let mut out = std::io::stdout();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("axl-sl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fake_git(root: &Path) {
        for f in ["bin/bash.exe", "cmd/git.exe"] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
    }

    #[test]
    fn bash_is_found_from_git_on_path_in_any_folder() {
        let root = tmp("path").join("Programas").join("Git");
        fake_git(&root);
        let path = std::env::join_paths([PathBuf::from("/nowhere/at/all"), root.join("cmd")]).unwrap();
        let b = find_git_bash(path.to_str().unwrap(), None, Vec::new, &[]).unwrap();
        assert_eq!(b, root.join("bin").join("bash.exe"));
    }

    #[test]
    fn bash_is_found_from_env_root_registry_and_defaults() {
        let (a, b, c) = (tmp("env").join("A"), tmp("reg").join("B"), tmp("def").join("C"));
        for r in [&a, &b, &c] {
            fake_git(r);
        }
        let reg = || vec![b.clone()];
        assert_eq!(find_git_bash("", Some(a.to_str().unwrap()), reg, std::slice::from_ref(&c)), bash_in_root(&a));
        assert_eq!(find_git_bash("", None, reg, &[]), bash_in_root(&b));
        assert_eq!(find_git_bash("", None, || panic!("registry must stay unread"), std::slice::from_ref(&c)), bash_in_root(&c));
        assert_eq!(find_git_bash("", None, Vec::new, &[]), None);
    }

    #[test]
    fn the_wsl_stub_is_never_taken() {
        assert!(is_wsl_stub(Path::new(r"C:\Windows\System32")));
        assert!(is_wsl_stub(Path::new(r"C:\Users\x\AppData\Local\Microsoft\WindowsApps")));
        assert!(!is_wsl_stub(Path::new(r"D:\Programas\Git\bin")));
        let d = tmp("stub").join("Windows").join("System32");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("bash.exe"), "").unwrap();
        assert_eq!(find_git_bash(d.to_str().unwrap(), None, Vec::new, &[]), None);
    }

    #[cfg(windows)]
    #[test]
    fn registry_api_reads_without_failing() {
        // a machine without Git for Windows gives an empty list
        for r in registry_roots() {
            assert!(!r.as_os_str().is_empty());
        }
    }

    #[test]
    fn plain_program_runs_without_a_shell_and_the_rest_does_not() {
        let d = tmp("direct");
        let f = d.join(if cfg!(windows) { "tool.exe" } else { "tool" });
        std::fs::write(&f, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = d.to_str().unwrap();
        let (p, args) = direct_program("tool --flag value", path).unwrap();
        assert_eq!(p, f);
        assert_eq!(args, ["--flag", "value"]);
        for shelly in ["tool | cat", "tool \"a b\"", "tool $HOME", "./tool", "missing"] {
            assert!(direct_program(shelly, path).is_none(), "{shelly}");
        }
    }

    #[test]
    fn a_fresh_output_is_reused_only_for_the_same_command_and_input() {
        let f = tmp("reuse").join("k.txt");
        let d = digest(Some("c"), "{}");
        assert_eq!(reused(&f, d), None);
        remember(&f, d, "line\nsecond");
        assert_eq!(reused(&f, d).as_deref(), Some("line\nsecond"));
        assert_eq!(reused(&f, digest(Some("c"), "{\"a\":1}")), None);
        assert_eq!(reused(&f, digest(Some("other"), "{}")), None);
    }

    #[test]
    fn fallback_line_uses_model_and_context() {
        assert_eq!(fallback_line(r#"{"model":{"display_name":"Opus"},"context_window":{"used_percentage":12.4}}"#), "Opus | ctx 12%");
        assert_eq!(fallback_line(r#"{"model":{"id":"m1"}}"#), "m1");
        assert_eq!(fallback_line("not json"), "Claude");
    }

    #[test]
    fn user_command_output_is_passed_through() {
        let out = run_user("echo hello-status", "{}").unwrap();
        assert!(out.contains("hello-status"), "{out}");
        assert!(render(Some("echo hello-status"), "{}").contains("hello-status"));
    }

    #[test]
    fn a_failing_or_silent_command_still_shows_a_line() {
        let input = r#"{"model":{"display_name":"Opus"}}"#;
        assert!(run_user("exit 3", input).is_err());
        assert_eq!(render(Some("exit 3"), input), "Opus");
        assert_eq!(render(Some("this-command-does-not-exist-xyz"), input), "Opus");
        assert_eq!(render(None, input), "Opus");
    }

    #[test]
    fn the_command_receives_claudes_json_on_stdin() {
        // without Git Bash on the machine `cat` may not exist: assert only when it ran
        if let Ok(o) = run_user("cat", r#"{"a":1}"#) {
            assert_eq!(o.trim(), r#"{"a":1}"#);
        }
    }
}
