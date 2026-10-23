//! Reads the transcripts the agent CLIs already keep on disk: full-text search across past
//! conversations and token usage of one session. Nothing is indexed or cached on disk; files are
//! scanned on demand, newest first, so an idle daemon costs nothing.
use crate::store::LockRecover;
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

/// Rate limits from the newest Codex session file that has them (looks at its last 256 KB).
pub fn codex_limits() -> Option<Value> {
    use std::io::{Read, Seek, SeekFrom};
    for path in newest_codex_files(&codex_root()?, 5) {
        let Ok(mut f) = std::fs::File::open(&path) else { continue };
        let Ok(len) = f.metadata().map(|m| m.len()) else { continue };
        if f.seek(SeekFrom::Start(len.saturating_sub(256 * 1024))).is_err() {
            continue;
        }
        let mut text = String::new();
        let _ = f.read_to_string(&mut text);
        if let Some(mut rl) = parse_codex_limits(text.lines().map(String::from)) {
            rl["as_of"] = serde_json::json!(mtime(&path));
            return Some(rl);
        }
    }
    None
}

/// per transcript: (length, mtime, bytes already consumed, totals so far)
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
fn file_usage(path: PathBuf) -> Option<Usage> {
    let meta = std::fs::metadata(&path).ok()?;
    let (len, mt) = (meta.len(), mtime(&path));
    let mut cache = CACHE.lock_recover();
    let map = cache.get_or_insert_with(HashMap::new);
    let mut prev = map.remove(&path);
    if let Some((l, m, _, acc)) = &prev {
        if *l == len && *m == mt {
            let u = acc.u.clone();
            map.insert(path, prev.take().unwrap());
            return Some(u);
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
    let mut line = String::new();
    loop {
        line.clear();
        let n = std::io::BufRead::read_line(&mut r, &mut line).ok()?;
        // a last line without its newline may still be being written: leave it for the next read
        if n == 0 || !line.ends_with('\n') {
            break;
        }
        off += n as u64;
        acc.feed(&line);
    }
    let u = acc.u.clone();
    map.insert(path, (len, mt, off, acc));
    Some(u)
}

/// What one worktree has spent, summed over every Claude and Codex conversation that ran in it.
#[derive(Serialize, Clone, Default, Debug, PartialEq)]
pub struct WorktreeUsage {
    pub worktree: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub claude_sessions: u64,
    pub codex_sessions: u64,
}

impl WorktreeUsage {
    fn add(&mut self, u: &Usage, claude: bool) {
        self.input += u.input;
        self.output += u.output;
        self.cache_read += u.cache_read;
        self.cache_write += u.cache_write;
        if claude {
            self.claude_sessions += 1;
        } else {
            self.codex_sessions += 1;
        }
    }
}

/// Paths from the UI and from the logs differ in case, slashes and a `\\?\` prefix on Windows.
fn norm_path(p: &str) -> String {
    p.trim_start_matches(r"\\?\").replace('\\', "/").trim_end_matches('/').to_lowercase()
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
    use std::io::{Read, Seek, SeekFrom};
    let meta = std::fs::metadata(path).ok()?;
    let (len, mt) = (meta.len(), mtime(path));
    let mut cache = CODEX_CACHE.lock_recover();
    let map = cache.get_or_insert_with(HashMap::new);
    if let Some((l, m, cwd, u)) = map.get(path) {
        if *l == len && *m == mt {
            return Some((cwd.clone(), u.clone()));
        }
    }
    let mut f = std::fs::File::open(path).ok()?;
    // the folder is on the first line, the running totals at the end: read those two ends
    let mut head = String::new();
    BufReader::new(&f).read_line(&mut head).ok()?;
    f.seek(SeekFrom::Start(len.saturating_sub(256 * 1024))).ok()?;
    let mut tail = String::new();
    let _ = f.read_to_string(&mut tail);
    let (cwd, u) = codex_totals(std::iter::once(head).chain(tail.lines().map(String::from)));
    map.insert(path.to_path_buf(), (len, mt, cwd.clone(), u.clone()));
    Some((cwd, u))
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
                    w.add(&u, true);
                }
            }
        }
    }
    if let Some(root) = codex_root() {
        let mut files = vec![];
        jsonl_files(&root, 4, &mut files);
        let sessions: Vec<(String, Usage)> = files.iter().filter_map(|f| codex_file(f)).filter_map(|(cwd, u)| Some((cwd?, u))).collect();
        add_codex(&mut out, &sessions);
    }
    out
}

/// Adds each Codex session (folder, totals) to the worktree it ran in, or in one of its subfolders.
fn add_codex(out: &mut [WorktreeUsage], sessions: &[(String, Usage)]) {
    for (cwd, u) in sessions {
        let c = norm_path(cwd);
        // the longest match wins, so a worktree nested in another one is not counted twice
        let best = out.iter().enumerate().filter(|(_, w)| {
            let n = norm_path(&w.worktree);
            c == n || c.starts_with(&format!("{n}/"))
        });
        if let Some((i, _)) = best.max_by_key(|(_, w)| w.worktree.len()) {
            out[i].add(u, false);
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
        out[0].add(&claude, true);
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
                codex_sessions: 1
            }
        );
        assert_eq!((out[1].input, out[1].codex_sessions), (1, 1));
        assert_eq!(out[2].codex_sessions, 0, "a sibling folder with the same prefix is not this worktree");
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
}
