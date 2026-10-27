//! How an agent process is started and supervised. The orchestrator only knows
//! this trait, so agents can run headless in-process or inside a daemon PTY.

use crate::job::Job;
use crate::sync::LockRecover;
use crate::wake::Wake;
use anyhow::{Context, Result};
use std::fs::File;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Debug, Clone)]
pub struct AgentSpawn {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    /// Shown in the UI session list, e.g. `run/task`.
    pub label: String,
    /// Headless runners redirect stdout/stderr here; PTY runners ignore it.
    pub log: Option<PathBuf>,
}

pub trait Handle: Send {
    /// `Some(description)` once the process is gone.
    fn try_wait(&mut self) -> Result<Option<String>>;
    /// Kills the whole process tree and waits for it.
    fn kill(&mut self);
    /// Asks the handle to `bump` `wake` the moment the process is gone (at once when it already is). Returns whether it
    /// can: a handle that cannot push (default) has to be asked with `try_wait` now and then.
    fn on_exit(&mut self, _wake: &Wake) -> bool {
        false
    }
}

pub trait Runner: Send + Sync {
    fn spawn(&self, s: &AgentSpawn) -> Result<Box<dyn Handle>>;
    /// Interactive runners start the agent's TUI instead of a one-shot `-p` run.
    fn interactive(&self) -> bool;
}

/// Headless: plain child process in a Job Object, output to a log file.
pub struct ProcessRunner;

struct ProcHandle {
    /// the exit was already seen (and the group cleaned up once): never signal the group id again
    gone: bool,
    pid: u32,
    exit: Arc<Exit>,
    job: Job,
}

/// What the thread that waits for the child leaves behind.
struct Exit {
    slot: Mutex<ExitSlot>,
    changed: Condvar,
}

#[derive(Default)]
struct ExitSlot {
    /// `Some(Ok)` once the child was reaped; `Some(Err)` when waiting for it failed
    status: Option<std::result::Result<ExitStatus, String>>,
    wake: Option<Wake>,
}

impl Runner for ProcessRunner {
    fn spawn(&self, s: &AgentSpawn) -> Result<Box<dyn Handle>> {
        let mut cmd = Command::new(&s.program);
        cmd.args(&s.args).current_dir(&s.cwd).stdin(Stdio::null());
        for (k, v) in &s.env {
            cmd.env(k, v);
        }
        if let Some(p) = &s.log {
            let log = File::create(p)?;
            cmd.stdout(log.try_clone()?).stderr(log);
        }
        let job = Job::new()?;
        let mut child = job.spawn(&mut cmd).with_context(|| format!("run {}", s.program))?;
        let pid = child.id();
        let exit = Arc::new(Exit { slot: Mutex::new(ExitSlot::default()), changed: Condvar::new() });
        // a thread blocked in the OS wait: the exit is pushed, nobody has to ask
        let e = exit.clone();
        std::thread::Builder::new().name("proc-exit".into()).spawn(move || {
            let st = child.wait().map_err(|e| e.to_string());
            let mut g = e.slot.lock_recover();
            g.status = Some(st);
            if let Some(w) = &g.wake {
                w.bump();
            }
            e.changed.notify_all();
        })?;
        Ok(Box::new(ProcHandle { gone: false, pid, exit, job }))
    }

    fn interactive(&self) -> bool {
        false
    }
}

impl Handle for ProcHandle {
    fn try_wait(&mut self) -> Result<Option<String>> {
        let st = match &self.exit.slot.lock_recover().status {
            None => return Ok(None),
            Some(Ok(s)) => *s,
            Some(Err(e)) => anyhow::bail!("{e}"),
        };
        if self.gone {
            return Ok(Some(format!("exited ({st})")));
        }
        self.gone = true;
        // the agent is gone and reaped: what it left running in its group must not outlive it (and the group id is
        // not signalled again later, when the number may belong to someone else)
        self.job.kill_and_forget(self.pid);
        Ok(Some(format!("exited ({st})")))
    }

    fn kill(&mut self) {
        self.job.kill();
        let g = self.exit.slot.lock_recover();
        drop(self.exit.changed.wait_while(g, |g| g.status.is_none()).unwrap_or_else(std::sync::PoisonError::into_inner));
    }

    fn on_exit(&mut self, wake: &Wake) -> bool {
        let mut g = self.exit.slot.lock_recover();
        if g.status.is_some() {
            wake.bump();
        }
        g.wake = Some(wake.clone());
        true
    }
}
