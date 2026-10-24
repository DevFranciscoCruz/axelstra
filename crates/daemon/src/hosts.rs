//! The host registry: the `hosts` setting (remote daemons the app can reach). See docs/remote-daemon.md.

use crate::remote::{check_bin, RemoteSpec};
use anyhow::{bail, Result};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

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
    let Some(list) = v.as_array() else { bail!("hosts must be an array") };
    if list.len() > MAX_HOSTS {
        bail!("hosts holds at most {MAX_HOSTS} entries");
    }
    let (mut ids, mut slots) = (HashSet::new(), HashSet::new());
    let mut out = Vec::with_capacity(list.len());
    for (i, h) in list.iter().enumerate() {
        let Some(o) = h.as_object() else { bail!("hosts[{i}] must be an object") };
        let id = o.get("id").and_then(Value::as_str).unwrap_or_default();
        if !id_ok(id) {
            bail!("hosts[{i}].id must match ^[a-z][a-z0-9-]{{0,23}}$");
        }
        if id == "local" {
            bail!("hosts[{i}].id \"local\" is reserved");
        }
        if !ids.insert(id.to_string()) {
            bail!("hosts[{i}].id {id:?} is used twice");
        }
        let slot = match o.get("slot").and_then(Value::as_u64) {
            Some(s @ 1..=127) => s,
            _ => bail!("hosts[{i}].slot must be a whole number from 1 to 127"),
        };
        if !slots.insert(slot) {
            bail!("hosts[{i}].slot {slot} is used twice");
        }
        let name = o.get("name").and_then(Value::as_str).unwrap_or_default();
        if !(1..=40).contains(&name.chars().count()) || name.chars().any(char::is_control) {
            bail!("hosts[{i}].name must have 1 to 40 characters, no control characters");
        }
        let spec = o.get("spec").and_then(Value::as_str).unwrap_or_default().trim();
        // `parse` never yields the test-only Exec variant
        RemoteSpec::parse(spec).map_err(|e| anyhow::anyhow!("hosts[{i}].spec: {e}"))?;
        let mut e = Map::new();
        e.insert("id".into(), json!(id));
        e.insert("name".into(), json!(name));
        e.insert("spec".into(), json!(spec));
        e.insert("slot".into(), json!(slot));
        match o.get("bin") {
            None | Some(Value::Null) => {}
            Some(Value::String(b)) => {
                check_bin(b).map_err(|e| anyhow::anyhow!("hosts[{i}].bin: {e}"))?;
                e.insert("bin".into(), json!(b));
            }
            Some(_) => bail!("hosts[{i}].bin must be a string"),
        }
        out.push(Value::Object(e));
    }
    Ok(Value::Array(out))
}

#[cfg(test)]
mod tests {
    use super::*;

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
