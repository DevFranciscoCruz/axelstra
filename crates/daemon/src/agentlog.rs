//! Reads the transcripts the agent CLIs (Claude, Codex, Gemini) already keep on disk: full-text search across past
//! conversations and token usage of one session. Nothing is indexed or cached on disk; files are
//! scanned on demand, newest first, so an idle daemon costs nothing.
use axelstra_core::sync::LockRecover;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from)
}

fn claude_root() -> Option<PathBuf> {
    Some(home()?.join(".claude").join("projects"))
}

fn codex_root() -> Option<PathBuf> {
    Some(home()?.join(".codex").join("sessions"))
}

fn gemini_root() -> Option<PathBuf> {
    Some(home()?.join(".gemini").join("tmp"))
}

/// Claude names a project folder after its path with every non-alphanumeric character as `-`.
pub fn claude_dir_name(cwd: &str) -> String {
    cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn mtime(p: &Path) -> u64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

fn jsonl_files(root: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(root) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                jsonl_files(&p, depth - 1, out);
            }
        } else if p.extension().is_some_and(|x| x == "jsonl") {
            out.push(p);
        }
    }
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Hit {
    pub agent: &'static str,
    pub session: String,
    pub cwd: String,
    pub ts: u64,
    pub snippet: String,
    /// Gemini keeps no folder in its logs, only the project hash: the app matches it to a worktree
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// Plain text of one transcript line: message content of either CLI, whatever its shape.
fn line_text(v: &Value) -> String {
    fn walk(v: &Value, out: &mut String, depth: usize) {
        if depth > 6 {
            return;
        }
        match v {
            Value::String(s) => {
                out.push_str(s);
                out.push('\n');
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out, depth + 1)),
            Value::Object(o) => {
                for (k, x) in o {
                    // skip ids, signatures and other machine fields
                    if matches!(
                        k.as_str(),
                        "id" | "uuid"
                            | "parentUuid"
                            | "sessionId"
                            | "signature"
                            | "type"
                            | "role"
                            | "model"
                            | "cwd"
                            | "timestamp"
                            | "version"
                            | "usage"
                    ) {
                        continue;
                    }
                    walk(x, out, depth + 1);
                }
            }
            _ => {}
        }
    }
    let mut s = String::new();
    walk(v, &mut s, 0);
    s
}

fn snippet(text: &str, needle_lc: &str) -> Option<String> {
    let lc = text.to_lowercase();
    let at = lc.find(needle_lc)?;
    // lowercasing can shift byte offsets for a few scripts; clamp to char boundaries on the original
    let mut start = at.saturating_sub(60).min(text.len());
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (at + needle_lc.len() + 100).min(text.len());
    while !text.is_char_boundary(end) {
        end += 1;
    }
    Some(text[start..end].split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Lines longer than this are skipped by the search (a pasted binary blob is not worth a parse).
const MAX_SEARCH_LINE: usize = 8 << 20;
/// Total bytes one search reads before it stops with what it found.
const SEARCH_BUDGET: u64 = 512 << 20;

/// True when `needle_lc` (lowercase) can be in `line`. For an ASCII needle this is a byte scan with no copy of the
/// line; a needle with other letters is compared on the lowercased text, as Unicode case folding is not bytewise.
fn may_contain(line: &[u8], needle_lc: &str) -> bool {
    if needle_lc.is_ascii() {
        let n = needle_lc.as_bytes();
        return line.len() >= n.len() && line.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n));
    }
    String::from_utf8_lossy(line).to_lowercase().contains(needle_lc)
}

/// Searches one transcript; returns the bytes read so the caller can bound the whole search.
fn search_file(p: &Path, agent: &'static str, needle_lc: &str, per_file: usize, out: &mut Vec<Hit>) -> u64 {
    let Ok(f) = std::fs::File::open(p) else { return 0 };
    let session = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ts = mtime(p);
    let mut cwd = String::new();
    let mut found = 0;
    let mut read = 0u64;
    let mut r = BufReader::new(f);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = match r.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        read += n as u64;
        if buf.len() > MAX_SEARCH_LINE {
            continue;
        }
        if cwd.is_empty() {
            let line = String::from_utf8_lossy(&buf);
            if let Some(i) = line.find("\"cwd\":\"") {
                let rest = &line[i + 7..];
                if let Some(j) = rest.find('"') {
                    cwd = rest[..j].replace("\\\\", "\\");
                }
            }
        }
        // cheap pre-filter on the raw bytes before decoding or parsing the line
        if !may_contain(&buf, needle_lc) {
            continue;
        }
        let line = String::from_utf8_lossy(&buf);
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        if let Some(sn) = snippet(&line_text(&v), needle_lc) {
            out.push(Hit { agent, session: session.clone(), cwd: cwd.clone(), ts, snippet: sn, project: None });
            found += 1;
            if found >= per_file {
                break;
            }
        }
    }
    read
}

/// Newest conversations first; at most `max` hits, two per conversation.
pub fn search(query: &str, max: usize) -> Vec<Hit> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return vec![];
    }
    let mut files: Vec<(PathBuf, &'static str)> = vec![];
    let mut v = vec![];
    if let Some(r) = claude_root() {
        jsonl_files(&r, 1, &mut v);
        files.extend(v.drain(..).map(|p| (p, "claude")));
    }
    if let Some(r) = codex_root() {
        jsonl_files(&r, 4, &mut v);
        files.extend(v.drain(..).map(|p| (p, "codex")));
    }
    if let Some(r) = gemini_root() {
        files.extend(gemini_files(&r).into_iter().map(|p| (p, "gemini")));
    }
    files.sort_by_key(|(p, _)| std::cmp::Reverse(mtime(p)));
    let mut hits = vec![];
    let mut spent = 0u64;
    // the newest 400 conversations are plenty; older ones are a click away in the CLI's own picker
    for (p, agent) in files.into_iter().take(400) {
        if agent == "gemini" {
            search_gemini(&p, &needle, 2, &mut hits);
        } else {
            spent += search_file(&p, agent, &needle, 2, &mut hits);
        }
        if hits.len() >= max {
            hits.truncate(max);
            break;
        }
        if spent >= SEARCH_BUDGET {
            break;
        }
    }
    hits
}

// ---- Gemini CLI ----
// Layout, verified against gemini-cli 0.65.0 (a real install writing to a throwaway HOME; fixtures in the tests):
//   ~/.gemini/tmp/PROJECT_DIR/chats/session-TIMESTAMP-ID8.jsonl   one session per file
//   PROJECT_DIR is a short slug of the folder name ("proj", "proj-1" on a clash) that `~/.gemini/projects.json` maps
//   from the root path (lower-cased on Windows); older installs named the folder after the SHA-256 of the root path.
//   Lines, in order: a header { sessionId, projectHash (SHA-256 of the root path as Gemini saw it), startTime,
//   lastUpdated, kind }, then message records { id, timestamp, type: "user" | "gemini" | "info" | "warning" | "error",
//   content: string or [ { text } | { functionResponse } ], model?, tokens?: { input, output, cached, thoughts, tool,
//   total }, toolCalls? }, interleaved with { $set: { lastUpdated, summary.. } }, { $patch } and { $rewindTo: id }.
//   A message is written again under the SAME id when its tool calls are filled in: the last record of an id wins
//   (counting every line would charge those tokens twice). `input` includes `cached`. The first user message is the
//   injected `<session_context>` block (date, OS, directory tree), which is not something the user typed.
// Older builds wrote one `.json` object { sessionId, projectHash, startTime, lastUpdated, messages: [..] }: still read.
// `logs.json` next to `chats` only holds the user prompts, so it is not read. `gemini --resume` takes `latest`, an
// index or a session id, and only sees the conversations of the project it runs in.

/// Gemini files larger than this are skipped, so one runaway log cannot stall a search.
const GEMINI_MAX_BYTES: u64 = 16 * 1024 * 1024;

fn gemini_files_of(project: &Path) -> Vec<PathBuf> {
    let Ok(chats) = std::fs::read_dir(project.join("chats")) else { return vec![] };
    chats.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json" || x == "jsonl")).collect()
}

/// Chat folders of the project rooted at `path`: the slug folder the registry (`~/.gemini/projects.json`, next to
/// `tmp`) maps it to, plus the SHA-256 named folders of older installs. A folder that does not exist yields no files.
fn gemini_project_dirs(root: &Path, registry: &[(String, String)], path: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = registry.iter().filter(|(p, _)| same_root(p, path)).map(|(_, slug)| root.join(slug)).collect();
    v.extend(gemini_hashes(path).into_iter().map(|h| root.join(h)));
    v.dedup();
    v
}

/// The registry as (root path, folder) pairs; empty when the file is missing or not the expected shape.
fn gemini_registry(root: &Path) -> Vec<(String, String)> {
    let Some(text) = root.parent().and_then(|g| std::fs::read_to_string(g.join("projects.json")).ok()) else { return vec![] };
    let Ok(v) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else { return vec![] };
    v["projects"].as_object().map(|m| m.iter().filter_map(|(k, s)| Some((k.clone(), s.as_str()?.to_string()))).collect()).unwrap_or_default()
}

/// Same folder spelled two ways: Gemini lower-cases a Windows root before it registers it, and slashes vary.
fn same_root(a: &str, b: &str) -> bool {
    let norm = |p: &str| {
        let p = p.trim_start_matches(r"\\?\").trim_end_matches(['/', '\\']).replace('\\', "/");
        if p.as_bytes().get(1) == Some(&b':') {
            p.to_ascii_lowercase()
        } else {
            p
        }
    };
    norm(a) == norm(b)
}

fn gemini_files(root: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(root) else { return vec![] };
    rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).flat_map(|p| gemini_files_of(&p)).collect()
}

/// SHA-256 as lowercase hex (Gemini names a project folder after the hash of its path).
pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be,
        0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa,
        0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85,
        0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
        0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f,
        0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for (i, b) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut a = h;
        for i in 0..64 {
            let s1 = a[4].rotate_right(6) ^ a[4].rotate_right(11) ^ a[4].rotate_right(25);
            let ch = (a[4] & a[5]) ^ (!a[4] & a[6]);
            let t1 = a[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a[0].rotate_right(2) ^ a[0].rotate_right(13) ^ a[0].rotate_right(22);
            let maj = (a[0] & a[1]) ^ (a[0] & a[2]) ^ (a[1] & a[2]);
            a = [t1.wrapping_add(s0.wrapping_add(maj)), a[0], a[1], a[2], a[3].wrapping_add(t1), a[4], a[5], a[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(a[i]);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

/// Project hashes a worktree path may have under Gemini: the path as written and the slash and trailing-slash variants
/// (Gemini hashes whatever string its process saw as the root, and on Windows that varies).
pub fn gemini_hashes(path: &str) -> Vec<String> {
    let t = path.trim_start_matches(r"\\?\").trim_end_matches(['/', '\\']);
    let mut v: Vec<String> = vec![];
    for cand in [path.to_string(), t.to_string(), t.replace('/', "\\"), t.replace('\\', "/")] {
        let h = sha256_hex(cand.as_bytes());
        if !v.contains(&h) {
            v.push(h);
        }
    }
    v
}

/// One Gemini conversation, reduced to what search and totals need.
#[derive(Debug, Default, PartialEq)]
pub struct GeminiSession {
    pub session: String,
    pub project_hash: String,
    /// first user prompt, one line
    pub title: String,
    pub started: u64,
    pub updated: u64,
    /// (role, text) of the user and model messages
    pub messages: Vec<(&'static str, String)>,
    pub usage: Usage,
}

fn iso_secs(v: &Value) -> Option<u64> {
    let t = chrono::DateTime::parse_from_rfc3339(v.as_str()?).ok()?.timestamp();
    u64::try_from(t).ok()
}

/// Text of a message `content`: a plain string or a list of parts with a `text`.
fn gemini_text(c: &Value) -> String {
    match c {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().filter_map(|p| p["text"].as_str().or_else(|| p.as_str())).collect::<Vec<_>>().join("\n"),
        Value::Object(_) => c["text"].as_str().unwrap_or_default().to_string(),
        _ => String::new(),
    }
}

fn absorb_header(v: &Value, s: &mut GeminiSession) {
    // `{"$set":{"lastUpdated":..}}` lines update the header in place
    if v["$set"].is_object() {
        return absorb_header(&v["$set"], s);
    }
    if let Some(id) = v["sessionId"].as_str() {
        s.session = id.to_string();
    }
    if let Some(h) = v["projectHash"].as_str() {
        s.project_hash = h.to_string();
    }
    if let Some(t) = iso_secs(&v["startTime"]) {
        s.started = t;
    }
    if let Some(t) = iso_secs(&v["lastUpdated"]) {
        s.updated = t;
    }
}

/// Parses one session file (`.json`, or `.jsonl` with one record per line). Never panics: anything unreadable
/// or unexpected is skipped, and `None` means the file holds no usable conversation.
pub fn parse_gemini(text: &str, jsonl: bool) -> Option<GeminiSession> {
    let mut sess = GeminiSession::default();
    let mut msgs: Vec<Value> = vec![];
    if jsonl {
        for line in text.lines() {
            let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
            let id_of = |m: &Value| m["id"].as_str().map(String::from);
            if let Some(to) = v["$rewindTo"].as_str() {
                // the conversation was rewound to before this message: it and everything after it are gone
                match msgs.iter().position(|m| m["id"].as_str() == Some(to)) {
                    Some(i) => msgs.truncate(i),
                    None => msgs.clear(),
                }
            } else if v["$patch"].is_object() {
                let p = &v["$patch"];
                for u in std::iter::once(p).chain(p["updates"].as_array().into_iter().flatten()) {
                    if let (Some(id), Some(c)) = (u["id"].as_str(), u.get("content")) {
                        if let Some(m) = msgs.iter_mut().find(|m| m["id"].as_str() == Some(id)) {
                            m["content"] = c.clone();
                        }
                    }
                }
                for rid in p["removeIds"].as_array().into_iter().flatten().filter_map(|x| x.as_str()) {
                    msgs.retain(|m| m["id"].as_str() != Some(rid));
                }
            } else if v["type"].is_string() && v.get("content").is_some() {
                // the same id written again replaces the earlier record (its tool calls were filled in)
                match id_of(&v).and_then(|id| msgs.iter().position(|m| m["id"].as_str() == Some(id.as_str()))) {
                    Some(i) => msgs[i] = v,
                    None => msgs.push(v),
                }
            } else {
                absorb_header(&v, &mut sess);
            }
        }
    } else {
        let v: Value = serde_json::from_str(text).ok()?;
        absorb_header(&v, &mut sess);
        msgs = v["messages"].as_array().cloned().unwrap_or_default();
    }
    let mut totals = Usage::default();
    for m in &msgs {
        let role = match m["type"].as_str() {
            Some("user") => "user",
            Some("gemini") | Some("model") => "gemini",
            _ => continue,
        };
        let body = gemini_text(&m["content"]);
        // context Gemini injects as a user turn (session setup, hook output): not typed by the user
        let injected = role == "user" && (body.starts_with("<session_context>") || body.starts_with("<hook_context>"));
        if let Some(ts) = iso_secs(&m["timestamp"]) {
            sess.started = if sess.started == 0 { ts } else { sess.started.min(ts) };
            sess.updated = sess.updated.max(ts);
        }
        // Gemini counts the cached part inside `input`
        let t = &m["tokens"];
        if role == "gemini" && t.is_object() {
            let n = |k: &str| t[k].as_u64().unwrap_or(0);
            let cached = n("cached");
            totals.input += n("input").saturating_sub(cached);
            totals.output += n("output") + n("thoughts");
            totals.cache_read += cached;
            totals.messages += 1;
        }
        if injected {
            continue;
        }
        if sess.title.is_empty() && role == "user" && !body.trim().is_empty() {
            sess.title = body.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(120).collect();
        }
        if !body.trim().is_empty() {
            sess.messages.push((role, body));
        }
    }
    sess.usage = totals;
    if sess.messages.is_empty() {
        return None;
    }
    Some(sess)
}

/// Reads and parses a session file, capped in size; the project hash falls back to the folder above `chats`.
fn read_gemini(p: &Path) -> Option<GeminiSession> {
    if std::fs::metadata(p).ok()?.len() > GEMINI_MAX_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(p).ok()?;
    let mut s = parse_gemini(&text, p.extension().is_some_and(|x| x == "jsonl"))?;
    if s.session.is_empty() {
        s.session = p.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
    }
    if s.project_hash.is_empty() {
        s.project_hash =
            p.parent().and_then(|c| c.parent()).and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    }
    if s.updated == 0 {
        s.updated = mtime(p);
    }
    Some(s)
}

fn search_gemini(p: &Path, needle_lc: &str, per_file: usize, out: &mut Vec<Hit>) {
    let Some(s) = read_gemini(p) else { return };
    let mut found = 0;
    for (_, body) in &s.messages {
        if let Some(sn) = snippet(body, needle_lc) {
            out.push(Hit {
                agent: "gemini",
                session: s.session.clone(),
                cwd: String::new(),
                ts: s.updated,
                snippet: sn,
                project: Some(s.project_hash.clone()),
            });
            found += 1;
            if found >= per_file {
                return;
            }
        }
    }
}

#[derive(Serialize, Clone, Default, Debug, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub messages: u64,
    /// size of the conversation as of the last reply (what the model read): the "context" Claude's own status line shows
    pub context: u64,
}

/// Running token totals of one conversation, fed line by line so a growing transcript is only read from where it stopped.
#[derive(Default, Clone)]
pub struct UsageAcc {
    pub u: Usage,
    seen: HashSet<String>,
}

impl UsageAcc {
    pub fn feed(&mut self, line: &str) {
        if !line.contains("\"usage\"") {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { return };
        let m = &v["message"];
        let usage = &m["usage"];
        if !usage.is_object() {
            return;
        }
        // a streamed reply is logged in pieces that repeat the same message id: count it once
        if let Some(id) = m["id"].as_str() {
            if !self.seen.insert(id.to_string()) {
                return;
            }
        }
        let n = |k: &str| usage[k].as_u64().unwrap_or(0);
        let u = &mut self.u;
        u.input += n("input_tokens");
        u.output += n("output_tokens");
        u.cache_read += n("cache_read_input_tokens");
        u.cache_write += n("cache_creation_input_tokens");
        u.messages += 1;
        u.context = n("input_tokens") + n("cache_read_input_tokens") + n("cache_creation_input_tokens");
    }
}

pub fn sum_usage(lines: impl Iterator<Item = String>) -> Usage {
    let mut acc = UsageAcc::default();
    for line in lines {
        acc.feed(&line);
    }
    acc.u
}

/// Codex logs its rate limits (`rate_limits.primary` / `secondary`: used_percent, window_minutes, resets_at or
/// resets_in_seconds) with every token count event; the last one in a session file is the current state.
pub fn parse_codex_limits(lines: impl Iterator<Item = String>) -> Option<Value> {
    let mut last = None;
    for line in lines {
        if !line.contains("rate_limits") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        let rl = v["payload"]["rate_limits"].clone();
        let rl = if rl.is_object() { rl } else { v["rate_limits"].clone() };
        if rl.is_object() && (rl["primary"].is_object() || rl["secondary"].is_object()) {
            last = Some(rl);
        }
    }
    last
}

/// The newest session files of Codex, which keeps them under `YYYY/MM/DD/rollout-<time>-<id>.jsonl`: walks the date
/// folders newest first and stops once it has `want` files, instead of listing every conversation ever made.
pub fn newest_codex_files(root: &Path, want: usize) -> Vec<PathBuf> {
    fn newest_first(dir: &Path) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        v.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        v
    }
    let mut out = vec![];
    for year in newest_first(root).into_iter().filter(|p| p.is_dir()) {
        for month in newest_first(&year).into_iter().filter(|p| p.is_dir()) {
            for day in newest_first(&month).into_iter().filter(|p| p.is_dir()) {
                out.extend(newest_first(&day).into_iter().filter(|p| p.extension().is_some_and(|x| x == "jsonl")));
                if out.len() >= want {
                    out.truncate(want);
                    return out;
                }
            }
        }
    }
    // an older layout with the files straight in the root
    out.extend(newest_first(root).into_iter().filter(|p| p.extension().is_some_and(|x| x == "jsonl")));
    out.truncate(want);
    out
}

/// The last `max` bytes of `f` (of length `len`) as text. Cutting at a byte offset can land inside a multibyte
/// character or in the middle of a line: bytes are decoded lossily and, when the cut was not at the start, the
/// partial first line is dropped, so a pt-BR or emoji transcript still yields its totals.
fn read_tail(f: &mut std::fs::File, len: u64, max: u64) -> Option<String> {
    use std::io::{Seek, SeekFrom};
    let start = len.saturating_sub(max);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    f.take(max).read_to_end(&mut bytes).ok()?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 {
        text = match text.find('\n') {
            Some(i) => text.split_off(i + 1),
            None => String::new(),
        };
    }
    Some(text)
}

/// Rate limits from the newest Codex session file that has them (looks at its last 256 KB).
pub fn codex_limits() -> Option<Value> {
    for path in newest_codex_files(&codex_root()?, 5) {
        let Ok(mut f) = std::fs::File::open(&path) else { continue };
        let Ok(len) = f.metadata().map(|m| m.len()) else { continue };
        let Some(text) = read_tail(&mut f, len, TAIL_BYTES) else { continue };
        if let Some(mut rl) = parse_codex_limits(text.lines().map(String::from)) {
            rl["as_of"] = serde_json::json!(mtime(&path));
            return Some(rl);
        }
    }
    None
}

/// per transcript: (length, mtime, bytes already consumed, totals so far)
/// Look-back window of the tail reads.
const TAIL_BYTES: u64 = 256 * 1024;
/// Entries each per-file cache keeps; past it the least recently modified transcripts are dropped.
const CACHE_CAP: usize = 256;

/// Makes room for one more entry: drops the entries with the oldest mtime until the map is under `CACHE_CAP`.
fn evict<V>(map: &mut HashMap<PathBuf, V>, mtime_of: impl Fn(&V) -> u64) {
    while map.len() >= CACHE_CAP {
        let Some(oldest) = map.iter().min_by_key(|(_, v)| mtime_of(v)).map(|(k, _)| k.clone()) else { return };
        map.remove(&oldest);
    }
}

static CACHE: Mutex<Option<HashMap<PathBuf, ClaudeEntry>>> = Mutex::new(None);
type ClaudeEntry = (u64, u64, u64, UsageAcc);

/// Token usage of one Claude conversation: the file named by `session_id`, or the newest one of the project.
pub fn claude_usage(cwd: &str, session_id: Option<&str>) -> Option<Usage> {
    let dir = claude_root()?.join(claude_dir_name(cwd));
    let path = match session_id {
        Some(id) if !id.contains(['/', '\\', '.']) => dir.join(format!("{id}.jsonl")),
        _ => {
            let mut v = vec![];
            jsonl_files(&dir, 0, &mut v);
            v.into_iter().max_by_key(|p| mtime(p))?
        }
    };
    file_usage(path)
}

/// Totals of one Claude transcript, kept between calls so a growing file is only read from where it stopped.
/// The cache lock is held only to take and put back the entry, never while reading the file.
fn file_usage(path: PathBuf) -> Option<Usage> {
    let meta = std::fs::metadata(&path).ok()?;
    let (len, mt) = (meta.len(), mtime(&path));
    let prev = CACHE.lock_recover().get_or_insert_with(HashMap::new).get(&path).cloned();
    if let Some((l, m, _, acc)) = &prev {
        if *l == len && *m == mt {
            return Some(acc.u.clone());
        }
    }
    // the file only grows: continue after the last complete line already read; a shorter file starts over
    let (mut off, mut acc) = match prev {
        Some((l, _, off, acc)) if len >= l => (off, acc),
        _ => (0, UsageAcc::default()),
    };
    let mut f = std::fs::File::open(&path).ok()?;
    std::io::Seek::seek(&mut f, std::io::SeekFrom::Start(off)).ok()?;
    let mut r = BufReader::new(f);
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = r.read_until(b'\n', &mut line).ok()?;
        // a last line without its newline may still be being written: leave it for the next read
        if n == 0 || !line.ends_with(b"\n") {
            break;
        }
        off += n as u64;
        acc.feed(&String::from_utf8_lossy(&line));
    }
    let u = acc.u.clone();
    let mut cache = CACHE.lock_recover();
    let map = cache.get_or_insert_with(HashMap::new);
    if !map.contains_key(&path) {
        evict(map, |e| e.1);
    }
    map.insert(path, (len, mt, off, acc));
    Some(u)
}

/// What one worktree has spent, summed over every Claude, Codex and Gemini conversation that ran in it.
#[derive(Serialize, Clone, Default, Debug, PartialEq)]
pub struct WorktreeUsage {
    pub worktree: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub claude_sessions: u64,
    pub codex_sessions: u64,
    pub gemini_sessions: u64,
}

impl WorktreeUsage {
    fn add(&mut self, u: &Usage, agent: &str) {
        self.input += u.input;
        self.output += u.output;
        self.cache_read += u.cache_read;
        self.cache_write += u.cache_write;
        match agent {
            "claude" => self.claude_sessions += 1,
            "gemini" => self.gemini_sessions += 1,
            _ => self.codex_sessions += 1,
        }
    }
}

/// A Codex session file: the folder it ran in (`session_meta`, first line) and its token totals. The
/// `token_count` events carry running totals, so the last one is the whole session.
pub fn codex_totals(lines: impl Iterator<Item = String>) -> (Option<String>, Usage) {
    let (mut cwd, mut u) = (None, Usage::default());
    for line in lines {
        if cwd.is_none() && line.contains("\"cwd\"") {
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                cwd = v["payload"]["cwd"].as_str().map(String::from);
            }
        }
        if !line.contains("total_token_usage") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        let t = &v["payload"]["info"]["total_token_usage"];
        if !t.is_object() {
            continue;
        }
        let n = |k: &str| t[k].as_u64().unwrap_or(0);
        // Codex counts the cached part inside input_tokens
        let cached = n("cached_input_tokens");
        u = Usage {
            input: n("input_tokens").saturating_sub(cached),
            output: n("output_tokens"),
            cache_read: cached,
            cache_write: 0,
            messages: 1,
            context: 0,
        };
    }
    (cwd, u)
}

/// per Codex file: (length, mtime, folder it ran in, totals)
static CODEX_CACHE: Mutex<Option<HashMap<PathBuf, CodexEntry>>> = Mutex::new(None);
type CodexEntry = (u64, u64, Option<String>, Usage);

fn codex_file(path: &Path) -> Option<(Option<String>, Usage)> {
    let meta = std::fs::metadata(path).ok()?;
    let (len, mt) = (meta.len(), mtime(path));
    if let Some((l, m, cwd, u)) = CODEX_CACHE.lock_recover().get_or_insert_with(HashMap::new).get(path) {
        if *l == len && *m == mt {
            return Some((cwd.clone(), u.clone()));
        }
    }
    let mut f = std::fs::File::open(path).ok()?;
    // the folder is on the first line, the running totals at the end: read those two ends
    let mut head = Vec::new();
    BufReader::new(&f).read_until(b'\n', &mut head).ok()?;
    let head = String::from_utf8_lossy(&head).into_owned();
    let tail = read_tail(&mut f, len, TAIL_BYTES)?;
    let (cwd, u) = codex_totals(std::iter::once(head).chain(tail.lines().map(String::from)));
    let mut cache = CODEX_CACHE.lock_recover();
    let map = cache.get_or_insert_with(HashMap::new);
    if !map.contains_key(path) {
        evict(map, |e| e.1);
    }
    map.insert(path.to_path_buf(), (len, mt, cwd.clone(), u.clone()));
    Some((cwd, u))
}

/// Codex rollout listing, kept while the newest date folders have not changed (`YYYY/MM/DD` sorts by name, and new
/// rollouts only appear in the newest day or open a new one, which touches the month).
static CODEX_LIST: Mutex<Option<(PathBuf, u64, Vec<PathBuf>)>> = Mutex::new(None);

/// Cheap change marker of a date-tree: the mtimes along the chain of the newest folder at each level.
fn newest_chain_stamp(root: &Path, depth: usize) -> u64 {
    let (mut dir, mut stamp) = (root.to_path_buf(), 0u64);
    for _ in 0..=depth {
        stamp = stamp.wrapping_mul(31).wrapping_add(mtime(&dir));
        let newest = std::fs::read_dir(&dir).into_iter().flatten().flatten().filter(|e| e.path().is_dir()).max_by_key(|e| e.file_name());
        match newest {
            Some(e) => dir = e.path(),
            None => break,
        }
    }
    stamp
}

fn codex_listing(root: &Path) -> Vec<PathBuf> {
    let stamp = newest_chain_stamp(root, 4);
    if let Some((r, s, files)) = CODEX_LIST.lock_recover().as_ref() {
        if r == root && *s == stamp {
            return files.clone();
        }
    }
    let mut files = vec![];
    jsonl_files(root, 4, &mut files);
    *CODEX_LIST.lock_recover() = Some((root.to_path_buf(), stamp, files.clone()));
    files
}

/// Token totals per worktree, in the order given. Claude's folder per worktree is read directly; Codex
/// files are matched to a worktree by the folder they started in.
pub fn worktree_usage(worktrees: &[String]) -> Vec<WorktreeUsage> {
    let mut out: Vec<WorktreeUsage> = worktrees.iter().map(|w| WorktreeUsage { worktree: w.clone(), ..Default::default() }).collect();
    if let Some(root) = claude_root() {
        for w in &mut out {
            let mut files = vec![];
            jsonl_files(&root.join(claude_dir_name(&w.worktree)), 0, &mut files);
            for f in files {
                if let Some(u) = file_usage(f).filter(|u| u.messages > 0) {
                    w.add(&u, "claude");
                }
            }
        }
    }
    if let Some(root) = codex_root() {
        let files = codex_listing(&root);
        let sessions: Vec<(String, Usage)> = files.iter().filter_map(|f| codex_file(f)).filter_map(|(cwd, u)| Some((cwd?, u))).collect();
        add_codex(&mut out, &sessions);
    }
    if let Some(root) = gemini_root() {
        let registry = gemini_registry(&root);
        for w in &mut out {
            for dir in gemini_project_dirs(&root, &registry, &w.worktree) {
                for f in gemini_files_of(&dir) {
                    if let Some(u) = read_gemini(&f).map(|s| s.usage).filter(|u| u.messages > 0) {
                        w.add(&u, "gemini");
                    }
                }
            }
        }
    }
    out
}

/// Adds each Codex session (folder, totals) to the worktree it ran in, or in one of its subfolders.
fn add_codex(out: &mut [WorktreeUsage], sessions: &[(String, Usage)]) {
    for (cwd, u) in sessions {
        // the longest match wins, so a worktree nested in another one is not counted twice
        let best = out.iter().enumerate().filter(|(_, w)| axelstra_core::paths::is_within(cwd, &w.worktree));
        if let Some((i, _)) = best.max_by_key(|(_, w)| w.worktree.len()) {
            out[i].add(u, "codex");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_dir_name_matches_claudes_scheme() {
        assert_eq!(claude_dir_name(r"C:\Users\franc\Desktop\Projetos\axelstra"), "C--Users-franc-Desktop-Projetos-axelstra");
        assert_eq!(claude_dir_name("/home/a b/x.y"), "-home-a-b-x-y");
    }

    #[test]
    fn codex_files_are_found_under_the_date_folders_newest_first() {
        let root = std::env::temp_dir().join(format!("axl-codex-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (d, f) in [("2026/10/19", "rollout-a.jsonl"), ("2026/10/20", "rollout-b.jsonl"), ("2026/09/30", "rollout-c.jsonl")] {
            std::fs::create_dir_all(root.join(d)).unwrap();
            std::fs::write(root.join(d).join(f), "{}").unwrap();
        }
        let names: Vec<String> =
            newest_codex_files(&root, 2).iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["rollout-b.jsonl", "rollout-a.jsonl"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn codex_limits_take_the_last_event() {
        let a = r#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":3.0,"window_minutes":300,"resets_at":1000},"secondary":{"used_percent":10.0,"window_minutes":10080,"resets_at":2000}}}}"#;
        let b = r#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":7.5,"window_minutes":300,"resets_at":1500},"secondary":{"used_percent":11.0,"window_minutes":10080,"resets_at":2000}}}}"#;
        let r = parse_codex_limits([a, "junk", b, r#"{"type":"user"}"#].into_iter().map(String::from)).unwrap();
        assert_eq!(r["primary"]["used_percent"], 7.5);
        assert!(parse_codex_limits(std::iter::once("{}".to_string())).is_none());
    }

    #[test]
    fn usage_counts_a_repeated_message_once() {
        let a = r#"{"type":"assistant","message":{"id":"m1","usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":100,"cache_creation_input_tokens":7}}}"#;
        let b = r#"{"type":"assistant","message":{"id":"m1","usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":100,"cache_creation_input_tokens":7}}}"#;
        let c = r#"{"type":"assistant","message":{"id":"m2","usage":{"input_tokens":1,"output_tokens":2}}}"#;
        let u = sum_usage([a, b, c, "not json", r#"{"type":"user"}"#].into_iter().map(String::from));
        assert_eq!(u, Usage { input: 11, output: 7, cache_read: 100, cache_write: 7, messages: 2, context: 1 });
    }

    #[test]
    fn search_finds_text_in_both_shapes_and_skips_ids() {
        let dir = std::env::temp_dir().join(format!("axelstra-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("abc-123.jsonl");
        let l1 = r#"{"cwd":"C:\\proj","sessionId":"needle-id","message":{"role":"user","content":"please fix the Flux Capacitor bug"}}"#;
        let l2 = r#"{"message":{"content":[{"type":"text","text":"done, the flux capacitor works"}]}}"#;
        std::fs::write(&f, format!("{l1}\n{l2}\n")).unwrap();
        let mut hits = vec![];
        search_file(&f, "claude", "flux capacitor", 5, &mut hits);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].session, "abc-123");
        assert_eq!(hits[0].cwd, r"C:\proj");
        assert!(hits[0].snippet.contains("Flux Capacitor"));
        let mut none = vec![];
        search_file(&f, "claude", "needle-id", 5, &mut none);
        assert!(none.is_empty(), "ids are not searchable text");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn codex_session_totals_use_the_last_count_and_the_first_folder() {
        let meta = r#"{"type":"session_meta","payload":{"id":"x","cwd":"C:\\wt\\a"}}"#;
        let early = r#"{"type":"event_msg","payload":{"type":"token_count","info":null}}"#;
        let one = r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":60,"output_tokens":10,"total_tokens":110}}}}"#;
        let two = r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"cached_input_tokens":200,"output_tokens":40,"total_tokens":340}}}}"#;
        let (cwd, u) = codex_totals([meta, early, one, "junk", two].into_iter().map(String::from));
        assert_eq!(cwd.as_deref(), Some(r"C:\wt\a"));
        assert_eq!(u, Usage { input: 100, output: 40, cache_read: 200, cache_write: 0, messages: 1, context: 0 });
    }

    #[test]
    fn worktree_totals_add_up_both_agents_and_match_paths_loosely() {
        let mut out = vec![
            WorktreeUsage { worktree: r"C:\wt\a".into(), ..Default::default() },
            WorktreeUsage { worktree: r"C:\wt\a\nested".into(), ..Default::default() },
            WorktreeUsage { worktree: r"C:\wt\b".into(), ..Default::default() },
        ];
        let claude = Usage { input: 5, output: 6, cache_read: 7, cache_write: 8, messages: 2, context: 0 };
        out[0].add(&claude, "claude");
        let codex = Usage { input: 1, output: 2, cache_read: 3, cache_write: 0, messages: 1, context: 0 };
        add_codex(
            &mut out,
            &[
                (r"\\?\c:\WT\a/".into(), codex.clone()),
                (r"C:\wt\a\nested\sub".into(), codex.clone()),
                (r"C:\wt\ab".into(), codex.clone()),
                (r"D:\elsewhere".into(), codex),
            ],
        );
        assert_eq!(
            out[0],
            WorktreeUsage {
                worktree: r"C:\wt\a".into(),
                input: 6,
                output: 8,
                cache_read: 10,
                cache_write: 8,
                claude_sessions: 1,
                codex_sessions: 1,
                gemini_sessions: 0
            }
        );
        assert_eq!((out[1].input, out[1].codex_sessions), (1, 1));
        assert_eq!(out[2].codex_sessions, 0, "a sibling folder with the same prefix is not this worktree");
    }

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn gemini_hashes_cover_slash_and_trailing_variants() {
        let v = gemini_hashes(r"C:\wt\a\");
        assert!(v.contains(&sha256_hex(br"C:\wt\a")));
        assert!(v.contains(&sha256_hex(b"C:/wt/a")));
        assert_eq!(gemini_hashes("/home/x").len(), 2, "plain and backslash form");
    }

    const GEMINI_JSON: &str = r#"{
      "sessionId": "7d9c1c0e-2b0a-4d7e-9a53-0f6f5d0f1a11",
      "projectHash": "abc123",
      "startTime": "2026-10-20T10:00:00.000Z",
      "lastUpdated": "2026-10-20T10:05:00.000Z",
      "messages": [
        {"id":"1","timestamp":"2026-10-20T10:00:01.000Z","type":"user","content":"Corrige o   bug do Flux Capacitor\nagora"},
        {"id":"2","timestamp":"2026-10-20T10:00:09.000Z","type":"gemini","content":"Feito: o flux capacitor funciona.","model":"gemini-2.5-pro",
         "tokens":{"input":1000,"output":50,"cached":400,"thoughts":20,"tool":0,"total":1070}},
        {"id":"3","timestamp":"2026-10-20T10:01:00.000Z","type":"info","content":"Update available"},
        {"id":"4","timestamp":"2026-10-20T10:02:00.000Z","type":"user","content":[{"text":"outra pergunta"},{"text":"em partes"}]},
        {"id":"5","timestamp":"2026-10-20T10:05:00.000Z","type":"gemini","content":"resposta","tokens":{"input":2000,"output":10,"cached":1500,"thoughts":0,"tool":0,"total":2010}}
      ]
    }"#;

    #[test]
    fn gemini_session_is_parsed_with_title_times_and_tokens() {
        let s = parse_gemini(GEMINI_JSON, false).unwrap();
        assert_eq!(s.session, "7d9c1c0e-2b0a-4d7e-9a53-0f6f5d0f1a11");
        assert_eq!(s.project_hash, "abc123");
        assert_eq!(s.title, "Corrige o bug do Flux Capacitor agora");
        assert_eq!(s.updated - s.started, 300);
        // user and model messages only: the info line is dropped, array content is joined
        assert_eq!(s.messages.len(), 4);
        assert_eq!(s.messages[2].1, "outra pergunta\nem partes");
        assert_eq!(s.usage, Usage { input: 600 + 500, output: 70 + 10, cache_read: 1900, cache_write: 0, messages: 2, context: 0 });
    }

    #[test]
    fn gemini_jsonl_and_broken_input_do_not_panic() {
        let lines = "{\"sessionId\":\"s1\",\"projectHash\":\"h\",\"startTime\":\"2026-10-20T10:00:00Z\"}\nnot json\n{\"id\":\"a\",\"timestamp\":\"2026-10-20T10:00:05Z\",\"type\":\"user\",\"content\":\"olá\"}\n";
        let s = parse_gemini(lines, true).unwrap();
        assert_eq!((s.session.as_str(), s.title.as_str(), s.messages.len()), ("s1", "olá", 1));
        assert!(parse_gemini("{ truncated", false).is_none());
        assert!(parse_gemini("[]", false).is_none());
        assert!(parse_gemini("", true).is_none());
        assert!(parse_gemini(r#"{"messages":[{"type":"user","content":null},{"type":"gemini"},7]}"#, false).is_none());
    }

    /// A conversation recorded by gemini-cli 0.65.0 (a shell tool call, then `--resume latest`), paths anonymised.
    const GEMINI_REAL: &str = include_str!("../tests/fixtures/gemini-0.65.0/session-tool-call.jsonl");

    #[test]
    fn a_real_gemini_0_65_log_counts_each_message_once_and_skips_the_injected_context() {
        let s = parse_gemini(GEMINI_REAL, true).unwrap();
        assert_eq!(s.session, "8831b7c4-c371-4ac4-8710-9d19aaa3ff82");
        assert_eq!(s.project_hash, "a963c4182a8f90c0628a7e01edfd4a4b662ab50e93ae4ebd872814ec444a9083");
        // the setup block is not the title, and its directory tree is not searchable
        assert_eq!(s.title, "RUNTOOL please");
        assert_eq!(s.messages, vec![("user", "RUNTOOL please".to_string()), ("gemini", "stub answer ok".to_string())]);
        // the tool-call message is in the file twice under one id: 200+5 once, plus 120 (30 cached) + 7 + 3 thoughts
        assert_eq!(s.usage, Usage { input: 200 + 90, output: 5 + 7 + 3, cache_read: 30, cache_write: 0, messages: 2, context: 0 });
        assert_eq!(s.updated, iso_secs(&serde_json::json!("2026-10-24T15:08:15.463Z")).unwrap());
    }

    #[test]
    fn gemini_rewind_and_removed_messages_leave_the_conversation() {
        let rec = |id: &str, t: &str, c: &str| format!("{{\"id\":\"{id}\",\"timestamp\":\"2026-10-20T10:00:00Z\",\"type\":\"{t}\",\"content\":\"{c}\"}}\n");
        let head = "{\"sessionId\":\"s\",\"projectHash\":\"h\",\"startTime\":\"2026-10-20T10:00:00Z\"}\n";
        let base = format!("{head}{}{}{}{}", rec("1", "user", "um"), rec("2", "gemini", "dois"), rec("3", "user", "três"), rec("4", "gemini", "quatro"));
        let rewound = format!("{base}{{\"$rewindTo\":\"3\"}}\n");
        assert_eq!(parse_gemini(&rewound, true).unwrap().messages.len(), 2);
        let removed = format!("{base}{{\"$patch\":{{\"removeIds\":[\"2\",\"4\"]}}}}\n");
        assert_eq!(parse_gemini(&removed, true).unwrap().messages.iter().map(|m| m.1.as_str()).collect::<Vec<_>>(), ["um", "três"]);
        let edited = format!("{base}{{\"$patch\":{{\"id\":\"1\",\"content\":\"editado\"}}}}\n");
        assert_eq!(parse_gemini(&edited, true).unwrap().messages[0].1, "editado");
    }

    #[test]
    fn gemini_project_folders_come_from_the_registry_or_the_old_hash_name() {
        let root = std::env::temp_dir().join(format!("axl-gemreg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let tmp = root.join(".gemini").join("tmp");
        let chats = tmp.join("gem").join("chats");
        std::fs::create_dir_all(&chats).unwrap();
        std::fs::write(chats.join("session-2026-10-24T15-07-8831b7c4.jsonl"), GEMINI_REAL).unwrap();
        // Gemini lower-cases a Windows root in the registry
        std::fs::write(root.join(".gemini").join("projects.json"), r#"{"projects":{"c:\\wt\\gem":"gem","c:\\wt\\other":"other"}}"#).unwrap();
        let reg = gemini_registry(&tmp);
        assert_eq!(reg.len(), 2);
        let dirs = gemini_project_dirs(&tmp, &reg, r"C:\wt\gem\");
        assert_eq!(dirs[0], tmp.join("gem"));
        assert!(dirs.contains(&tmp.join(sha256_hex(br"C:\wt\gem"))), "the pre-slug folder name is still tried");
        let used: u64 = dirs.iter().flat_map(|d| gemini_files_of(d)).filter_map(|f| read_gemini(&f)).map(|s| s.usage.messages).sum();
        assert_eq!(used, 2);
        assert!(gemini_project_dirs(&tmp, &reg, r"C:\wt\nope").iter().all(|d| gemini_files_of(d).is_empty()));
        assert!(same_root("/home/Me/x", "/home/Me/x/") && !same_root("/home/Me/x", "/home/me/x"));
        assert!(gemini_registry(&root).is_empty(), "no projects.json next to a tmp folder that is not ~/.gemini/tmp");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn gemini_search_and_worktree_usage_read_the_chats_folder() {
        let root = std::env::temp_dir().join(format!("axl-gem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let wt = r"C:\wt\gem";
        let chats = root.join(sha256_hex(wt.as_bytes())).join("chats");
        std::fs::create_dir_all(&chats).unwrap();
        std::fs::write(chats.join("session-2026-10-20T10-00-7d9c1c0e.json"), GEMINI_JSON).unwrap();
        std::fs::write(chats.join("session-broken.json"), "{ nope").unwrap();
        let files = gemini_files(&root);
        assert_eq!(files.len(), 2);
        let mut hits = vec![];
        for f in &files {
            search_gemini(f, "flux capacitor", 5, &mut hits);
        }
        assert_eq!(hits.len(), 2, "user prompt and model reply");
        assert_eq!(hits[0].agent, "gemini");
        assert_eq!(hits[0].session, "7d9c1c0e-2b0a-4d7e-9a53-0f6f5d0f1a11");
        assert_eq!(hits[0].project.as_deref(), Some("abc123"));
        let mine = gemini_files_of(&root.join(&gemini_hashes(wt)[0]));
        assert_eq!(mine.iter().filter_map(|f| read_gemini(f)).map(|s| s.usage.messages).sum::<u64>(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn file_usage_reads_a_transcript_and_picks_up_growth() {
        let f = std::env::temp_dir().join(format!("axl-usage-{}.jsonl", std::process::id()));
        let a = r#"{"message":{"id":"m1","usage":{"input_tokens":10,"output_tokens":5}}}"#;
        std::fs::write(&f, format!("{a}\n")).unwrap();
        assert_eq!(file_usage(f.clone()).unwrap().output, 5);
        let b = r#"{"message":{"id":"m2","usage":{"input_tokens":1,"output_tokens":2}}}"#;
        std::fs::write(&f, format!("{a}\n{b}\n")).unwrap();
        let u = file_usage(f.clone()).unwrap();
        assert_eq!((u.input, u.output, u.messages), (11, 7, 2));
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn a_tail_cut_inside_a_multibyte_char_still_yields_whole_lines() {
        let dir = std::env::temp_dir().join(format!("axl-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("r.jsonl");
        let first = r#"{"type":"session_meta","payload":{"cwd":"/w/ç"}}"#;
        let last =
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":50,"output_tokens":9}}}}"#;
        // enough 2-byte characters that the 256 KiB cut lands inside one, in both parities
        for pad in ["ç", "xç"] {
            let filler = format!(
                "{{\"t\":\"{}\"}}
",
                pad.repeat(200_000)
            );
            std::fs::write(
                &f,
                format!(
                    "{first}
{filler}{last}
"
                ),
            )
            .unwrap();
            let (cwd, u) = codex_file(&f).unwrap();
            assert_eq!(cwd.as_deref(), Some("/w/ç"));
            assert_eq!((u.input, u.output), (50, 9), "pad {pad}");
            let mut h = std::fs::File::open(&f).unwrap();
            let len = h.metadata().unwrap().len();
            let tail = read_tail(&mut h, len, TAIL_BYTES).unwrap();
            assert_eq!(tail.trim_end(), last);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn caches_stay_bounded_dropping_the_oldest() {
        let mut m: HashMap<PathBuf, u64> = HashMap::new();
        for i in 0..(CACHE_CAP as u64 + 40) {
            if !m.contains_key(&PathBuf::from(format!("f{i}"))) {
                evict(&mut m, |v| *v);
            }
            m.insert(PathBuf::from(format!("f{i}")), i);
        }
        assert_eq!(m.len(), CACHE_CAP);
        assert!(!m.contains_key(&PathBuf::from("f0")) && m.contains_key(&PathBuf::from(format!("f{}", CACHE_CAP + 39))));
    }

    #[test]
    fn the_byte_prefilter_ignores_ascii_case_and_never_misses_other_scripts() {
        assert!(may_contain(b"{\"text\":\"Hello World\"}", "hello"));
        assert!(!may_contain(b"{\"text\":\"Help\"}", "hello"));
        assert!(may_contain("{\"text\":\"AÇÃO\"}".as_bytes(), "ação"));
    }
}
