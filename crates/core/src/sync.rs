//! Poison-tolerant locking. One panicking handler must not make every later caller of the same mutex panic too.

use std::sync::{Mutex, MutexGuard, PoisonError};

/// `lock()` that survives a poisoned mutex (the data is returned as the panicking thread left it).
pub trait LockRecover<T> {
    fn lock_recover(&self) -> MutexGuard<'_, T>;
}

impl<T> LockRecover<T> for Mutex<T> {
    fn lock_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poisoned_mutex_is_still_usable() {
        let m = std::sync::Arc::new(Mutex::new(1));
        let m2 = m.clone();
        let _ = std::thread::spawn(move || {
            let _g = m2.lock().unwrap();
            panic!("poison it");
        })
        .join();
        assert!(m.is_poisoned());
        *m.lock_recover() += 1;
        assert_eq!(*m.lock_recover(), 2);
    }
}
