//! Resolves a path seen in a terminal (`Ctrl+click`) to an HTML file the app may hand to the system opener.
//!
//! Decision: the file must be a regular `.html`/`.htm` file after following symlinks. A *relative* path must also land
//! inside one of the session's roots (cwd, worktree, project root) once canonical, so a symlink or `..` cannot lead out.
//! An *absolute* path is allowed anywhere: the user clicked it explicitly, and only html/htm pass, never an executable
//! or a script. Roots are compared by path components, so `/a/proj-evil` is not inside `/a/proj`.

use anyhow::Result;
use axelstra_core::coded;
use std::path::{Path, PathBuf};

pub struct Resolved {
    /// Canonical absolute path, without the Windows `\\?\` prefix.
    pub path: String,
    pub url: String,
}

/// Drops trailing punctuation a terminal line puts after a path, and a `:line[:col]` suffix, in any order.
pub(crate) fn clean(raw: &str) -> &str {
    let mut s = raw.trim().trim_matches(['"', '\'', '`', '<', '>']);
    loop {
        let before = s;
        s = s.trim_end_matches(['.', ',', ';', ':', ')', ']']);
        // `:12` or `:12:5`; never the colon of a drive letter (`C:`)
        if let Some(i) = s.rfind(':') {
            let digits = &s[i + 1..];
            if i > 1 && !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                s = &s[..i];
            }
        }
        if s == before {
            return s;
        }
    }
}

fn is_html(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
}

/// `\\?\C:\x` -> `C:\x`, `\\?\UNC\srv\share` -> `\\srv\share`.
pub(crate) fn strip_verbatim(p: &str) -> String {
    if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        p.strip_prefix(r"\\?\").unwrap_or(p).to_string()
    }
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `file://` URL of a plain (non-verbatim) absolute path.
fn file_url(path: &str) -> String {
    let p = path.replace('\\', "/");
    if let Some(unc) = p.strip_prefix("//") {
        let (host, rest) = unc.split_once('/').unwrap_or((unc, ""));
        return format!("file://{host}/{}", encode(rest));
    }
    // `C:/x` -> `file:///C:/x`; the drive colon stays literal
    match p.as_bytes() {
        [d, b':', ..] if d.is_ascii_alphabetic() => format!("file:///{}:{}", &p[..1], encode(&p[2..])),
        _ => format!("file://{}", encode(&p)),
    }
}

/// `cwd` is the session's folder (relative paths start there); `roots` are the folders a relative path may stay in.
pub fn resolve(raw: &str, cwd: Option<&Path>, roots: &[PathBuf]) -> Result<Resolved> {
    let cleaned = clean(raw);
    if cleaned.is_empty() {
        return Err(coded!("files.html_not_found", "file not found: {path}", path = raw));
    }
    let given = Path::new(cleaned);
    if !is_html(given) {
        return Err(coded!("files.html_not_allowed", "not allowed to open {path}: only .html and .htm files", path = cleaned));
    }
    let explicit = given.is_absolute() || given.has_root();
    let full = if explicit {
        given.to_path_buf()
    } else {
        cwd.ok_or_else(|| coded!("files.html_not_found", "file not found: {path}", path = cleaned))?.join(given)
    };
    let real = std::fs::canonicalize(&full).map_err(|_| coded!("files.html_not_found", "file not found: {path}", path = cleaned))?;
    if !real.is_file() {
        return Err(coded!("files.html_not_file", "{path} is not a file", path = cleaned));
    }
    if !is_html(&real) {
        return Err(coded!("files.html_not_allowed", "not allowed to open {path}: only .html and .htm files", path = cleaned));
    }
    if !explicit {
        let inside = roots.iter().filter_map(|r| std::fs::canonicalize(r).ok()).any(|r| real.starts_with(r));
        if !inside {
            return Err(coded!("files.html_not_allowed", "not allowed to open {path}: it is outside the session folder", path = cleaned));
        }
    }
    let path = strip_verbatim(&real.to_string_lossy());
    let url = file_url(&path);
    Ok(Resolved { path, url })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("axh-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::canonicalize(&d).unwrap()
    }

    fn code(r: Result<Resolved>) -> String {
        axelstra_core::msg::find(&r.err().expect("expected an error")).map(|c| c.code.clone()).unwrap_or_default()
    }

    #[test]
    fn clean_strips_punctuation_and_line_suffix() {
        assert_eq!(clean("a/b.html"), "a/b.html");
        assert_eq!(clean("a/b.html."), "a/b.html");
        assert_eq!(clean("(a/b.html)"), "(a/b.html");
        assert_eq!(clean("a/b.html),"), "a/b.html");
        assert_eq!(clean("a/b.html:12"), "a/b.html");
        assert_eq!(clean("a/b.html:12:5"), "a/b.html");
        assert_eq!(clean("a/b.html:12:5."), "a/b.html");
        assert_eq!(clean("a/b.html]:"), "a/b.html");
        assert_eq!(clean("C:\\x\\b.html:7"), "C:\\x\\b.html");
        assert_eq!(clean("\"C:\\x y\\b.html\""), "C:\\x y\\b.html");
        assert_eq!(clean("   "), "");
    }

    #[test]
    fn urls_are_encoded() {
        assert_eq!(file_url("/a b/c#d.html"), "file:///a%20b/c%23d.html");
        assert_eq!(file_url("C:\\My Dir\\é.html"), "file:///C:/My%20Dir/%C3%A9.html");
        assert_eq!(file_url(r"\\srv\share\a b.html"), "file://srv/share/a%20b.html");
        assert_eq!(strip_verbatim(r"\\?\C:\x\a.html"), r"C:\x\a.html");
        assert_eq!(strip_verbatim(r"\\?\UNC\srv\share\a.html"), r"\\srv\share\a.html");
    }

    #[test]
    fn relative_with_spaces_line_and_punctuation() {
        let d = tmp("rel");
        std::fs::create_dir_all(d.join("my docs")).unwrap();
        std::fs::write(d.join("my docs/Page One.HTML"), "<p>").unwrap();
        for raw in ["my docs/Page One.HTML", "./my docs/Page One.HTML:3:9", "my docs/Page One.HTML)."] {
            let r = resolve(raw, Some(&d), std::slice::from_ref(&d)).unwrap_or_else(|e| panic!("{raw}: {e:#}"));
            assert!(r.path.ends_with("Page One.HTML"), "{}", r.path);
            assert!(!r.path.starts_with(r"\\?\"), "{}", r.path);
            assert!(r.url.starts_with("file:///") && r.url.contains("my%20docs/Page%20One.HTML"), "{}", r.url);
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn absolute_outside_roots_is_allowed_when_html() {
        let (a, b) = (tmp("abs-a"), tmp("abs-b"));
        std::fs::write(b.join("x.htm"), "").unwrap();
        std::fs::write(b.join("x.js"), "").unwrap();
        let abs = b.join("x.htm");
        let r = resolve(&abs.to_string_lossy(), Some(&a), std::slice::from_ref(&a)).unwrap();
        assert!(r.url.ends_with("/x.htm"));
        assert_eq!(code(resolve(&b.join("x.js").to_string_lossy(), Some(&a), std::slice::from_ref(&a))), "files.html_not_allowed");
        // the same file relative is outside the root
        let rel = format!("..{}{}{}x.htm", std::path::MAIN_SEPARATOR, b.file_name().unwrap().to_string_lossy(), std::path::MAIN_SEPARATOR);
        assert_eq!(code(resolve(&rel, Some(&a), std::slice::from_ref(&a))), "files.html_not_allowed");
        let _ = (std::fs::remove_dir_all(&a), std::fs::remove_dir_all(&b));
    }

    #[test]
    fn sibling_prefix_dir_is_not_inside() {
        let base = tmp("sib");
        let (proj, evil) = (base.join("proj"), base.join("proj-evil"));
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::create_dir_all(&evil).unwrap();
        std::fs::write(evil.join("e.html"), "").unwrap();
        std::fs::write(proj.join("p.html"), "").unwrap();
        assert_eq!(code(resolve("../proj-evil/e.html", Some(&proj), std::slice::from_ref(&proj))), "files.html_not_allowed");
        assert!(resolve("p.html", Some(&proj), std::slice::from_ref(&proj)).is_ok());
        // a root of the project is enough even when the cwd is elsewhere inside it
        std::fs::create_dir_all(proj.join("sub")).unwrap();
        assert!(resolve("../p.html", Some(&proj.join("sub")), std::slice::from_ref(&proj)).is_ok());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn missing_directory_and_wrong_extension() {
        let d = tmp("bad");
        std::fs::create_dir_all(d.join("dir.html")).unwrap();
        std::fs::write(d.join("a.txt"), "").unwrap();
        std::fs::write(d.join("run.html.exe"), "").unwrap();
        let roots = std::slice::from_ref(&d);
        assert_eq!(code(resolve("nope.html", Some(&d), roots)), "files.html_not_found");
        assert_eq!(code(resolve("dir.html", Some(&d), roots)), "files.html_not_file");
        assert_eq!(code(resolve("a.txt", Some(&d), roots)), "files.html_not_allowed");
        assert_eq!(code(resolve("run.html.exe", Some(&d), roots)), "files.html_not_allowed");
        assert_eq!(code(resolve("", Some(&d), roots)), "files.html_not_found");
        assert_eq!(code(resolve("a.html", None, roots)), "files.html_not_found");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_refused_for_relative_paths() {
        let base = tmp("sym");
        let (proj, out) = (base.join("proj"), base.join("out"));
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("secret.html"), "").unwrap();
        std::fs::write(out.join("secret.txt"), "").unwrap();
        std::os::unix::fs::symlink(out.join("secret.html"), proj.join("link.html")).unwrap();
        std::os::unix::fs::symlink(&out, proj.join("dir")).unwrap();
        // a link named .html whose target is not html is refused too
        std::os::unix::fs::symlink(out.join("secret.txt"), proj.join("t.html")).unwrap();
        let roots = std::slice::from_ref(&proj);
        assert_eq!(code(resolve("link.html", Some(&proj), roots)), "files.html_not_allowed");
        assert_eq!(code(resolve("dir/secret.html", Some(&proj), roots)), "files.html_not_allowed");
        assert_eq!(code(resolve("t.html", Some(&proj), roots)), "files.html_not_allowed");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(windows)]
    #[test]
    fn windows_drive_path_and_verbatim_prefix() {
        let d = tmp("win");
        std::fs::write(d.join("a b.html"), "").unwrap();
        let abs = d.join("a b.html");
        let r = resolve(&format!("{}:4:2", strip_verbatim(&abs.to_string_lossy())), None, &[]).unwrap();
        assert!(!r.path.starts_with(r"\\?\"), "{}", r.path);
        assert!(r.url.starts_with("file:///") && r.url.ends_with("/a%20b.html") && !r.url.contains('\\'), "{}", r.url);
        // forward slashes and a verbatim path both resolve
        assert!(resolve(&abs.to_string_lossy().replace('\\', "/"), None, &[]).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }
}
