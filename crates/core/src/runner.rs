//! How an agent process is started and supervised. The orchestrator only knows
//! this trait, so agents can run headless in-process or inside a daemon PTY.

use crate::job::Job;
use anyhow::{Context, Result};
use std::fs::File;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

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
    child: Child,
    job: Job,
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
        let child = job.spawn(&mut cmd).with_context(|| format!("run {}", s.program))?;
        Ok(Box::new(ProcHandle { gone: false, child, job }))
    }

    fn interactive(&self) -> bool {
        false
    }
}

impl Handle for ProcHandle {
    fn try_wait(&mut self) -> Result<Option<String>> {
        let Some(s) = self.child.try_wait()? else { return Ok(None) };
        if self.gone {
            return Ok(Some(format!("exited ({s})")));
        }
        self.gone = true;
        // the agent is gone and reaped: what it left running in its group must not outlive it (and the group id is
        // not signalled again later, when the number may belong to someone else)
        self.job.kill_and_forget(self.child.id());
        Ok(Some(format!("exited ({s})")))
    }

    fn kill(&mut self) {
        self.job.kill();
        let _ = self.child.wait();
    }
}
