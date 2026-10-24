//! Stable message codes for errors and notices that reach the UI.
//!
//! A [`Coded`] error carries a `code` (`"git.not_repo"`, in the frontend dictionaries), `params` (`{"path": ".."}`) and the English text. The text is
//! what the CLI and the logs show (`Display`); the UI looks the code up in its dictionaries (`app/src/msg/*.ts`) and
//! fills the params into the sentence of its language, so a daemon that knows no better still reads fine.
//!
//! On the wire the code is additive: a response is `{"id":n,"error":"English text","code":"..","params":{..}}`, so an
//! old frontend keeps showing `error`. Raise one with [`coded!`]:
//!
//! ```ignore
//! return Err(coded!("test.example", "{path} is not a git repository", path = dir.display()));
//! ```
//!
//! Every `{name}` of the text must be listed as `name = value`; those values are the params. A code is
//! `domain.snake_case` and its params are the same wherever it is used. `all_codes()` (test only) scans the sources, and
//! the frontend test checks that every code has a pt-PT, English and pt-BR text.
use anyhow::Error;
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Coded {
    pub code: String,
    pub params: BTreeMap<String, String>,
    /// English text: what the CLI and the logs show
    pub text: String,
}

impl Coded {
    pub fn new(code: &str, text: String, params: &[(&str, String)]) -> Coded {
        Coded { code: code.to_string(), params: params.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(), text }
    }

    /// The same code and params with other English text (a caller prefix such as `method: `).
    pub fn with_text(&self, text: String) -> Coded {
        Coded { text, ..self.clone() }
    }

    pub fn params_json(&self) -> Value {
        json!(self.params)
    }

    /// `{"code":..,"params":{..}}`, the fields a response, event or notice carries next to its English text.
    pub fn fields(&self) -> Value {
        json!({"code": self.code, "params": self.params_json()})
    }

    /// Reads back what [`Coded::fields`] wrote (`text` is the English message that came with it).
    pub fn from_wire(code: &str, params: &Value, text: &str) -> Option<Coded> {
        if code.is_empty() {
            return None;
        }
        let params = params
            .as_object()
            .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))).collect())
            .unwrap_or_default();
        Some(Coded { code: code.to_string(), params, text: text.to_string() })
    }
}

impl std::fmt::Display for Coded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl std::error::Error for Coded {}

/// The first coded error in the chain of `e`, if any (an outer `.context()` does not hide it).
pub fn find(e: &Error) -> Option<&Coded> {
    e.chain().find_map(|c| c.downcast_ref::<Coded>())
}

/// `{"code":..,"params":{..}}` for the error, or `null` when it carries no code.
pub fn fields(e: &Error) -> Value {
    find(e).map(Coded::fields).unwrap_or(Value::Null)
}

/// Builds an `anyhow::Error` that carries a message code; see the module docs.
#[macro_export]
macro_rules! coded {
    ($code:literal, $text:literal $(, $k:ident = $v:expr)* $(,)?) => {{
        $( let $k = ($v).to_string(); )*
        $crate::msg::Coded::new($code, format!($text), &[$((stringify!($k), $k.clone())),*]).into_error()
    }};
}

impl Coded {
    pub fn into_error(self) -> Error {
        Error::new(self)
    }
}

#[cfg(test)]
pub fn all_codes() -> Vec<(String, std::path::PathBuf)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let mut files = vec![];
    for d in ["crates/core/src", "crates/daemon/src", "crates/cli/src", "app/src-tauri/src"] {
        walk(&root.join(d), &mut files);
    }
    let mut out = vec![];
    for f in files {
        let src = std::fs::read_to_string(&f).unwrap_or_default();
        for pat in ["coded!(\"", "Coded::new(\"", "notice_coded(\""] {
            for (i, _) in src.match_indices(pat) {
                let rest = &src[i + pat.len()..];
                if let Some(end) = rest.find('"') {
                    out.push((rest[..end].to_string(), f.clone()));
                }
            }
        }
    }
    // the codes the tests of this module use are not real ones
    out.retain(|(c, _)| !c.starts_with("test."));
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macro_fills_text_and_params() {
        let e = coded!("test.thing", "{n} items in {dir}", n = 3, dir = "x/y");
        assert_eq!(format!("{e}"), "3 items in x/y");
        let c = find(&e).unwrap();
        assert_eq!(c.code, "test.thing");
        assert_eq!(c.params["n"], "3");
        assert_eq!(c.params["dir"], "x/y");
    }

    #[test]
    fn context_does_not_hide_the_code() {
        let e = coded!("test.thing", "plain").context("while doing x");
        assert_eq!(find(&e).unwrap().code, "test.thing");
        assert_eq!(format!("{e:#}"), "while doing x: plain");
        assert_eq!(fields(&anyhow::anyhow!("no code")), Value::Null);
    }

    #[test]
    fn wire_roundtrip() {
        let c = Coded::new("test.b", "t".into(), &[("k", "v".into())]);
        let f = c.fields();
        let back = Coded::from_wire(f["code"].as_str().unwrap(), &f["params"], "t").unwrap();
        assert_eq!(back, c);
        assert!(Coded::from_wire("", &Value::Null, "x").is_none());
    }

    /// every code is `domain.snake_case` (letters, digits, `_`, at least one dot); the real list is `all_codes()`
    #[test]
    fn codes_are_well_formed() {
        for (code, file) in all_codes() {
            let ok = code.contains('.')
                && code.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'));
            assert!(ok, "bad code {code:?} in {}", file.display());
        }
    }

    /// every code raised in Rust has a text in each of the three frontend dictionaries (`app/src/msg/*.ts`); the
    /// vitest `msg.test.ts` checks the same from the other side, plus the placeholders
    #[test]
    fn every_code_has_pt_en_and_ptbr_texts_in_the_frontend() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/src/msg");
        let mut texts = String::new();
        for e in std::fs::read_dir(&dir).expect("app/src/msg").flatten() {
            if e.path().extension().is_some_and(|x| x == "ts") && !e.file_name().to_string_lossy().contains("test") {
                texts.push_str(&std::fs::read_to_string(e.path()).unwrap());
            }
        }
        let mut missing = vec![];
        for (code, file) in all_codes() {
            let n = texts.matches(&format!("\"{code}\":")).count();
            if n < 3 {
                missing.push(format!("{code} ({n} of 3 texts, raised in {})", file.display()));
            }
        }
        assert!(
            missing.is_empty(),
            "codes without pt-PT, en and pt-BR texts:
{}",
            missing.join(
                "
"
            )
        );
    }
}
