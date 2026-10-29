//! Why memory is short: the breakdown `mem.status` carries (`breakdown`) and the kernel-pool alert.
//!
//! Everything reads through `CauseSource`, so the math (`breakdown`) and the alert (`KernelWatch`) are pure and tests inject
//! fakes. The OS source (`OsCause`) reads kernel pools, cache, compressed memory and the per-process table with handle counts.
//! Nothing here runs on a timer: the alert is evaluated on the governor's own wake (one cheap read of the kernel pools) and the
//! full breakdown only when `mem.status` asks or when an episode starts (to name the processes holding most handles).

use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Kernel memory in MB. `alert_mb` is what the alert compares: paged + nonpaged on Windows; on Linux only the unreclaimable
/// slab (the reclaimable part is a cache the kernel gives back, not a leak).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct KernelMem {
    pub paged_mb: u64,
    pub nonpaged_mb: u64,
    pub alert_mb: u64,
}

impl KernelMem {
    pub fn total_mb(&self) -> u64 {
        self.paged_mb + self.nonpaged_mb
    }
}

/// One process of the machine's table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcRow {
    pub pid: u32,
    pub ppid: u32,
    /// lowercase, no directory, no `.exe`
    pub name: String,
    pub rss_kib: u64,
    pub handles: Option<u32>,
    /// cumulative CPU time (user + kernel) in 100 ns units
    pub cpu_100ns: Option<u64>,
    /// cumulative bytes read + written
    pub io_bytes: Option<u64>,
}

/// Cumulative CPU time of the machine, any unit (only ratios are used). `kernel` excludes idle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CpuTimes {
    pub idle: u64,
    pub kernel: u64,
    pub user: u64,
}

/// What the OS counts for the whole machine. Cumulative values turn into rates between two readings (`LoadWatch`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SysCounters {
    pub cpu: Option<CpuTimes>,
    /// pages moved to or from disk since boot (Windows: page reads + dirty/mapped writes; Linux: major faults)
    pub pages: Option<u64>,
    pub commit_mb: Option<u64>,
    pub commit_limit_mb: Option<u64>,
    pub modified_mb: Option<u64>,
}

/// Where the numbers come from. Every method answers `None`/empty when the OS does not say.
pub trait CauseSource: Send + Sync {
    fn kernel(&self) -> Option<KernelMem>;
    fn cache_mb(&self) -> Option<u64>;
    /// Compressed memory; `rows` is the process table already taken (Windows keeps it in the `Memory Compression` process).
    fn compressed_mb(&self, rows: &[ProcRow]) -> Option<u64>;
    /// Handles open on the whole system, when the OS has one number for it.
    fn handle_total(&self) -> Option<u64> {
        None
    }
    /// Machine counters read directly from the OS (never PDH / `Get-Counter`, which fail on some installs).
    fn system(&self) -> Option<SysCounters> {
        None
    }
    fn procs(&self) -> Vec<ProcRow>;
    /// Kernel pool use per allocation tag (Windows only; `None` when the OS refuses or does not have it).
    fn pool_tags(&self) -> Option<Vec<PoolTag>> {
        None
    }
}

/// The process whose working set IS the compressed memory (Windows).
pub const COMPRESSION_PROC: &str = "memory compression";
/// `handles` is filled from this many handles on the machine, or this many in one process.
pub const HANDLES_TOTAL_MIN: u64 = 200_000;
pub const HANDLES_ONE_MIN: u64 = 50_000;
const TOP_EXTERNAL: usize = 8;
const TOP_HANDLES: usize = 5;

/// What an axelstra session is, for the sums.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Main,
    Lead,
    Agent,
    Worker,
    Terminal,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Main => "main",
            Tier::Lead => "lead",
            Tier::Agent => "agent",
            Tier::Worker => "worker",
            Tier::Terminal => "terminal",
        }
    }
}

/// Whose a process is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Session(Tier),
    /// under `axelstra lock` (`mem.hold`); counted apart from the session it runs in
    Build,
    External,
}

/// Pool bytes one allocation tag holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolTag {
    pub tag: String,
    pub paged: u64,
    pub nonpaged: u64,
}

/// `SystemPoolTagInformation` (class 22): a `ULONG` count, 4 bytes of padding, then `SYSTEM_POOLTAG` entries of 40 bytes
/// (x64): `Tag[4]` at 0, `PagedUsed` (u64) at 16, `NonPagedUsed` (u64) at 32. `None` when the buffer is shorter than its own
/// count says. Tags that hold nothing are dropped. Pure.
pub fn parse_pool_tags(buf: &[u8]) -> Option<Vec<PoolTag>> {
    const HEAD: usize = 8;
    const STRIDE: usize = 40;
    let count = u32::from_le_bytes(buf.get(0..4)?.try_into().ok()?) as usize;
    if buf.len() < HEAD.checked_add(count.checked_mul(STRIDE)?)? {
        return None;
    }
    let u64_at = |o: usize| u64::from_le_bytes(buf[o..o + 8].try_into().unwrap());
    Some(
        (0..count)
            .filter_map(|i| {
                let o = HEAD + i * STRIDE;
                let (paged, nonpaged) = (u64_at(o + 16), u64_at(o + 32));
                (paged + nonpaged > 0).then(|| PoolTag {
                    tag: buf[o..o + 4].iter().map(|b| if b.is_ascii_graphic() || *b == b' ' { *b as char } else { '?' }).collect(),
                    paged,
                    nonpaged,
                })
            })
            .collect(),
    )
}

/// What a tag usually means: a short id the frontend translates. Only tags whose owner is well known; others have none.
pub fn pool_owner(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "FMfn" => "filter",
        "Toke" => "tokens",
        "File" => "files",
        "Ntfc" => "watchers",
        "Ntff" => "ntfs",
        "MmSt" => "mm",
        "IoNm" => "io",
        "EtwB" => "etw",
        _ => return None,
    })
}

/// The owner id in English (CLI and logs); the app has its own texts.
pub fn owner_text(owner: &str) -> &'static str {
    match owner {
        "filter" => "file-name cache of the filter manager (antivirus and other file filters)",
        "tokens" => "security tokens (many processes started)",
        "files" => "open file objects",
        "watchers" => "folder change notifications (watchers)",
        "ntfs" => "NTFS file structures",
        "mm" => "memory manager",
        "io" => "I/O name buffers",
        "etw" => "event tracing (ETW) buffers",
        _ => "",
    }
}

const TOP_TAGS: usize = 8;

/// The `pool_tags` of the breakdown: the 8 biggest tags, and what the pool counters hold beyond the sum of all tags
/// (`sem_etiqueta_mb`: large allocations and special pools carry no tag). Always `partial`: it is an attribution, not a ledger.
pub fn pool_tags_json(tags: &[PoolTag], kernel: Option<KernelMem>) -> Value {
    let mut v: Vec<&PoolTag> = tags.iter().collect();
    v.sort_by(|a, b| (b.paged + b.nonpaged).cmp(&(a.paged + a.nonpaged)).then(a.tag.cmp(&b.tag)));
    let (tp, tn) = tags.iter().fold((0u64, 0u64), |a, t| (a.0 + t.paged, a.1 + t.nonpaged));
    let (tp_mb, tn_mb) = (tp / MIB, tn / MIB);
    let total = kernel.map(|k| k.total_mb());
    json!({
        "top": v.iter().take(TOP_TAGS).map(|t| json!({
            "tag": t.tag, "paged_mb": t.paged / MIB, "nonpaged_mb": t.nonpaged / MIB, "owner": pool_owner(&t.tag),
        })).collect::<Vec<_>>(),
        "tagged_mb": tp_mb + tn_mb,
        "sem_etiqueta_mb": total.map(|t| t.saturating_sub(tp_mb + tn_mb)),
        "partial": true,
    })
}

const MIB: u64 = 1024 * 1024;

pub struct Inputs<'a> {
    pub total_mb: u64,
    pub free_mb: u64,
    pub kernel: Option<KernelMem>,
    pub cache_mb: Option<u64>,
    pub compressed_mb: Option<u64>,
    pub handle_total: Option<u64>,
    pub rows: &'a [ProcRow],
    /// live sessions (for the counts; a session whose memory the OS hides still counts)
    pub sessions: &'a [Tier],
    pub builds: usize,
    pub classify: &'a dyn Fn(&ProcRow) -> Class,
    pub baseline_mb: Option<u64>,
    pub kernel_high: bool,
    /// kernel pool per tag, when the OS gives it
    pub pool_tags: Option<Vec<PoolTag>>,
    /// what Axelstra itself adds: `own_json`
    pub own: Value,
    /// `system_json` of the counters and rates now
    pub system: Value,
    /// (top_cpu, top_io) from `top_rates`, only measured while alerting
    pub top: Option<(Vec<Value>, Vec<Value>)>,
}

fn mb(kib: u64) -> u64 {
    kib / 1024
}

/// The `breakdown` of `mem.status`. Pure. `others_mb` is the memory in use (total minus free) that nothing above explains:
/// the cache is not subtracted because `free_mb` (available memory) already counts the reclaimable cache as free.
pub fn breakdown(i: &Inputs) -> Value {
    let mut tiers: HashMap<Tier, (u64, u64)> = HashMap::new();
    for t in i.sessions {
        tiers.entry(*t).or_default().0 += 1;
    }
    let mut build_kib = 0u64;
    let mut ext: HashMap<&str, (u64, u64)> = HashMap::new();
    let mut tier_kib: HashMap<Tier, u64> = HashMap::new();
    let compressed_is_proc = i.compressed_mb.is_some();
    for r in i.rows {
        match (i.classify)(r) {
            Class::Session(t) => *tier_kib.entry(t).or_default() += r.rss_kib,
            Class::Build => build_kib += r.rss_kib,
            Class::External => {
                if compressed_is_proc && r.name == COMPRESSION_PROC {
                    continue;
                }
                let e = ext.entry(r.name.as_str()).or_default();
                e.0 += 1;
                e.1 += r.rss_kib;
            }
        }
    }
    let mut sessions: Vec<Value> = [Tier::Main, Tier::Lead, Tier::Agent, Tier::Worker, Tier::Terminal]
        .into_iter()
        .filter_map(|t| {
            let count = tiers.get(&t).map(|x| x.0).unwrap_or(0);
            let kib = tier_kib.get(&t).copied().unwrap_or(0);
            (count > 0 || kib > 0).then(|| json!({"tier": t.as_str(), "count": count, "rss_mb": mb(kib)}))
        })
        .collect();
    sessions.shrink_to_fit();
    let mut external: Vec<(&str, u64, u64)> = ext.into_iter().map(|(n, (c, k))| (n, c, k)).collect();
    external.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(b.0)));
    // `others` counts the externals outside the top 8 too: they are not named, so they are not explained
    let listed_kib: u64 = external.iter().take(TOP_EXTERNAL).map(|e| e.2).sum();
    let external_json: Vec<Value> =
        external.iter().take(TOP_EXTERNAL).map(|(n, c, k)| json!({"name": n, "count": c, "rss_mb": mb(*k)})).collect();

    let sessions_mb: u64 = tier_kib.values().map(|k| mb(*k)).sum();
    let used = i.total_mb.saturating_sub(i.free_mb);
    let explained =
        i.kernel.map(|k| k.total_mb()).unwrap_or(0) + i.compressed_mb.unwrap_or(0) + sessions_mb + mb(build_kib) + mb(listed_kib);
    let others_mb = used.saturating_sub(explained);

    let handle_sum: u64 = i.rows.iter().filter_map(|r| r.handles).map(u64::from).sum();
    let handle_total = i.handle_total.unwrap_or(handle_sum);
    let any_handles = i.rows.iter().any(|r| r.handles.is_some());
    let biggest = i.rows.iter().filter_map(|r| r.handles).max().unwrap_or(0) as u64;
    let handles = if any_handles && (i.kernel_high || handle_total > HANDLES_TOTAL_MIN || biggest > HANDLES_ONE_MIN) {
        Some(json!({"total": handle_total, "top": top_handles(i.rows)}))
    } else {
        None
    };
    let handles_present = handles.is_some();

    json!({
        "kernel": i.kernel.map(|k| json!({
            "paged_mb": k.paged_mb, "nonpaged_mb": k.nonpaged_mb, "total_mb": k.total_mb(),
            "baseline_mb": i.baseline_mb, "high": i.kernel_high,
        })),
        "pool_tags": i.pool_tags.as_deref().map(|t| pool_tags_json(t, i.kernel)),
        "own": i.own,
        "cache_mb": i.cache_mb,
        "compressed_mb": i.compressed_mb,
        "sessions": sessions,
        "builds": {"count": i.builds, "rss_mb": mb(build_kib)},
        "external": external_json,
        "others_mb": others_mb,
        "handles": handles,
        "system": i.system,
        "top_cpu": i.top.as_ref().map(|t| &t.0),
        "top_io": i.top.as_ref().map(|t| &t.1),
        "unavailable": unavailable(std::env::consts::OS, &Present {
            kernel: i.kernel.is_some(),
            pool_tags: i.pool_tags.is_some(),
            cache: i.cache_mb.is_some(),
            compressed: i.compressed_mb.is_some(),
            handles: handles_present,
        }),
    })
}

/// Which `breakdown` fields hold a value.
pub struct Present {
    pub kernel: bool,
    pub pool_tags: bool,
    pub cache: bool,
    pub compressed: bool,
    pub handles: bool,
}

/// Reason code for a `null` field of the breakdown, so a reader can tell "this OS has no such number" from "should be
/// here but could not be read" from "left out on purpose":
/// `unsupported_os` (no source on this OS), `read_failed` (the OS has it, the read gave nothing),
/// `not_needed` (`handles`: only listed when the kernel is high or counts are big).
pub const UNSUPPORTED_OS: &str = "unsupported_os";
pub const READ_FAILED: &str = "read_failed";
pub const NOT_NEEDED: &str = "not_needed";

/// Whether `os` (`std::env::consts::OS`) has a source for `field`.
fn supported(os: &str, field: &str) -> bool {
    matches!(
        (field, os),
        ("kernel" | "compressed_mb" | "cache_mb", "windows" | "linux")
            | ("cache_mb" | "compressed_mb", "macos")
            | ("pool_tags" | "handles", "windows")
    )
}

/// `breakdown.unavailable`: `{field: reason}` for each null field only (`{}` when all are present). Pure; additive.
pub fn unavailable(os: &str, p: &Present) -> Value {
    let mut m = serde_json::Map::new();
    for (field, there) in
        [("kernel", p.kernel), ("pool_tags", p.pool_tags), ("cache_mb", p.cache), ("compressed_mb", p.compressed), ("handles", p.handles)]
    {
        if there {
            continue;
        }
        let reason = match (supported(os, field), field) {
            (false, _) => UNSUPPORTED_OS,
            (true, "handles") => NOT_NEEDED,
            (true, _) => READ_FAILED,
        };
        m.insert(field.to_string(), json!(reason));
    }
    Value::Object(m)
}

/// What this daemon itself adds to the kernel pools: children started (each takes a security token and file objects) and
/// directories watched (each is a change-notification handle). `spawns`/`secs` come from `core::proc::spawn_stats`.
pub fn own_json(spawns: u64, secs: u64, watches: u64) -> Value {
    let per_min = (secs >= 60).then(|| round1(spawns as f64 * 60.0 / secs as f64));
    json!({"spawns": spawns, "uptime_secs": secs, "spawns_per_min": per_min, "watches": watches})
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// `breakdown.system`: the counters now and the rates the last two governor evaluations gave (`null` before the second).
pub fn system_json(c: Option<&SysCounters>, r: Option<&SysRates>) -> Value {
    json!({
        "cpu_user_pct": r.and_then(|r| r.cpu_user_pct).map(round1),
        "cpu_kernel_pct": r.and_then(|r| r.cpu_kernel_pct).map(round1),
        "commit_mb": c.and_then(|c| c.commit_mb),
        "commit_limit_mb": c.and_then(|c| c.commit_limit_mb),
        "modified_mb": c.and_then(|c| c.modified_mb),
        "pages_per_sec": r.and_then(|r| r.pages_per_sec),
    })
}

/// The five processes that used most CPU (percent of the whole machine) and most I/O (bytes/s) between two process tables
/// `dt_secs` apart. Processes in only one table are skipped. Pure.
pub fn top_rates(a: &[ProcRow], b: &[ProcRow], dt_secs: f64, ncpu: usize) -> (Vec<Value>, Vec<Value>) {
    if dt_secs <= 0.0 {
        return (vec![], vec![]);
    }
    let before: HashMap<u32, &ProcRow> = a.iter().map(|r| (r.pid, r)).collect();
    let mut cpu: Vec<(f64, &ProcRow)> = vec![];
    let mut io: Vec<(u64, &ProcRow)> = vec![];
    for r in b {
        let Some(p) = before.get(&r.pid) else { continue };
        if let (Some(x), Some(y)) = (p.cpu_100ns, r.cpu_100ns) {
            let pct = y.saturating_sub(x) as f64 / 1e7 / dt_secs * 100.0 / ncpu.max(1) as f64;
            if pct >= 0.1 {
                cpu.push((pct, r));
            }
        }
        if let (Some(x), Some(y)) = (p.io_bytes, r.io_bytes) {
            let bps = (y.saturating_sub(x) as f64 / dt_secs) as u64;
            if bps > 0 {
                io.push((bps, r));
            }
        }
    }
    cpu.sort_by(|x, y| y.0.total_cmp(&x.0).then(x.1.pid.cmp(&y.1.pid)));
    io.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.pid.cmp(&y.1.pid)));
    (
        cpu.into_iter().take(TOP_HANDLES).map(|(v, r)| json!({"name": r.name, "pid": r.pid, "value": round1(v)})).collect(),
        io.into_iter().take(TOP_HANDLES).map(|(v, r)| json!({"name": r.name, "pid": r.pid, "value": v})).collect(),
    )
}

/// Kernel CPU above this many percent of the machine, or paging above this many pages/s, for `SUSTAIN` evaluations in a row.
pub const KERNEL_CPU_PCT: f64 = 15.0;
pub const PAGES_PER_SEC_HIGH: u64 = 5000;
pub const SUSTAIN: u32 = 3;
/// A load episode ends when both are under this share of their lines.
const LOAD_REARM_PCT: f64 = 70.0;
/// Readings closer than this are skipped (a rate over a few ms says nothing).
const MIN_DT_SECS: f64 = 0.2;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct SysRates {
    pub cpu_user_pct: Option<f64>,
    pub cpu_kernel_pct: Option<f64>,
    pub pages_per_sec: Option<u64>,
}

/// Rates between two readings `dt` seconds apart. Pure.
pub fn sys_rates(prev: &SysCounters, now: &SysCounters, dt: f64) -> SysRates {
    let mut r = SysRates::default();
    if let (Some(a), Some(b)) = (prev.cpu, now.cpu) {
        let (di, dk, du) = (b.idle.saturating_sub(a.idle), b.kernel.saturating_sub(a.kernel), b.user.saturating_sub(a.user));
        let all = di + dk + du;
        if all > 0 {
            r.cpu_kernel_pct = Some(dk as f64 * 100.0 / all as f64);
            r.cpu_user_pct = Some(du as f64 * 100.0 / all as f64);
        }
    }
    if let (Some(a), Some(b)) = (prev.pages, now.pages) {
        r.pages_per_sec = Some((b.saturating_sub(a) as f64 / dt) as u64);
    }
    r
}

/// Load alert state: the previous reading, the run of evaluations over a line, whether an episode is open.
#[derive(Debug, Default)]
pub struct LoadWatch {
    prev: Option<(std::time::Instant, SysCounters)>,
    streak: u32,
    raised: bool,
    last: Option<SysRates>,
}

impl LoadWatch {
    pub fn last(&self) -> Option<SysRates> {
        self.last
    }

    pub fn raised(&self) -> bool {
        self.raised
    }

    /// One reading. Returns the rates exactly once per episode: when the third evaluation in a row is over a line.
    pub fn observe(&mut self, c: SysCounters, now: std::time::Instant) -> Option<SysRates> {
        let Some((at, p)) = self.prev else {
            self.prev = Some((now, c));
            return None;
        };
        let dt = now.saturating_duration_since(at).as_secs_f64();
        if dt < MIN_DT_SECS {
            return None;
        }
        let r = sys_rates(&p, &c, dt);
        self.prev = Some((now, c));
        self.last = Some(r);
        let (k, pg) = (r.cpu_kernel_pct.unwrap_or(0.0), r.pages_per_sec.unwrap_or(0));
        let over = k > KERNEL_CPU_PCT || pg > PAGES_PER_SEC_HIGH;
        let clear = k < KERNEL_CPU_PCT * LOAD_REARM_PCT / 100.0 && (pg as f64) < PAGES_PER_SEC_HIGH as f64 * LOAD_REARM_PCT / 100.0;
        if over {
            self.streak += 1;
            if !self.raised && self.streak >= SUSTAIN {
                self.raised = true;
                return Some(r);
            }
        } else {
            self.streak = 0;
            if clear {
                self.raised = false;
            }
        }
        None
    }
}

/// What the governor's wake found worth a notice.
#[derive(Clone, Debug, PartialEq)]
pub enum Alert {
    /// kernel pools over the line (MB)
    Kernel(u64),
    Load(SysRates),
}

/// The source plus the alert state, held by the `Governor`. `evaluate` is called on each governor wake (cheap: the pool
/// sizes and the machine counters, no process table).
pub struct Monitor {
    source: std::sync::Mutex<std::sync::Arc<dyn CauseSource>>,
    kernel: std::sync::Mutex<KernelWatch>,
    load: std::sync::Mutex<LoadWatch>,
}

impl Monitor {
    pub fn new(source: std::sync::Arc<dyn CauseSource>) -> Monitor {
        Monitor { source: source.into(), kernel: Default::default(), load: Default::default() }
    }

    pub fn os() -> Monitor {
        Monitor::new(std::sync::Arc::new(OsCause))
    }

    pub fn source(&self) -> std::sync::Arc<dyn CauseSource> {
        lock(&self.source).clone()
    }

    pub fn set_source(&self, s: std::sync::Arc<dyn CauseSource>) {
        *lock(&self.source) = s;
    }

    pub fn evaluate(&self, limit_mb: u64, now: std::time::Instant) -> Vec<Alert> {
        let src = self.source();
        let mut out = vec![];
        if let Some(k) = src.kernel() {
            if lock(&self.kernel).observe(k.alert_mb, limit_mb) {
                out.push(Alert::Kernel(k.alert_mb));
            }
        }
        if let Some(c) = src.system() {
            if let Some(r) = lock(&self.load).observe(c, now) {
                out.push(Alert::Load(r));
            }
        }
        out
    }

    /// (baseline, whether the pool is over its line now) for a reading.
    pub fn kernel_state(&self, now_mb: u64, limit_mb: u64) -> (Option<u64>, bool) {
        let w = lock(&self.kernel);
        (w.baseline(), w.is_high(now_mb, limit_mb))
    }

    /// (rates of the last evaluation, whether a load episode is open)
    pub fn load_last(&self) -> (Option<SysRates>, bool) {
        let w = lock(&self.load);
        (w.last(), w.raised())
    }
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The processes holding most handles, biggest first (ties by pid so the answer is stable).
pub fn top_handles(rows: &[ProcRow]) -> Vec<Value> {
    let mut v: Vec<(&ProcRow, u32)> = rows.iter().filter_map(|r| r.handles.filter(|h| *h > 0).map(|h| (r, h))).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.pid.cmp(&b.0.pid)));
    v.into_iter().take(TOP_HANDLES).map(|(r, h)| json!({"name": r.name, "pid": r.pid, "handles": h})).collect()
}

/// Smallest total (MB) the "twice the baseline" rule can trip at: a machine that starts at 300 MB of pool is not leaking at 600.
pub const REL_FLOOR_MB: u64 = 1024;
/// An episode ends only when the pool falls below this share of the line that started it.
const REARM_PCT: u64 = 85;

/// Kernel-pool alert state: the baseline (first reading, taken when the daemon starts) and whether an episode is open.
#[derive(Debug, Default)]
pub struct KernelWatch {
    baseline: Option<u64>,
    raised: bool,
}

impl KernelWatch {
    pub fn baseline(&self) -> Option<u64> {
        self.baseline
    }

    pub fn raised(&self) -> bool {
        self.raised
    }

    /// The line `now_mb` is judged against: the configured limit, or twice the baseline (with a floor) when that is lower.
    pub fn line(&self, limit_mb: u64) -> u64 {
        match self.baseline {
            Some(b) => limit_mb.min((b * 2).max(REL_FLOOR_MB)),
            None => limit_mb,
        }
    }

    /// Whether the pool is over the line now (stateless: the `high` flag of the breakdown).
    pub fn is_high(&self, now_mb: u64, limit_mb: u64) -> bool {
        self.raised || now_mb > self.line(limit_mb)
    }

    /// One reading. True exactly once per episode: when the pool first goes over the line. The episode ends when it falls
    /// below `REARM_PCT` of the line, so a pool hovering at the line raises once.
    pub fn observe(&mut self, now_mb: u64, limit_mb: u64) -> bool {
        if self.baseline.is_none() {
            self.baseline = Some(now_mb);
        }
        let line = self.line(limit_mb);
        if self.raised {
            if now_mb < line * REARM_PCT / 100 {
                self.raised = false;
            }
            return false;
        }
        if now_mb > line {
            self.raised = true;
            return true;
        }
        false
    }
}

/// One decimal, `.` separator (the frontend text takes it as given).
pub fn fmt_gb(mb: u64) -> String {
    format!("{:.1}", mb as f64 / 1024.0)
}

// ---- OS source ----

pub struct OsCause;

/// `Key:   123 kB` lines of /proc/meminfo as MB.
pub fn parse_meminfo(text: &str) -> HashMap<String, u64> {
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            let kb: u64 = v.split_whitespace().next()?.parse().ok()?;
            Some((k.trim().to_string(), kb / 1024))
        })
        .collect()
}

/// The `cpu` line of /proc/stat: user+nice, system+irq+softirq+steal, idle+iowait.
pub fn parse_proc_stat(text: &str) -> Option<CpuTimes> {
    let l = text.lines().find(|l| l.starts_with("cpu "))?;
    let v: Vec<u64> = l.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    let g = |i: usize| v.get(i).copied().unwrap_or(0);
    (v.len() >= 4).then(|| CpuTimes { user: g(0) + g(1), kernel: g(2) + g(5) + g(6) + g(7), idle: g(3) + g(4) })
}

/// `key value` line of /proc/vmstat.
pub fn vmstat_value(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once(' ')?;
        (k == key).then(|| v.trim().parse().ok()).flatten()
    })
}

#[cfg(target_os = "linux")]
fn linux_cpu(pid: u32) -> Option<u64> {
    // fields 14 and 15 (utime, stime) in clock ticks, after the `(comm)` which may hold spaces
    let t = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &t[t.rfind(')')? + 2..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    let ticks: u64 = f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?;
    // USER_HZ is 100 on every Linux we run on
    Some(ticks * 100_000)
}

#[cfg(target_os = "linux")]
fn linux_io(pid: u32) -> Option<u64> {
    let t = std::fs::read_to_string(format!("/proc/{pid}/io")).ok()?;
    let v = |k: &str| t.lines().find_map(|l| l.strip_prefix(k)?.strip_prefix(':')?.trim().parse::<u64>().ok());
    Some(v("read_bytes")? + v("write_bytes")?)
}

#[cfg(all(not(windows), not(target_os = "linux")))]
fn linux_cpu(_pid: u32) -> Option<u64> {
    None
}

#[cfg(all(not(windows), not(target_os = "linux")))]
fn linux_io(_pid: u32) -> Option<u64> {
    None
}

#[cfg(not(windows))]
fn meminfo() -> Option<HashMap<String, u64>> {
    std::fs::read_to_string("/proc/meminfo").ok().map(|t| parse_meminfo(&t))
}

#[cfg(windows)]
impl CauseSource for OsCause {
    fn kernel(&self) -> Option<KernelMem> {
        super::win::kernel_and_cache().map(|(k, _)| k)
    }

    fn cache_mb(&self) -> Option<u64> {
        super::win::kernel_and_cache().map(|(_, c)| c)
    }

    fn compressed_mb(&self, rows: &[ProcRow]) -> Option<u64> {
        rows.iter().find(|r| r.name == COMPRESSION_PROC && r.rss_kib > 0).map(|r| mb(r.rss_kib))
    }

    fn handle_total(&self) -> Option<u64> {
        super::win::system_handles()
    }

    fn system(&self) -> Option<SysCounters> {
        super::win::system_counters()
    }

    fn pool_tags(&self) -> Option<Vec<PoolTag>> {
        super::win::pool_tags()
    }

    fn procs(&self) -> Vec<ProcRow> {
        let snap = axelstra_core::procs::snapshot();
        let all: std::collections::HashSet<u32> = snap.procs.iter().map(|p| p.pid).collect();
        let names = axelstra_core::procs::names_of(&all);
        snap.procs
            .iter()
            .map(|p| {
                let st = super::win::process_stats(p.pid);
                ProcRow {
                    pid: p.pid,
                    ppid: p.ppid,
                    name: names.get(&p.pid).cloned().unwrap_or_default(),
                    rss_kib: st.rss_kib,
                    handles: st.handles,
                    cpu_100ns: st.cpu_100ns,
                    io_bytes: st.io_bytes,
                }
            })
            .collect()
    }
}

#[cfg(not(windows))]
impl CauseSource for OsCause {
    fn kernel(&self) -> Option<KernelMem> {
        if !cfg!(target_os = "linux") {
            return None;
        }
        let m = meminfo()?;
        let unreclaim = *m.get("SUnreclaim")?;
        Some(KernelMem { paged_mb: m.get("SReclaimable").copied().unwrap_or(0), nonpaged_mb: unreclaim, alert_mb: unreclaim })
    }

    fn cache_mb(&self) -> Option<u64> {
        if cfg!(target_os = "macos") {
            return super::mac::read_counts().map(|(c, page, _)| super::mac::cache_mb(&c, page));
        }
        let m = meminfo()?;
        Some(m.get("Cached")? + m.get("Buffers").copied().unwrap_or(0))
    }

    fn compressed_mb(&self, _rows: &[ProcRow]) -> Option<u64> {
        if cfg!(target_os = "macos") {
            return super::mac::read_counts().map(|(c, page, _)| super::mac::compressed_mb(&c, page));
        }
        meminfo()?.get("Zswapped").copied()
    }

    fn system(&self) -> Option<SysCounters> {
        if !cfg!(target_os = "linux") {
            return None;
        }
        let stat = std::fs::read_to_string("/proc/stat").ok()?;
        let m = meminfo();
        let vm = std::fs::read_to_string("/proc/vmstat").ok();
        Some(SysCounters {
            cpu: parse_proc_stat(&stat),
            pages: vm.as_deref().and_then(|v| vmstat_value(v, "pgmajfault")),
            commit_mb: m.as_ref().and_then(|m| m.get("Committed_AS").copied()),
            commit_limit_mb: m.as_ref().and_then(|m| m.get("CommitLimit").copied()),
            modified_mb: m.as_ref().and_then(|m| m.get("Dirty").copied()),
        })
    }

    fn procs(&self) -> Vec<ProcRow> {
        let Ok(o) = std::process::Command::new("ps").args(["-A", "-o", "pid=,ppid=,rss=,comm="]).output() else { return Vec::new() };
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|l| {
                let mut it = l.split_whitespace();
                let (pid, ppid, rss_kib) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
                let comm = it.collect::<Vec<_>>().join(" ");
                let name = comm.rsplit('/').next().unwrap_or(&comm).to_ascii_lowercase();
                Some(ProcRow { pid, ppid, name, rss_kib, handles: None, cpu_100ns: linux_cpu(pid), io_bytes: linux_io(pid) })
            })
            .collect()
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::sync::Mutex;

    /// A source with fixed answers; `kernel` can be changed between reads.
    pub struct FakeCause {
        pub kernel: Mutex<Option<KernelMem>>,
        pub cache: Option<u64>,
        pub compressed: Option<u64>,
        pub rows: Vec<ProcRow>,
        pub handle_total: Option<u64>,
        pub system: Mutex<Option<SysCounters>>,
    }

    impl FakeCause {
        pub fn new(kernel_mb: u64) -> FakeCause {
            let k = KernelMem { paged_mb: kernel_mb / 2, nonpaged_mb: kernel_mb - kernel_mb / 2, alert_mb: kernel_mb };
            FakeCause {
                kernel: Mutex::new(Some(k)),
                cache: None,
                compressed: None,
                rows: vec![],
                handle_total: None,
                system: Mutex::new(None),
            }
        }

        pub fn set_kernel(&self, mb: u64) {
            *self.kernel.lock().unwrap() = Some(KernelMem { paged_mb: mb / 2, nonpaged_mb: mb - mb / 2, alert_mb: mb });
        }
    }

    impl CauseSource for FakeCause {
        fn kernel(&self) -> Option<KernelMem> {
            *self.kernel.lock().unwrap()
        }
        fn cache_mb(&self) -> Option<u64> {
            self.cache
        }
        fn compressed_mb(&self, _: &[ProcRow]) -> Option<u64> {
            self.compressed
        }
        fn handle_total(&self) -> Option<u64> {
            self.handle_total
        }
        fn system(&self) -> Option<SysCounters> {
            *self.system.lock().unwrap()
        }
        fn procs(&self) -> Vec<ProcRow> {
            self.rows.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, name: &str, mb: u64, handles: Option<u32>) -> ProcRow {
        ProcRow { pid, ppid: 0, name: name.into(), rss_kib: mb * 1024, handles, cpu_100ns: None, io_bytes: None }
    }

    #[test]
    fn totals_tiers_builds_externals_and_others_add_up() {
        let rows = vec![
            row(1, "claude", 350, None),
            row(2, "claude", 350, None),
            row(3, "claude", 400, None),
            row(4, "rustc", 3200, None),
            row(5, "chrome", 2000, None),
            row(6, "chrome", 1900, None),
            row(7, "msmpeng", 800, None),
            row(8, "memory compression", 800, None),
            row(9, "pwsh", 100, None),
        ];
        let classify = |r: &ProcRow| match r.pid {
            1 => Class::Session(Tier::Agent),
            2 => Class::Session(Tier::Worker),
            3 => Class::Session(Tier::Agent),
            4 => Class::Build,
            9 => Class::Session(Tier::Terminal),
            _ => Class::External,
        };
        let i = Inputs {
            total_mb: 31_000,
            free_mb: 2_900,
            kernel: Some(KernelMem { paged_mb: 4600, nonpaged_mb: 2300, alert_mb: 6900 }),
            cache_mb: Some(1200),
            compressed_mb: Some(800),
            handle_total: None,
            rows: &rows,
            sessions: &[Tier::Agent, Tier::Agent, Tier::Worker, Tier::Terminal, Tier::Main],
            builds: 1,
            classify: &classify,
            baseline_mb: Some(1900),
            kernel_high: true,
            pool_tags: None,
            own: Value::Null,
            system: Value::Null,
            top: None,
        };
        let b = breakdown(&i);
        assert_eq!(b["kernel"]["total_mb"], 6900);
        assert_eq!(b["kernel"]["high"], true);
        assert_eq!(b["kernel"]["baseline_mb"], 1900);
        assert_eq!(b["cache_mb"], 1200);
        assert_eq!(b["builds"], json!({"count": 1, "rss_mb": 3200}));
        let s = b["sessions"].as_array().unwrap();
        let get = |t: &str| s.iter().find(|x| x["tier"] == t).cloned();
        assert_eq!(get("agent").unwrap(), json!({"tier": "agent", "count": 2, "rss_mb": 750}));
        assert_eq!(get("worker").unwrap(), json!({"tier": "worker", "count": 1, "rss_mb": 350}));
        assert_eq!(get("terminal").unwrap(), json!({"tier": "terminal", "count": 1, "rss_mb": 100}));
        // a session the OS reports no memory for still counts
        assert_eq!(get("main").unwrap(), json!({"tier": "main", "count": 1, "rss_mb": 0}));
        assert!(get("lead").is_none());
        // the compression process is the compressed memory, not an external
        let e = b["external"].as_array().unwrap();
        assert_eq!(e[0], json!({"name": "chrome", "count": 2, "rss_mb": 3900}));
        assert_eq!(e[1], json!({"name": "msmpeng", "count": 1, "rss_mb": 800}));
        assert!(e.iter().all(|x| x["name"] != "memory compression"));
        // used 28100; explained = 6900 + 800 + 1200 (sessions) + 3200 + 4700 = 16800
        assert_eq!(b["others_mb"], 28_100 - 16_800);
        // no handle counts on this fake OS: null even though the kernel is high
        assert!(b["handles"].is_null());
    }

    #[test]
    fn others_never_goes_negative_and_externals_are_cut_at_eight() {
        let rows: Vec<ProcRow> = (0..12).map(|n| row(n, &format!("p{n:02}"), 100 + n as u64, None)).collect();
        let classify = |_: &ProcRow| Class::External;
        let i = Inputs {
            total_mb: 1000,
            free_mb: 900,
            kernel: None,
            cache_mb: None,
            compressed_mb: None,
            handle_total: None,
            rows: &rows,
            sessions: &[],
            builds: 0,
            classify: &classify,
            baseline_mb: None,
            kernel_high: false,
            pool_tags: None,
            own: Value::Null,
            system: Value::Null,
            top: None,
        };
        let b = breakdown(&i);
        assert_eq!(b["external"].as_array().unwrap().len(), 8);
        assert_eq!(b["external"][0]["name"], "p11");
        assert_eq!(b["others_mb"], 0);
        assert!(b["kernel"].is_null() && b["cache_mb"].is_null() && b["compressed_mb"].is_null());
    }

    #[test]
    fn handles_appear_when_the_kernel_is_high_or_the_counts_are_big_and_name_the_top_five() {
        let mut rows: Vec<ProcRow> = (1..=7).map(|n| row(n, &format!("p{n}"), 10, Some(n * 1000))).collect();
        let classify = |_: &ProcRow| Class::External;
        let mk = |rows: &[ProcRow], high: bool, total: Option<u64>| {
            let i = Inputs {
                total_mb: 8000,
                free_mb: 4000,
                kernel: None,
                cache_mb: None,
                compressed_mb: None,
                handle_total: total,
                rows,
                sessions: &[],
                builds: 0,
                classify: &classify,
                baseline_mb: None,
                kernel_high: high,
                pool_tags: None,
                own: Value::Null,
                system: Value::Null,
                top: None,
            };
            breakdown(&i)
        };
        // small counts, kernel fine: nothing
        assert!(mk(&rows, false, None)["handles"].is_null());
        // kernel high: filled, top 5 biggest first
        let h = mk(&rows, true, None)["handles"].clone();
        assert_eq!(h["total"], 28_000);
        let top = h["top"].as_array().unwrap();
        assert_eq!(top.len(), 5);
        assert_eq!(top[0], json!({"name": "p7", "pid": 7, "handles": 7000}));
        assert_eq!(top[4]["pid"], 3);
        // the system total beyond 200k fills it without the kernel flag
        assert_eq!(mk(&rows, false, Some(250_000))["handles"]["total"], 250_000);
        // one process beyond 50k does too
        rows.push(row(99, "leaky", 10, Some(60_000)));
        assert_eq!(mk(&rows, false, None)["handles"]["top"][0]["name"], "leaky");
    }

    #[test]
    fn alert_raises_once_per_episode_and_rearms_with_hysteresis() {
        let mut w = KernelWatch::default();
        let limit = 3072;
        // first reading is the baseline (700); 2x is below the floor, so the line is the limit
        assert!(!w.observe(700, limit));
        assert_eq!(w.baseline(), Some(700));
        assert_eq!(w.line(limit), 1400.max(REL_FLOOR_MB).min(limit));
        assert!(!w.observe(1000, limit));
        // over the line (1400): once
        assert!(w.observe(1500, limit));
        assert!(w.raised());
        assert!(!w.observe(1600, limit));
        assert!(!w.observe(1300, limit)); // under the line but not under 85% of it (1190): same episode
        assert!(!w.observe(1390, limit));
        assert!(!w.observe(1100, limit)); // below 85%: the episode ends
        assert!(!w.raised());
        // a new episode raises again
        assert!(w.observe(1500, limit));
    }

    #[test]
    fn the_line_is_the_lower_of_the_limit_and_twice_the_baseline() {
        let mut w = KernelWatch::default();
        assert_eq!(w.line(3072), 3072);
        w.observe(1900, 3072); // 1.9 GB after a reboot
        assert_eq!(w.baseline(), Some(1900));
        // 2x = 3800 > limit: the limit rules
        assert_eq!(w.line(3072), 3072);
        // a higher limit: twice the baseline rules
        assert_eq!(w.line(8192), 3800);
        assert!(w.observe(3900, 8192));
        // a tiny baseline is floored so normal use does not trip it
        let mut small = KernelWatch::default();
        small.observe(200, 3072);
        assert_eq!(small.line(3072), REL_FLOOR_MB);
        assert!(!small.observe(900, 3072));
        assert!(small.observe(1100, 3072));
        // is_high is stateless apart from an open episode
        let fresh = KernelWatch::default();
        assert!(fresh.is_high(4000, 3072));
        assert!(!fresh.is_high(3000, 3072));
    }

    #[test]
    fn a_lowered_limit_raises_on_the_next_reading() {
        let mut w = KernelWatch::default();
        assert!(!w.observe(2000, 3072));
        assert!(w.observe(2100, 2048)); // above the new line; baseline 2000 makes 2x = 4000, limit rules
    }

    fn ctr(idle: u64, kernel: u64, user: u64, pages: u64) -> SysCounters {
        SysCounters { cpu: Some(CpuTimes { idle, kernel, user }), pages: Some(pages), ..Default::default() }
    }

    #[test]
    fn rates_come_from_deltas_and_the_load_alert_needs_three_in_a_row() {
        use std::time::{Duration, Instant};
        let r = sys_rates(&ctr(0, 0, 0, 0), &ctr(60, 20, 20, 1000), 2.0);
        assert_eq!((r.cpu_kernel_pct, r.cpu_user_pct, r.pages_per_sec), (Some(20.0), Some(20.0), Some(500)));
        let t0 = Instant::now();
        let mut w = LoadWatch::default();
        let mut at = t0;
        let mut c = ctr(0, 0, 0, 0);
        let mut step = |w: &mut LoadWatch, kernel: u64, pages: u64| {
            at += Duration::from_secs(10);
            c = ctr(c.cpu.unwrap().idle + 100 - kernel, c.cpu.unwrap().kernel + kernel, c.cpu.unwrap().user, c.pages.unwrap() + pages);
            w.observe(c, at)
        };
        assert!(step(&mut w, 0, 0).is_none()); // dt of the first reading is unknown (here: the baseline reading made by observe)
        assert!(step(&mut w, 30, 0).is_none()); // 1st over
        assert!(step(&mut w, 30, 0).is_none()); // 2nd
        let raised = step(&mut w, 30, 0).unwrap(); // 3rd: raise
        assert_eq!(raised.cpu_kernel_pct, Some(30.0));
        assert!(w.raised());
        assert!(step(&mut w, 30, 0).is_none()); // one notice per episode
        assert!(step(&mut w, 12, 0).is_none()); // under the line, not under 70% of it: still the same episode
        assert!(w.raised());
        assert!(step(&mut w, 2, 0).is_none()); // clear: re-armed
        assert!(!w.raised());
        // a gap in the run resets it; pages alone also count (60000 pages in 10 s = 6000/s)
        assert!(step(&mut w, 0, 60_000).is_none());
        assert!(step(&mut w, 0, 0).is_none());
        assert!(step(&mut w, 0, 60_000).is_none());
        assert!(step(&mut w, 0, 60_000).is_none());
        assert!(step(&mut w, 0, 60_000).is_some());
    }

    #[test]
    fn the_monitor_raises_a_kernel_alert_once_with_an_injected_source() {
        use std::sync::Arc;
        let fake = Arc::new(fake::FakeCause::new(1800));
        let m = Monitor::new(fake.clone());
        let now = std::time::Instant::now();
        assert!(m.evaluate(3072, now).is_empty());
        assert_eq!(m.kernel_state(1800, 3072), (Some(1800), false));
        fake.set_kernel(3500);
        assert_eq!(m.evaluate(3072, now), vec![Alert::Kernel(3500)]);
        assert!(m.evaluate(3072, now).is_empty());
        assert!(m.kernel_state(3500, 3072).1);
        fake.set_kernel(2000);
        assert!(m.evaluate(3072, now).is_empty());
        fake.set_kernel(3600);
        assert_eq!(m.evaluate(3072, now).len(), 1);
    }

    #[test]
    fn top_cpu_and_io_use_the_delta_between_two_tables() {
        let mk = |pid, name: &str, cpu, io| ProcRow {
            pid,
            ppid: 0,
            name: name.into(),
            rss_kib: 0,
            handles: None,
            cpu_100ns: Some(cpu),
            io_bytes: Some(io),
        };
        let a = vec![mk(1, "a", 0, 0), mk(2, "b", 0, 0), mk(3, "idle", 5, 5)];
        // 2 s apart, 4 cpus: process 1 used 4 s of cpu = 50% of the machine
        let b = vec![mk(1, "a", 40_000_000, 100), mk(2, "b", 10_000_000, 9_000), mk(3, "idle", 5, 5), mk(4, "new", 99, 99)];
        let (cpu, io) = top_rates(&a, &b, 2.0, 4);
        assert_eq!(cpu[0], json!({"name": "a", "pid": 1, "value": 50.0}));
        assert_eq!(cpu[1]["name"], "b");
        assert_eq!(cpu.len(), 2);
        assert_eq!(io[0], json!({"name": "b", "pid": 2, "value": 4500}));
        assert_eq!(io.len(), 2);
    }

    #[test]
    fn proc_stat_and_vmstat_parse() {
        let c = parse_proc_stat(
            "cpu  100 10 50 800 40 5 5 0 0 0
cpu0 1 1 1 1
",
        )
        .unwrap();
        assert_eq!((c.user, c.kernel, c.idle), (110, 60, 840));
        assert_eq!(
            vmstat_value(
                "pgfault 5
pgmajfault 77
",
                "pgmajfault"
            ),
            Some(77)
        );
        let sj = system_json(None, None);
        assert!(sj["cpu_kernel_pct"].is_null() && sj["pages_per_sec"].is_null());
    }

    #[test]
    fn meminfo_lines_parse_to_mb() {
        let m = parse_meminfo("MemTotal:       16384000 kB\nSUnreclaim:      204800 kB\nSReclaimable:    1024 kB\nbroken line\n");
        assert_eq!(m["MemTotal"], 16000);
        assert_eq!(m["SUnreclaim"], 200);
        assert_eq!(m["SReclaimable"], 1);
        assert_eq!(fmt_gb(4608), "4.5");
    }

    #[test]
    fn the_os_source_reads_something_on_this_machine() {
        let rows = OsCause.procs();
        assert!(rows.iter().any(|r| r.pid == std::process::id()), "own process missing");
        if cfg!(any(windows, target_os = "linux")) {
            assert!(OsCause.kernel().is_some());
            let c = OsCause.system().expect("machine counters");
            assert!(c.cpu.is_some() && c.pages.is_some() && c.commit_limit_mb.unwrap_or(0) > 0, "{c:?}");
            eprintln!(
                "os cause: kernel {:?} cache {:?} system {c:?} handles {:?}",
                OsCause.kernel(),
                OsCause.cache_mb(),
                OsCause.handle_total()
            );
        }
    }

    /// a class-22 buffer: count, pad, then 40-byte entries (tag at 0, PagedUsed at 16, NonPagedUsed at 32)
    fn pool_buf(entries: &[(&[u8; 4], u64, u64)]) -> Vec<u8> {
        let mut b = vec![0u8; 8 + entries.len() * 40];
        b[0..4].copy_from_slice(&(entries.len() as u32).to_le_bytes());
        for (i, (tag, paged, non)) in entries.iter().enumerate() {
            let o = 8 + i * 40;
            b[o..o + 4].copy_from_slice(*tag);
            b[o + 16..o + 24].copy_from_slice(&paged.to_le_bytes());
            b[o + 32..o + 40].copy_from_slice(&non.to_le_bytes());
        }
        b
    }

    #[test]
    fn pool_tags_parse_from_a_synthetic_buffer_and_skip_empty_ones() {
        const M: u64 = 1024 * 1024;
        let b = pool_buf(&[(b"FMfn", 1541 * M, 0), (b"File", 0, 587 * M), (b"Zero", 0, 0)]);
        let t = parse_pool_tags(&b).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].tag.as_str(), t[0].paged, t[0].nonpaged), ("FMfn", 1541 * M, 0));
        assert_eq!((t[1].tag.as_str(), t[1].nonpaged), ("File", 587 * M));
        // a count larger than the buffer, or no buffer: nothing, never a panic
        assert!(parse_pool_tags(&b[..b.len() - 1]).is_none());
        assert!(parse_pool_tags(&[1, 0]).is_none());
        let mut huge = b.clone();
        huge[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_pool_tags(&huge).is_none());
        // a non-printable tag byte is shown as ?
        assert_eq!(parse_pool_tags(&pool_buf(&[(&[b'A', 1, b'b', b'c'], M, 0)])).unwrap()[0].tag, "A?bc");
    }

    #[test]
    fn pool_tags_json_ranks_by_bytes_names_known_owners_and_shows_the_untagged_rest() {
        const M: u64 = 1024 * 1024;
        let mut tags: Vec<PoolTag> = (0..10).map(|i| PoolTag { tag: format!("T{i:03}"), paged: (i + 1) * M, nonpaged: 0 }).collect();
        tags.push(PoolTag { tag: "FMfn".into(), paged: 1500 * M, nonpaged: 0 });
        tags.push(PoolTag { tag: "File".into(), paged: 0, nonpaged: 500 * M });
        let k = KernelMem { paged_mb: 1800, nonpaged_mb: 600, alert_mb: 2400 };
        let j = pool_tags_json(&tags, Some(k));
        let top = j["top"].as_array().unwrap();
        assert_eq!(top.len(), 8);
        assert_eq!((top[0]["tag"].as_str(), top[0]["owner"].as_str()), (Some("FMfn"), Some("filter")));
        assert_eq!((top[1]["tag"].as_str(), top[1]["nonpaged_mb"].as_u64()), (Some("File"), Some(500)));
        // an unknown tag has no owner: nothing invented
        assert!(top[2]["owner"].is_null());
        // 1500 + 500 + (1..=10 summed = 55) tagged; counters say 2400
        assert_eq!((j["tagged_mb"].as_u64(), j["sem_etiqueta_mb"].as_u64(), j["partial"].as_bool()), (Some(2055), Some(345), Some(true)));
        // no counters: the rest is unknown, not zero
        assert!(pool_tags_json(&tags, None)["sem_etiqueta_mb"].is_null());
    }

    #[test]
    fn unavailable_names_why_per_os() {
        let none = Present { kernel: false, pool_tags: false, cache: false, compressed: false, handles: false };
        let mac = unavailable("macos", &none);
        assert_eq!(mac["kernel"], UNSUPPORTED_OS);
        assert_eq!(mac["pool_tags"], UNSUPPORTED_OS);
        assert_eq!(mac["handles"], UNSUPPORTED_OS);
        assert_eq!(mac["compressed_mb"], READ_FAILED);
        let lin = unavailable("linux", &none);
        assert_eq!(lin["pool_tags"], UNSUPPORTED_OS);
        assert_eq!(lin["handles"], UNSUPPORTED_OS);
        assert_eq!(lin["kernel"], READ_FAILED);
        let win = unavailable("windows", &none);
        assert_eq!(win["handles"], NOT_NEEDED);
        assert_eq!(win["pool_tags"], READ_FAILED);
        let all = Present { kernel: true, pool_tags: true, cache: true, compressed: true, handles: true };
        assert_eq!(unavailable("windows", &all), json!({}));
        assert_eq!(unavailable("freebsd", &none)["cache_mb"], UNSUPPORTED_OS);
    }

    #[test]
    fn own_rate_needs_a_minute_of_uptime() {
        assert!(own_json(30, 59, 4)["spawns_per_min"].is_null());
        let o = own_json(120, 120, 7);
        assert_eq!((o["spawns_per_min"].as_f64(), o["watches"].as_u64()), (Some(60.0), Some(7)));
    }
}
