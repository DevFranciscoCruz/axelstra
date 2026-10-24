//! One path normalization for comparing paths from different sources (UI, git, the OS).

use std::path::Path;

/// True where the file system ignores case by default (Windows, macOS); only there is lowercasing correct.
const CASE_INSENSITIVE: bool = cfg!(any(windows, target_os = "macos"));

/// Comparison key for a path string: `\?\` verbatim prefix dropped, separators unified to `/` and trailing ones
/// trimmed (the root itself stays `/`), lowercased only on case-insensitive platforms. Backslashes are separators
/// only on Windows: on Unix they are legal file name characters. Use it for equality/prefix checks, not for I/O.
pub fn norm(p: &str) -> String {
    let p = p.strip_prefix(r"\\?\").unwrap_or(p);
    let s: String = if cfg!(windows) { p.replace('\\', "/") } else { p.to_string() };
    let t = s.trim_end_matches('/');
    let t = if t.is_empty() && s.starts_with('/') { "/" } else { t };
    if CASE_INSENSITIVE {
        t.to_lowercase()
    } else {
        t.to_string()
    }
}

/// `norm` of a `Path`.
pub fn norm_path(p: &Path) -> String {
    norm(&p.to_string_lossy())
}

/// True when both paths name the same location after normalization.
pub fn same(a: &str, b: &str) -> bool {
    norm(a) == norm(b)
}

/// True when `child` is `parent` or lies below it (component-wise: `/a/bc` is not inside `/a/b`).
pub fn is_within(child: &str, parent: &str) -> bool {
    let (c, p) = (norm(child), norm(parent));
    if p == "/" {
        return c.starts_with('/');
    }
    c == p || c.strip_prefix(&p).is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_and_unifies() {
        assert_eq!(norm("/a/b/"), "/a/b");
        assert_eq!(norm("/"), "/");
        if cfg!(windows) {
            assert_eq!(norm(r"\\?\C:\Users\X\"), "c:/users/x");
        }
    }

    #[test]
    fn case_only_folds_where_the_fs_does() {
        assert_eq!(same("/Repo/A", "/repo/a"), CASE_INSENSITIVE);
    }

    #[test]
    fn backslash_is_a_separator_only_on_windows() {
        assert_eq!(same(r"a\b", "a/b"), cfg!(windows));
    }

    #[test]
    fn within_is_component_wise() {
        assert!(is_within("/a/b/c", "/a/b"));
        assert!(is_within("/a/b", "/a/b/"));
        assert!(!is_within("/a/bc", "/a/b"));
        assert!(is_within("/x", "/"));
    }
}
