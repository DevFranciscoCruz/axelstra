//! Scheduled agent runs: "every morning, have an agent look at X". Each run gets its own fresh worktree and
//! an interactive agent session started with the prompt, so the result waits in the sidebar like any other agent.
//! Runs missed while the daemon was off are skipped, not replayed.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDateTime, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Schedule {
    /// every `minutes` minutes
    Interval { minutes: u32 },
    /// every day at local `time` ("HH:MM")
    Daily { time: String },
    /// every week on `day` (0 = Monday .. 6 = Sunday) at local `time`
    Weekly { day: u32, time: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Automation {
    pub id: String,
    pub name: String,
    pub project: String,
    /// key of the agent CLI: "claude" or "codex"
    pub agent: String,
    pub prompt: String,
    pub schedule: Schedule,
    pub enabled: bool,
    /// unix seconds of the last start (or of creation: nothing fires retroactively)
    pub last_run: i64,
}

pub fn store_path(projects_store: &Path) -> PathBuf {
    projects_store.with_file_name("automations.json")
}

pub fn load(store: &Path) -> Vec<Automation> {
    std::fs::read_to_string(store).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save_all(store: &Path, v: &[Automation]) -> Result<()> {
    std::fs::create_dir_all(store.parent().unwrap())?;
    let tmp = store.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v)?)?;
    std::fs::rename(&tmp, store)?;
    Ok(())
}

fn parse_time(t: &str) -> Result<NaiveTime> {
    NaiveTime::parse_from_str(t.trim(), "%H:%M").with_context(|| format!("time must be HH:MM, got {t:?}"))
}

pub fn validate(a: &Automation) -> Result<()> {
    if a.name.trim().is_empty() {
        bail!("name is empty");
    }
    if a.prompt.trim().is_empty() {
        bail!("prompt is empty");
    }
    if !matches!(a.agent.as_str(), "claude" | "codex") {
        bail!("unknown agent {}", a.agent);
    }
    match &a.schedule {
        Schedule::Interval { minutes } if *minutes < 5 => bail!("interval must be at least 5 minutes"),
        Schedule::Interval { .. } => {}
        Schedule::Daily { time } => {
            parse_time(time)?;
        }
        Schedule::Weekly { day, time } => {
            if *day > 6 {
                bail!("day must be 0 (Monday) to 6 (Sunday)");
            }
            parse_time(time)?;
        }
    }
    Ok(())
}

/// Latest scheduled instant at or before `now`; for an interval, the next one after `last`.
fn latest_slot(s: &Schedule, now: NaiveDateTime, last: NaiveDateTime) -> Result<NaiveDateTime> {
    Ok(match s {
        Schedule::Interval { minutes } => last + Duration::minutes(*minutes as i64),
        Schedule::Daily { time } => {
            let today = now.date().and_time(parse_time(time)?);
            if today <= now { today } else { today - Duration::days(1) }
        }
        Schedule::Weekly { day, time } => {
            let back = (now.date().weekday().num_days_from_monday() + 7 - *day) % 7;
            let this = (now.date() - Duration::days(back as i64)).and_time(parse_time(time)?);
            if this <= now { this } else { this - Duration::days(7) }
        }
    })
}

/// Whether the automation should start now.
pub fn due(a: &Automation, now: NaiveDateTime, last: NaiveDateTime) -> bool {
    if !a.enabled {
        return false;
    }
    match &a.schedule {
        Schedule::Interval { .. } => latest_slot(&a.schedule, now, last).map(|s| s <= now).unwrap_or(false),
        _ => latest_slot(&a.schedule, now, last).map(|s| s > last).unwrap_or(false),
    }
}

fn local(ts: i64) -> NaiveDateTime {
    Local.timestamp_opt(ts, 0).single().unwrap_or_else(Local::now).naive_local()
}

/// Automations that should start at `now_ts` (unix seconds).
pub fn due_now(list: &[Automation], now_ts: i64) -> Vec<String> {
    let now = local(now_ts);
    list.iter().filter(|a| due(a, now, local(a.last_run))).map(|a| a.id.clone()).collect()
}

pub fn now_ts() -> i64 {
    DateTime::<Local>::from(std::time::SystemTime::now()).timestamp()
}

/// `'` doubled, the way PowerShell single-quoted strings escape it.
pub fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// POSIX single-quote escaping.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Program and args that run `cli "<prompt>"` in a shell and leave that shell open afterwards.
pub fn agent_command(cli: &str, prompt: &str) -> (String, Vec<String>) {
    if cfg!(windows) {
        (
            "powershell".into(),
            vec!["-NoLogo".into(), "-NoExit".into(), "-Command".into(), format!("{cli} {}", ps_quote(prompt))],
        )
    } else {
        let sh = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let line = format!("{cli} {}; exec {} -l", sh_quote(prompt), sh_quote(&sh));
        (sh, vec!["-l".into(), "-c".into(), line])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(y: i32, m: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
    }

    fn auto(s: Schedule) -> Automation {
        Automation { id: "a".into(), name: "n".into(), project: "p".into(), agent: "claude".into(), prompt: "x".into(), schedule: s, enabled: true, last_run: 0 }
    }

    #[test]
    fn interval_fires_after_the_gap() {
        let a = auto(Schedule::Interval { minutes: 30 });
        let last = at(2026, 10, 20, 9, 0);
        assert!(!due(&a, at(2026, 10, 20, 9, 29), last));
        assert!(due(&a, at(2026, 10, 20, 9, 30), last));
    }

    #[test]
    fn daily_fires_once_per_slot() {
        let a = auto(Schedule::Daily { time: "09:00".into() });
        // created yesterday evening: the 09:00 slot today is new
        assert!(due(&a, at(2026, 10, 20, 9, 1), at(2026, 10, 19, 18, 0)));
        // already ran at 09:00:30: not again the same day
        assert!(!due(&a, at(2026, 10, 20, 15, 0), at(2026, 10, 20, 9, 0)));
        // before today's slot and the last run is after yesterday's slot
        assert!(!due(&a, at(2026, 10, 20, 8, 0), at(2026, 10, 19, 9, 5)));
        // created at 10:00: nothing retroactive for 09:00 the same day
        assert!(!due(&a, at(2026, 10, 20, 10, 5), at(2026, 10, 20, 10, 0)));
    }

    #[test]
    fn weekly_uses_the_chosen_weekday() {
        // 2026-10-20 is a Tuesday (day 1)
        let a = auto(Schedule::Weekly { day: 1, time: "08:30".into() });
        assert!(due(&a, at(2026, 10, 20, 8, 31), at(2026, 10, 14, 12, 0)));
        assert!(!due(&a, at(2026, 10, 21, 8, 31), at(2026, 10, 20, 8, 31)));
        let monday = auto(Schedule::Weekly { day: 0, time: "08:30".into() });
        assert!(!due(&monday, at(2026, 10, 20, 8, 31), at(2026, 10, 19, 8, 31)));
        assert!(due(&monday, at(2026, 10, 26, 8, 31), at(2026, 10, 19, 8, 31)));
    }

    #[test]
    fn disabled_never_fires_and_validation_catches_nonsense() {
        let mut a = auto(Schedule::Interval { minutes: 5 });
        a.enabled = false;
        assert!(!due(&a, at(2030, 1, 1, 0, 0), at(2020, 1, 1, 0, 0)));
        assert!(validate(&auto(Schedule::Interval { minutes: 1 })).is_err());
        assert!(validate(&auto(Schedule::Daily { time: "25:00".into() })).is_err());
        assert!(validate(&auto(Schedule::Weekly { day: 7, time: "10:00".into() })).is_err());
        let mut b = auto(Schedule::Daily { time: "10:00".into() });
        b.agent = "rm".into();
        assert!(validate(&b).is_err());
        assert!(validate(&auto(Schedule::Daily { time: "10:00".into() })).is_ok());
    }

    #[test]
    fn prompts_are_quoted_for_the_shell() {
        assert_eq!(ps_quote("it's $x"), "'it''s $x'");
        assert_eq!(sh_quote("it's $x"), "'it'\\''s $x'");
        let (_, args) = agent_command("claude", "a 'b'");
        assert!(args.last().unwrap().contains("claude"));
    }
}
