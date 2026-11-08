param([int]$Port = 4262)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
$m0Exe = (Resolve-Path 'target/debug/axiom.exe').Path
$m0Hash = (Get-FileHash -LiteralPath $m0Exe -Algorithm SHA256).Hash
$m0Stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$m0Dir = Join-Path $PWD "artifacts/m0-$m0Stamp"
$m0Db = Join-Path $PWD "data/m0-$m0Stamp.sqlite"
New-Item -ItemType Directory -Force -Path $m0Dir | Out-Null
function Invoke-M0([string[]]$M0Args) {
    $m0Raw = & $m0Exe @M0Args
    if ($LASTEXITCODE -ne 0) { throw "axiom failed: $M0Args" }
    return ($m0Raw | ConvertFrom-Json)
}

& "$PSScriptRoot/smoke-test.ps1" -Port $Port
$m0FpsDir = Get-ChildItem -LiteralPath 'artifacts' -Directory | Where-Object Name -Like 'qa-*' | Sort-Object LastWriteTime -Descending | Select-Object -First 1
$m0Fps = Get-Content -LiteralPath (Join-Path $m0FpsDir.FullName 'report.json') -Raw | ConvertFrom-Json
if ($m0Fps.gameplay.observation.fps_revision -ne $m0Fps.gameplay.observation.rev) { throw 'FPS HUD revision differs from applied revision' }

$m0Empty = Invoke-M0 @('--db', $m0Db, '--pack', 'content/empty', 'play', '--port', '0', '--start', '--smoke-frames', '8', '--screenshot', "$m0Dir/empty.png")
if ($m0Empty.frames -ne 8 -or $m0Empty.observation.entities -ne 0 -or $m0Empty.observation.cameras -ne 0) { throw 'Empty world requires gameplay state' }
if ($m0Empty.observation.PSObject.Properties.Name -contains 'player_health') { throw 'FPS metrics leaked into generic world' }

$m0Views = Invoke-M0 @('--db', $m0Db, '--pack', 'content/two-cameras', 'play', '--port', '0', '--start', '--smoke-frames', '120', '--screenshot', "$m0Dir/two-cameras.png")
if ($m0Views.frames -ne 120 -or $m0Views.observation.tick -ne 240 -or $m0Views.observation.cameras -ne 2 -or $m0Views.observation.entities -ne 5 -or $m0Views.observation.rev -ne 2) { throw 'Generic multi-view world failed' }
$m0Restored = Invoke-M0 @('--db', $m0Db, 'undo', '1', '--expected-rev', '2')
$m0Before = Invoke-M0 @('--db', $m0Db, 'query')
$m0Preview = Invoke-M0 @('--db', $m0Db, 'preview', 'examples/v2-add-box.json')
$m0After = Invoke-M0 @('--db', $m0Db, 'query')
if ($m0Before.rev -ne $m0After.rev -or $m0After.entities -ne 0 -or $m0Preview.diff.added -ne 2) { throw 'Preview modified authoring state' }
$m0Applied = Invoke-M0 @('--db', $m0Db, 'apply', 'examples/v2-add-box.json')
$m0Verify = Invoke-M0 @('--db', $m0Db, 'verify', '--ticks', '120')
if ($m0Verify.entities -ne 2 -or $m0Verify.schema_version -ne 2) { throw 'Generic entity editing failed' }

$m0FpsDb = Join-Path $PWD "data/$($m0FpsDir.Name).sqlite"
$m0Scenario = Invoke-M0 @('--db', $m0FpsDb, 'scenario', 'examples/fps-regression.scenario.json')
if (-not $m0Scenario.ok) { throw 'FPS input scenario failed' }
if ((Get-FileHash -LiteralPath $m0Exe -Algorithm SHA256).Hash -ne $m0Hash) { throw 'Executable changed during content operations' }

Copy-Item -LiteralPath (Join-Path $m0FpsDir.FullName 'gameplay.png') -Destination 'artifacts/m0-fps.png'
Copy-Item -LiteralPath "$m0Dir/empty.png" -Destination 'artifacts/m0-empty.png'
Copy-Item -LiteralPath "$m0Dir/two-cameras.png" -Destination 'artifacts/m0-two-cameras.png'
$m0Report = [ordered]@{ ok = $true; executable_sha256 = $m0Hash; same_executable = $true; fps_report = $m0FpsDir.FullName; fps = $m0Fps; empty = $m0Empty; two_cameras = $m0Views; undo = $m0Restored; preview = $m0Preview; apply = $m0Applied; verify = $m0Verify; fps_scenario = $m0Scenario }
$m0Report | ConvertTo-Json -Depth 30 | Set-Content -Encoding utf8 "$m0Dir/report.json"
Copy-Item -LiteralPath "$m0Dir/report.json" -Destination 'artifacts/m0-validation.json'
Write-Host "PASS M0: same executable / FPS / empty world / two cameras / preview / generic edits / activation / input replay. Report: $m0Dir/report.json"
