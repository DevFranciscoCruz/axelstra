//! Messages between sessions and the bookkeeping of ephemeral workers: who is whose child, who reported
//! "done", who escalated, and how many workers may run at once. Pure data; the server delivers nudges.

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
    /// sessions counted against the worker limit
    workers: Mutex<HashSet<u32>>,
    /// workers with a reaper thread: it delivers their final report itself
    reaped: Mutex<HashSet<u32>>,
    limit: AtomicUsize,
}

/// What an agent TUI receives as one pasted block followed by Enter.
pub fn paste(text: &str) -> Vec<u8> {
    format!("\x1b[200~{text}\x1b[201~\r").into_bytes()
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
        while target.since_input_ms() < QUIET_MS && start.elapsed().as_secs() < 30 {
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        let _ = target.write_system(&data);
        DEFERRED.fetch_sub(1, Ordering::SeqCst);
    });
}

/// One-line notice typed into an agent that got a message; the full text is also kept in its inbox.
pub fn notice(m: &Msg, from_label: &str) -> String {
    let what = match m.kind.as_str() {
        "ask" => format!("pergunta #{} de {from_label} (responda com `axelstra reply {} \"…\"`)", m.id, m.id),
        "escalate" => format!("pedido de ajuda de {from_label} (responda com `axelstra msg <sessão> \"…\"`)"),
        "done" => format!("{from_label} terminou"),
        "reply" => format!("resposta de {from_label} à mensagem #{}", m.reply_to.unwrap_or(0)),
        _ => format!("mensagem #{} de {from_label}", m.id),
    };
    let body: String = m.text.chars().take(600).collect();
    let more = if m.text.chars().count() > 600 { " … (mais em `axelstra inbox`)" } else { "" };
    format!("[axelstra] {what}: {body}{more}")
}

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
        self.parents.lock().unwrap().insert(child, parent);
    }
    pub fn parent_of(&self, child: u32) -> Option<u32> {
        self.parents.lock().unwrap().get(&child).copied()
    }

    pub fn add_worker(&self, id: u32) {
        self.workers.lock().unwrap().insert(id);
    }
    pub fn mark_reaped(&self, id: u32) {
        self.reaped.lock().unwrap().insert(id);
    }
    pub fn is_reaped(&self, id: u32) -> bool {
        self.reaped.lock().unwrap().contains(&id)
    }
    /// workers still alive according to `alive`; forgets the ones that are gone
    pub fn live_workers(&self, alive: impl Fn(u32) -> bool) -> usize {
        let mut w = self.workers.lock().unwrap();
        w.retain(|id| alive(*id));
        w.len()
    }

    pub fn finished(&self, id: u32) -> Option<Value> {
        self.finished.lock().unwrap().get(&id).cloned()
    }
    pub fn is_escalated(&self, id: u32) -> bool {
        self.escalated.lock().unwrap().contains(&id)
    }

    /// Stores a message and applies its side effects. The caller checked that `to` exists.
    pub fn send(&self, from: Option<u32>, to: u32, kind: &str, text: &str, reply_to: Option<u64>, thread: Option<String>) -> Msg {
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        // an answer belongs to the conversation of the message it answers
        let thread = thread.or_else(|| reply_to.and_then(|r| self.get(r)).and_then(|o| o.thread));
        let m = Msg { id, from, to, kind: kind.to_string(), text: text.to_string(), reply_to, thread, ts: now(), read: false };
        self.inbox.lock().unwrap().entry(to).or_default().push(m.clone());
        match (kind, from) {
            ("escalate", Some(f)) => {
                self.escalated.lock().unwrap().insert(f);
            }
            ("done", Some(f)) => {
                self.finished.lock().unwrap().insert(f, json!({"text": text, "id": id}));
            }
            // any answer from the parent unblocks an escalated worker
            ("reply" | "message", _) => {
                self.escalated.lock().unwrap().remove(&to);
            }
            _ => {}
        }
        m
    }

    pub fn unread(&self, session: u32) -> usize {
        self.inbox.lock().unwrap().get(&session).map(|v| v.iter().filter(|m| !m.read).count()).unwrap_or(0)
    }

    /// Messages of `session`, oldest first; unread only unless `all`. `mark` flags the returned ones as read.
    pub fn inbox(&self, session: u32, all: bool, mark: bool) -> Vec<Msg> {
        let mut g = self.inbox.lock().unwrap();
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
        let mut v: Vec<Msg> = self.inbox.lock().unwrap().values().flatten().filter(|m| m.thread.as_deref() == Some(name)).cloned().collect();
        v.sort_by_key(|m| m.id);
        v
    }

    pub fn get(&self, id: u64) -> Option<Msg> {
        self.inbox.lock().unwrap().values().flatten().find(|m| m.id == id).cloned()
    }

    /// The reply to message `id`, if one arrived.
    pub fn reply_to(&self, id: u64) -> Option<Msg> {
        self.inbox.lock().unwrap().values().flatten().find(|m| m.reply_to == Some(id) && m.kind == "reply").cloned()
    }

    /// Drops everything kept about a session that is gone.
    pub fn forget(&self, id: u32) {
        self.inbox.lock().unwrap().remove(&id);
        self.parents.lock().unwrap().remove(&id);
        self.finished.lock().unwrap().remove(&id);
        self.escalated.lock().unwrap().remove(&id);
        self.workers.lock().unwrap().remove(&id);
        self.reaped.lock().unwrap().remove(&id);
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
    fn escalate_until_answered() {
        let b = Bus::new();
        b.send(Some(3), 1, "escalate", "stuck", None, None);
        assert!(b.is_escalated(3));
        b.send(Some(1), 3, "message", "try X", None, None);
        assert!(!b.is_escalated(3));
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
