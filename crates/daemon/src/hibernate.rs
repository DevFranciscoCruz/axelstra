//! Session hibernation, the parts that need no registry: the setting, the one deadline per session, the command that
//! brings an agent back, and the bookkeeping (`Hib`). The methods and the threads are in `server/hibernate.rs`.
//!
//! A hibernated agent has no process and no `Session`, but keeps its id, its label, its meta, its saved spec and its
//! bus state (inbox, parent, worker flag), so everything that names the id still finds it. Waking spawns a new process
//! under the same id with the agent's own resume command.

use crate::store::LockRecover;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Default quiet time (minutes) before a waiting agent is hibernated.
pub const DEFAULT_IDLE_MINUTES: u64 = 30;
pub const MAX_IDLE_MINUTES: u64 = 24 * 60;
/// A session that produced output, got input or an event in the last seconds is at the start of (or in) a turn.
pub const QUIET_MS: u64 = 20_000;

/// `QUIET_MS`, or what `AXELSTRA_HIBERNATE_QUIET_MS` says (tests shorten it).
pub fn quiet_ms() -> u64 {
    std::env::var("AXELSTRA_HIBERNATE_QUIET_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(QUIET_MS)
}
/// Exit code of the wake line when the agent could not resume.
pub const RESUME_FAILED_EXIT: i32 = 97;
/// The longest a woken session is given to settle (print its banner and go quiet) before its inbox notice is typed.
pub const SETTLE_CAP: Duration = Duration::from_secs(90);
/// Output must be quiet this long after a wake before anything is typed into the agent.
pub const SETTLE_QUIET_MS: u64 = 2500;

/// The `hibernate` setting: `{enabled, idle_minutes}`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Policy {
    pub enabled: bool,
    pub idle_ms: u64,
}

pub fn policy(settings: &Value) -> Policy {
    let h = &settings["hibernate"];
    let minutes = h["idle_minutes"].as_u64().map_or(DEFAULT_IDLE_MINUTES, |m| m.clamp(1, MAX_IDLE_MINUTES));
    // `AXELSTRA_HIBERNATE_IDLE_MS` replaces the minutes (tests)
    let idle_ms = std::env::var("AXELSTRA_HIBERNATE_IDLE_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(minutes * 60_000);
    Policy { enabled: h["enabled"].as_bool().unwrap_or(true), idle_ms }
}

/// The value `settings.get` reports.
pub fn policy_json(settings: &Value) -> Value {
    let p = policy(settings);
    json!({"enabled": p.enabled, "idle_minutes": p.idle_ms / 60_000})
}

/// Checks a `hibernate` value from `settings.set` and merges it into the saved one; `None` when it is malformed.
pub fn validate(saved: &Value, new: &Value) -> Option<Value> {
    let obj = new.as_object()?;
    let mut out = json!({"enabled": policy(saved).enabled, "idle_minutes": policy(saved).idle_ms / 60_000});
    for (k, v) in obj {
        match k.as_str() {
            "enabled" => out["enabled"] = json!(v.as_bool()?),
            "idle_minutes" => {
                let m = v.as_u64().filter(|m| (1..=MAX_IDLE_MINUTES).contains(m))?;
                out["idle_minutes"] = json!(m);
            }
            _ => return None,
        }
    }
    Some(out)
}

/// What decides whether, and when, a running agent session hibernates by itself.
#[derive(Clone, Copy, Debug, Default)]
pub struct Facts {
    pub idle_ms: u64,
    pub since_input_ms: u64,
    /// time since the last event that concerned the session (message, wake, subscribe...)
    pub since_event_ms: u64,
    pub awaiting: bool,
    /// it reported with `axelstra done` and has not been retired
    pub finished: bool,
    pub lead_with_children: bool,
    pub main: bool,
}

/// How long until the session has been waiting for real for `idle_ms`: `Some(0)` is due now, `None` means no trigger
/// holds (it works, or it is the project main).
pub fn due_in(f: &Facts, p: &Policy) -> Option<Duration> {
    if !p.enabled || f.main || !(f.awaiting || f.finished || f.lead_with_children) {
        return None;
    }
    let quiet = f.idle_ms.min(f.since_input_ms).min(f.since_event_ms);
    Some(Duration::from_millis(p.idle_ms.saturating_sub(quiet)))
}

/// A conversation id from a file the agent's hook wrote (untrusted): only a UUID-shaped value may reach a shell line.
pub fn valid_resume_id(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// Process names (see `procs::names_of`) that mean a command is running in the foreground or a build is going.
/// Conservative: a false positive only postpones hibernation.
pub fn busy_process(name: &str) -> bool {
    const BUSY: &[&str] = &[
        "axelstra", "cargo", "rustc", "cc", "gcc", "clang", "link", "cl", "msbuild", "npm", "npx", "pnpm", "yarn", "bun", "tsc", "vite",
        "webpack", "jest", "vitest", "pytest", "python", "python3", "go", "gradle", "mvn", "make", "cmake", "ninja", "dotnet", "docker",
        "git", "ssh", "curl", "wget", "bash", "sh", "zsh", "dash", "fish", "powershell", "pwsh",
    ];
    BUSY.contains(&name)
}

// ---------------------------------------------------------------------------------------------------------------
// the command that brings an agent back

/// Byte ranges of the whitespace-separated tokens of a shell line. Single and double quotes keep a token together
/// (a doubled quote of PowerShell and sh's `'\''` simply close and reopen it), a backslash escapes outside quotes only
/// for sh; the tokens are only ever copied verbatim, never interpreted.
pub fn tokens(line: &str) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut start: Option<usize> = None;
    let mut quote: Option<char> = None;
    let mut prev_backslash = false;
    for (i, c) in line.char_indices() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => {
                if c.is_whitespace() {
                    if let Some(s) = start.take() {
                        out.push((s, i));
                    }
                    prev_backslash = false;
                    continue;
                }
                if (c == '\'' || c == '"') && !prev_backslash {
                    quote = Some(c);
                }
                prev_backslash = c == '\\';
            }
        }
        start.get_or_insert(i);
    }
    if quote.is_some() {
        return out;
    }
    if let Some(s) = start {
        out.push((s, line.len()));
    }
    out
}

/// The verbatim text of `flag` and its value token in `line`, if present and well formed.
fn flag_with_value(line: &str, flag: &str) -> Option<String> {
    let t = tokens(line);
    let i = t.iter().position(|(a, b)| &line[*a..*b] == flag)?;
    let (a, b) = *t.get(i + 1)?;
    let v = &line[a..b];
    (!v.starts_with("--")).then(|| format!("{flag} {v}"))
}

fn has_flag(line: &str, flag: &str) -> bool {
    tokens(line).iter().any(|(a, b)| &line[*a..*b] == flag)
}

pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn is_ps(program: &str) -> bool {
    let p = program.to_ascii_lowercase();
    p.contains("powershell") || p.contains("pwsh")
}

/// A process to start: what `Spawn` is made from.
#[derive(Debug, Clone, PartialEq)]
pub struct Wake {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
}

/// Why a session cannot be recreated.
#[derive(Debug, PartialEq)]
pub enum Cannot {
    /// the agent is not claude, codex or gemini
    NoSupport,
    /// claude with no conversation id that exists on disk
    NoResume,
    /// the saved start command has no shell line to build on
    NoLine,
}

/// The line that resumes `agent`'s conversation, built from the line the session was started with (its flags that
/// matter are copied verbatim: the hooks file, the reply-style rules, the permission mode). `None`: no such agent.
fn agent_line(agent: &str, resume: Option<&str>, orig: &str) -> Result<String, Cannot> {
    let a = axelstra_core::agents::get(agent).filter(|a| a.resume.is_some()).ok_or(Cannot::NoSupport)?;
    let keep = |flags: &[&str], bools: &[&str]| {
        let mut s = String::new();
        for f in flags {
            if let Some(x) = flag_with_value(orig, f) {
                s.push(' ');
                s.push_str(&x);
            }
        }
        for f in bools {
            if has_flag(orig, f) {
                s.push(' ');
                s.push_str(f);
            }
        }
        s
    };
    Ok(match a.id {
        "claude" => {
            let id = resume.filter(|r| valid_resume_id(r)).ok_or(Cannot::NoResume)?;
            let kept = keep(&["--settings", "--append-system-prompt", "--permission-mode", "--model"], &["--dangerously-skip-permissions"]);
            format!("claude{kept} --resume {}", sh_quote(id))
        }
        "codex" => format!("codex resume --last{}", keep(&[], &["--dangerously-bypass-approvals-and-sandbox"])),
        _ => format!("gemini --resume latest{}", keep(&["--approval-mode"], &[])),
    })
}

/// The start command of a hibernated session, from its saved spawn request `spec`, its meta and the conversation id
/// found for it. The agent runs under the same shell as before; if it exits with an error the shell exits with
/// `RESUME_FAILED_EXIT`, so a resume that fails is seen at once (the original line falls back to a fresh start, which
/// would hide it).
pub fn wake_command(spec: &Value, meta: &Value, resume: Option<&str>) -> Result<Wake, Cannot> {
    let program = spec["program"].as_str().ok_or(Cannot::NoLine)?.to_string();
    let args: Vec<String> = spec["args"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    let ps = is_ps(&program);
    let at = args
        .iter()
        .rposition(|a| if ps { a.eq_ignore_ascii_case("-Command") } else { a == "-c" })
        .filter(|i| i + 1 < args.len())
        .ok_or(Cannot::NoLine)?;
    let orig = &args[at + 1];
    let prefix: Vec<String> = args[..at].to_vec();
    let line = agent_line(meta["agent"].as_str().unwrap_or_default(), resume, orig)?;
    let (cmd, tail): (String, Vec<String>) = if ps {
        // `-NoExit` sits right before `-Command` when the shell was to stay open
        (format!("{line}; if (!$?) {{ exit {RESUME_FAILED_EXIT} }}"), vec!["-Command".into()])
    } else {
        let keep_open = orig.contains("; exec ");
        let exec = if keep_open { format!("; exec {} -l", sh_quote(&program)) } else { String::new() };
        (format!("{line} || exit {RESUME_FAILED_EXIT}{exec}"), vec!["-c".into()])
    };
    let mut args = prefix;
    args.extend(tail);
    args.push(cmd);
    let mut env: Vec<(String, String)> = spec["env"]
        .as_object()
        .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect())
        .unwrap_or_default();
    env.retain(|(k, _)| k != "AXELSTRA_PARENT");
    if let Some(p) = meta["parent"].as_u64() {
        env.push(("AXELSTRA_PARENT".into(), p.to_string()));
    }
    let dim = |v: &Value, d: u16| v.as_u64().map_or(d, |x| x.clamp(1, 1000) as u16);
    Ok(Wake {
        program,
        args,
        cwd: spec["cwd"].as_str().or(meta["worktree"].as_str()).map(String::from),
        env,
        cols: dim(&spec["cols"], 120),
        rows: dim(&spec["rows"], 30),
    })
}

// ---------------------------------------------------------------------------------------------------------------
// bookkeeping

/// What is kept of a hibernated session besides its registry rows.
#[derive(Clone, Debug)]
pub struct Sleeper {
    /// unix ms
    pub since: u64,
    pub saved_bytes: u64,
}

#[derive(Default)]
struct State {
    sleeping: HashMap<u32, Sleeper>,
    /// being put to sleep right now: the exit of its process is not an end
    hibernating: HashSet<u32>,
    /// FIFO of sessions that asked to wake (a session appears once, however many events asked)
    queue: VecDeque<u32>,
    /// process started, inbox notice not typed yet
    waking: HashMap<u32, u64>,
    /// events per session: a hibernation that saw the counter move gives up
    events: HashMap<u32, (u64, Instant)>,
    /// `ephemeral` start parameters, to arm the reaper again after a wake
    reap: HashMap<u32, Value>,
    /// the coded error of a wake that failed, kept for `session.status`
    failed: HashMap<u32, Value>,
    /// refusals so far per session by the policy (backoff of the next try)
    attempts: HashMap<u32, (u32, Instant)>,
}

/// All hibernation state of a registry: one leaf lock, never held while taking another lock or doing IO.
#[derive(Default)]
pub struct Hib(Mutex<State>);

impl Hib {
    /// An event concerned `id` (a message, a wake, a subscribe...): a hibernation under way gives up.
    pub fn touch(&self, id: u32) {
        let mut s = self.0.lock_recover();
        let e = s.events.entry(id).or_insert((0, Instant::now()));
        *e = (e.0 + 1, Instant::now());
        s.attempts.remove(&id);
    }

    /// `(counter, ms since the last event)`.
    pub fn events(&self, id: u32) -> (u64, u64) {
        self.0.lock_recover().events.get(&id).map_or((0, u64::MAX), |(n, at)| (*n, at.elapsed().as_millis() as u64))
    }

    /// Records the hibernation if no event arrived since `seen`. The caller holds the `sessions` lock and removes the
    /// session from it in the same step, so an event cannot slip between the check and the removal.
    pub fn commit(&self, id: u32, seen: u64, saved_bytes: u64) -> bool {
        let mut s = self.0.lock_recover();
        if s.events.get(&id).map_or(0, |e| e.0) != seen || s.sleeping.contains_key(&id) {
            return false;
        }
        let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        s.sleeping.insert(id, Sleeper { since, saved_bytes });
        s.hibernating.insert(id);
        s.failed.remove(&id);
        true
    }

    /// Records a session found hibernated in `sessions.json` at start.
    pub fn adopt(&self, id: u32, since: u64, saved_bytes: u64) {
        self.0.lock_recover().sleeping.insert(id, Sleeper { since, saved_bytes });
    }

    pub fn is_sleeping(&self, id: u32) -> bool {
        self.0.lock_recover().sleeping.contains_key(&id)
    }

    /// The process of `id` is being killed for a hibernation, or was and its watcher has not looked yet.
    pub fn is_hibernating(&self, id: u32) -> bool {
        self.0.lock_recover().hibernating.contains(&id)
    }

    pub fn end_hibernating(&self, id: u32) {
        self.0.lock_recover().hibernating.remove(&id);
    }

    pub fn sleeper(&self, id: u32) -> Option<Sleeper> {
        self.0.lock_recover().sleeping.get(&id).cloned()
    }

    pub fn sleepers(&self) -> Vec<(u32, Sleeper)> {
        let mut v: Vec<_> = self.0.lock_recover().sleeping.iter().map(|(i, s)| (*i, s.clone())).collect();
        v.sort_by_key(|x| x.0);
        v
    }

    pub fn is_waking(&self, id: u32) -> Option<u64> {
        self.0.lock_recover().waking.get(&id).copied()
    }

    pub fn queued(&self, id: u32) -> bool {
        self.0.lock_recover().queue.contains(&id)
    }

    /// Asks for a wake. False: it is not sleeping (or already queued), nothing to do.
    pub fn enqueue(&self, id: u32) -> bool {
        let mut s = self.0.lock_recover();
        if !s.sleeping.contains_key(&id) || s.queue.contains(&id) {
            return false;
        }
        s.queue.push_back(id);
        true
    }

    /// The queue in order.
    pub fn queue(&self) -> Vec<u32> {
        self.0.lock_recover().queue.iter().copied().collect()
    }

    /// The wake of `id` starts: it leaves the queue and the sleepers, and counts as waking until `finish_waking`.
    pub fn begin_wake(&self, id: u32) -> Option<Sleeper> {
        let mut s = self.0.lock_recover();
        s.queue.retain(|q| *q != id);
        let sl = s.sleeping.remove(&id)?;
        let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        s.waking.insert(id, since);
        Some(sl)
    }

    /// A wake that could not even start puts the session back to the sleepers (the caller then fails it).
    pub fn finish_waking(&self, id: u32) {
        self.0.lock_recover().waking.remove(&id);
    }

    pub fn note_reap(&self, id: u32, reap: Value) {
        if reap.is_object() {
            self.0.lock_recover().reap.insert(id, reap);
        }
    }

    pub fn reap_of(&self, id: u32) -> Option<Value> {
        self.0.lock_recover().reap.get(&id).cloned()
    }

    pub fn set_failed(&self, id: u32, err: Value) {
        self.0.lock_recover().failed.insert(id, err);
    }

    pub fn failed(&self, id: u32) -> Option<Value> {
        self.0.lock_recover().failed.get(&id).cloned()
    }

    /// The session left the registry for good: nothing about it is kept (except `failed`, which `status` still reports).
    pub fn forget(&self, id: u32) {
        let mut s = self.0.lock_recover();
        s.sleeping.remove(&id);
        s.hibernating.remove(&id);
        s.queue.retain(|q| *q != id);
        s.waking.remove(&id);
        s.events.remove(&id);
        s.reap.remove(&id);
        s.attempts.remove(&id);
    }

    /// After a refusal by the policy: when to try this session again (bounded exponential backoff).
    pub fn backoff(&self, id: u32) -> Duration {
        let mut s = self.0.lock_recover();
        let n = s.attempts.get(&id).map_or(0, |a| a.0) + 1;
        let d = crate::janitor::retry_delay(n);
        s.attempts.insert(id, (n, Instant::now() + d));
        d
    }

    /// Time left of a backoff, if one is running.
    pub fn backoff_left(&self, id: u32) -> Option<Duration> {
        self.0.lock_recover().attempts.get(&id).and_then(|a| a.1.checked_duration_since(Instant::now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(args: &[&str], program: &str) -> Value {
        json!({"program": program, "args": args, "cwd": "C:/w", "env": {"AXELSTRA_KEY": "k"}, "cols": 100, "rows": 40})
    }

    #[test]
    fn policy_defaults_and_bounds() {
        let p = policy(&json!({}));
        assert!(p.enabled);
        assert_eq!(p.idle_ms, 30 * 60_000);
        assert_eq!(policy(&json!({"hibernate": {"enabled": false, "idle_minutes": 5}})), Policy { enabled: false, idle_ms: 300_000 });
        assert!(validate(&json!({}), &json!({"idle_minutes": 0})).is_none());
        assert!(validate(&json!({}), &json!({"idle_minutes": 5000})).is_none());
        assert!(validate(&json!({}), &json!({"nope": 1})).is_none());
        assert_eq!(validate(&json!({"hibernate": {"idle_minutes": 7}}), &json!({"enabled": false})).unwrap(), json!({"enabled": false, "idle_minutes": 7}));
    }

    #[test]
    fn one_deadline_per_session() {
        let p = Policy { enabled: true, idle_ms: 600_000 };
        let f = Facts { idle_ms: 100_000, since_input_ms: 700_000, since_event_ms: u64::MAX, awaiting: true, ..Default::default() };
        assert_eq!(due_in(&f, &p), Some(Duration::from_millis(500_000)));
        // recent input moves it
        let g = Facts { since_input_ms: 1000, ..f };
        assert_eq!(due_in(&g, &p), Some(Duration::from_millis(599_000)));
        // long quiet: due now
        let h = Facts { idle_ms: 900_000, since_input_ms: 900_000, ..f };
        assert_eq!(due_in(&h, &p), Some(Duration::ZERO));
        // no trigger, the main, a disabled policy: never
        assert_eq!(due_in(&Facts { awaiting: false, ..f }, &p), None);
        assert_eq!(due_in(&Facts { main: true, ..f }, &p), None);
        assert_eq!(due_in(&f, &Policy { enabled: false, ..p }), None);
        // each of the three triggers counts
        for t in [Facts { finished: true, ..Default::default() }, Facts { lead_with_children: true, ..Default::default() }] {
            assert!(due_in(&Facts { idle_ms: 0, since_input_ms: 0, since_event_ms: 0, ..t }, &p).is_some());
        }
    }

    #[test]
    fn tokens_keep_quoted_values_whole() {
        let line = "claude --settings 'C:/a b/hooks.json' --append-system-prompt 'it'\\''s --resume' --session-id abc";
        assert_eq!(flag_with_value(line, "--settings").unwrap(), "--settings 'C:/a b/hooks.json'");
        assert_eq!(flag_with_value(line, "--append-system-prompt").unwrap(), "--append-system-prompt 'it'\\''s --resume'");
        assert!(flag_with_value(line, "--model").is_none());
        assert!(has_flag(line, "--session-id"));
    }

    #[test]
    fn claude_wake_line_resumes_the_conversation_and_flags_failure() {
        let orig = "claude --settings 'C:/d/hooks.json' --append-system-prompt 'be short' --session-id 0f8fad5b-d9cb-469f-a165-70867728950e; exec x";
        let meta = json!({"agent": "claude", "parent": 4, "worktree": "C:/w"});
        let sh = wake_command(&spec(&["-l", "-c", &format!("{orig}; exec 'sh' -l")], "sh"), &meta, Some("0f8fad5b-d9cb-469f-a165-70867728950e")).unwrap();
        assert_eq!(sh.args[..2], ["-l", "-c"]);
        let cmd = &sh.args[2];
        assert!(cmd.starts_with("claude --settings 'C:/d/hooks.json' --append-system-prompt 'be short' --resume '0f8f"), "{cmd}");
        assert!(cmd.contains("|| exit 97; exec 'sh' -l"), "{cmd}");
        assert!(!cmd.contains("--session-id"));
        assert!(sh.env.contains(&("AXELSTRA_PARENT".into(), "4".into())));
        assert!(sh.env.contains(&("AXELSTRA_KEY".into(), "k".into())));
        assert_eq!((sh.cols, sh.rows), (100, 40));
        let ps = wake_command(&spec(&["-NoExit", "-Command", "claude --session-id abcdef12"], "powershell"), &meta, Some("abcdef12")).unwrap();
        assert_eq!(ps.args[..2], ["-NoExit", "-Command"]);
        assert_eq!(ps.args[2], "claude --resume 'abcdef12'; if (!$?) { exit 97 }");
    }

    #[test]
    fn agents_without_resume_or_id_cannot_wake() {
        let s = spec(&["-c", "foo"], "sh");
        assert_eq!(wake_command(&s, &json!({"agent": "aider"}), None), Err(Cannot::NoSupport));
        assert_eq!(wake_command(&s, &json!({"agent": "claude"}), None), Err(Cannot::NoResume));
        assert_eq!(wake_command(&s, &json!({"agent": "claude"}), Some("x; calc")), Err(Cannot::NoResume));
        assert_eq!(wake_command(&spec(&["a"], "sh"), &json!({"agent": "codex"}), None), Err(Cannot::NoLine));
        let c = wake_command(&s, &json!({"agent": "codex"}), None).unwrap();
        assert_eq!(c.args[1], "codex resume --last || exit 97");
        let g = wake_command(&s, &json!({"agent": "gemini"}), None).unwrap();
        assert_eq!(g.args[1], "gemini --resume latest || exit 97");
    }

    #[test]
    fn a_hibernation_gives_up_when_an_event_arrived() {
        let h = Hib::default();
        let (seen, _) = h.events(3);
        h.touch(3);
        assert!(!h.commit(3, seen, 10));
        let (seen, _) = h.events(3);
        assert!(h.commit(3, seen, 10));
        assert!(h.is_sleeping(3) && h.is_hibernating(3));
        assert!(h.enqueue(3));
        assert!(!h.enqueue(3), "coalesced");
        assert!(h.begin_wake(3).is_some());
        assert!(!h.is_sleeping(3) && h.is_waking(3).is_some() && h.queue().is_empty());
    }
}
