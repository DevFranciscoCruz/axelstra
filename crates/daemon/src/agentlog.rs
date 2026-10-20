//! Reads the transcripts the agent CLIs already keep on disk: full-text search across past
//! conversations and token usage of one session. Nothing is indexed or cached on disk; files are
//! scanned on demand, newest first, so an idle daemon costs nothing.
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
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
                    if matches!(k.as_str(), "id" | "uuid" | "parentUuid" | "sessionId" | "signature" | "type" | "role" | "model" | "cwd" | "timestamp" | "version" | "usage") {
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

fn search_file(p: &Path, agent: &'static str, needle_lc: &str, per_file: usize, out: &mut Vec<Hit>) {
    let Ok(f) = std::fs::File::open(p) else { return };
    let session = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ts = mtime(p);
    let mut cwd = String::new();
    let mut found = 0;
    for line in BufReader::new(f).lines().map_while(Result::ok) {
        if cwd.is_empty() {
            if let Some(i) = line.find("\"cwd\":\"") {
                let rest = &line[i + 7..];
                if let Some(j) = rest.find('"') {
                    cwd = rest[..j].replace("\\\\", "\\");
                }
            }
        }
        // cheap pre-filter on the raw line before parsing it
        if !line.to_lowercase().contains(needle_lc) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        if let Some(sn) = snippet(&line_text(&v), needle_lc) {
            out.push(Hit { agent, session: session.clone(), cwd: cwd.clone(), ts, snippet: sn });
            found += 1;
            if found >= per_file {
                return;
            }
        }
    }
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
    files.sort_by_key(|(p, _)| std::cmp::Reverse(mtime(p)));
    let mut hits = vec![];
    // the newest 400 conversations are plenty; older ones are a click away in the CLI's own picker
    for (p, agent) in files.into_iter().take(400) {
        search_file(&p, agent, &needle, 2, &mut hits);
        if hits.len() >= max {
            hits.truncate(max);
            break;
        }
    }
    hits
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

pub fn sum_usage(lines: impl Iterator<Item = String>) -> Usage {
    let mut u = Usage::default();
    let mut seen: HashSet<String> = HashSet::new();
    for line in lines {
        if !line.contains("\"usage\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        let m = &v["message"];
        let usage = &m["usage"];
        if !usage.is_object() {
            continue;
        }
        // a streamed reply is logged in pieces that repeat the same message id: count it once
        if let Some(id) = m["id"].as_str() {
            if !seen.insert(id.to_string()) {
                continue;
            }
        }
        let n = |k: &str| usage[k].as_u64().unwrap_or(0);
        u.input += n("input_tokens");
        u.output += n("output_tokens");
        u.cache_read += n("cache_read_input_tokens");
        u.cache_write += n("cache_creation_input_tokens");
        u.messages += 1;
        u.context = n("input_tokens") + n("cache_read_input_tokens") + n("cache_creation_input_tokens");
    }
    u
}

static CACHE: Mutex<Option<HashMap<PathBuf, (u64, u64, Usage)>>> = Mutex::new(None);

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
    let meta = std::fs::metadata(&path).ok()?;
    let (len, mt) = (meta.len(), mtime(&path));
    let mut cache = CACHE.lock().unwrap();
    let map = cache.get_or_insert_with(HashMap::new);
    if let Some((l, m, u)) = map.get(&path) {
        if *l == len && *m == mt {
            return Some(u.clone());
        }
    }
    let f = std::fs::File::open(&path).ok()?;
    let u = sum_usage(BufReader::new(f).lines().map_while(Result::ok));
    map.insert(path, (len, mt, u.clone()));
    Some(u)
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
}
