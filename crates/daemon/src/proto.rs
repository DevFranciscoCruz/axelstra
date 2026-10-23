//! Wire format shared by daemon and client.
//!
//! Frame: `[u32 LE len][u8 kind][payload]`, where `len = 1 + payload.len()`.
//!   kind 0  JSON request  `{"id":n,"method":"..","params":{..}}`  (client -> daemon)
//!           JSON response `{"id":n,"result":..}` or `{"id":n,"error":".."}` (daemon -> client)
//!   kind 1  PTY output    `[u32 LE session][bytes]`                 (daemon -> client)
//!   kind 2  PTY input     `[u32 LE session][bytes]`                 (client -> daemon)
//!   kind 3  JSON event    `{"event":"session.exit","id":n,"code":c}` (daemon -> client)

/// Version of the app <-> daemon contract. A daemon outlives app updates, so the app only restarts
/// a daemon older than this; bump it when the app starts calling something an old daemon lacks.
/// 2: app.dir, session.pending, session.pending_clear. 6: settings.get / settings.set. 7: settings.get / settings.set carry shell, notify_sound, notify_mode. 8: session.rename. 9: git.suggest. 10: docker.*.
pub const PROTO: u64 = 10;

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

/// Where the daemon listens: a named pipe on Windows, a Unix socket elsewhere.
#[cfg(windows)]
pub fn pipe_name() -> String {
    let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
    format!(r"\\.\pipe\axelstra-{user}")
}

#[cfg(not(windows))]
pub fn pipe_name() -> String {
    data_dir().join("axelstra.sock").to_string_lossy().into_owned()
}

/// Per-user private state (token, project list, socket).
pub fn data_dir() -> std::path::PathBuf {
    let home = || std::env::var_os("HOME").map(std::path::PathBuf::from);
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from)
    } else if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library").join("Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| home().map(|h| h.join(".local").join("share")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("axelstra")
}

/// Shared secret, stored under the user's private profile dir.
pub fn token_path() -> std::path::PathBuf {
    data_dir().join("token")
}

/// Creates the data dir; on Unix it is private to the user (0700).
pub fn ensure_data_dir() -> anyhow::Result<std::path::PathBuf> {
    let d = data_dir();
    std::fs::create_dir_all(&d)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(d)
}

pub fn load_or_create_token() -> anyhow::Result<String> {
    let p = token_path();
    if let Ok(t) = std::fs::read_to_string(&p) {
        let t = t.trim().to_string();
        if t.len() >= 32 {
            return Ok(t);
        }
    }
    let mut b = [0u8; 24];
    getrandom::getrandom(&mut b).map_err(|e| anyhow::anyhow!("getrandom: {e}"))?;
    let t: String = b.iter().map(|x| format!("{x:02x}")).collect();
    ensure_data_dir()?;
    std::fs::write(&p, &t)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(t)
}
