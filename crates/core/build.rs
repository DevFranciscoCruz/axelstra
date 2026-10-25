//! Bakes the commit and build date into `buildinfo`. `AXELSTRA_GIT_SHA` (short sha) and `AXELSTRA_BUILD_DATE`
//! (UTC `YYYY-MM-DD`) win; otherwise git and the clock fill them in.
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=AXELSTRA_GIT_SHA");
    println!("cargo:rerun-if-env-changed=AXELSTRA_BUILD_DATE");
    // best effort: a new commit re-stamps the build (the git dir of a worktree is not `.git`)
    if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        println!("cargo:rerun-if-changed={dir}/HEAD");
    }

    let commit = env("AXELSTRA_GIT_SHA").or_else(|| git(&["rev-parse", "--short=7", "HEAD"])).unwrap_or_else(|| "unknown".into());
    let built = env("AXELSTRA_BUILD_DATE").unwrap_or_else(today);
    println!("cargo:rustc-env=AXELSTRA_BUILDINFO_COMMIT={commit}");
    println!("cargo:rustc-env=AXELSTRA_BUILDINFO_BUILT={built}");
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8(out.stdout).ok()?.trim().to_string()).filter(|s| !s.is_empty())
}

/// Today in UTC as `YYYY-MM-DD` (days-to-civil, no date crate).
fn today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let z = (secs / 86400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}
