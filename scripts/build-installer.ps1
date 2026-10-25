# One command to produce the installer, in steps: daemon sidecar, frontend, NSIS bundle.
# Each step prints its time and the peak memory of the processes it started, so a heavy step is easy to spot.
# Output: target/release/bundle/nsis/Axelstra_<version>_x64-setup.exe
#
#   -Version 0.1.19   installer version (default: the one in tauri.conf.json)
#   -Jobs 4           parallel compile jobs (default: half the logical cores; fewer = lower memory peak)
param(
    [string]$Version,
    [int]$Jobs = [Math]::Max(2, [int]([Environment]::ProcessorCount / 2))
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$app = Join-Path $root "app"
$env:CARGO_BUILD_JOBS = "$Jobs"
if (-not $env:LIBCLANG_PATH -and (Test-Path "C:\Program Files\LLVM\bin")) { $env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin" }

# build metadata embedded in the sidecar and the app (Settings > About, `axelstra --version`); the installer name does not change
if (-not $env:AXELSTRA_GIT_SHA) {
    $sha = (git -C $root rev-parse --short=7 HEAD 2>$null)
    $env:AXELSTRA_GIT_SHA = if ($LASTEXITCODE -eq 0 -and $sha) { "$sha".Trim() } else { "unknown" }
}
if (-not $env:AXELSTRA_BUILD_DATE) { $env:AXELSTRA_BUILD_DATE = (Get-Date).ToUniversalTime().ToString("yyyy-MM-dd") }
Write-Host "Build: commit $($env:AXELSTRA_GIT_SHA), date $($env:AXELSTRA_BUILD_DATE)"

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
        if ($Version) { $over.version = $Version }
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

Write-Host "`nSummary (jobs: $Jobs)" -ForegroundColor Cyan
$summary | Format-Table -AutoSize | Out-String | Write-Host
Get-ChildItem (Join-Path $root "target/release/bundle/nsis/*-setup.exe") | Sort-Object LastWriteTime | Select-Object -Last 1 | ForEach-Object { Write-Host "Installer: $($_.FullName)" }
