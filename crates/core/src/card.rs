use crate::agents;
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
    /// claude --permission-mode value
    #[serde(default = "default_perm")]
    pub permission_mode: String,
}

fn default_perm() -> String {
    "auto".into()
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

/// `--permission-mode` values of the claude CLI (2.1.309 lists "manual"). "default" is what older claude versions
/// called it: still accepted so cards saved before the rename keep loading.
pub const PERMISSION_MODES: [&str; 7] = ["acceptEdits", "auto", "bypassPermissions", "default", "dontAsk", "manual", "plan"];

/// Ids and run names end up in worktree paths and branch names: `[A-Za-z0-9._-]`, no `..`.
pub fn valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && !s.starts_with('.')
        && !s.contains("..")
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Upper bound of `timeout_min` (one week): keeps `Duration` math far from overflow.
pub const MAX_TIMEOUT_MIN: f64 = 7.0 * 24.0 * 60.0;

/// `bypass*` modes switch the agent's permission checks off.
pub fn is_bypass(mode: &str) -> bool {
    mode.starts_with("bypass")
}

fn validate(m: &Meta) -> Result<()> {
    if !valid_slug(&m.id) {
        bail!("invalid card id {:?}: use letters, digits, '.', '_' or '-' (no '..')", m.id);
    }
    if agents::get(&m.agent).is_none() {
        bail!("card '{}': unknown agent {:?}", m.id, m.agent);
    }
    if !PERMISSION_MODES.contains(&m.permission_mode.as_str()) {
        bail!("card '{}': unknown permission_mode {:?}", m.id, m.permission_mode);
    }
    if !m.timeout_min.is_finite() || m.timeout_min <= 0.0 || m.timeout_min > MAX_TIMEOUT_MIN {
        bail!("card '{}': timeout_min must be a number in (0, {MAX_TIMEOUT_MIN}], got {}", m.id, m.timeout_min);
    }
    Ok(())
}

/// SHA-256 over everything of the cards that decides what runs (agent, mode, acceptance, prompt, ...), in id order.
/// Pins the content a user confirmed: any later edit of a card changes it.
pub fn fingerprint(cards: &[Card]) -> String {
    let mut v: Vec<&Card> = cards.iter().collect();
    v.sort_by(|a, b| a.meta.id.cmp(&b.meta.id));
    let mut buf = Vec::new();
    for c in v {
        let m = &c.meta;
        let fields = [
            m.id.clone(),
            m.agent.clone(),
            m.permission_mode.clone(),
            m.acceptance.clone().unwrap_or_default(),
            m.acceptance.is_some().to_string(),
            m.timeout_min.to_bits().to_string(),
            m.max_retries.to_string(),
            m.priority.to_string(),
            m.depends_on.join(","),
            c.body.clone(),
        ];
        for f in fields {
            buf.extend_from_slice(&(f.len() as u64).to_le_bytes());
            buf.extend_from_slice(f.as_bytes());
        }
    }
    sha256_hex(&buf)
}

fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be,
        0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa,
        0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85,
        0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
        0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f,
        0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let t1 = hh
                .wrapping_add(e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25))
                .wrapping_add((e & f) ^ (!e & g))
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let t2 = (a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22)).wrapping_add((a & b) ^ (a & c) ^ (b & c));
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

pub fn parse(text: &str, path: &Path) -> Result<Card> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let rest = text.strip_prefix("---\n").with_context(|| format!("{}: missing frontmatter", path.display()))?;
    let (front, body) = rest.split_once("\n---").with_context(|| format!("{}: unterminated frontmatter", path.display()))?;
    let body = body.trim_start_matches('\n').to_string();
    let meta: Meta = serde_norway::from_str(front).with_context(|| format!("{}: bad frontmatter", path.display()))?;
    validate(&meta).with_context(|| path.display().to_string())?;
    Ok(Card { meta, body, path: path.to_path_buf() })
}

/// Cards found in a directory plus the files that could not be read or parsed.
#[derive(Debug, Default)]
pub struct Loaded {
    pub cards: Vec<Card>,
    pub broken: Vec<(PathBuf, String)>,
}

impl IntoIterator for Loaded {
    type Item = Card;
    type IntoIter = std::vec::IntoIter<Card>;
    fn into_iter(self) -> Self::IntoIter {
        self.cards.into_iter()
    }
}

/// One unreadable card does not hide the others: it is returned in `broken`.
pub fn load_dir(dir: &Path) -> Result<Loaded> {
    let mut out = Loaded::default();
    if !dir.is_dir() {
        return Ok(out);
    }
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.extension().is_some_and(|x| x == "md") {
            match std::fs::read_to_string(&p).map_err(anyhow::Error::from).and_then(|t| parse(&t, &p)) {
                Ok(c) => out.cards.push(c),
                Err(e) => out.broken.push((p, format!("{e:#}"))),
            }
        }
    }
    out.cards.sort_by(|a, b| a.meta.id.cmp(&b.meta.id));
    out.broken.sort();
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
    let mut indeg: HashMap<&str, usize> = cards.iter().map(|c| (c.meta.id.as_str(), c.meta.depends_on.len())).collect();
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
        let t = format!("---\nid: {id}\nstatus: todo\ndepends_on: [{}]\n---\nbody\n", deps.join(","));
        parse(&t, Path::new(id)).unwrap()
    }

    #[test]
    fn orders_by_deps() {
        let cs = vec![card("b", &["a"]), card("a", &[]), card("c", &["b"])];
        assert_eq!(topo_order(&cs).unwrap(), ["a", "b", "c"]);
    }

    #[test]
    fn permission_mode_defaults_to_auto_and_can_be_overridden() {
        let c = card("a", &[]);
        assert_eq!(c.meta.permission_mode, "auto");
        let t = "---
id: x
permission_mode: plan
---
";
        assert_eq!(parse(t, Path::new("x")).unwrap().meta.permission_mode, "plan");
    }

    #[test]
    fn current_and_old_permission_modes_are_accepted() {
        for mode in ["manual", "default", "acceptEdits"] {
            let t = format!(
                "---
id: x
permission_mode: {mode}
---
"
            );
            assert_eq!(parse(&t, Path::new("x")).unwrap().meta.permission_mode, mode);
        }
    }

    #[test]
    fn agent_check_follows_the_agents_table_including_aliases() {
        for a in ["claude", "codex", "gemini", "chatgpt"] {
            let t = format!(
                "---
id: x
agent: {a}
---
"
            );
            assert!(parse(&t, Path::new("x")).is_ok(), "{a}");
        }
        assert!(parse(
            "---
id: x
agent: nope
---
",
            Path::new("x")
        )
        .is_err());
    }

    #[test]
    fn rejects_unsafe_ids_agents_and_modes() {
        for id in ["x/../../y", "..", "a..b", "a b", r"a\b", ".hidden", ""] {
            let t = format!(
                "---
id: {id:?}
---
"
            );
            assert!(parse(&t, Path::new("x")).is_err(), "id {id:?} accepted");
        }
        assert!(parse(
            "---
id: ok.1_x-y
---
",
            Path::new("x")
        )
        .is_ok());
        assert!(parse(
            "---
id: x
agent: rm
---
",
            Path::new("x")
        )
        .is_err());
        assert!(parse(
            "---
id: x
permission_mode: \"--evil\"
---
",
            Path::new("x")
        )
        .is_err());
    }

    #[test]
    fn load_dir_skips_bad_files() {
        let d = std::env::temp_dir().join(format!("axelstra_card_dir_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("a.md"),
            "---
id: a
---
hi
",
        )
        .unwrap();
        std::fs::write(d.join("bad.md"), "no frontmatter").unwrap();
        std::fs::write(
            d.join("evil.md"),
            "---
id: ../evil
---
",
        )
        .unwrap();
        let l = load_dir(&d).unwrap();
        assert_eq!(l.cards.len(), 1);
        assert_eq!(l.broken.len(), 2);
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

    #[test]
    fn timeout_min_must_be_finite_positive_and_capped() {
        let mk = |v: &str| format!("---
id: a
timeout_min: {v}
---
body
");
        for bad in ["-1", "0", ".nan", ".inf", "-.inf", "1e30", "10081"] {
            assert!(parse(&mk(bad), Path::new("x")).is_err(), "{bad} accepted");
        }
        for good in ["0.5", "45", "10080"] {
            assert!(parse(&mk(good), Path::new("x")).is_ok(), "{good} rejected");
        }
    }

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn fingerprint_changes_when_a_card_changes() {
        let c = |mode: &str, body: &str| parse(&format!("---
id: a
permission_mode: {mode}
---
{body}
"), Path::new("x")).unwrap();
        let base = fingerprint(&[c("auto", "do")]);
        assert_eq!(base, fingerprint(&[c("auto", "do")]));
        assert_ne!(base, fingerprint(&[c("bypassPermissions", "do")]));
        assert_ne!(base, fingerprint(&[c("auto", "do more")]));
        assert!(is_bypass("bypassPermissions") && !is_bypass("auto"));
    }
}
