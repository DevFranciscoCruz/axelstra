//! Wire format shared by daemon and client.
//!
//! Frame: `[u32 LE len][u8 kind][payload]`, where `len = 1 + payload.len()`.
//!   kind 0  JSON request  `{"id":n,"method":"..","params":{..}}`  (client -> daemon)
//!           JSON response `{"id":n,"result":..}` or `{"id":n,"error":".."}` (daemon -> client)
//!           An error may also carry `"code":"domain.name"` and `"params":{..}` (additive, so PROTO did not change): a stable
//!           message code the UI translates (`app/src/msg/*.ts`); `error` stays the English text for the CLI, logs and old
//!           frontends. Raise one with `axelstra_core::coded!`, see `crates/core/src/msg.rs`. Results and events that carry
//!           text for the UI add `<field>_code` / `<field>_params` next to the string; `notices.take` adds `items`.
//!   kind 1  PTY output    `[u32 LE session][bytes]`                 (daemon -> client)
//!   kind 2  PTY input     `[u32 LE session][bytes]`                 (client -> daemon)
//!   kind 3  JSON event    `{"event":"session.exit","id":n,"code":c}` (daemon -> client)

/// Version of the app <-> daemon contract. A daemon outlives app updates, so the app only restarts
/// a daemon older than this; bump it when the app starts calling something an old daemon lacks.
/// 2: app.dir, session.pending, session.pending_clear. 6: settings.get / settings.set. 7: settings.get / settings.set carry shell, notify_sound, notify_mode. 8: session.rename. 9: git.suggest. 10: agent.worktree_usage. 11: docker.*. 12: agents.list, git.checks, git.merge, project.env_preview, worktree.rename, dispatch.overlaps, notes.*, worktree.overview, session.hint. 13: worktree.set_parent, notes.worktree, notes.read_file, worktree.create parent. 14: hooks.status, hooks.install. 15: notices.take, audit.list, backup.export, backup.restore, orch.trust_info, env.tools. 16: janitor.events. 17: git.suggest_pr, gh.comments, gh.send_comment, triggers.get, triggers.preview, triggers.set, triggers.events. 18: budget.status, budget.check, budget.events, dispatch.overlap_files, worktree.rebase, git.timeline, and `budgets` in settings.get / settings.set, workspace.export, workspace.import_preview, workspace.import. 19: host.info, host.browse, and `hosts` in settings.get / settings.set; hooks.status / hooks.install take `agent` (claude or gemini). 20: `stalled` in session.list and `stall_minutes` in settings.get / settings.set. 21: events on the watch stream the app relies on instead of polling: `janitor.events`, `triggers.events`, `budget.events` (take them with the call of the same name) and `dispatch.changed`. 22: worktree.integrate, project.integrate, `pending` in worktree.overview, `kind` in janitor.events, `auto_integrate` and `push_after_integrate` in the project list.
pub const PROTO: u64 = 22;

/// Oldest daemon the app accepts for security reasons, whatever its `PROTO`: bump it when a release fixes something in
/// the daemon that must not keep running after an update. The app restarts a daemon that reports less (or nothing).
/// 2: roles come from the process on the other end of a connection (S3b), not from the token it presents; a daemon
/// below this still trusts any holder of the token file as the user.
pub const SECURITY_MIN: u64 = 2;

/// Identifies this build. `hello` and `ping` report it, so an app can tell a daemon left over from an older release.
pub const BUILD_ID: &str = concat!(env!("CARGO_PKG_VERSION"), "+", env!("CARGO_PKG_NAME"));

pub const MAX_FRAME: usize = 16 << 20;

pub const JSON: u8 = 0;
pub const OUTPUT: u8 = 1;
pub const INPUT: u8 = 2;
pub const EVENT: u8 = 3;

pub fn encode(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(5 + payload.len());
    f.extend_from_slice(&((payload.len() + 1) as u32).to_le_bytes());
    f.push(kind);
    f.extend_from_slice(payload);
    f
}

pub fn session_frame(kind: u8, session: u32, data: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(4 + data.len());
    p.extend_from_slice(&session.to_le_bytes());
    p.extend_from_slice(data);
    encode(kind, &p)
}

/// Splits a kind 1/2 payload into (session, bytes).
pub fn split_session(payload: &[u8]) -> Option<(u32, &[u8])> {
    if payload.len() < 4 {
        return None;
    }
    Some((u32::from_le_bytes(payload[..4].try_into().ok()?), &payload[4..]))
}

/// Where the daemon listens: a named pipe on Windows, a Unix socket elsewhere. The Windows name carries the user's SID,
/// not a name taken from the environment: `%USERNAME%` is anyone's to set, a SID is not.
/// `AXELSTRA_TEST_PIPE` (letters and digits) appends a suffix so the smoke and E2E scripts get a pipe of their own
/// instead of the developer's daemon; it only moves where a process listens or connects, the owner check still applies.
#[cfg(windows)]
pub fn pipe_name() -> String {
    let suffix: String = std::env::var("AXELSTRA_TEST_PIPE").unwrap_or_default().chars().filter(char::is_ascii_alphanumeric).collect();
    let suffix = if suffix.is_empty() { suffix } else { format!("-t{suffix}") };
    // an overridden data dir gets a pipe of its own, so one variable isolates a whole daemon
    let suffix = match override_dir() {
        Some(d) => {
            format!("-d{:016x}{suffix}", fnv1a(d.to_string_lossy().to_lowercase().replace('\\', "/").trim_end_matches('/').as_bytes()))
        }
        None => suffix,
    };
    match crate::pipesec::current_user_sid() {
        Ok(sid) => format!(r"\\.\pipe\axelstra-{sid}{suffix}"),
        Err(_) => legacy_pipe_name(),
    }
}

/// The pipe of the real, default data dir: no `AXELSTRA_DATA_DIR`, no `AXELSTRA_TEST_PIPE`.
#[cfg(windows)]
pub fn default_pipe_name() -> String {
    match crate::pipesec::current_user_sid() {
        Ok(sid) => format!(r"\\.\pipe\axelstra-{sid}"),
        Err(_) => legacy_pipe_name(),
    }
}

#[cfg(not(windows))]
pub fn default_pipe_name() -> String {
    default_data_dir().map(|d| d.join("axelstra.sock").to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(windows)]
fn fnv1a(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf29ce484222325, |h, x| (h ^ u64::from(*x)).wrapping_mul(0x100000001b3))
}

/// The pipe name older releases used (`%USERNAME%`). A client falls back to it so an update does not strand the
/// sessions of a daemon that is still running; the owner check applies to it all the same.
#[cfg(windows)]
pub fn legacy_pipe_name() -> String {
    let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
    format!(r"\\.\pipe\axelstra-{user}")
}

#[cfg(not(windows))]
pub fn pipe_name() -> String {
    data_dir().join("axelstra.sock").to_string_lossy().into_owned()
}

/// Per-user private state (token, project list, socket), or why there is none: with no profile directory the
/// shared temp dir would hold the daemon's secret, so there is no fallback to it.
///
/// `AXELSTRA_DATA_DIR` (a non-empty path) replaces the whole directory: tests and scripts set it so they never touch
/// the real one. The Windows pipe name follows it (see `pipe_name`).
pub fn try_data_dir() -> anyhow::Result<std::path::PathBuf> {
    match override_dir() {
        Some(d) => Ok(d),
        None => default_data_dir(),
    }
}

/// `AXELSTRA_DATA_DIR`, when set and not empty.
pub fn override_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("AXELSTRA_DATA_DIR").filter(|v| !v.is_empty()).map(std::path::PathBuf::from)
}

/// The real per-user data dir, ignoring `AXELSTRA_DATA_DIR`. Tests compare against it to refuse touching it.
pub fn default_data_dir() -> anyhow::Result<std::path::PathBuf> {
    let home = || std::env::var_os("HOME").filter(|h| !h.is_empty()).map(std::path::PathBuf::from);
    let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(std::path::PathBuf::from);
    let base = if cfg!(windows) {
        var("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library").join("Application Support"))
    } else {
        var("XDG_DATA_HOME").or_else(|| home().map(|h| h.join(".local").join("share")))
    };
    base.map(|b| b.join("axelstra")).ok_or_else(|| anyhow::anyhow!("no per-user data directory (LOCALAPPDATA / HOME is not set)"))
}

/// Per-user private state (token, project list, socket). Reads that cannot name a directory get the temp dir (never
/// anything secret: `load_or_create_token` and `ensure_data_dir` use `try_data_dir` and fail instead).
pub fn data_dir() -> std::path::PathBuf {
    try_data_dir().unwrap_or_else(|_| std::env::temp_dir().join("axelstra"))
}

/// Shared secret, stored under the user's private profile dir.
pub fn token_path() -> std::path::PathBuf {
    data_dir().join("token")
}

/// Creates the data dir; on Unix it is private to the user (0700).
pub fn ensure_data_dir() -> anyhow::Result<std::path::PathBuf> {
    let d = try_data_dir()?;
    std::fs::create_dir_all(&d)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(d)
}

/// Compares secrets without stopping at the first differing byte.
pub fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = (a.len() ^ b.len()) as u8;
    for i in 0..a.len().max(b.len()) {
        diff |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    diff == 0
}

pub fn load_or_create_token() -> anyhow::Result<String> {
    let d = ensure_data_dir()?;
    let p = d.join("token");
    if let Ok(t) = std::fs::read_to_string(&p) {
        let t = t.trim().to_string();
        if t.len() >= 32 {
            return Ok(t);
        }
    }
    let mut b = [0u8; 24];
    getrandom::getrandom(&mut b).map_err(|e| anyhow::anyhow!("getrandom: {e}"))?;
    let t: String = b.iter().map(|x| format!("{x:02x}")).collect();
    // an unusable token file (short, empty) is replaced; the new one is born private, never world-readable even briefly
    let _ = std::fs::remove_file(&p);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    match opts.open(&p) {
        Ok(mut f) => {
            use std::io::Write;
            f.write_all(t.as_bytes())?;
            Ok(t)
        }
        // another process won the race to create it: use its token
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let t = std::fs::read_to_string(&p)?.trim().to_string();
            anyhow::ensure!(t.len() >= 32, "the token file is unusable: {}", p.display());
            Ok(t)
        }
        Err(e) => Err(e.into()),
    }
}

/// True under `cfg(test)` or when `AXELSTRA_TESTING=1` (the test helper sets it).
pub fn testing() -> bool {
    cfg!(test) || std::env::var_os("AXELSTRA_TESTING").is_some_and(|v| v == "1")
}

fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    let n = |p: &std::path::Path| p.to_string_lossy().to_lowercase().replace('\\', "/").trim_end_matches('/').to_string();
    n(a) == n(b)
}

/// A daemon started by a test must never own the real data dir or the real pipe. Called by `serve_with_store` with the
/// directory that holds its projects file.
pub fn refuse_real_when_testing(pipe: &str, dir: &std::path::Path) -> anyhow::Result<()> {
    if !testing() {
        return Ok(());
    }
    if default_data_dir().is_ok_and(|real| same_path(&real, dir)) || pipe == default_pipe_name() {
        anyhow::bail!("refusing to serve the real data dir or pipe from a test (set AXELSTRA_DATA_DIR; see docs/TESTING.md)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ct_eq;

    #[test]
    fn a_test_daemon_is_refused_the_real_data_dir_and_pipe() {
        let real = super::default_data_dir().unwrap();
        assert!(super::refuse_real_when_testing("x", &real).is_err());
        // case and separator style must not dodge the check
        assert!(
            super::refuse_real_when_testing("x", std::path::Path::new(&real.to_string_lossy().to_uppercase().replace('\\', "/"))).is_err()
        );
        assert!(super::refuse_real_when_testing(&super::default_pipe_name(), &std::env::temp_dir().join("axl-other")).is_err());
        assert!(super::refuse_real_when_testing("x", &std::env::temp_dir().join("axl-other")).is_ok());
    }

    #[test]
    fn constant_time_compare_matches_equality() {
        assert!(ct_eq("abc", "abc"));
        assert!(!ct_eq("abc", "abd"));
        assert!(!ct_eq("abc", "ab"));
        assert!(!ct_eq("", "a"));
        assert!(ct_eq("", ""));
    }
}
