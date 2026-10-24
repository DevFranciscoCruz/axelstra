//! Token budgets per worktree. The limits live in `settings.json` (`budgets`), the daily and weekly spending is the
//! difference between the cumulative totals read from the agent logs (`agentlog::worktree_usage`) and a snapshot of
//! that total taken when the day or the week began, kept in `budget-state.json`. A level crossing (warn at 80 %,
//! over at 100 %) is reported once per period; pausing a runaway agent is a separate opt-in per budget.
//!
//! Days and ISO weeks are the user's local ones. A worktree seen for the first time starts its period at the total it
//! has then: the logs do not say when each token was spent, so earlier spending is not charged to the period.

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Mutex;

/// Share of a limit at which the level turns to `warn`.
pub const WARN_PCT: u128 = 80;
/// Largest limit accepted (tokens), far above any real spending but safe from overflow.
const MAX_LIMIT: u64 = 1_000_000_000_000;
/// Most per-worktree entries a `budgets` setting may hold.
const MAX_WORKTREES: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Warn,
    Over,
}

impl Level {
    fn rank(self) -> u8 {
        self as u8
    }
}

/// `used` against `limit`: warn from 80 %, over from 100 %. No limit (0) never alerts.
pub fn level(used: u64, limit: u64) -> Level {
    if limit == 0 {
        return Level::Ok;
    }
    let (u, l) = (u128::from(used) * 100, u128::from(limit));
    if u >= l * 100 {
        Level::Over
    } else if u >= l * WARN_PCT {
        Level::Warn
    } else {
        Level::Ok
    }
}

/// Whole-percent share of the limit used (not capped), 0 without a limit.
pub fn pct(used: u64, limit: u64) -> u64 {
    if limit == 0 {
        0
    } else {
        (u128::from(used) * 100 / u128::from(limit)).min(u128::from(u64::MAX)) as u64
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Limit {
    pub daily: Option<u64>,
    pub weekly: Option<u64>,
    /// Interrupt the agents of the worktree when a limit goes over. Off unless the user turns it on for this budget.
    pub auto_pause: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Budgets {
    pub default: Limit,
    /// per worktree path (as the UI saved it; compared normalized)
    pub worktrees: Vec<(String, Limit)>,
}

impl Budgets {
    pub fn is_empty(&self) -> bool {
        self.default.daily.is_none() && self.default.weekly.is_none() && self.worktrees.is_empty()
    }

    /// The budget of a worktree: its own entry, else the default.
    pub fn limit_for(&self, path: &str) -> &Limit {
        let want = norm(path);
        self.worktrees.iter().find(|(p, _)| norm(p) == want).map(|(_, l)| l).unwrap_or(&self.default)
    }
}

/// Path as compared (and keyed): forward slashes, no trailing slash, lowercase.
pub fn norm(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_ascii_lowercase()
}

fn parse_limit(v: &Value) -> Limit {
    let n = |k: &str| v[k].as_u64().filter(|n| (1..=MAX_LIMIT).contains(n));
    Limit { daily: n("daily"), weekly: n("weekly"), auto_pause: v["auto_pause"] == json!(true) }
}

/// The `budgets` setting as saved. Lenient: whatever is missing or malformed means no limit (and no auto-pause), so an
/// older or hand-edited settings.json never breaks the daemon.
pub fn parse(saved: &Value) -> Budgets {
    let b = &saved["budgets"];
    let worktrees = b["worktrees"]
        .as_object()
        .map(|m| m.iter().filter(|(k, _)| !k.is_empty()).take(MAX_WORKTREES).map(|(k, v)| (k.clone(), parse_limit(v))).collect())
        .unwrap_or_default();
    Budgets { default: parse_limit(&b["default"]), worktrees }
}

fn check_limit(what: &str, v: &Value) -> anyhow::Result<Value> {
    let o = v.as_object().ok_or_else(|| anyhow::anyhow!("budgets {what} must be an object"))?;
    if let Some(k) = o.keys().find(|k| !["daily", "weekly", "auto_pause"].contains(&k.as_str())) {
        anyhow::bail!("budgets {what}: unknown key {k:?}");
    }
    let mut out = serde_json::Map::new();
    for k in ["daily", "weekly"] {
        match o.get(k) {
            None | Some(Value::Null) => {}
            Some(n) => match n.as_u64() {
                Some(n) if (1..=MAX_LIMIT).contains(&n) => {
                    out.insert(k.into(), json!(n));
                }
                _ => anyhow::bail!("budgets {what}.{k} must be a whole number of tokens between 1 and {MAX_LIMIT}, or null"),
            },
        }
    }
    match o.get("auto_pause") {
        None | Some(Value::Null) => {}
        Some(Value::Bool(b)) => {
            out.insert("auto_pause".into(), json!(b));
        }
        Some(_) => anyhow::bail!("budgets {what}.auto_pause must be true or false"),
    }
    Ok(Value::Object(out))
}

/// Checks a `budgets` value of `settings.set` and returns it normalized; nothing is saved from an invalid one.
/// Shape: `{"default": Limit, "worktrees": {"<path>": Limit}}` with `Limit = {daily?, weekly?, auto_pause?}`.
pub fn validate(v: &Value) -> anyhow::Result<Value> {
    let o = v.as_object().ok_or_else(|| anyhow::anyhow!("budgets must be an object"))?;
    if let Some(k) = o.keys().find(|k| !["default", "worktrees"].contains(&k.as_str())) {
        anyhow::bail!("budgets: unknown key {k:?}");
    }
    let mut out = serde_json::Map::new();
    if let Some(d) = o.get("default").filter(|d| !d.is_null()) {
        out.insert("default".into(), check_limit("default", d)?);
    }
    if let Some(w) = o.get("worktrees").filter(|w| !w.is_null()) {
        let w = w.as_object().ok_or_else(|| anyhow::anyhow!("budgets.worktrees must be an object"))?;
        if w.len() > MAX_WORKTREES {
            anyhow::bail!("budgets.worktrees holds at most {MAX_WORKTREES} entries");
        }
        let mut m = serde_json::Map::new();
        for (path, l) in w {
            if path.is_empty() || path.len() > 1024 {
                anyhow::bail!("budgets.worktrees: a path must have 1 to 1024 characters");
            }
            m.insert(path.clone(), check_limit("worktrees entry", l)?);
        }
        out.insert("worktrees".into(), Value::Object(m));
    }
    Ok(Value::Object(out))
}

/// What one worktree had at the start of the current day and week, and which levels were already reported.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub day: String,
    pub day_base: u64,
    pub week: String,
    pub week_base: u64,
    /// the last cumulative total seen
    pub last: u64,
    pub alerted_day: u8,
    pub alerted_week: u8,
}

/// per normalized worktree path
pub type State = BTreeMap<String, Track>;

/// `(day, ISO week)` keys of a date, e.g. `("2026-10-24", "2026-W43")`.
pub fn period_keys(date: NaiveDate) -> (String, String) {
    let w = date.iso_week();
    (date.to_string(), format!("{}-W{:02}", w.year(), w.week()))
}

impl Track {
    /// Starts a new day or week when the keys changed, and follows the total. A total that went down (logs removed)
    /// lowers the bases with it, so the spending never turns negative.
    fn roll(&mut self, total: u64, day: &str, week: &str) {
        let fresh = self.day.is_empty();
        // what was spent after the last look and before the change still counts for the new period
        let start = if fresh { total } else { self.last.min(total) };
        if self.day != day {
            self.day = day.into();
            self.day_base = start;
            self.alerted_day = 0;
        }
        if self.week != week {
            self.week = week.into();
            self.week_base = start;
            self.alerted_week = 0;
        }
        self.day_base = self.day_base.min(total);
        self.week_base = self.week_base.min(total);
        self.last = total;
    }

    fn used(&self, weekly: bool) -> u64 {
        self.last - if weekly { self.week_base } else { self.day_base }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Row {
    pub worktree: String,
    pub period: &'static str,
    pub used: u64,
    pub limit: u64,
    pub pct: u64,
    pub level: Level,
    pub auto_pause: bool,
}

fn rows_of(path: &str, t: &Track, l: &Limit) -> Vec<Row> {
    [("daily", false, l.daily), ("weekly", true, l.weekly)]
        .into_iter()
        .filter_map(|(period, weekly, limit)| {
            let (limit, used) = (limit?, t.used(weekly));
            Some(Row {
                worktree: path.into(),
                period,
                used,
                limit,
                pct: pct(used, limit),
                level: level(used, limit),
                auto_pause: l.auto_pause,
            })
        })
        .collect()
}

/// Spending against the limits for each `(path, cumulative total)`; nothing is recorded.
pub fn status(state: &State, budgets: &Budgets, totals: &[(String, u64)], date: NaiveDate) -> Vec<Row> {
    let (day, week) = period_keys(date);
    totals
        .iter()
        .flat_map(|(path, total)| {
            let mut t = state.get(&norm(path)).cloned().unwrap_or_default();
            t.roll(*total, &day, &week);
            rows_of(path, &t, budgets.limit_for(path))
        })
        .collect()
}

/// Records the totals and returns the rows whose level went up since it was last reported in their period.
pub fn tick(state: &mut State, budgets: &Budgets, totals: &[(String, u64)], date: NaiveDate) -> Vec<Row> {
    let (day, week) = period_keys(date);
    let mut alerts = vec![];
    for (path, total) in totals {
        let t = state.entry(norm(path)).or_default();
        t.roll(*total, &day, &week);
        for row in rows_of(path, t, budgets.limit_for(path)) {
            let seen = if row.period == "weekly" { &mut t.alerted_week } else { &mut t.alerted_day };
            if row.level.rank() > *seen {
                *seen = row.level.rank();
                alerts.push(row);
            }
        }
    }
    alerts
}

/// Whether a reported crossing may interrupt the agents: only when the budget has the opt-in and the limit is passed.
pub fn should_pause(row: &Row) -> bool {
    row.auto_pause && row.level == Level::Over
}

static EVENTS: Mutex<Vec<Value>> = Mutex::new(Vec::new());

/// Remembers a crossing for the app's toast and notification center (the newest 100 are kept).
pub fn push_event(row: &Row, paused: &[u32], at: u64) {
    let mut q = EVENTS.lock().unwrap_or_else(|e| e.into_inner());
    q.push(json!({"worktree": row.worktree, "period": row.period, "level": row.level, "used": row.used, "limit": row.limit,
        "pct": row.pct, "paused": paused, "at": at}));
    let n = q.len();
    if n > 100 {
        q.drain(..n - 100);
    }
}

/// Takes (and clears) the crossings since the last call.
pub fn take_events() -> Vec<Value> {
    std::mem::take(&mut *EVENTS.lock().unwrap_or_else(|e| e.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }
    fn budgets(daily: u64, weekly: u64, auto_pause: bool) -> Budgets {
        Budgets { default: Limit { daily: Some(daily), weekly: Some(weekly), auto_pause }, worktrees: vec![] }
    }
    fn totals(n: u64) -> Vec<(String, u64)> {
        vec![("C:\\wt\\a".into(), n)]
    }

    #[test]
    fn levels_turn_at_80_and_100_percent() {
        assert_eq!(level(0, 100), Level::Ok);
        assert_eq!(level(79, 100), Level::Ok);
        assert_eq!(level(80, 100), Level::Warn);
        assert_eq!(level(99, 100), Level::Warn);
        assert_eq!(level(100, 100), Level::Over);
        assert_eq!(level(5, 0), Level::Ok, "no limit, no alert");
        assert_eq!(level(u64::MAX, u64::MAX), Level::Over, "no overflow");
        assert_eq!(pct(150, 100), 150);
    }

    #[test]
    fn periods_are_the_local_day_and_the_iso_week() {
        assert_eq!(period_keys(d(2026, 10, 24)), ("2026-10-24".into(), "2026-W43".into()));
        // Monday starts the week, Sunday ends it; the year boundary follows ISO
        assert_eq!(period_keys(d(2026, 10, 25)).1, "2026-W43");
        assert_eq!(period_keys(d(2026, 10, 26)).1, "2026-W44");
        assert_eq!(period_keys(d(2024, 12, 30)).1, "2025-W01");
    }

    #[test]
    fn spending_is_counted_from_the_start_of_the_period() {
        let b = budgets(1000, 5000, false);
        let mut st = State::new();
        // first sight: the 10_000 already in the logs is history
        assert!(tick(&mut st, &b, &totals(10_000), d(2026, 10, 24)).is_empty());
        let rows = status(&st, &b, &totals(10_900), d(2026, 10, 24));
        assert_eq!((rows[0].used, rows[0].pct, rows[0].level), (900, 90, Level::Warn));
        assert_eq!((rows[1].period, rows[1].used, rows[1].level), ("weekly", 900, Level::Ok));
        // next day, same week: the day starts again where the last look left it, the week goes on
        tick(&mut st, &b, &totals(10_900), d(2026, 10, 24));
        let rows = status(&st, &b, &totals(11_000), d(2026, 10, 25));
        assert_eq!((rows[0].used, rows[1].used), (100, 1000));
        // a new week resets both
        tick(&mut st, &b, &totals(11_000), d(2026, 10, 25));
        let rows = status(&st, &b, &totals(11_000), d(2026, 10, 26));
        assert_eq!((rows[0].used, rows[1].used), (0, 0));
    }

    #[test]
    fn a_crossing_is_reported_once_per_period() {
        let b = budgets(1000, 100_000, false);
        let mut st = State::new();
        tick(&mut st, &b, &totals(0), d(2026, 10, 24));
        assert!(tick(&mut st, &b, &totals(500), d(2026, 10, 24)).is_empty());
        let a = tick(&mut st, &b, &totals(850), d(2026, 10, 24));
        assert_eq!(a.len(), 1);
        assert_eq!((a[0].period, a[0].level), ("daily", Level::Warn));
        assert!(tick(&mut st, &b, &totals(900), d(2026, 10, 24)).is_empty(), "still warn: silent");
        let a = tick(&mut st, &b, &totals(1200), d(2026, 10, 24));
        assert_eq!((a.len(), a[0].level), (1, Level::Over));
        assert!(tick(&mut st, &b, &totals(5000), d(2026, 10, 24)).is_empty(), "still over: silent");
        // the next day it may alert again
        tick(&mut st, &b, &totals(5000), d(2026, 10, 25));
        let a = tick(&mut st, &b, &totals(6000), d(2026, 10, 25));
        assert_eq!((a.len(), a[0].level), (1, Level::Over));
    }

    #[test]
    fn jumping_straight_to_over_reports_over_only() {
        let b = budgets(1000, 100_000, false);
        let mut st = State::new();
        tick(&mut st, &b, &totals(0), d(2026, 10, 24));
        let a = tick(&mut st, &b, &totals(2000), d(2026, 10, 24));
        assert_eq!(a.iter().map(|r| r.level).collect::<Vec<_>>(), vec![Level::Over]);
    }

    #[test]
    fn a_total_that_shrinks_never_goes_negative() {
        let b = budgets(1000, 1000, false);
        let mut st = State::new();
        tick(&mut st, &b, &totals(5000), d(2026, 10, 24));
        tick(&mut st, &b, &totals(5400), d(2026, 10, 24));
        // logs were deleted
        let rows = status(&st, &b, &totals(100), d(2026, 10, 24));
        assert_eq!(rows[0].used, 0);
        tick(&mut st, &b, &totals(100), d(2026, 10, 24));
        let rows = status(&st, &b, &totals(300), d(2026, 10, 24));
        assert_eq!(rows[0].used, 200);
    }

    #[test]
    fn a_worktree_entry_replaces_the_default_and_paths_compare_normalized() {
        let mut b = budgets(1000, 1000, false);
        b.worktrees.push(("C:/WT/a/".into(), Limit { daily: Some(10), weekly: None, auto_pause: true }));
        assert_eq!(b.limit_for("c:\\wt\\a").daily, Some(10));
        assert_eq!(b.limit_for("c:\\wt\\b").daily, Some(1000));
        let mut st = State::new();
        tick(&mut st, &b, &totals(0), d(2026, 10, 24));
        let rows = status(&st, &b, &totals(50), d(2026, 10, 24));
        assert_eq!(rows.len(), 1, "no weekly limit on that worktree");
        assert_eq!((rows[0].level, rows[0].auto_pause), (Level::Over, true));
    }

    #[test]
    fn pausing_needs_the_opt_in_and_a_limit_passed() {
        let mut st = State::new();
        for (opt_in, total, want) in [(false, 5000, false), (true, 850, false), (true, 5000, true)] {
            let b = budgets(1000, 1_000_000, opt_in);
            st.clear();
            tick(&mut st, &b, &totals(0), d(2026, 10, 24));
            let alerts = tick(&mut st, &b, &totals(total), d(2026, 10, 24));
            assert!(!alerts.is_empty());
            assert_eq!(alerts.iter().any(should_pause), want, "opt_in={opt_in} total={total}");
        }
        // the default of a missing field is off
        assert!(!parse(&json!({"budgets": {"default": {"daily": 5}}})).default.auto_pause);
        assert!(!Limit::default().auto_pause);
    }

    #[test]
    fn settings_without_budgets_or_with_junk_mean_no_limits() {
        assert!(parse(&json!({})).is_empty());
        assert!(parse(&json!({"budgets": "x"})).is_empty());
        assert!(parse(&json!({"budgets": {"default": {"daily": -1, "weekly": "9", "auto_pause": "yes"}}})).is_empty());
        let b = parse(&json!({"budgets": {"default": {"daily": 7}, "worktrees": {"C:/a": {"weekly": 9, "auto_pause": true}}}}));
        assert_eq!((b.default.daily, b.worktrees.len(), b.worktrees[0].1.auto_pause), (Some(7), 1, true));
    }

    #[test]
    fn validate_normalizes_and_rejects_bad_input() {
        let ok = validate(&json!({"default": {"daily": 1000, "weekly": null}, "worktrees": {"C:/a": {"auto_pause": true}}})).unwrap();
        assert_eq!(ok, json!({"default": {"daily": 1000}, "worktrees": {"C:/a": {"auto_pause": true}}}));
        assert_eq!(validate(&json!({})).unwrap(), json!({}));
        for bad in [
            json!("x"),
            json!({"other": 1}),
            json!({"default": {"daily": 0}}),
            json!({"default": {"daily": -5}}),
            json!({"default": {"daily": 1.5}}),
            json!({"default": {"daily": "9"}}),
            json!({"default": {"weekly": MAX_LIMIT + 1}}),
            json!({"default": {"auto_pause": "yes"}}),
            json!({"default": {"nope": 1}}),
            json!({"default": []}),
            json!({"worktrees": []}),
            json!({"worktrees": {"": {}}}),
        ] {
            assert!(validate(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn events_are_taken_once() {
        let _ = take_events();
        let row = Row { worktree: "w".into(), period: "daily", used: 9, limit: 10, pct: 90, level: Level::Warn, auto_pause: false };
        push_event(&row, &[], 1);
        let e = take_events();
        assert_eq!((e.len(), e[0]["level"].as_str(), e[0]["paused"].clone()), (1, Some("warn"), json!([])));
        assert!(take_events().is_empty());
    }
}
