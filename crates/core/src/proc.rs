//! The one place that decides whether a child process may show a console window.
//!
//! The daemon and the desktop app have no console, so a console child (git, gh, docker, robocopy, ssh, wsl, cmd...)
//! would flash a new window every time it starts. Call [`hidden`] on every `Command` they spawn themselves.
//!
//! Deliberately NOT hidden: the user's terminal and agent ptys (`portable-pty` spawns in `pty.rs`, they are the visible
//! terminal), and the build `axelstra lock` runs in the user's own terminal (it already goes through `Job::spawn`, which
//! hides only the extra window). Tests that spawn helper processes are left alone too.

use std::process::Command;

/// `CREATE_NO_WINDOW`: the child gets no console window. Combine it with other flags only through `creation_flags`
/// (that call replaces, it does not add: the desktop app starts the daemon with `DETACHED_PROCESS | NO_WINDOW`).
pub const NO_WINDOW: u32 = 0x0800_0000;

/// Keeps a console window from flashing on Windows; a no-op elsewhere. Do not call it after another `creation_flags`.
pub fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(NO_WINDOW);
    }
    cmd
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn a_hidden_child_still_runs_and_reports_its_exit_code() {
        let ok = hidden(Command::new("cmd").args(["/d", "/c", "exit 0"])).status().unwrap();
        assert!(ok.success());
        let bad = hidden(Command::new("cmd").args(["/d", "/c", "exit 3"])).status().unwrap();
        assert_eq!(bad.code(), Some(3));
    }

    #[test]
    fn a_hidden_child_keeps_its_piped_output() {
        let o = hidden(Command::new("cmd").args(["/d", "/c", "echo hi"])).output().unwrap();
        assert!(String::from_utf8_lossy(&o.stdout).contains("hi"));
    }
}
