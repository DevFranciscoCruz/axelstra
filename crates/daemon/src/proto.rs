//! Wire format shared by daemon and client.
//!
//! Frame: `[u32 LE len][u8 kind][payload]`, where `len = 1 + payload.len()`.
//!   kind 0  JSON request  `{"id":n,"method":"..","params":{..}}`  (client -> daemon)
//!           JSON response `{"id":n,"result":..}` or `{"id":n,"error":".."}` (daemon -> client)
//!   kind 1  PTY output    `[u32 LE session][bytes]`                 (daemon -> client)
//!   kind 2  PTY input     `[u32 LE session][bytes]`                 (client -> daemon)
//!   kind 3  JSON event    `{"event":"session.exit","id":n,"code":c}` (daemon -> client)

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

pub fn pipe_name() -> String {
    let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
    format!(r"\\.\pipe\axelstra-{user}")
}

/// Shared secret, stored under the user's private profile dir.
pub fn token_path() -> std::path::PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("axelstra").join("token")
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
    std::fs::create_dir_all(p.parent().unwrap())?;
    std::fs::write(&p, &t)?;
    Ok(t)
}
