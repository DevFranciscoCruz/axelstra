//! The host registry: the `hosts` setting (remote daemons the app can reach). See docs/remote-daemon.md.

use crate::remote::{check_bin, RemoteSpec};
use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Most hosts the registry holds.
pub const MAX_HOSTS: usize = 16;

fn id_ok(id: &str) -> bool {
    let mut c = id.chars();
    id.len() <= 24
        && c.next().is_some_and(|f| f.is_ascii_lowercase())
        && c.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Checks a `hosts` value of `settings.set` and returns the normalized list; nothing is saved from an invalid one.
/// `null` and `[]` clear it. Unknown keys of an entry are dropped.
pub fn validate(v: &Value) -> Result<Value> {
    if v.is_null() {
        return Ok(json!([]));
    }
    let Some(list) = v.as_array() else { return Err(axelstra_core::coded!("host.list_not_array", "hosts must be an array")) };
    if list.len() > MAX_HOSTS {
        return Err(axelstra_core::coded!("host.list_too_long", "hosts holds at most {max} entries", max = MAX_HOSTS));
    }
    let (mut ids, mut slots) = (HashSet::new(), HashSet::new());
    let mut out = Vec::with_capacity(list.len());
    for (i, h) in list.iter().enumerate() {
        let Some(o) = h.as_object() else {
            return Err(axelstra_core::coded!("host.entry_not_object", "hosts[{i}] must be an object", i = i));
        };
        let id = o.get("id").and_then(Value::as_str).unwrap_or_default();
        if !id_ok(id) {
            return Err(axelstra_core::coded!("host.id_invalid", "hosts[{i}].id must match ^[a-z][a-z0-9-]{{0,23}}$", i = i));
        }
        if id == "local" {
            return Err(axelstra_core::coded!("host.id_reserved", "hosts[{i}].id \"local\" is reserved", i = i));
        }
        if !ids.insert(id.to_string()) {
            return Err(axelstra_core::coded!("host.id_twice", "hosts[{i}].id {id:?} is used twice", i = i, id = id));
        }
        let slot = match o.get("slot").and_then(Value::as_u64) {
            Some(s @ 1..=127) => s,
            _ => return Err(axelstra_core::coded!("host.slot_invalid", "hosts[{i}].slot must be a whole number from 1 to 127", i = i)),
        };
        if !slots.insert(slot) {
            return Err(axelstra_core::coded!("host.slot_twice", "hosts[{i}].slot {slot} is used twice", i = i, slot = slot));
        }
        let name = o.get("name").and_then(Value::as_str).unwrap_or_default();
        if !(1..=40).contains(&name.chars().count()) || name.chars().any(char::is_control) {
            return Err(axelstra_core::coded!(
                "host.name_invalid",
                "hosts[{i}].name must have 1 to 40 characters, no control characters",
                i = i
            ));
        }
        let spec = o.get("spec").and_then(Value::as_str).unwrap_or_default().trim();
        // `parse` never yields the test-only Exec variant
        RemoteSpec::parse(spec).with_context(|| format!("hosts[{i}].spec"))?;
        let mut e = Map::new();
        e.insert("id".into(), json!(id));
        e.insert("name".into(), json!(name));
        e.insert("spec".into(), json!(spec));
        e.insert("slot".into(), json!(slot));
        match o.get("bin") {
            None | Some(Value::Null) => {}
            Some(Value::String(b)) => {
                check_bin(b).with_context(|| format!("hosts[{i}].bin"))?;
                e.insert("bin".into(), json!(b));
            }
            Some(_) => return Err(axelstra_core::coded!("host.bin_not_string", "hosts[{i}].bin must be a string", i = i)),
        }
        out.push(Value::Object(e));
    }
    Ok(Value::Array(out))
}

/// Most directories `host.browse` lists.
pub const MAX_ENTRIES: usize = 500;

fn home() -> Option<PathBuf> {
    ["USERPROFILE", "HOME"].iter().filter_map(std::env::var_os).find(|h| !h.is_empty()).map(PathBuf::from)
}

/// A path as shown to the user: no `\\?\` prefix, and `\\?\UNC\` turned back into `\\`.
fn display(p: &Path) -> String {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\UNC\") {
        Some(r) => format!(r"\\{r}"),
        None => s.strip_prefix(r"\\?\").unwrap_or(&s).to_string(),
    }
}

fn hostname() -> String {
    ["HOSTNAME", "COMPUTERNAME"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|h| !h.is_empty())
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|h| h.trim().to_string()).filter(|h| !h.is_empty()))
        .unwrap_or_default()
}

/// What `host.info` answers.
pub fn info() -> Value {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let shell = if cfg!(windows) { None } else { std::env::var("SHELL").ok().filter(|s| !s.is_empty()) };
    let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
    json!({"os": os, "home": home().map(|h| display(&h)), "user": user, "hostname": hostname(), "shell": shell,
        "sep": std::path::MAIN_SEPARATOR.to_string(), "proto": crate::proto::PROTO, "build": crate::proto::BUILD_ID})
}

/// What `host.browse` answers: the directories under `path` (default: home), sorted without regard to case.
pub fn browse(path: Option<&str>) -> Result<Value> {
    let home = home().map(|h| display(&h));
    let asked = match path.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(home.clone().ok_or_else(|| axelstra_core::coded!("host.no_home", "no home directory on this machine"))?),
    };
    if !asked.is_absolute() {
        return Err(axelstra_core::coded!("host.not_absolute", "{path} is not an absolute path", path = asked.display()));
    }
    let dir = match std::fs::canonicalize(&asked) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(axelstra_core::coded!("host.dir_missing", "{path} does not exist", path = asked.display()))
        }
        Err(e) => {
            return Err(axelstra_core::coded!("host.cannot_open", "cannot open {path}: {detail}", path = asked.display(), detail = e))
        }
    };
    if !dir.is_dir() {
        return Err(axelstra_core::coded!("host.not_dir", "{path} is not a directory", path = display(&dir)));
    }
    let mut names: Vec<(String, bool)> = std::fs::read_dir(&dir)
        .map_err(|e| axelstra_core::coded!("host.cannot_read", "cannot read {path}: {detail}", path = display(&dir), detail = e))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path().join(".git").exists()))
        .collect();
    names.sort_by_key(|(n, _)| (n.to_lowercase(), n.clone()));
    let truncated = names.len() > MAX_ENTRIES;
    names.truncate(MAX_ENTRIES);
    let entries: Vec<Value> = names.into_iter().map(|(name, git)| json!({"name": name, "git": git})).collect();
    let mut out = json!({"path": display(&dir), "parent": dir.parent().map(display), "home": home, "entries": entries});
    if truncated {
        out["truncated"] = json!(true);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_rejects_relative_and_missing() {
        assert!(browse(Some("rel/dir")).unwrap_err().to_string().contains("not an absolute"));
        let missing = std::env::temp_dir().join("axelstra-no-such-dir-xyz");
        assert!(browse(Some(&missing.to_string_lossy())).unwrap_err().to_string().contains("does not exist"));
    }

    #[test]
    fn browse_truncates_and_sorts_without_case() {
        let d = std::env::temp_dir().join(format!("axelstra-browse-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for n in ["b", "A", "c"] {
            std::fs::create_dir_all(d.join(n)).unwrap();
        }
        std::fs::write(d.join("file.txt"), "x").unwrap();
        let r = browse(Some(&d.to_string_lossy())).unwrap();
        let names: Vec<&str> = r["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["A", "b", "c"]);
        assert!(r.get("truncated").is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn display_drops_the_verbatim_prefix() {
        assert_eq!(display(Path::new(r"\\?\C:\a")), r"C:\a");
        assert_eq!(display(Path::new(r"\\?\UNC\srv\sh")), r"\\srv\sh");
        assert_eq!(display(Path::new("/a/b")), "/a/b");
    }

    fn h(id: &str, slot: u64) -> Value {
        json!({"id": id, "name": "Box", "spec": "ssh:me@box:2222", "slot": slot})
    }

    #[test]
    fn accepts_and_normalizes() {
        let v = json!([
            {"id": "box", "name": "Box", "spec": "  ssh:me@box  ", "slot": 1, "bin": "~/bin/axelstra", "extra": 1},
            {"id": "wsl-1", "name": "WSL", "spec": "wsl:Ubuntu", "slot": 127}
        ]);
        let out = validate(&v).unwrap();
        assert_eq!(out[0], json!({"id": "box", "name": "Box", "spec": "ssh:me@box", "slot": 1, "bin": "~/bin/axelstra"}));
        assert_eq!(out[1], json!({"id": "wsl-1", "name": "WSL", "spec": "wsl:Ubuntu", "slot": 127}));
        assert_eq!(validate(&Value::Null).unwrap(), json!([]));
        assert_eq!(validate(&json!([])).unwrap(), json!([]));
    }

    #[test]
    fn rejects_bad_ids() {
        for id in ["", "local", "Box", "1a", "-a", "a_b", "a b", "a".repeat(25).as_str()] {
            assert!(validate(&json!([h(id, 1)])).is_err(), "{id:?}");
        }
        assert!(validate(&json!([h(&"a".repeat(24), 1)])).is_ok());
        assert!(validate(&json!([h("a", 1), h("a", 2)])).is_err());
        assert!(validate(&json!([{"name": "x", "spec": "wsl:U", "slot": 1}])).is_err());
    }

    #[test]
    fn rejects_bad_slots() {
        for s in [json!(0), json!(128), json!(-1), json!(1.5), json!("1"), Value::Null] {
            let v = json!([{"id": "a", "name": "A", "spec": "wsl:U", "slot": s}]);
            assert!(validate(&v).is_err(), "{s}");
        }
        assert!(validate(&json!([h("a", 3), h("b", 3)])).is_err());
    }

    #[test]
    fn rejects_bad_names() {
        for n in [json!(""), json!("x".repeat(41)), json!("a\nb"), json!("a\u{7}"), json!(5)] {
            let v = json!([{"id": "a", "name": n, "spec": "wsl:U", "slot": 1}]);
            assert!(validate(&v).is_err(), "{n}");
        }
        let ok = json!([{"id": "a", "name": "x".repeat(40), "spec": "wsl:U", "slot": 1}]);
        assert!(validate(&ok).is_ok());
    }

    #[test]
    fn rejects_bad_specs_and_bins() {
        for s in ["ssh:-oProxyCommand=x", "ssh:a b", "ssh:host\nfoo", "", "ftp:host", "exec:sh", "wsl:", "ssh:-l@host", "ssh:host:0"] {
            let v = json!([{"id": "a", "name": "A", "spec": s, "slot": 1}]);
            assert!(validate(&v).is_err(), "{s:?}");
        }
        for b in ["", "-x", "a b", "a;b", "$(x)"] {
            let v = json!([{"id": "a", "name": "A", "spec": "wsl:U", "slot": 1, "bin": b}]);
            assert!(validate(&v).is_err(), "{b:?}");
        }
        assert!(validate(&json!([{"id": "a", "name": "A", "spec": "wsl:U", "slot": 1, "bin": 3}])).is_err());
    }

    #[test]
    fn rejects_bad_shapes_and_too_many() {
        assert!(validate(&json!({"a": 1})).is_err());
        assert!(validate(&json!(["x"])).is_err());
        let many: Vec<Value> = (1..=17).map(|i| h(&format!("h{i}"), i)).collect();
        assert!(validate(&Value::Array(many.clone())).is_err());
        assert!(validate(&Value::Array(many[..16].to_vec())).is_ok());
    }

    #[test]
    fn the_exec_variant_cannot_be_produced() {
        for s in ["exec:sh", "exec:sh -c x", "sh", "/bin/sh", "wsl:U", "ssh:h"] {
            if let Ok(spec) = RemoteSpec::parse(s) {
                assert!(!matches!(spec, RemoteSpec::Exec { .. }), "{s}");
            }
        }
    }
}

#[cfg(test)]
mod code_tests {
    use super::*;
    use axelstra_core::msg::find;

    #[test]
    fn validation_errors_carry_codes() {
        let c = |v: Value| find(&validate(&v).unwrap_err()).unwrap().code.clone();
        assert_eq!(c(json!({})), "host.list_not_array");
        assert_eq!(c(json!([1])), "host.entry_not_object");
        assert_eq!(c(json!([{"id": "local", "slot": 1, "name": "x", "spec": "wsl:a"}])), "host.id_reserved");
        assert_eq!(c(json!([{"id": "a", "slot": 0, "name": "x", "spec": "wsl:a"}])), "host.slot_invalid");
        // the inner spec error keeps its own code under the `hosts[i].spec` context
        assert_eq!(c(json!([{"id": "a", "slot": 1, "name": "x", "spec": "nope"}])), "host.spec_unknown");
        assert_eq!(find(&browse(Some("relative/dir")).unwrap_err()).unwrap().code, "host.not_absolute");
    }
}
