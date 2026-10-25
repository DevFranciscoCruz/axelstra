//! What this build is: version, short commit and build date (UTC). Set by `build.rs` from `AXELSTRA_GIT_SHA` and
//! `AXELSTRA_BUILD_DATE`, else from git and the clock.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COMMIT: &str = env!("AXELSTRA_BUILDINFO_COMMIT");
pub const BUILT: &str = env!("AXELSTRA_BUILDINFO_BUILT");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_filled() {
        assert!(!VERSION.is_empty());
        assert!(COMMIT == "unknown" || (COMMIT.len() >= 7 && COMMIT.chars().all(|c| c.is_ascii_alphanumeric())), "{COMMIT}");
        let b: Vec<&str> = BUILT.split('-').collect();
        assert!(b.len() == 3 && b[0].len() == 4 && b[1].len() == 2 && b[2].len() == 2, "{BUILT}");
        assert!(b.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())), "{BUILT}");
    }
}
