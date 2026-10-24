//! Scheduled agent runs: "every morning, have an agent look at X". Each run gets its own fresh worktree and
//! an interactive agent session started with the prompt, so the result waits in the sidebar like any other agent.
//! Runs missed while the daemon was off are skipped, not replayed.

use anyhow::Result;
use axelstra_core::coded;
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
    /// id of an agent in the core agents table (claude, codex, gemini)
    pub agent: String,
    pub prompt: String,
    pub schedule: Schedule,
    pub enabled: bool,
    /// unix seconds of the last start (or of creation: nothing fires retroactively)
    pub last_run: i64,
}

const MAX_INTERVAL_MINUTES: u32 = 60 * 24 * 366;

pub fn store_path(projects_store: &Path) -> PathBuf {
    projects_store.with_file_name("automations.json")
}

/// Entries that do not parse or fail `validate` are skipped, so one bad entry cannot drop the rest.
pub fn load(store: &Path) -> Vec<Automation> {
    let Some(items) = crate::store::read_json::<Vec<serde_json::Value>>(store).ok().flatten() else {
        return Vec::new();
    };
    items.into_iter().filter_map(|v| serde_json::from_value::<Automation>(v).ok()).filter(|a| validate(a).is_ok()).collect()
}

pub fn save_all(store: &Path, v: &[Automation]) -> Result<()> {
    crate::store::write_json(store, v)?;
    Ok(())
}

fn parse_time(t: &str) -> Result<NaiveTime> {
    NaiveTime::parse_from_str(t.trim(), "%H:%M").map_err(|_| coded!("automation.bad_time", "time must be HH:MM, got {time:?}", time = t))
}

pub fn validate(a: &Automation) -> Result<()> {
    if a.name.trim().is_empty() {
        return Err(coded!("automation.name_empty", "name is empty"));
    }
    if a.prompt.trim().is_empty() {
        return Err(coded!("automation.prompt_empty", "prompt is empty"));
    }
    axelstra_core::agents::require(&a.agent)?;
    match &a.schedule {
        Schedule::Interval { minutes } if *minutes < 5 => {
            return Err(coded!("automation.interval_min", "interval must be at least {min} minutes", min = 5))
        }
        Schedule::Interval { minutes } if *minutes > MAX_INTERVAL_MINUTES => {
            return Err(coded!("automation.interval_max", "interval must be at most a year"))
        }
        Schedule::Interval { .. } => {}
        Schedule::Daily { time } => {
            parse_time(time)?;
        }
        Schedule::Weekly { day, time } => {
            if *day > 6 {
                return Err(coded!("automation.bad_day", "day must be 0 (Monday) to 6 (Sunday)"));
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
            if today <= now {
                today
            } else {
                today - Duration::days(1)
            }
        }
        Schedule::Weekly { day, time } => {
            let back = (now.date().weekday().num_days_from_monday() + 7 - *day) % 7;
            let this = (now.date() - Duration::days(back as i64)).and_time(parse_time(time)?);
            if this <= now {
                this
            } else {
                this - Duration::days(7)
            }
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

/// Every character PowerShell reads as a single quote (`'` and U+2018..U+201B) doubled, the way its
/// single-quoted strings escape it.
pub fn ps_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        out.push(c);
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// POSIX single-quote escaping.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Program and args that run the agent `cli` (an id from the agents table) on `prompt` in a shell and leave that
/// shell open afterwards. Every argument is quoted; the agent's own flags come from the table.
pub fn agent_command(cli: &str, prompt: &str, trusted: bool) -> Result<(String, Vec<String>)> {
    let argv = axelstra_core::agents::require(cli)?.start_argv(prompt, trusted);
    Ok(shell_command(&argv))
}

/// Same, for a program named outside the table (the `AXELSTRA_AGENT_CLI` override).
pub fn raw_command(cli: &str, prompt: &str) -> (String, Vec<String>) {
    shell_command(&[cli.to_string(), prompt.to_string()])
}

fn shell_command(argv: &[String]) -> (String, Vec<String>) {
    if cfg!(windows) {
        let line = argv.iter().map(|a| ps_quote(a)).collect::<Vec<_>>().join(" ");
        // `&` calls the quoted program name (a .cmd shim included)
        ("powershell".into(), vec!["-NoLogo".into(), "-NoExit".into(), "-Command".into(), format!("& {line}")])
    } else {
        let sh = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let line = format!("{}; exec {} -l", argv.iter().map(|a| sh_quote(a)).collect::<Vec<_>>().join(" "), sh_quote(&sh));
        (sh, vec!["-l".into(), "-c".into(), line])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_carry_message_codes() {
        let c = |a: &Automation| axelstra_core::msg::find(&validate(a).unwrap_err()).map(|c| c.code.clone()).unwrap_or_default();
        assert_eq!(c(&auto(Schedule::Interval { minutes: 1 })), "automation.interval_min");
        assert_eq!(c(&auto(Schedule::Interval { minutes: u32::MAX })), "automation.interval_max");
        assert_eq!(c(&auto(Schedule::Weekly { day: 7, time: "09:00".into() })), "automation.bad_day");
        assert_eq!(c(&auto(Schedule::Daily { time: "9h".into() })), "automation.bad_time");
        let mut a = auto(Schedule::Interval { minutes: 10 });
        a.name = " ".into();
        assert_eq!(c(&a), "automation.name_empty");
        a.name = "n".into();
        a.prompt = "".into();
        assert_eq!(c(&a), "automation.prompt_empty");
    }
    use chrono::NaiveDate;

    fn at(y: i32, m: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
    }

    fn auto(s: Schedule) -> Automation {
        Automation {
            id: "a".into(),
            name: "n".into(),
            project: "p".into(),
            agent: "claude".into(),
            prompt: "x".into(),
            schedule: s,
            enabled: true,
            last_run: 0,
        }
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
    fn load_skips_bad_entries() {
        let dir = std::env::temp_dir().join(format!("axl-auto-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("automations.json");
        let good = serde_json::to_value(auto(Schedule::Daily { time: "10:00".into() })).unwrap();
        let mut bad_agent = good.clone();
        bad_agent["agent"] = "rm".into();
        let mut bad_sched = good.clone();
        bad_sched["schedule"] = serde_json::json!({"kind": "interval", "minutes": 1});
        let items = serde_json::json!([good, bad_agent, bad_sched, {"nonsense": 1}]);
        std::fs::write(&f, items.to_string()).unwrap();
        assert_eq!(load(&f).len(), 1);
        assert!(validate(&auto(Schedule::Interval { minutes: u32::MAX })).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prompts_are_quoted_for_the_shell() {
        assert_eq!(ps_quote("it's $x"), "'it''s $x'");
        // typographic quotes would end the string too if left single
        assert_eq!(ps_quote("a\u{2019};calc;\u{2018}"), "'a\u{2019}\u{2019};calc;\u{2018}\u{2018}'");
        assert_eq!(ps_quote("\u{201A}\u{201B}"), "'\u{201A}\u{201A}\u{201B}\u{201B}'");
        assert_eq!(sh_quote("it's $x"), "'it'\\''s $x'");
        let (_, args) = agent_command("claude", "a 'b'", true).unwrap();
        assert!(args.last().unwrap().contains("claude"));
    }

    #[test]
    fn automation_commands_come_from_the_agents_table() {
        let gem = agent_command("gemini", "do it", true).unwrap().1.last().unwrap().clone();
        assert!(gem.contains("gemini") && gem.contains("-i") && gem.contains("do it"), "{gem}");
        assert!(agent_command("powershell", "x", true).is_err());
    }

    #[test]
    fn unknown_agent_is_rejected() {
        for ok in ["claude", "codex", "gemini"] {
            let mut a = auto(Schedule::Interval { minutes: 30 });
            a.agent = ok.into();
            assert!(validate(&a).is_ok(), "{ok}");
        }
        let mut a = auto(Schedule::Interval { minutes: 30 });
        a.agent = "rm".into();
        assert!(validate(&a).unwrap_err().to_string().contains("unknown agent"));
    }
}
