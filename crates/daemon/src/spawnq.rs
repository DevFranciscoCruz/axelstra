//! The queue of `session.spawn` calls that asked to wait (`queue: true`) because the worker limit or the memory gate
//! refused them. First in, first out: only the head may be admitted, so a later caller never passes an earlier one. Pure
//! bookkeeping; the daemon sleeps on its pulse (a freed slot, a limit change or a memory level change is a pulse) and
//! looks at the head again. A ticket whose caller stopped coming back (its CLI died) is dropped when it is next looked
//! at, never by a timer.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// A waiting caller comes back at least this often (the daemon answers `queued` after its own shorter wait).
pub const STALE: Duration = Duration::from_secs(90);

#[derive(Default)]
pub struct SpawnQ {
    next: u64,
    q: VecDeque<(u64, Instant)>,
}

impl SpawnQ {
    /// Joins at the back, or, with a ticket the queue still knows, keeps its place and renews it. Returns the ticket.
    pub fn enter(&mut self, ticket: Option<u64>, now: Instant) -> u64 {
        if let Some(t) = ticket {
            if let Some(e) = self.q.iter_mut().find(|e| e.0 == t) {
                e.1 = now;
                return t;
            }
        }
        self.next += 1;
        self.q.push_back((self.next, now));
        self.next
    }

    /// Whether `ticket` is first, after dropping the stale ones in front of it.
    pub fn is_head(&mut self, ticket: u64, now: Instant) -> bool {
        while let Some(&(t, seen)) = self.q.front() {
            if t != ticket && now.saturating_duration_since(seen) > STALE {
                self.q.pop_front();
            } else {
                break;
            }
        }
        self.q.front().is_some_and(|e| e.0 == ticket)
    }

    /// 1 for the head; 0 when the ticket is not queued.
    pub fn position(&self, ticket: u64) -> usize {
        self.q.iter().position(|e| e.0 == ticket).map_or(0, |i| i + 1)
    }

    pub fn leave(&mut self, ticket: u64) -> bool {
        let n = self.q.len();
        self.q.retain(|e| e.0 != ticket);
        self.q.len() != n
    }

    pub fn len(&self) -> usize {
        self.q.len()
    }

    pub fn is_empty(&self) -> bool {
        self.q.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_in_first_out_and_only_the_head_goes() {
        let (mut q, t0) = (SpawnQ::default(), Instant::now());
        let (a, b, c) = (q.enter(None, t0), q.enter(None, t0), q.enter(None, t0));
        assert!(q.is_head(a, t0) && !q.is_head(b, t0) && !q.is_head(c, t0));
        assert_eq!((q.position(a), q.position(b), q.position(c)), (1, 2, 3));
        assert!(q.leave(a));
        assert!(q.is_head(b, t0));
        assert_eq!(q.position(a), 0);
        assert!(!q.leave(a));
    }

    #[test]
    fn a_known_ticket_keeps_its_place() {
        let (mut q, t0) = (SpawnQ::default(), Instant::now());
        let (a, b) = (q.enter(None, t0), q.enter(None, t0));
        assert_eq!(q.enter(Some(a), t0), a);
        assert_eq!(q.len(), 2);
        // an unknown ticket (expired) joins again at the back
        let c = q.enter(Some(99), t0);
        assert!(c != 99 && q.position(c) == 3 && q.position(b) == 2);
    }

    #[test]
    fn a_caller_that_stopped_coming_back_is_dropped_when_looked_at() {
        let (mut q, t0) = (SpawnQ::default(), Instant::now());
        let (a, b) = (q.enter(None, t0), q.enter(None, t0));
        let later = t0 + STALE + Duration::from_secs(1);
        q.enter(Some(b), later);
        assert!(q.is_head(b, later));
        assert_eq!(q.position(a), 0);
        assert!(!q.is_empty());
    }
}
