use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default = "default_agent")]
    pub agent: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_timeout")]
    pub timeout_min: f64,
    #[serde(default)]
    pub max_retries: u32,
    #[serde(default)]
    pub acceptance: Option<String>,
}

fn default_status() -> String {
    "backlog".into()
}
fn default_agent() -> String {
    "claude".into()
}
fn default_timeout() -> f64 {
    45.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub meta: Meta,
    pub body: String,
    #[serde(skip)]
    pub path: PathBuf,
}

pub fn parse(text: &str, path: &Path) -> Result<Card> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let rest = text
        .strip_prefix("---\n")
        .with_context(|| format!("{}: missing frontmatter", path.display()))?;
    let (front, body) = rest
        .split_once("\n---")
        .with_context(|| format!("{}: unterminated frontmatter", path.display()))?;
    let body = body.trim_start_matches('\n').to_string();
    let meta: Meta = serde_yaml::from_str(front)
        .with_context(|| format!("{}: bad frontmatter", path.display()))?;
    Ok(Card { meta, body, path: path.to_path_buf() })
}

pub fn load_dir(dir: &Path) -> Result<Vec<Card>> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.extension().is_some_and(|x| x == "md") {
            let t = std::fs::read_to_string(&p)?;
            out.push(parse(&t, &p)?);
        }
    }
    out.sort_by(|a, b| a.meta.id.cmp(&b.meta.id));
    Ok(out)
}

/// Rewrites only the `status:` line so the rest of the card stays byte-identical.
pub fn set_status(path: &Path, status: &str) -> Result<()> {
    let text = std::fs::read_to_string(path)?;
    let mut done = false;
    let mut in_front = false;
    let mut out = String::with_capacity(text.len() + 16);
    for (i, line) in text.split_inclusive('\n').enumerate() {
        let t = line.trim_end();
        if i == 0 && t == "---" {
            in_front = true;
        } else if in_front && t == "---" {
            if !done {
                out.push_str(&format!("status: {status}\n"));
                done = true;
            }
            in_front = false;
        } else if in_front && !done && t.starts_with("status:") {
            let nl = if line.ends_with("\r\n") { "\r\n" } else { "\n" };
            out.push_str(&format!("status: {status}{nl}"));
            done = true;
            continue;
        }
        out.push_str(line);
    }
    // temp file + rename keeps the write atomic
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Validates ids/deps and returns ids in a dependency-respecting order.
pub fn topo_order(cards: &[Card]) -> Result<Vec<String>> {
    let ids: HashSet<&str> = cards.iter().map(|c| c.meta.id.as_str()).collect();
    if ids.len() != cards.len() {
        bail!("duplicate card ids");
    }
    for c in cards {
        for d in &c.meta.depends_on {
            if !ids.contains(d.as_str()) {
                bail!("card '{}' depends on unknown '{}'", c.meta.id, d);
            }
        }
    }
    let mut indeg: HashMap<&str, usize> =
        cards.iter().map(|c| (c.meta.id.as_str(), c.meta.depends_on.len())).collect();
    let mut order = Vec::new();
    let mut ready: Vec<&Card> = cards.iter().filter(|c| c.meta.depends_on.is_empty()).collect();
    while !ready.is_empty() {
        ready.sort_by_key(|c| (-c.meta.priority, c.meta.id.clone()));
        let c = ready.remove(0);
        order.push(c.meta.id.clone());
        for o in cards.iter().filter(|o| o.meta.depends_on.contains(&c.meta.id)) {
            let n = indeg.get_mut(o.meta.id.as_str()).unwrap();
            *n -= 1;
            if *n == 0 {
                ready.push(o);
            }
        }
    }
    if order.len() != cards.len() {
        bail!("dependency cycle among cards");
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str, deps: &[&str]) -> Card {
        let t = format!(
            "---\nid: {id}\nstatus: todo\ndepends_on: [{}]\n---\nbody\n",
            deps.join(",")
        );
        parse(&t, Path::new(id)).unwrap()
    }

    #[test]
    fn orders_by_deps() {
        let cs = vec![card("b", &["a"]), card("a", &[]), card("c", &["b"])];
        assert_eq!(topo_order(&cs).unwrap(), ["a", "b", "c"]);
    }

    #[test]
    fn detects_cycle() {
        let cs = vec![card("a", &["b"]), card("b", &["a"])];
        assert!(topo_order(&cs).is_err());
    }

    #[test]
    fn set_status_keeps_rest() {
        let d = std::env::temp_dir().join("axelstra_card_test.md");
        std::fs::write(&d, "---\nid: x\nstatus: todo\n---\nhello\n").unwrap();
        set_status(&d, "done").unwrap();
        assert_eq!(std::fs::read_to_string(&d).unwrap(), "---\nid: x\nstatus: done\n---\nhello\n");
    }
}
