# One command to produce the installer, in steps: daemon sidecar, frontend, NSIS bundle.
# Each step prints its time and the peak memory of the processes it started, so a heavy step is easy to spot.
# Output: target/release/bundle/nsis/Axelstra_<version>_x64-setup.exe
#
#   -Version 0.1.19   installer version (default: auto, see below)
#   -Jobs 4           parallel compile jobs (default: half the logical cores, fewer when free memory is short, see below;
#                     an explicit -Jobs always wins)
#
# Version: without -Version the build is stamped with the next release version (scripts/next-version.mjs: the
# Conventional Commits since the last tag), raised one patch above the installed copy when that one is already as
# high, so the installer always upgrades what is installed. The version goes into the four files that carry it
# (Cargo.toml, Cargo.lock, app/package.json, app/src-tauri/tauri.conf.json) for the build, and their exact previous
# bytes are written back afterwards, also when the build fails: the tree is left as it was. Do not edit those four
# files while a build runs.
param(
    [string]$Version,
    [int]$Jobs
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$app = Join-Path $root "app"
if (-not $PSBoundParameters.ContainsKey("Jobs")) {
    # same arithmetic as jobsFor() in scripts/lib/jobs.mjs: min(cores / 2, (free memory - 2 GB reserve) / 1.5 GB per job).
    # A release link with lto is the heavy part, so the floor here is 2 (a release build with one job takes too long), not 1.
    $freeBytes = [uint64](Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory * 1KB
    $byCpu = [Math]::Max(1, [int]([Environment]::ProcessorCount / 2))
    $byMem = [int][Math]::Floor([Math]::Max(0.0, [double]$freeBytes - 2GB) / 1.5GB)
    $Jobs = [Math]::Max(2, [Math]::Min($byCpu, $byMem))
    Write-Host "Jobs: $Jobs (derived from $([Environment]::ProcessorCount) logical cores and $([Math]::Round($freeBytes / 1GB, 1)) GB free memory; -Jobs overrides)"
}
$env:CARGO_BUILD_JOBS = "$Jobs"
if (-not $env:LIBCLANG_PATH -and (Test-Path "C:\Program Files\LLVM\bin")) { $env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin" }

# build metadata embedded in the sidecar and the app (Settings > About, `axelstra --version`); the installer name does not change
if (-not $env:AXELSTRA_GIT_SHA) {
    $sha = (git -C $root rev-parse --short=7 HEAD 2>$null)
    $env:AXELSTRA_GIT_SHA = if ($LASTEXITCODE -eq 0 -and $sha) { "$sha".Trim() } else { "unknown" }
}
if (-not $env:AXELSTRA_BUILD_DATE) { $env:AXELSTRA_BUILD_DATE = (Get-Date).ToUniversalTime().ToString("yyyy-MM-dd") }
Write-Host "Build: commit $($env:AXELSTRA_GIT_SHA), date $($env:AXELSTRA_BUILD_DATE)"

# the version of the installed Axelstra (the NSIS uninstall entry, per user or per machine), or $null
function Get-InstalledVersion {
    $keys = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall", "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall"
    $found = Get-ChildItem $keys -ErrorAction SilentlyContinue | Get-ItemProperty -ErrorAction SilentlyContinue |
        Where-Object { $_.DisplayName -eq "Axelstra" -and "$($_.DisplayVersion)" -match '^\d+\.\d+\.\d+$' } |
        ForEach-Object { [version]$_.DisplayVersion } | Sort-Object -Descending | Select-Object -First 1
    if ($found) { "$found" } else { $null }
}

if ($Version) {
    $Version = $Version.TrimStart("v")
    Write-Host "Version: $Version (-Version)"
} else {
    $installed = Get-InstalledVersion
    $nextArgs = @("--local")
    if ($installed) { $nextArgs += "--above", $installed }
    $Version = node (Join-Path $PSScriptRoot "next-version.mjs") @nextArgs
    if ($LASTEXITCODE -ne 0 -or -not $Version) { throw "next-version.mjs failed" }
    $Version = "$Version".Trim()
    $why = if ($installed) { "installed: $installed" } else { "nothing installed" }
    Write-Host "Version: $Version (next release from the commits since the last tag, $why; -Version overrides)"
}

# stamp the version for this build; the finally at the bottom puts the exact previous bytes back
$stamped = "Cargo.toml", "Cargo.lock", "app/package.json", "app/src-tauri/tauri.conf.json"
$saved = @{}
foreach ($f in $stamped) { $saved[$f] = [System.IO.File]::ReadAllBytes((Join-Path $root $f)) }
function Restore-VersionFiles {
    foreach ($f in $stamped) { [System.IO.File]::WriteAllBytes((Join-Path $root $f), $saved[$f]) }
    Write-Host "Version files restored: $($stamped -join ', ')"
}

try {
    node (Join-Path $PSScriptRoot "stamp-version.mjs") --tag "v$Version"
    if ($LASTEXITCODE -ne 0) { throw "stamp-version.mjs failed" }

    # the build should not make the computer sluggish
    (Get-Process -Id $PID).PriorityClass = "BelowNormal"

    $summary = @()

    function Invoke-Step([string]$Name, [scriptblock]$Body) {
        Write-Host "`n== $Name" -ForegroundColor Cyan
        $peak = 0
        $watch = [System.Diagnostics.Stopwatch]::StartNew()
        # peak memory of everything this build started, sampled from a background job
        $sampler = Start-Job -ArgumentList $PID {
            param($root)
            $max = 0
            while ($true) {
                # only this build's own process tree: other sessions compiling at the same time do not count
                $all = Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, WorkingSetSize
                $kids = $all | Group-Object ParentProcessId -AsHashTable -AsString
                $stack = [System.Collections.Generic.Stack[uint32]]::new(); $stack.Push($root)
                $sum = 0
                while ($stack.Count) {
                    $p = $stack.Pop()
                    foreach ($c in $kids["$p"]) { if ($c) { $sum += $c.WorkingSetSize; $stack.Push($c.ProcessId) } }
                }
                if ($sum -gt $max) { $max = $sum; Write-Output $max }
                Start-Sleep -Milliseconds 1500
            }
        }
        try { & $Body } finally {
            $watch.Stop()
            $samples = @(Receive-Job $sampler)
            Stop-Job $sampler; Remove-Job $sampler -Force
            if ($samples.Count) { $peak = ($samples | Measure-Object -Maximum).Maximum }
        }
        $script:summary += [pscustomobject]@{ Step = $Name; Seconds = [int]$watch.Elapsed.TotalSeconds; PeakGB = [Math]::Round($peak / 1GB, 1) }
    }

    Invoke-Step "Daemon sidecar (cargo, release)" {
        & (Join-Path $PSScriptRoot "prepare-sidecar.ps1")
    }

    Invoke-Step "Frontend (tsc + vite)" {
        Push-Location $app
        try {
            if (-not (Test-Path node_modules)) { npm ci; if ($LASTEXITCODE -ne 0) { throw "npm ci failed" } }
            npm run build
            if ($LASTEXITCODE -ne 0) { throw "frontend build failed" }
        } finally { Pop-Location }
    }

    Invoke-Step "App + installer (tauri, NSIS)" {
        Push-Location $app
        try {
            # the frontend is already built above, so tauri must not build it a second time
            $over = @{ build = @{ beforeBuildCommand = "" } }
            $tmp = Join-Path $app "src-tauri/build.tmp.json"
            Set-Content -Encoding ascii $tmp ($over | ConvertTo-Json -Depth 4 -Compress)
            $extra = "--config", "src-tauri/build.tmp.json"
            npx tauri build --config src-tauri/tauri.release.conf.json @extra
            if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }
        } finally {
            Remove-Item (Join-Path $app "src-tauri/build.tmp.json") -ErrorAction SilentlyContinue
            Pop-Location
        }
    }
} finally {
    Restore-VersionFiles
}

Write-Host "`nSummary (version: $Version, jobs: $Jobs)" -ForegroundColor Cyan
$summary | Format-Table -AutoSize | Out-String | Write-Host
Get-ChildItem (Join-Path $root "target/release/bundle/nsis/*-setup.exe") | Sort-Object LastWriteTime | Select-Object -Last 1 | ForEach-Object { Write-Host "Installer: $($_.FullName)" }
