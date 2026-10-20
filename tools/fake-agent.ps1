# Fake agent for axelstra tests. Behaviour is keyed on the task id.
param([string]$Marker)
$id = $env:AXELSTRA_TASK
if ($id -eq 'hang') { Start-Sleep -Seconds 600 }
if ($id -eq 'flaky' -and -not (Test-Path $Marker)) { New-Item -Force $Marker | Out-Null; exit 1 }
Start-Sleep -Milliseconds 300
Set-Content -Path "$id.txt" -Value "work by $id"
if ($id -eq 'conflict') { Set-Content -Path "shared.txt" -Value "from conflict" }
if ($id -eq 'a')        { Set-Content -Path "shared.txt" -Value "from a" }
New-Item -ItemType Directory -Force .axelstra | Out-Null
Set-Content -Path ".axelstra/result.json" -Value "{`"status`":`"done`",`"summary`":`"did $id`"}"
