//! The Gemini hook contract, checked with payloads a real gemini-cli 0.65.0 sent to its hooks (fixtures, paths
//! anonymised): stdin is one JSON object, stdout must be JSON (an empty object means "no decision"), exit 0. Nothing
//! here reaches the user's home, data dir or daemon: every child gets a throwaway one.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const EVENTS: [&str; 4] = ["BeforeAgent", "AfterAgent", "Notification", "AfterTool"];

fn sandbox(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("axl-gemhook-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn isolate(cmd: &mut Command, home: &Path) {
    cmd.env("HOME", home)
        .env("USERPROFILE", home)
        .env("LOCALAPPDATA", home)
        .env("XDG_DATA_HOME", home)
        .env("AXELSTRA_DATA_DIR", home.join("axelstra"))
        .env("AXELSTRA_TESTING", "1")
        .env("AXELSTRA_TEST_PIPE", format!("gh{}", std::process::id()))
        .env("USERNAME", format!("axgh{}", std::process::id()));
}

fn fixture(event: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gemini-0.65.0").join(format!("hook-{event}.json"));
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn run_hook(home: &Path, event: &str, session: Option<&str>, stdin: &str) -> (Output, Duration) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_axelstra"));
    cmd.args(["hook", "gemini", event]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    isolate(&mut cmd, home);
    cmd.env_remove("AXELSTRA_SESSION");
    if let Some(s) = session {
        cmd.env("AXELSTRA_SESSION", s);
    }
    let t = Instant::now();
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    (out, t.elapsed())
}

#[test]
fn the_fixtures_are_the_payloads_gemini_documents() {
    for e in EVENTS {
        let v: serde_json::Value = serde_json::from_str(&fixture(e)).unwrap();
        assert_eq!(v["hook_event_name"], e);
        assert!(v["session_id"].is_string() && v["transcript_path"].is_string() && v["cwd"].is_string() && v["timestamp"].is_string());
    }
    let n: serde_json::Value = serde_json::from_str(&fixture("Notification")).unwrap();
    assert_eq!(n["notification_type"], "ToolPermission");
}

#[test]
fn every_event_answers_an_empty_json_object_and_exit_zero_with_or_without_a_session() {
    let home = sandbox("answers");
    for e in EVENTS {
        for session in [None, Some("4242")] {
            let (out, took) = run_hook(&home, e, session, &fixture(e));
            assert!(out.status.success(), "{e} {session:?}: {out:?}");
            assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "{e} {session:?}");
            // no daemon is running in the sandbox: the hook gives up fast, it never holds the agent up
            assert!(took < Duration::from_secs(4), "{e} took {took:?}");
        }
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn claude_hooks_still_print_nothing() {
    let home = sandbox("claude");
    let (out, _) = run_hook(&home, "Stop", Some("1"), "{}");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_axelstra"));
    cmd.args(["hook", "claude", "Stop"]).stdin(Stdio::null());
    isolate(&mut cmd, &home);
    let claude = cmd.output().unwrap();
    assert!(claude.status.success() && claude.stdout.is_empty(), "{claude:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "the gemini one prints the object");
    let _ = std::fs::remove_dir_all(&home);
}

/// What `axelstra hook install gemini` writes is run the way Gemini runs it: `powershell -NoProfile -Command <command>`
/// on Windows, `bash -c <command>` elsewhere, with the command as ONE argument. A quoted path alone is not a call in
/// PowerShell, which is how the hooks silently never ran on Windows before this was checked against the real CLI.
#[test]
fn the_installed_command_runs_under_the_shell_gemini_uses() {
    let home = sandbox("shell");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_axelstra"));
    cmd.args(["hook", "install", "gemini"]);
    isolate(&mut cmd, &home);
    assert!(cmd.output().unwrap().status.success());
    let settings: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(home.join(".gemini/settings.json")).unwrap()).unwrap();
    let hooks = settings["hooks"].as_object().unwrap();
    assert_eq!(hooks.len(), EVENTS.len(), "{hooks:?}");
    for e in EVENTS {
        let cmdline = hooks[e][0]["hooks"][0]["command"].as_str().unwrap();
        assert_eq!(hooks[e][0]["hooks"][0]["type"], "command");
        let mut sh = if cfg!(windows) {
            let mut c = Command::new("powershell.exe");
            c.args(["-NoProfile", "-NonInteractive", "-Command", cmdline]);
            c
        } else {
            let mut c = Command::new("bash");
            c.args(["-c", cmdline]);
            c
        };
        isolate(&mut sh, &home);
        sh.env_remove("AXELSTRA_SESSION").stdin(Stdio::null());
        let out = sh.output().unwrap();
        assert!(out.status.success(), "{e}: `{cmdline}` -> {out:?}");
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "{e}: `{cmdline}`");
    }
    let _ = std::fs::remove_dir_all(&home);
}
