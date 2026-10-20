# Fake agent for axelstra tests: writes a file named after the task, then the sentinel.
$id = $env:AXELSTRA_TASK
Start-Sleep -Milliseconds 800
Set-Content -Path "$id.txt" -Value "work by $id"
if ($id -eq 'conflict') { Set-Content -Path "shared.txt" -Value "from conflict" }
if ($id -eq 'a')        { Set-Content -Path "shared.txt" -Value "from a" }
if ($id -eq 'hang')     { Start-Sleep -Seconds 600 }
New-Item -ItemType Directory -Force .axelstra | Out-Null
Set-Content -Path ".axelstra/result.json" -Value "{`"status`":`"done`",`"summary`":`"did $id`"}"
