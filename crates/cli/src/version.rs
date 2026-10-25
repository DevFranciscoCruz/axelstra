//! `axelstra --version` and `axelstra version [--json]`: what this binary is, and the daemon it would talk to.

use crate::remote;
use anyhow::Result;
use axelstra_core::buildinfo;
use axelstra_daemon::proto::{BUILD_ID, PROTO, SECURITY_MIN};
use serde_json::{json, Value};

/// The part clap prints after the program name: `X.Y.Z (commit, built DATE, PROTO N)`.
pub fn short() -> &'static str {
    static S: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    S.get_or_init(|| format!("{} ({}, built {}, PROTO {PROTO})", buildinfo::VERSION, buildinfo::COMMIT, buildinfo::BUILT))
}

/// This binary as JSON (what `version --json` prints, minus the daemon).
pub fn local() -> Value {
    json!({"version": buildinfo::VERSION, "commit": buildinfo::COMMIT, "built": buildinfo::BUILT, "proto": PROTO,
        "security_min": SECURITY_MIN, "build_id": BUILD_ID})
}

/// What the daemon said in `hello`, or null when it is not reachable. A daemon from before the build fields existed
/// answers without `version`/`commit`/`built`, which show as null.
fn daemon(host: Option<&str>) -> Value {
    match remote::connect(host) {
        Ok(c) => {
            let h = c.hello_result();
            json!({"version": h["version"], "commit": h["commit"], "built": h["built"], "proto": c.proto,
                "security_min": c.security, "build_id": c.build})
        }
        Err(_) => Value::Null,
    }
}

fn text(v: &Value) -> String {
    v.as_str().map_or_else(|| "unknown".into(), str::to_string)
}

pub fn run(json: bool, host: Option<&str>) -> Result<()> {
    let mut out = local();
    out["daemon"] = daemon(host);
    if json {
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    println!("axelstra {}", short());
    let d = &out["daemon"];
    if d.is_null() {
        println!("daemon: not running");
    } else {
        println!("daemon: {} ({}, built {}, PROTO {})", text(&d["version"]), text(&d["commit"]), text(&d["built"]), d["proto"]);
    }
    Ok(())
}
