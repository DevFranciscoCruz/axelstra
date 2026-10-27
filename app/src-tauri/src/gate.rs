//! One caller does a slow job (reconnecting to the daemon) while the others wait for it to end, without sleeping in a
//! loop: they block on a condition variable that the leader's guard notifies when it is dropped.

use axelstra_core::sync::LockRecover;
use std::sync::{Condvar, Mutex};
use std::time::Instant;

pub struct Gate {
    busy: Mutex<bool>,
    done: Condvar,
}

pub enum Entry<'a> {
    /// This caller does the job; dropping the guard lets the waiters go.
    Lead(Lead<'a>),
    /// Another caller did the job and has finished: look at the result.
    Waited,
    /// The other caller was still at it at the deadline.
    TimedOut,
}

pub struct Lead<'a>(&'a Gate);

impl Drop for Lead<'_> {
    fn drop(&mut self) {
        *self.0.busy.lock_recover() = false;
        self.0.done.notify_all();
    }
}

impl Gate {
    pub const fn new() -> Self {
        Gate { busy: Mutex::new(false), done: Condvar::new() }
    }

    pub fn enter(&self, deadline: Instant) -> Entry<'_> {
        let mut busy = self.busy.lock_recover();
        if !*busy {
            *busy = true;
            return Entry::Lead(Lead(self));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        let (busy, _) = self.done.wait_timeout_while(busy, left, |b| *b).unwrap_or_else(std::sync::PoisonError::into_inner);
        if *busy {
            Entry::TimedOut
        } else {
            Entry::Waited
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn the_first_caller_leads_and_the_rest_wake_when_it_ends() {
        let g = Arc::new(Gate::new());
        let far = Instant::now() + Duration::from_secs(60);
        let Entry::Lead(lead) = g.enter(far) else { panic!("first caller must lead") };
        let g2 = g.clone();
        let waiter = std::thread::spawn(move || {
            let t = Instant::now();
            let e = g2.enter(Instant::now() + Duration::from_secs(60));
            (matches!(e, Entry::Waited), t.elapsed())
        });
        std::thread::sleep(Duration::from_millis(100));
        drop(lead);
        let (waited, took) = waiter.join().unwrap();
        assert!(waited, "the waiter must see the leader's end, not time out");
        assert!(took < Duration::from_secs(30), "took {took:?}");
    }

    #[test]
    fn a_waiter_gives_up_at_its_deadline() {
        let g = Gate::new();
        let _lead = g.enter(Instant::now() + Duration::from_secs(1));
        let t = Instant::now();
        assert!(matches!(g.enter(Instant::now() + Duration::from_millis(80)), Entry::TimedOut));
        assert!(t.elapsed() >= Duration::from_millis(70));
    }

    #[test]
    fn after_the_leader_a_new_caller_leads_again() {
        let g = Gate::new();
        let far = Instant::now() + Duration::from_secs(1);
        drop(g.enter(far));
        assert!(matches!(g.enter(far), Entry::Lead(_)));
    }
}
