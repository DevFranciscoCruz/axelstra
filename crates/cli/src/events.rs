//! Waiting for the daemon to say something changed, instead of asking again and again.
//!
//! `Changes` holds a second connection subscribed to the daemon's watch stream (`session.changed`, `session.exited`,
//! `dispatch.changed`). `wait(limit)` blocks until one of those arrives or `limit` passes. A caller subscribes first,
//! then looks at the state, then waits: a change between the look and the wait is already queued, so none is lost.
//! `limit` is a deadline the caller knows (its own timeout, the moment an idle threshold would be reached), never a
//! polling period.

use anyhow::{bail, Result};
use axelstra_daemon::client::{Client, Frame};
use serde_json::json;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

pub struct Changes {
    rx: Receiver<()>,
}

/// Events that can change what a waiting command looks at.
fn wakes(event: &str) -> bool {
    matches!(event, "session.changed" | "session.exited" | "dispatch.changed")
}

impl Changes {
    /// Subscribes on a new connection to the same daemon the command talks to.
    pub fn open() -> Result<Self> {
        let mut c = super::remote::connect_same()?;
        // registers the connection for `session.changed` and friends; it watches no folder
        c.call("watch.unsubscribe", json!({}))?;
        let (tx, rx) = channel();
        std::thread::Builder::new().name("cli-events".into()).spawn(move || pump(c, tx))?;
        Ok(Self { rx })
    }

    /// A source that is fed by hand (tests).
    #[cfg(test)]
    fn from_channel(rx: Receiver<()>) -> Self {
        Self { rx }
    }

    /// Blocks until a change arrives (`true`) or `limit` passes (`false`). Several changes that piled up count as one.
    /// Fails when the daemon connection is gone: nothing could wake the caller any more.
    pub fn wait(&self, limit: Duration) -> Result<bool> {
        match self.rx.recv_timeout(limit) {
            Ok(()) => {
                while self.rx.try_recv().is_ok() {}
                Ok(true)
            }
            Err(RecvTimeoutError::Timeout) => Ok(false),
            Err(RecvTimeoutError::Disconnected) => bail!("lost the connection to the daemon"),
        }
    }
}

fn pump(mut c: Client, tx: Sender<()>) {
    while let Ok(f) = c.read() {
        if let Frame::Event(v) = f {
            if v["event"].as_str().is_some_and(wakes) && tx.send(()).is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn a_change_wakes_the_wait_at_once() {
        let (tx, rx) = channel();
        let ch = Changes::from_channel(rx);
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            tx.send(()).unwrap();
            tx
        });
        let at = Instant::now();
        assert!(ch.wait(Duration::from_secs(30)).unwrap());
        assert!(at.elapsed() < Duration::from_secs(5));
        drop(t.join().unwrap());
    }

    #[test]
    fn a_change_that_came_before_the_wait_is_not_lost_and_pile_ups_count_once() {
        let (tx, rx) = channel();
        let ch = Changes::from_channel(rx);
        tx.send(()).unwrap();
        tx.send(()).unwrap();
        assert!(ch.wait(Duration::from_secs(30)).unwrap());
        assert!(!ch.wait(Duration::from_millis(60)).unwrap());
    }

    #[test]
    fn the_deadline_ends_the_wait() {
        let (_tx, rx) = channel::<()>();
        let ch = Changes::from_channel(rx);
        let at = Instant::now();
        assert!(!ch.wait(Duration::from_millis(80)).unwrap());
        assert!(at.elapsed() >= Duration::from_millis(70));
    }

    #[test]
    fn a_lost_connection_is_an_error_not_a_busy_loop() {
        let (tx, rx) = channel::<()>();
        let ch = Changes::from_channel(rx);
        drop(tx);
        assert!(ch.wait(Duration::from_secs(30)).is_err());
    }

    #[test]
    fn only_state_events_wake() {
        assert!(wakes("session.changed") && wakes("session.exited") && wakes("dispatch.changed"));
        assert!(!wakes("fs.changed") && !wakes("projects.changed"));
    }
}
