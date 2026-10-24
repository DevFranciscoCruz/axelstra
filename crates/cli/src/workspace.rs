//! `axelstra workspace export|import`: move the workspace configuration (projects, automations, settings, kanban cards)
//! to a file and back. The daemon does the work (`workspace.*`) and refuses it to agents, so these commands only work
//! from a normal terminal: inside an Axelstra terminal the CLI has the agent role.

use anyhow::{bail, Context, Result};
use axelstra_daemon::{client::Client, proto};
use clap::Subcommand;
use serde_json::{json, Value};
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub enum WsCmd {
    /// Write projects, automations, settings and kanban cards to one JSON file (never tokens or sessions).
    /// Not available inside an Axelstra terminal: run it from a normal terminal
    Export {
        file: PathBuf,
        /// overwrite the file when it exists
        #[arg(long)]
        force: bool,
    },
    /// Read a file written by `export`: shows what would change, then asks. Without --merge or --replace it merges.
    /// Not available inside an Axelstra terminal: run it from a normal terminal
    Import {
        file: PathBuf,
        /// add what is missing and keep what is here (the default)
        #[arg(long, conflicts_with = "replace")]
        merge: bool,
        /// swap the projects, automations and settings for the file's (the old files stay as `.1` copies)
        #[arg(long)]
        replace: bool,
        /// what to do with an item that exists here and differs: keep the local one, or take the file's
        #[arg(long, value_parser = ["keep", "overwrite"], default_value = "keep")]
        conflicts: String,
        /// OLD=NEW: project folders under OLD in the file live under NEW on this machine (repeat for more)
        #[arg(long = "map", value_name = "OLD=NEW")]
        map: Vec<String>,
        /// only show what would change
        #[arg(long)]
        dry_run: bool,
        /// do not ask
        #[arg(long)]
        yes: bool,
    },
}

/// `--map OLD=NEW` pairs as the daemon takes them; NEW is made absolute, because it names a folder here.
fn parse_maps(raw: &[String]) -> Result<Vec<Value>> {
    raw.iter()
        .map(|m| {
            let Some((old, new)) = m.split_once('=').filter(|(o, n)| !o.is_empty() && !n.is_empty()) else {
                bail!("--map expects OLD=NEW, got {m:?}");
            };
            Ok(json!([old, absolute(Path::new(new))?]))
        })
        .collect()
}

/// The daemon resolves paths from its own working directory, so the CLI sends absolute ones.
fn absolute(p: &Path) -> Result<String> {
    Ok(std::path::absolute(p).with_context(|| format!("bad path {}", p.display()))?.to_string_lossy().into_owned())
}

fn names(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// The preview as text: per kind of item, how many are new, already here, different, and skipped (with why).
fn render(pv: &Value) -> String {
    let mut out = String::new();
    for (key, title) in [("projects", "projects"), ("automations", "automations"), ("settings", "settings"), ("cards", "cards")] {
        let s = &pv[key];
        let skipped = s["skipped"].as_array().cloned().unwrap_or_default();
        out.push_str(&format!(
            "{title}: {} new, {} already here, {} different, {} skipped, {} here only\n",
            names(&s["new"]).len(),
            names(&s["present"]).len(),
            names(&s["conflict"]).len(),
            skipped.len(),
            s["local_only"].as_u64().unwrap_or(0)
        ));
        for (label, list) in [("different", names(&s["conflict"]))] {
            for n in list {
                out.push_str(&format!("  {label}: {n}\n"));
            }
        }
        for sk in skipped.iter().take(20) {
            out.push_str(&format!("  skipped ({}): {}\n", sk["reason"].as_str().unwrap_or("?"), sk["label"].as_str().unwrap_or("?")));
        }
        if skipped.len() > 20 {
            out.push_str(&format!("  ... and {} more skipped\n", skipped.len() - 20));
        }
    }
    let risky = pv["risky_cards"].as_u64().unwrap_or(0);
    if risky > 0 {
        out.push_str(&format!("warning: {risky} card(s) run with permission checks off or with an acceptance command\n"));
    }
    out
}

/// Asks on the terminal. A replace needs the word `replace`; a merge a `y`.
fn confirm(replace: bool) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!("not a terminal: pass --yes to import without asking");
    }
    if replace {
        eprint!("Replace replaces the projects, automations and settings of this machine (the old files stay as .1 copies). Type `replace` to go on: ");
    } else {
        eprint!("Import? [y/N] ");
    }
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let a = line.trim().to_lowercase();
    Ok(if replace { a == "replace" } else { a == "y" || a == "yes" })
}

fn connect(host: Option<&str>) -> Result<Client> {
    if host.is_some() {
        bail!("workspace export and import work on this machine's daemon: run them on the machine itself");
    }
    Client::connect_cli(&proto::pipe_name(), &proto::load_or_create_token()?)
}

pub fn run(cmd: WsCmd, host: Option<&str>) -> Result<()> {
    match cmd {
        WsCmd::Export { file, force } => {
            if file.exists() && !force {
                bail!("{} exists: pass --force to overwrite it", file.display());
            }
            let mut c = connect(host)?;
            let r = c.call("workspace.export", json!({"path": absolute(&file)?}))?;
            let s = &r["summary"];
            println!(
                "{}: {} projects, {} automations, {} cards",
                r["path"].as_str().unwrap_or_default(),
                s["projects"],
                s["automations"],
                s["cards"]
            );
        }
        WsCmd::Import { file, merge: _, replace, conflicts, map, dry_run, yes } => {
            let map = parse_maps(&map)?;
            let path = absolute(&file)?;
            let mut c = connect(host)?;
            let pv = c.call("workspace.import_preview", json!({"path": path, "map": map}))?;
            print!("{}", render(&pv));
            if dry_run {
                return Ok(());
            }
            if !yes && !confirm(replace)? {
                println!("nothing imported");
                return Ok(());
            }
            let mode = if replace { "replace" } else { "merge" };
            let r = c.call("workspace.import", json!({"path": path, "mode": mode, "conflicts": conflicts, "map": map}))?;
            println!(
                "imported: {} projects added, {} updated; {} automations added, {} updated; {} settings; {} cards written, {} kept",
                r["projects_added"],
                r["projects_updated"],
                r["automations_added"],
                r["automations_updated"],
                r["settings_set"],
                r["cards_written"],
                r["cards_kept"]
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_pairs_need_both_sides() {
        let ok = parse_maps(&["/a=/b".to_string()]).unwrap();
        assert_eq!(ok[0][0], "/a");
        assert!(ok[0][1].as_str().unwrap().ends_with('b'));
        for bad in ["", "=", "/a=", "=/b", "/a"] {
            assert!(parse_maps(&[bad.to_string()]).is_err(), "{bad}");
        }
    }

    #[test]
    fn render_counts_and_lists_what_needs_a_look() {
        let pv = json!({
            "projects": {"new": ["a"], "present": [], "conflict": ["b (/b)"], "skipped": [{"label": "c (/c)", "reason": "missing"}], "local_only": 2},
            "automations": {"new": [], "present": [], "conflict": [], "skipped": [], "local_only": 0},
            "settings": {"new": ["terse"], "present": [], "conflict": [], "skipped": [], "local_only": 0},
            "cards": {"new": [], "present": [], "conflict": [], "skipped": [], "local_only": 0},
            "risky_cards": 2
        });
        let t = render(&pv);
        assert!(t.contains("projects: 1 new, 0 already here, 1 different, 1 skipped, 2 here only"), "{t}");
        assert!(t.contains("different: b (/b)"), "{t}");
        assert!(t.contains("skipped (missing): c (/c)"), "{t}");
        assert!(t.contains("2 card(s) run with permission checks off"), "{t}");
    }

    #[test]
    fn remote_hosts_are_refused() {
        assert!(connect(Some("ssh:box")).err().is_some_and(|e| e.to_string().contains("this machine")));
    }
}
