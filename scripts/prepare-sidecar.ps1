# Builds the daemon and places it where Tauri's externalBin expects it. The installer then
# ships it beside the app exe as `axelstra.exe`, which is where the app looks for it.
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    cargo build --release -p axelstra
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    $dest = Join-Path $root "app/src-tauri/binaries"
    New-Item -ItemType Directory -Force $dest | Out-Null
    Copy-Item "target/release/axelstra.exe" (Join-Path $dest "axelstra-x86_64-pc-windows-msvc.exe") -Force
} finally { Pop-Location }
