/// UTC `YYYY-MM-DD` of today (days since the epoch to a civil date, no date crate in the build).
fn today_utc() -> String {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Build metadata shared with the engine: `AXELSTRA_GIT_SHA` (7 chars, else `git rev-parse`, else "unknown") and
/// `AXELSTRA_BUILD_DATE` (UTC date, else today), handed to the crate as `env!` values.
fn build_metadata() {
    println!("cargo:rerun-if-env-changed=AXELSTRA_GIT_SHA");
    println!("cargo:rerun-if-env-changed=AXELSTRA_BUILD_DATE");
    // best effort: the git dir of a worktree is not `.git` (there it is a file)
    let git_dir = std::process::Command::new("git")
        .args(["rev-parse", "--absolute-git-dir"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match git_dir {
        Some(dir) => println!("cargo:rerun-if-changed={dir}/HEAD"),
        None => println!("cargo:rerun-if-changed=../../.git/HEAD"),
    }
    let given = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let sha = given("AXELSTRA_GIT_SHA")
        .map(|s| s.chars().take(7).collect::<String>())
        .or_else(|| {
            let out = std::process::Command::new("git").args(["rev-parse", "--short=7", "HEAD"]).output().ok()?;
            let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
            (out.status.success() && !s.is_empty()).then_some(s)
        })
        .unwrap_or_else(|| "unknown".into());
    let date = given("AXELSTRA_BUILD_DATE").unwrap_or_else(today_utc);
    println!("cargo:rustc-env=AXELSTRA_GIT_SHA={sha}");
    println!("cargo:rustc-env=AXELSTRA_BUILD_DATE={date}");
}

fn main() {
    build_metadata();
    // The prebuilt static sherpa-onnx libs are /MDd builds in a debug profile and import the debug CRT
    // (ucrtbased.dll), which the release CRT Rust links cannot share: the debug test exe aborts at startup.
    // Dropping the debug CRT default libs for this binary makes those imports resolve to the release CRT.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") && std::env::var("PROFILE").as_deref() == Ok("debug") {
        for lib in ["ucrtd.lib", "msvcrtd.lib", "vcruntimed.lib"] {
            println!("cargo:rustc-link-arg-bins=/NODEFAULTLIB:{lib}");
        }
    }
    tauri_build::build()
}
