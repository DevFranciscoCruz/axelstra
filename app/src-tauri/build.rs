fn main() {
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
