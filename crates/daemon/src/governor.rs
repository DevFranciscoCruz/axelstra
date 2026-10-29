//! The resource governor: tells the daemon how much memory pressure the machine is under (`Level`), so nothing else
//! has to read free memory itself.
//!
//! No fixed polling. One thread waits on the OS's own memory signal (`MemSource::wait_change`: Windows memory resource
//! notifications, Linux PSI) and re-reads free memory when it fires. Those signals only fire near exhaustion (Windows)
//! or under stall (Linux), which is later than our thresholds, so the wait also carries ONE computed deadline
//! (`next_deadline`): far from the thresholds it is long (headroom divided by a worst-case burn rate, capped), close or
//! while the level is not Normal it is short, with a bounded backoff while nothing changes. A platform with no event
//! (macOS) is the same loop with the deadline as its only wake. Nothing wakes at a fixed rate.
//!
//! The state machine (`next_level`) is pure and has hysteresis: a level is entered at the threshold and left only
//! above threshold plus a margin, so a machine hovering at the line does not flap.

use anyhow::Result;
use axelstra_core::sync::LockRecover;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub mod cause;
#[cfg(target_os = "linux")]
mod linux;
mod sample;
pub mod suspend;
#[cfg(windows)]
mod win;

pub use sample::Sampler;

/// Memory pressure. Ordered: `Normal < High < Critical`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Normal,
    High,
    Critical,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Normal => "normal",
            Level::High => "high",
            Level::Critical => "critical",
        }
    }
}

/// What the machine's memory looks like now. `free_mb` is memory a new process could get without paging
/// (Windows `ullAvailPhys`, Linux `MemAvailable`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize)]
pub struct MemInfo {
    pub total_mb: u64,
    pub free_mb: u64,
    /// committed memory as a percentage of the commit limit, where the OS says (Windows)
    pub commit_pct: Option<u8>,
}

impl MemInfo {
    /// Free memory as a percentage of the total (100 when the total is unknown, so a broken read never raises pressure).
    pub fn free_pct(&self) -> f64 {
        if self.total_mb == 0 {
            100.0
        } else {
            self.free_mb as f64 * 100.0 / self.total_mb as f64
        }
    }
}

/// Settings of the governor, saved in `settings.json` under `governor`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Config {
    /// off: the level stays Normal and nothing is announced
    pub enabled: bool,
    /// (part 2) new agents wait for memory
    pub admission: bool,
    /// (part 2) agents are paused under Critical
    pub auto_pause: bool,
    /// (part 2) say what would be done, do nothing
    pub warn_only: bool,
    pub high_free_pct: u8,
    pub crit_free_pct: u8,
    pub high_free_mb: u64,
    pub crit_free_mb: u64,
    /// kernel pools (paged + nonpaged) above this many MB raise one notice per episode (`cause::KernelWatch`)
    pub kernel_pool_mb: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: true,
            admission: true,
            auto_pause: true,
            warn_only: false,
            high_free_pct: 20,
            crit_free_pct: 10,
            high_free_mb: 3072,
            crit_free_mb: 1536,
            kernel_pool_mb: 3072,
        }
    }
}

const KEYS: [&str; 9] =
    ["enabled", "admission", "auto_pause", "warn_only", "high_free_pct", "crit_free_pct", "high_free_mb", "crit_free_mb", "kernel_pool_mb"];
const KERNEL_POOL_RANGE: std::ops::RangeInclusive<u64> = 256..=1_048_576;
const MAX_MB: u64 = 4 * 1024 * 1024;

impl Config {
    /// The `governor` key of a settings object; a missing or broken one is the defaults, a bad single field too.
    pub fn from_settings(settings: &Value) -> Config {
        let mut c = Config::default();
        if let Some(o) = settings.get("governor").and_then(Value::as_object) {
            let patched = Config::default().patch(&Value::Object(o.clone()));
            if let Ok(p) = patched {
                c = p;
            } else {
                // field by field, so one broken value does not throw the rest away
                for (k, v) in o {
                    if let Ok(p) = c.patch(&json!({ k: v })) {
                        c = p;
                    }
                }
            }
        }
        c
    }

    /// Writes this config as the `governor` key of a settings object.
    pub fn into_settings(&self, settings: &mut Value) {
        if !settings.is_object() {
            *settings = json!({});
        }
        settings["governor"] = serde_json::to_value(self).unwrap_or(Value::Null);
    }

    /// This config with the fields of `patch` replaced. Unknown keys, wrong types and values that make no sense are errors.
    pub fn patch(&self, patch: &Value) -> Result<Config> {
        let bad = |detail: String| axelstra_core::coded!("governor.bad_config", "invalid governor config: {detail}", detail = detail);
        let o = patch.as_object().ok_or_else(|| bad("patch must be an object".into()))?;
        let mut c = self.clone();
        for (k, v) in o {
            let flag = || v.as_bool().ok_or_else(|| bad(format!("{k} must be true or false")));
            let num = |max: u64| {
                v.as_u64().filter(|n| (1..=max).contains(n)).ok_or_else(|| bad(format!("{k} must be a whole number from 1 to {max}")))
            };
            match k.as_str() {
                "enabled" => c.enabled = flag()?,
                "admission" => c.admission = flag()?,
                "auto_pause" => c.auto_pause = flag()?,
                "warn_only" => c.warn_only = flag()?,
                "high_free_pct" => c.high_free_pct = num(90)? as u8,
                "crit_free_pct" => c.crit_free_pct = num(90)? as u8,
                "high_free_mb" => c.high_free_mb = num(MAX_MB)?,
                "crit_free_mb" => c.crit_free_mb = num(MAX_MB)?,
                "kernel_pool_mb" => {
                    c.kernel_pool_mb = v.as_u64().filter(|n| KERNEL_POOL_RANGE.contains(n)).ok_or_else(|| {
                        bad(format!("{k} must be a whole number from {} to {}", KERNEL_POOL_RANGE.start(), KERNEL_POOL_RANGE.end()))
                    })?
                }
                other => return Err(bad(format!("unknown key {other} (known: {})", KEYS.join(", ")))),
            }
        }
        if c.crit_free_pct >= c.high_free_pct {
            return Err(bad("crit_free_pct must be below high_free_pct".into()));
        }
        if c.crit_free_mb >= c.high_free_mb {
            return Err(bad("crit_free_mb must be below high_free_mb".into()));
        }
        Ok(c)
    }

    /// Whether admission control acts (not just reports): on, not warn-only.
    pub fn gate_active(&self) -> bool {
        self.enabled && self.admission && !self.warn_only
    }

    /// Whether the pauser acts: on, not warn-only.
    pub fn pause_active(&self) -> bool {
        self.enabled && self.auto_pause && !self.warn_only
    }

    pub fn thresholds(&self) -> Thresholds {
        Thresholds {
            high_pct: self.high_free_pct as f64,
            crit_pct: self.crit_free_pct as f64,
            high_mb: self.high_free_mb,
            crit_mb: self.crit_free_mb,
            ..Thresholds::default()
        }
    }
}

/// The lines the state machine uses. Pressure starts when free memory is at or below the percentage OR the megabytes;
/// it ends only when free memory is above both plus the margin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thresholds {
    pub high_pct: f64,
    pub crit_pct: f64,
    pub high_mb: u64,
    pub crit_mb: u64,
    /// hysteresis: how far above a line free memory must climb to leave the level
    pub margin_pct: f64,
    pub margin_mb: u64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds { high_pct: 20.0, crit_pct: 10.0, high_mb: 3072, crit_mb: 1536, margin_pct: 5.0, margin_mb: 512 }
    }
}

impl Thresholds {
    fn at_or_below(i: &MemInfo, pct: f64, mb: u64) -> bool {
        i.free_pct() <= pct || i.free_mb <= mb
    }

    fn clear_of(&self, i: &MemInfo, pct: f64, mb: u64) -> bool {
        i.free_pct() > pct + self.margin_pct && i.free_mb > mb + self.margin_mb
    }
}

/// The next level given the current one and a reading. Pure.
pub fn next_level(cur: Level, i: &MemInfo, t: &Thresholds) -> Level {
    let crit_in = Thresholds::at_or_below(i, t.crit_pct, t.crit_mb);
    let high_in = Thresholds::at_or_below(i, t.high_pct, t.high_mb);
    let crit_hold = !t.clear_of(i, t.crit_pct, t.crit_mb);
    let high_hold = !t.clear_of(i, t.high_pct, t.high_mb);
    match cur {
        Level::Critical if crit_hold => Level::Critical,
        Level::Critical => {
            if high_hold {
                Level::High
            } else {
                Level::Normal
            }
        }
        Level::High if crit_in => Level::Critical,
        Level::High if high_hold => Level::High,
        Level::High => Level::Normal,
        Level::Normal if crit_in => Level::Critical,
        Level::Normal if high_in => Level::High,
        Level::Normal => Level::Normal,
    }
}

/// Worst-case rate memory is assumed to be eaten at, for the Normal deadline (a build can take GBs in seconds).
const BURN_MB_PER_S: u64 = 1024;
const MIN_WAIT: Duration = Duration::from_secs(1);
/// Longest wait in Normal when the platform has an event (a backstop) and when it has none.
const MAX_WAIT_EVENT: Duration = Duration::from_secs(60);
const MAX_WAIT_BLIND: Duration = Duration::from_secs(15);
/// While the level is not Normal the wait doubles from `MIN_WAIT` up to this while nothing changes.
const MAX_WAIT_ELEVATED: Duration = Duration::from_secs(10);

/// How long the thread may sleep before it must look again if no event comes. Normal: the time memory would need to
/// reach the High line at `BURN_MB_PER_S`, between `MIN_WAIT` and the cap. Not Normal: bounded exponential backoff on
/// `quiet` (consecutive looks without a level change).
pub fn next_deadline(level: Level, i: &MemInfo, t: &Thresholds, quiet: u32, has_events: bool) -> Duration {
    if level != Level::Normal {
        return (MIN_WAIT * 2u32.saturating_pow(quiet.min(8))).min(MAX_WAIT_ELEVATED);
    }
    let line_mb = (i.total_mb as f64 * t.high_pct / 100.0).max(t.high_mb as f64) as u64;
    let headroom = i.free_mb.saturating_sub(line_mb);
    let cap = if has_events { MAX_WAIT_EVENT } else { MAX_WAIT_BLIND };
    Duration::from_secs(headroom / BURN_MB_PER_S).clamp(MIN_WAIT, cap)
}

/// Why `MemSource::wait_change` returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    /// the OS said memory changed state
    Event,
    /// the deadline passed
    Timeout,
    /// the stop token was set
    Stop,
}

/// Stop switch of the governor thread. `trigger` sets the flag and wakes `sleep`; a source with its own wait also
/// gets `MemSource::wake`.
#[derive(Default)]
pub struct Stop {
    flag: AtomicBool,
    m: Mutex<()>,
    cv: Condvar,
}

impl Stop {
    pub fn is_set(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    pub fn trigger(&self) {
        self.flag.store(true, Ordering::SeqCst);
        let _g = self.m.lock_recover();
        self.cv.notify_all();
    }

    /// Sleeps up to `d`; true when stopped.
    pub fn sleep(&self, d: Duration) -> bool {
        let g = self.m.lock_recover();
        if self.is_set() {
            return true;
        }
        let _ = self.cv.wait_timeout(g, d);
        self.is_set()
    }
}

/// Where memory readings and the OS's pressure signal come from. Tests use `fake::FakeSource`.
pub trait MemSource: Send + Sync {
    /// Current memory; `None` when it could not be read (the level then stays as it is).
    fn snapshot(&self) -> Option<MemInfo>;
    /// Blocks until the OS signals a memory change, `stop` is triggered, or `timeout` passes.
    fn wait_change(&self, stop: &Stop, timeout: Duration) -> Wake;
    /// Makes a blocked `wait_change` return (called after `Stop::trigger`).
    fn wake(&self) {}
    /// Whether `wait_change` can return `Event` (false: the deadline is its only wake).
    fn has_events(&self) -> bool;
    fn name(&self) -> &'static str;
}

/// The best source this OS offers; falls back to the adaptive sampler when the event cannot be set up.
pub fn os_source() -> Arc<dyn MemSource> {
    #[cfg(windows)]
    match win::WinSource::new() {
        Ok(s) => return Arc::new(s),
        Err(e) => eprintln!("governor: memory notification unavailable ({e}); sampling instead"),
    }
    #[cfg(target_os = "linux")]
    match linux::PsiSource::new() {
        Ok(s) => return Arc::new(s),
        Err(e) => eprintln!("governor: PSI unavailable ({e}); sampling instead"),
    }
    Arc::new(Sampler::new())
}

/// A level change, handed to the `on_change` callback after the state lock is released.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub from: Level,
    pub to: Level,
    pub info: MemInfo,
}

struct State {
    level: Level,
    info: MemInfo,
    cfg: Config,
    /// bumped on every level change: waiters compare it to the one they saw
    gen: u64,
    /// set by `stop`: every waiter returns at once
    closed: bool,
}

/// Callback run on each level change (outside the state lock).
pub type OnChange = Arc<dyn Fn(&Change) + Send + Sync>;
/// Callback run on each wake of the governor thread (no argument; it reads what it needs).
pub type OnTick = Arc<dyn Fn() + Send + Sync>;

pub struct Governor {
    st: Mutex<State>,
    cv: Condvar,
    stop: Arc<Stop>,
    source: Mutex<Option<Arc<dyn MemSource>>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// why memory is short: the breakdown source and the kernel / load alert state
    pub cause: cause::Monitor,
    /// run on each wake of the governor thread, after the reading is fed (the alert evaluation)
    tick: Mutex<Option<OnTick>>,
}

impl Governor {
    /// A governor that has not started: Normal, no reading. `start` runs the thread.
    pub fn new(cfg: Config) -> Governor {
        Governor {
            st: Mutex::new(State { level: Level::Normal, info: MemInfo::default(), cfg, gen: 0, closed: false }),
            cv: Condvar::new(),
            stop: Arc::new(Stop::default()),
            source: Mutex::new(None),
            thread: Mutex::new(None),
            cause: cause::Monitor::os(),
            tick: Mutex::new(None),
        }
    }

    /// Sets what runs on each wake of the thread (before `start`).
    pub fn set_tick(&self, f: OnTick) {
        *self.tick.lock_recover() = Some(f);
    }

    pub fn level(&self) -> Level {
        self.st.lock_recover().level
    }

    /// The last reading.
    pub fn snapshot(&self) -> MemInfo {
        self.st.lock_recover().info
    }

    pub fn config(&self) -> Config {
        self.st.lock_recover().cfg.clone()
    }

    pub fn thresholds(&self) -> Thresholds {
        self.st.lock_recover().cfg.thresholds()
    }

    /// Level and reading together.
    pub fn state(&self) -> (Level, MemInfo) {
        let s = self.st.lock_recover();
        (s.level, s.info)
    }

    /// Number of level changes so far; pass it to `wait_change`.
    pub fn generation(&self) -> u64 {
        self.st.lock_recover().gen
    }

    /// Blocks until the level changes after generation `seen`, or `timeout` passes. Returns the level and the
    /// generation now. Admission waiters use this: they sleep until the governor says something changed.
    pub fn wait_change(&self, seen: u64, timeout: Duration) -> (Level, u64) {
        let g = self.st.lock_recover();
        let (g, _) = self.cv.wait_timeout_while(g, timeout, |s| s.gen == seen && !s.closed).unwrap_or_else(|e| e.into_inner());
        (g.level, g.gen)
    }

    /// Whether `stop` was called.
    pub fn is_closed(&self) -> bool {
        self.st.lock_recover().closed
    }

    /// Wakes everyone in `wait_change` without a level change (a setting moved: they look again).
    pub fn poke(&self) {
        self.st.lock_recover().gen += 1;
        self.cv.notify_all();
    }

    /// Replaces the config (the caller saved it) and applies it to the last reading at once.
    pub fn set_config(&self, cfg: Config, on_change: Option<&OnChange>) {
        let ch = {
            let mut s = self.st.lock_recover();
            s.cfg = cfg;
            let info = s.info;
            Self::apply_locked(&mut s, &self.cv, info)
        };
        if let (Some(ch), Some(f)) = (ch, on_change) {
            f(&ch);
        }
    }

    /// Feeds one reading to the state machine. Returns the change, when the level moved.
    pub fn feed(&self, info: MemInfo) -> Option<Change> {
        let mut s = self.st.lock_recover();
        Self::apply_locked(&mut s, &self.cv, info)
    }

    fn apply_locked(s: &mut State, cv: &Condvar, info: MemInfo) -> Option<Change> {
        s.info = info;
        let to = if s.cfg.enabled { next_level(s.level, &info, &s.cfg.thresholds()) } else { Level::Normal };
        if to == s.level {
            return None;
        }
        let from = std::mem::replace(&mut s.level, to);
        s.gen += 1;
        cv.notify_all();
        Some(Change { from, to, info })
    }

    /// Starts the thread on `source` (once). It reads memory, feeds the state machine, calls `on_change` for each
    /// level change, then waits for an OS event or the computed deadline.
    pub fn start(self: &Arc<Self>, source: Arc<dyn MemSource>, on_change: OnChange) {
        let mut th = self.thread.lock_recover();
        if th.is_some() {
            return;
        }
        *self.source.lock_recover() = Some(source.clone());
        let me = Arc::clone(self);
        *th = std::thread::Builder::new().name("governor".into()).spawn(move || me.run(source, on_change)).ok();
    }

    fn run(&self, source: Arc<dyn MemSource>, on_change: OnChange) {
        let mut quiet = 0u32;
        while !self.stop.is_set() {
            if let Some(info) = source.snapshot() {
                match self.feed(info) {
                    Some(ch) => {
                        quiet = 0;
                        on_change(&ch);
                    }
                    None => quiet = quiet.saturating_add(1),
                }
            }
            let tick = self.tick.lock_recover().clone();
            if let Some(f) = tick {
                f();
            }
            let (level, info) = self.state();
            let wait = next_deadline(level, &info, &self.thresholds(), quiet, source.has_events());
            if source.wait_change(&self.stop, wait) == Wake::Stop {
                break;
            }
        }
    }

    /// Stops the thread and waits for it. Also releases everyone blocked in `wait_change`.
    pub fn stop(&self) {
        self.st.lock_recover().closed = true;
        self.cv.notify_all();
        self.stop.trigger();
        if let Some(s) = self.source.lock_recover().clone() {
            s.wake();
        }
        if let Some(h) = self.thread.lock_recover().take() {
            let _ = h.join();
        }
    }
}

/// Worker limit under pressure. Normal: the configured `limit`. High: half of it (at least 1). Critical: 1.
pub fn effective_worker_limit(limit: usize, level: Level) -> usize {
    match level {
        Level::Normal => limit,
        Level::High => (limit / 2).max(1).min(limit),
        Level::Critical => 1.min(limit),
    }
}

/// What kind of spawn is asking to start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnKind {
    /// an ephemeral worker (counts against the worker limit)
    pub worker: bool,
    /// the request came over an agent's connection (an agent starting something), not the app or the user's CLI
    pub by_agent: bool,
    /// the new session is an agent (not a plain terminal)
    pub agent: bool,
    /// workers already running
    pub live_workers: usize,
}

/// Whether a new session must wait for memory. Pure. The user's own terminals and sessions started by the app are never
/// refused. At High and Critical: an agent started by an agent is refused outright; a worker is refused once the
/// (lowered) worker limit is reached.
pub fn admission_denied(cfg: &Config, level: Level, limit: usize, k: SpawnKind) -> bool {
    if !cfg.gate_active() || level == Level::Normal {
        return false;
    }
    if k.agent && k.by_agent && !k.worker {
        return true;
    }
    k.worker && k.live_workers >= effective_worker_limit(limit, level)
}

#[cfg(test)]
pub mod fake {
    use super::*;

    /// A memory source the test drives: `set` changes the reading and wakes the thread like an OS event would.
    pub struct FakeSource {
        st: Mutex<(MemInfo, u64)>,
        cv: Condvar,
        /// how many times the thread read memory
        pub reads: std::sync::atomic::AtomicUsize,
    }

    impl FakeSource {
        pub fn new(i: MemInfo) -> Arc<FakeSource> {
            Arc::new(FakeSource { st: Mutex::new((i, 0)), cv: Condvar::new(), reads: Default::default() })
        }

        pub fn set(&self, i: MemInfo) {
            let mut g = self.st.lock_recover();
            *g = (i, g.1 + 1);
            self.cv.notify_all();
        }
    }

    impl MemSource for FakeSource {
        fn snapshot(&self) -> Option<MemInfo> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Some(self.st.lock_recover().0)
        }

        fn wait_change(&self, stop: &Stop, timeout: Duration) -> Wake {
            let seen = self.st.lock_recover().1;
            let g = self.st.lock_recover();
            let (_g, r) = self.cv.wait_timeout_while(g, timeout, |s| s.1 == seen && !stop.is_set()).unwrap_or_else(|e| e.into_inner());
            if stop.is_set() {
                Wake::Stop
            } else if r.timed_out() {
                Wake::Timeout
            } else {
                Wake::Event
            }
        }

        fn wake(&self) {
            let _g = self.st.lock_recover();
            self.cv.notify_all();
        }

        fn has_events(&self) -> bool {
            true
        }

        fn name(&self) -> &'static str {
            "fake"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeSource;
    use super::*;
    use std::time::Instant;

    fn mi(total_gb: u64, free_mb: u64) -> MemInfo {
        MemInfo { total_mb: total_gb * 1024, free_mb, commit_pct: None }
    }

    #[test]
    fn state_machine_table_with_hysteresis() {
        let t = Thresholds::default();
        use Level::*;
        // 16 GB: High line max(20% = 3276, 3072) = 3276 MB; Critical line max(10% = 1638, 1536) = 1638 MB
        // 64 GB: percentages dominate nowhere; the MB lines (3072 / 1536) are below 20% / 10% of 64 GB (13107 / 6553)
        let table: &[(Level, MemInfo, Level)] = &[
            (Normal, mi(16, 8000), Normal),
            (Normal, mi(16, 3276), High),   // at the line: enters
            (Normal, mi(16, 3277), Normal), // one above: not yet
            (Normal, mi(16, 1638), Critical),
            (Normal, mi(16, 100), Critical),
            (High, mi(16, 3500), High), // above the line, inside the margin (3276 + 5% of 16 GB = 4096 and +512)
            (High, mi(16, 4000), High),
            (High, mi(16, 4200), Normal), // clear of both margins
            (High, mi(16, 1500), Critical),
            (Critical, mi(16, 1700), Critical), // above the Critical line, inside its margin
            (Critical, mi(16, 2500), High),     // clear of Critical, still under the High margin
            (Critical, mi(16, 5000), Normal),   // clear of both
            // small machine, the MB lines rule: 4 GB, High at 3072 MB (75%)
            (Normal, mi(4, 3000), High),
            (Normal, mi(4, 1400), Critical),
            // big machine, the percentage rules
            (Normal, mi(64, 13000), High),
            (Normal, mi(64, 20000), Normal),
            // unknown total never raises pressure by percentage, the MB line still does
            (Normal, MemInfo { total_mb: 0, free_mb: 8000, commit_pct: None }, Normal),
        ];
        for (cur, info, want) in table {
            assert_eq!(next_level(*cur, info, &t), *want, "{cur:?} with free {} of {} MB", info.free_mb, info.total_mb);
        }
    }

    #[test]
    fn hovering_at_the_line_does_not_flap() {
        let t = Thresholds::default();
        let mut l = Level::Normal;
        let mut changes = 0;
        for free in [3300, 3200, 3300, 3400, 3200, 3600, 3276, 3900, 3500, 3100] {
            let n = next_level(l, &mi(16, free), &t);
            if n != l {
                changes += 1;
            }
            l = n;
        }
        assert_eq!(changes, 1, "entered High once and stayed");
        assert_eq!(l, Level::High);
    }

    #[test]
    fn deadline_shrinks_near_the_line_and_backs_off_when_elevated() {
        let t = Thresholds::default();
        let far = next_deadline(Level::Normal, &mi(256, 250_000), &t, 0, true);
        let near = next_deadline(Level::Normal, &mi(16, 4000), &t, 0, true);
        assert_eq!(far, MAX_WAIT_EVENT);
        assert_eq!(near, MIN_WAIT);
        assert_eq!(next_deadline(Level::Normal, &mi(256, 250_000), &t, 0, false), MAX_WAIT_BLIND);
        let steps: Vec<_> = (0..6).map(|q| next_deadline(Level::High, &mi(16, 2000), &t, q, true).as_secs()).collect();
        assert_eq!(steps, [1, 2, 4, 8, 10, 10]);
    }

    #[test]
    fn config_patch_validates_and_roundtrips_through_settings() {
        let c = Config::default();
        let p = c.patch(&json!({"high_free_pct": 30, "warn_only": true})).unwrap();
        assert_eq!((p.high_free_pct, p.warn_only, p.crit_free_pct), (30, true, 10));
        for bad in [
            json!({"nope": 1}),
            json!({"enabled": "yes"}),
            json!({"high_free_pct": 0}),
            json!({"high_free_pct": 95}),
            json!({"crit_free_pct": 20}),
            json!({"crit_free_mb": 4000}),
            json!({"kernel_pool_mb": 255}),
            json!({"kernel_pool_mb": 1_048_577}),
            json!({"kernel_pool_mb": 0}),
            json!({"kernel_pool_mb": "big"}),
            json!(3),
        ] {
            let e = c.patch(&bad).unwrap_err();
            assert_eq!(axelstra_core::msg::find(&e).map(|c| c.code.as_str()), Some("governor.bad_config"), "{bad}");
        }
        assert_eq!(c.kernel_pool_mb, 3072);
        assert_eq!(c.patch(&json!({"kernel_pool_mb": 256})).unwrap().kernel_pool_mb, 256);
        assert_eq!(c.patch(&json!({"kernel_pool_mb": 1_048_576})).unwrap().kernel_pool_mb, 1_048_576);
        let mut settings = json!({"shell": "bash"});
        p.into_settings(&mut settings);
        assert_eq!(settings["shell"], "bash");
        assert_eq!(Config::from_settings(&settings), p);
        // missing key, junk, and one bad field among good ones
        assert_eq!(Config::from_settings(&json!({})), Config::default());
        assert_eq!(Config::from_settings(&json!({"governor": 7})), Config::default());
        let mixed = Config::from_settings(&json!({"governor": {"high_free_pct": 99, "warn_only": true}}));
        assert!(mixed.warn_only);
        assert_eq!(mixed.high_free_pct, 20);
    }

    #[test]
    fn config_persists_in_a_settings_file() {
        let dir = std::env::temp_dir().join(format!("axl-gov-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let mut saved = json!({});
        Config::default().patch(&json!({"crit_free_mb": 1000, "auto_pause": false})).unwrap().into_settings(&mut saved);
        crate::store::write_json(&path, &saved).unwrap();
        let back: Value = crate::store::read_json(&path).unwrap().unwrap();
        let c = Config::from_settings(&back);
        assert_eq!((c.crit_free_mb, c.auto_pause), (1000, false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn collector() -> (OnChange, Arc<Mutex<Vec<Change>>>) {
        let seen = Arc::new(Mutex::new(vec![]));
        let s = seen.clone();
        (Arc::new(move |c: &Change| s.lock_recover().push(c.clone())), seen)
    }

    /// Waits until the governor is at `want`.
    fn until(g: &Governor, want: Level) {
        let t = Instant::now();
        while g.level() != want {
            assert!(t.elapsed() < Duration::from_secs(10), "never reached {want:?}, at {:?}", g.level());
            g.wait_change(g.generation(), Duration::from_millis(50));
        }
    }

    #[test]
    fn one_event_per_transition_and_the_thread_stops() {
        let src = FakeSource::new(mi(16, 8000));
        let g = Arc::new(Governor::new(Config::default()));
        let (cb, seen) = collector();
        g.start(src.clone(), cb);
        // a first reading at Normal announces nothing
        let t = Instant::now();
        while src.reads.load(Ordering::SeqCst) == 0 {
            assert!(t.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(5));
        }
        let gen0 = g.generation();
        src.set(mi(16, 3000));
        until(&g, Level::High);
        // more readings inside High change nothing
        src.set(mi(16, 2800));
        src.set(mi(16, 2000));
        src.set(mi(16, 1000));
        until(&g, Level::Critical);
        src.set(mi(16, 900));
        src.set(mi(16, 12_000));
        until(&g, Level::Normal);
        g.stop();
        let ev = seen.lock_recover().clone();
        let path: Vec<_> = ev.iter().map(|c| (c.from, c.to)).collect();
        assert_eq!(
            path,
            [(Level::Normal, Level::High), (Level::High, Level::Critical), (Level::Critical, Level::Normal)],
            "exactly one event per transition"
        );
        assert_eq!(g.generation(), gen0 + 3);
        assert_eq!(ev[1].info.free_mb, 1000);
        // after stop the thread is gone: another reading is not looked at
        let reads = src.reads.load(Ordering::SeqCst);
        src.set(mi(16, 10));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(src.reads.load(Ordering::SeqCst), reads);
    }

    #[test]
    fn stop_joins_a_thread_that_is_waiting() {
        let g = Arc::new(Governor::new(Config::default()));
        let (cb, _) = collector();
        g.start(FakeSource::new(mi(16, 8000)), cb);
        std::thread::sleep(Duration::from_millis(50));
        let t = Instant::now();
        g.stop();
        assert!(t.elapsed() < Duration::from_secs(2), "stop waited for the deadline");
        assert!(g.thread.lock_recover().is_none());
    }

    #[test]
    fn waiters_wake_by_event_when_the_level_changes() {
        let g = Arc::new(Governor::new(Config::default()));
        let seen = g.generation();
        let g2 = g.clone();
        let h = std::thread::spawn(move || g2.wait_change(seen, Duration::from_secs(30)));
        std::thread::sleep(Duration::from_millis(50));
        let t = Instant::now();
        assert!(g.feed(mi(16, 100)).is_some());
        let (level, gen) = h.join().unwrap();
        assert_eq!((level, gen), (Level::Critical, seen + 1));
        assert!(t.elapsed() < Duration::from_secs(5));
        // an unchanged level wakes nobody: the call times out
        assert_eq!(g.wait_change(gen, Duration::from_millis(30)), (Level::Critical, gen));
    }

    #[test]
    fn disabled_stays_normal_and_config_applies_to_the_last_reading() {
        let g = Governor::new(Config::default());
        assert!(g.feed(mi(16, 2000)).is_some());
        let (cb, seen) = collector();
        let off = Config { enabled: false, ..Config::default() };
        g.set_config(off.clone(), Some(&cb));
        assert_eq!(g.level(), Level::Normal);
        assert_eq!(seen.lock_recover().len(), 1);
        assert!(g.feed(mi(16, 100)).is_none(), "disabled: no level, no event");
        // a threshold moved below the reading clears High; no source involved
        g.set_config(Config::default(), Some(&cb));
        assert_eq!(g.level(), Level::Critical);
        g.feed(mi(16, 3000));
        assert_eq!(g.level(), Level::High);
        g.set_config(Config { high_free_pct: 5, crit_free_pct: 2, high_free_mb: 200, crit_free_mb: 100, ..Config::default() }, Some(&cb));
        assert_eq!(g.level(), Level::Normal);
    }

    #[test]
    fn admission_is_denied_at_high_and_critical_and_allowed_at_normal() {
        let on = Config::default();
        let agent_by_agent = SpawnKind { worker: false, by_agent: true, agent: true, live_workers: 0 };
        let terminal_by_agent = SpawnKind { worker: false, by_agent: true, agent: false, live_workers: 0 };
        let by_user = SpawnKind { worker: false, by_agent: false, agent: true, live_workers: 0 };
        let worker = |live| SpawnKind { worker: true, by_agent: true, agent: true, live_workers: live };
        use Level::*;
        assert!(!admission_denied(&on, Normal, 6, agent_by_agent));
        assert!(!admission_denied(&on, Normal, 6, worker(5)));
        assert!(admission_denied(&on, High, 6, agent_by_agent));
        assert!(admission_denied(&on, Critical, 6, agent_by_agent));
        // the user's own agents and terminals are never refused
        assert!(!admission_denied(&on, Critical, 6, by_user));
        assert!(!admission_denied(&on, Critical, 6, terminal_by_agent));
        // the worker limit drops: High half, Critical one
        assert!(!admission_denied(&on, High, 6, worker(2)));
        assert!(admission_denied(&on, High, 6, worker(3)));
        assert!(!admission_denied(&on, Critical, 6, worker(0)));
        assert!(admission_denied(&on, Critical, 6, worker(1)));
        assert_eq!((effective_worker_limit(1, High), effective_worker_limit(0, Critical), effective_worker_limit(6, Normal)), (1, 0, 6));
        // switched off, warn-only or admission off: nothing is refused
        for cfg in
            [Config { enabled: false, ..on.clone() }, Config { warn_only: true, ..on.clone() }, Config { admission: false, ..on.clone() }]
        {
            assert!(!admission_denied(&cfg, Critical, 6, agent_by_agent));
            assert!(!admission_denied(&cfg, Critical, 6, worker(9)));
        }
    }
}
