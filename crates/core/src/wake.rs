//! A counter with a condition variable: whoever has something to say calls `bump`, whoever waits wakes at once.
//!
//! The waiter reads `now()` before it looks at the state it cares about and hands that number to `wait`, so a bump
//! that lands between the look and the wait is not lost (`wait` returns at once when the count moved on).

use crate::sync::LockRecover;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

#[derive(Clone, Default)]
pub struct Wake(Arc<(Mutex<u64>, Condvar)>);

impl Wake {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bump(&self) {
        *self.0 .0.lock_recover() += 1;
        self.0 .1.notify_all();
    }

    pub fn now(&self) -> u64 {
        *self.0 .0.lock_recover()
    }

    /// Waits for a bump after `seen`, for at most `limit` (`None`: for as long as it takes). Returns the count now.
    pub fn wait(&self, seen: u64, limit: Option<Duration>) -> u64 {
        let g = self.0 .0.lock_recover();
        let g = match limit {
            None => self.0 .1.wait_while(g, |n| *n == seen).unwrap_or_else(PoisonError::into_inner),
            Some(t) => self.0 .1.wait_timeout_while(g, t, |n| *n == seen).unwrap_or_else(PoisonError::into_inner).0,
        };
        *g
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn a_bump_wakes_a_waiter_before_its_limit() {
        let w = Wake::new();
        let seen = w.now();
        let w2 = w.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            w2.bump();
        });
        let t = Instant::now();
        assert_ne!(w.wait(seen, Some(Duration::from_secs(30))), seen);
        assert!(t.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_bump_before_the_wait_is_not_lost() {
        let w = Wake::new();
        let seen = w.now();
        w.bump();
        let t = Instant::now();
        assert_ne!(w.wait(seen, None), seen);
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn the_limit_ends_the_wait_without_a_bump() {
        let w = Wake::new();
        let seen = w.now();
        let t = Instant::now();
        assert_eq!(w.wait(seen, Some(Duration::from_millis(60))), seen);
        assert!(t.elapsed() >= Duration::from_millis(50));
    }
}
