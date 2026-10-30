//! Windows file metadata for `axelstra.exe`: a VERSIONINFO resource (product, publisher, version, copyright) and an
//! application manifest (`axelstra.exe.manifest`: asInvoker, Windows 10/11, long paths), so the binary says plainly
//! what it is and who ships it. Nothing is done for other targets.

const OWNER: &str = "Francisco Cruz";
/// Year of the copyright line in `LICENSE`.
const COPYRIGHT_YEAR: &str = "2026";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=axelstra.exe.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let part = |k: &str| env(k).parse::<u16>().unwrap_or(0);
    let (major, minor, patch) = (part("CARGO_PKG_VERSION_MAJOR"), part("CARGO_PKG_VERSION_MINOR"), part("CARGO_PKG_VERSION_PATCH"));
    let version = env("CARGO_PKG_VERSION");
    let dir = std::path::PathBuf::from(env("CARGO_MANIFEST_DIR"));
    // rc.exe and windres both read forward slashes; a backslash would start an escape inside the quoted name
    let manifest = dir.join("axelstra.exe.manifest").display().to_string().replace('\\', "/");
    let rc = format!(
        r#"1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3F
FILEFLAGS 0x0
FILEOS 0x40004
FILETYPE 0x1
FILESUBTYPE 0x0
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "{OWNER}"
      VALUE "ProductName", "Axelstra"
      VALUE "FileDescription", "Axelstra CLI and daemon"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
      VALUE "InternalName", "axelstra"
      VALUE "OriginalFilename", "axelstra.exe"
      VALUE "LegalCopyright", "Copyright (c) {COPYRIGHT_YEAR} {OWNER}. MIT License."
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let out = std::path::PathBuf::from(env("OUT_DIR")).join("axelstra.rc");
    std::fs::write(&out, rc).expect("write axelstra.rc");
    embed_resource::compile(&out, embed_resource::NONE).manifest_required().expect("embed the Windows resources of axelstra.exe");
}
