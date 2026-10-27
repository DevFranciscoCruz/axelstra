//! Messages between sessions and the bookkeeping of ephemeral workers: who is whose child, who reported
//! "done", who escalated, and how many workers may run at once. Pure data; the server delivers nudges.

use crate::store::LockRecover;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

pub const DEFAULT_MAX_WORKERS: usize = 6;

#[derive(Serialize, Clone, Debug)]
pub struct Msg {
    pub id: u64,
    pub from: Option<u32>,
    pub to: u32,
    /// message | ask | reply | done | escalate
    pub kind: String,
    pub text: String,
    pub reply_to: Option<u64>,
    /// conversation name shared by related messages (replies inherit it)
    pub thread: Option<String>,
    pub ts: u64,
    pub read: bool,
}

#[derive(Default)]
pub struct Bus {
    next: AtomicU64,
    inbox: Mutex<HashMap<u32, Vec<Msg>>>,
    /// worker -> the session it reports to
    parents: Mutex<HashMap<u32, u32>>,
    /// sessions that called `done`, with the payload the parent receives
    finished: Mutex<HashMap<u32, Value>>,
    /// sessions that asked their parent for help and have not been answered yet
    escalated: Mutex<HashSet<u32>>,
    /// sessions that asked (`ask` or `escalate`) and wait for the answer, with the question: this is what makes them "waiting"
    pending: Mutex<HashMap<u32, String>>,
    /// sessions counted against the worker limit
    workers: Mutex<HashSet<u32>>,
    /// workers with a reaper thread: it delivers their final report itself
    reaped: Mutex<HashSet<u32>>,
    limit: AtomicUsize,
}

/// Text with every control character removed except newline and tab: no escape sequence, no bracketed-paste
/// terminator and no carriage return survives to the terminal.
pub fn clean(text: &str) -> String {
    text.chars().filter(|c| matches!(c, '\n' | '\t') || !c.is_control()).collect()
}

/// Text safe to type after `echo` in an unknown shell: control characters and shell metacharacters become `_`.
pub fn echo_safe(text: &str) -> String {
    clean(text)
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | ':' | ',' | '/' | '#' | '[' | ']') { c } else { '_' })
        .collect()
}

/// What an agent TUI receives as one pasted block followed by Enter.
pub fn paste(text: &str) -> Vec<u8> {
    paste_with(text, true)
}

/// Like `paste`, with the Enter optional: text that came from another session is only typed (left for the user to
/// send) when the receiving agent runs without permission prompts, where an Enter would execute it unreviewed.
pub fn paste_with(text: &str, enter: bool) -> Vec<u8> {
    format!("\x1b[200~{}\x1b[201~{}", clean(text), if enter { "\r" } else { "" }).into_bytes()
}

/// Types a notice or report into a session without landing in the middle of what the user is typing:
/// while they typed in the last 3 s it waits for a pause (at most 30 s, then goes anyway). Deferred
/// deliveries keep their order, and a new one queues behind them.
pub fn deliver(target: std::sync::Arc<axelstra_core::pty::Session>, data: Vec<u8>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static DEFERRED: AtomicUsize = AtomicUsize::new(0);
    static ORDER: std::sync::Mutex<()> = std::sync::Mutex::new(());
    const QUIET_MS: u64 = 3000;
    if DEFERRED.load(Ordering::SeqCst) == 0 && target.since_input_ms() >= QUIET_MS {
        let _ = target.write_system(&data);
        return;
    }
    DEFERRED.fetch_add(1, Ordering::SeqCst);
    std::thread::spawn(move || {
        let _turn = ORDER.lock().unwrap_or_else(|e| e.into_inner());
        let start = std::time::Instant::now();
        // sleeps exactly until the pause would be long enough; typing in between moves that moment, so it looks again
        loop {
            let left = QUIET_MS.saturating_sub(target.since_input_ms());
            if left == 0 || start.elapsed().as_secs() >= 30 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(left));
        }
        let _ = target.write_system(&data);
        DEFERRED.fetch_sub(1, Ordering::SeqCst);
    });
}

pub use crate::prompts::notice;

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Bus {
    pub fn new() -> Self {
        let b = Bus::default();
        b.limit.store(DEFAULT_MAX_WORKERS, Ordering::SeqCst);
        b
    }

    pub fn limit(&self) -> usize {
        self.limit.load(Ordering::SeqCst)
    }
    pub fn set_limit(&self, n: usize) {
        self.limit.store(n.clamp(1, 64), Ordering::SeqCst);
    }

    pub fn set_parent(&self, child: u32, parent: u32) {
        self.parents.lock_recover().insert(child, parent);
    }
    pub fn parent_of(&self, child: u32) -> Option<u32> {
        self.parents.lock_recover().get(&child).copied()
    }

    pub fn add_worker(&self, id: u32) {
        self.workers.lock_recover().insert(id);
    }
    pub fn mark_reaped(&self, id: u32) {
        self.reaped.lock_recover().insert(id);
    }
    pub fn is_reaped(&self, id: u32) -> bool {
        self.reaped.lock_recover().contains(&id)
    }
    /// workers still alive according to `alive`; forgets the ones that are gone
    pub fn live_workers(&self, alive: impl Fn(u32) -> bool) -> usize {
        let mut w = self.workers.lock_recover();
        w.retain(|id| alive(*id));
        w.len()
    }

    pub fn finished(&self, id: u32) -> Option<Value> {
        self.finished.lock_recover().get(&id).cloned()
    }
    pub fn is_escalated(&self, id: u32) -> bool {
        self.escalated.lock_recover().contains(&id)
    }
    /// The question a session asked and has not had answered yet.
    pub fn pending_of(&self, id: u32) -> Option<String> {
        self.pending.lock_recover().get(&id).cloned()
    }

    /// Stores a message and applies its side effects. The caller checked that `to` exists.
    pub fn send(&self, from: Option<u32>, to: u32, kind: &str, text: &str, reply_to: Option<u64>, thread: Option<String>) -> Msg {
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        // an answer belongs to the conversation of the message it answers
        let thread = thread.or_else(|| reply_to.and_then(|r| self.get(r)).and_then(|o| o.thread));
        let m = Msg { id, from, to, kind: kind.to_string(), text: text.to_string(), reply_to, thread, ts: now(), read: false };
        self.inbox.lock_recover().entry(to).or_default().push(m.clone());
        match (kind, from) {
            ("escalate", Some(f)) => {
                self.escalated.lock_recover().insert(f);
                self.pending.lock_recover().insert(f, text.to_string());
            }
            ("ask", Some(f)) => {
                self.pending.lock_recover().insert(f, text.to_string());
            }
            ("done", Some(f)) => {
                self.finished.lock_recover().insert(f, json!({"text": text, "id": id}));
                self.pending.lock_recover().remove(&f);
            }
            // any answer from the parent unblocks an escalated worker
            ("reply" | "message", _) => {
                let was_escalated = self.escalated.lock_recover().remove(&to);
                // an `ask` is answered by a reply only: a status message from a child must not clear it
                if kind == "reply" || was_escalated {
                    self.pending.lock_recover().remove(&to);
                }
                // new work after a report: it is busy again until it reports again
                self.finished.lock_recover().remove(&to);
            }
            _ => {}
        }
        m
    }

    pub fn unread(&self, session: u32) -> usize {
        self.inbox.lock_recover().get(&session).map(|v| v.iter().filter(|m| !m.read).count()).unwrap_or(0)
    }

    /// Messages of `session`, oldest first; unread only unless `all`. `mark` flags the returned ones as read.
    pub fn inbox(&self, session: u32, all: bool, mark: bool) -> Vec<Msg> {
        let mut g = self.inbox.lock_recover();
        let Some(list) = g.get_mut(&session) else { return vec![] };
        let mut out = vec![];
        for m in list.iter_mut() {
            if all || !m.read {
                out.push(m.clone());
                if mark {
                    m.read = true;
                }
            }
        }
        out
    }

    /// Every message of a thread, in the order they were sent.
    pub fn thread(&self, name: &str) -> Vec<Msg> {
        let mut v: Vec<Msg> = self.inbox.lock_recover().values().flatten().filter(|m| m.thread.as_deref() == Some(name)).cloned().collect();
        v.sort_by_key(|m| m.id);
        v
    }

    pub fn get(&self, id: u64) -> Option<Msg> {
        self.inbox.lock_recover().values().flatten().find(|m| m.id == id).cloned()
    }

    /// The reply to message `id`, if one arrived.
    pub fn reply_to(&self, id: u64) -> Option<Msg> {
        self.inbox.lock_recover().values().flatten().find(|m| m.reply_to == Some(id) && m.kind == "reply").cloned()
    }

    /// Drops everything kept about a session that is gone.
    pub fn forget(&self, id: u32) {
        self.inbox.lock_recover().remove(&id);
        self.parents.lock_recover().remove(&id);
        self.finished.lock_recover().remove(&id);
        self.escalated.lock_recover().remove(&id);
        self.workers.lock_recover().remove(&id);
        self.reaped.lock_recover().remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ask_reply_roundtrip() {
        let b = Bus::new();
        let q = b.send(Some(2), 1, "ask", "which db?", None, Some("db".into()));
        assert_eq!(b.unread(1), 1);
        assert!(b.reply_to(q.id).is_none());
        let r = b.send(Some(1), 2, "reply", "postgres", Some(q.id), None);
        assert_eq!(r.thread.as_deref(), Some("db"));
        assert_eq!(b.thread("db").len(), 2);
        assert_eq!(b.reply_to(q.id).unwrap().text, "postgres");
        assert_eq!(r.to, 2);
        assert_eq!(b.inbox(1, false, true).len(), 1);
        assert_eq!(b.inbox(1, false, true).len(), 0);
        assert_eq!(b.inbox(1, true, false).len(), 1);
    }

    #[test]
    fn pasted_text_cannot_end_the_paste() {
        let evil = "hi\x1b[201~rm -rf /\r\x1b[200~ \u{9b}x\x07";
        let out = String::from_utf8(paste(evil)).unwrap();
        assert_eq!(out, "\x1b[200~hi[201~rm -rf /[200~ x\x1b[201~\r");
        assert_eq!(out.matches('\x1b').count(), 2);
        assert_eq!(clean("a\nb\tc"), "a\nb\tc");
        assert_eq!(echo_safe("x\x1b[0m; calc `id` $(id)&"), "x[0m_ calc _id_ __id__");
    }

    #[test]
    fn untrusted_text_can_be_left_without_enter() {
        assert!(paste_with("x", true).ends_with(b"\r"));
        assert!(paste_with("x", false).ends_with(b"\x1b[201~"));
        let m = Msg {
            id: 1,
            from: Some(2),
            to: 1,
            kind: "message".into(),
            text: "do it".into(),
            reply_to: None,
            thread: None,
            ts: 0,
            read: false,
        };
        assert!(notice(&m, "s").contains("untrusted"));
    }

    #[test]
    fn escalate_until_answered() {
        let b = Bus::new();
        b.send(Some(3), 1, "escalate", "stuck", None, None);
        assert!(b.is_escalated(3));
        b.send(Some(1), 3, "message", "try X", None, None);
        assert!(!b.is_escalated(3));
    }

    #[test]
    fn an_ask_or_escalation_is_pending_with_its_text_until_answered() {
        let b = Bus::new();
        b.send(Some(3), 1, "ask", "A or B?", None, None);
        assert_eq!(b.pending_of(3).as_deref(), Some("A or B?"));
        // not an answer
        b.send(Some(9), 3, "message", "fyi", None, None);
        assert_eq!(b.pending_of(3).as_deref(), Some("A or B?"));
        b.send(Some(1), 3, "reply", "A", Some(1), None);
        assert!(b.pending_of(3).is_none());
        b.send(Some(3), 1, "escalate", "stuck", None, None);
        assert_eq!(b.pending_of(3).as_deref(), Some("stuck"));
        b.send(Some(1), 3, "message", "try X", None, None);
        assert!(b.pending_of(3).is_none());
        b.send(Some(3), 1, "ask", "q", None, None);
        b.send(Some(3), 1, "done", "ok", None, None);
        assert!(b.pending_of(3).is_none());
    }

    #[test]
    fn a_new_message_makes_a_reported_session_busy_again() {
        let b = Bus::default();
        b.send(Some(4), 1, "done", "all green", None, None);
        assert!(b.finished(4).is_some());
        b.send(Some(1), 4, "message", "one more thing", None, None);
        assert!(b.finished(4).is_none());
    }

    #[test]
    fn done_is_recorded_and_forgotten() {
        let b = Bus::new();
        b.send(Some(4), 1, "done", "all green", None, None);
        assert_eq!(b.finished(4).unwrap()["text"], "all green");
        b.forget(4);
        assert!(b.finished(4).is_none());
    }

    #[test]
    fn worker_limit_counts_only_live() {
        let b = Bus::new();
        b.add_worker(1);
        b.add_worker(2);
        assert_eq!(b.live_workers(|id| id == 1), 1);
        assert_eq!(b.live_workers(|_| true), 1);
        b.set_limit(0);
        assert_eq!(b.limit(), 1);
    }
}
