# One command to produce the installer: daemon sidecar + frontend + NSIS bundle.
# Output: target/release/bundle/nsis/Axelstra_<version>_x64-setup.exe
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
& (Join-Path $PSScriptRoot "prepare-sidecar.ps1")
Push-Location (Join-Path $root "app")
try {
    if (-not (Test-Path node_modules)) { npm ci; if ($LASTEXITCODE -ne 0) { throw "npm ci failed" } }
    npx tauri build --config src-tauri/tauri.release.conf.json
    if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }
} finally { Pop-Location }
Get-ChildItem (Join-Path $root "target/release/bundle/nsis/*-setup.exe") | ForEach-Object { Write-Host "Installer: $($_.FullName)" }
