param([int]$Port = 4252)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
$axiomExe = (Resolve-Path 'target/debug/axiom.exe').Path
$axiomStamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$axiomDb = Join-Path $PWD "data/qa-$axiomStamp.sqlite"
$axiomArtifacts = Join-Path $PWD "artifacts/qa-$axiomStamp"
New-Item -ItemType Directory -Force -Path $axiomArtifacts | Out-Null

function Invoke-Axiom([string[]]$AxiomArgs) {
    $axiomRaw = & $axiomExe @AxiomArgs
    if ($LASTEXITCODE -ne 0) { throw "axiom failed: $AxiomArgs" }
    return ($axiomRaw | ConvertFrom-Json)
}

$axiomHeadless = Invoke-Axiom @('--db', $axiomDb, 'verify', '--ticks', '1200')
$axiomMenu = Invoke-Axiom @('--db', $axiomDb, 'play', '--port', '0', '--smoke-frames', '8', '--screenshot', "$axiomArtifacts/menu.png")
$axiomGame = Invoke-Axiom @('--db', $axiomDb, 'play', '--port', '0', '--start', '--smoke-frames', '120', '--screenshot', "$axiomArtifacts/gameplay.png")
if ($axiomMenu.frames -ne 8 -or $axiomGame.frames -ne 120) { throw 'Incorrect smoke frame count' }
if ($axiomGame.observation.tick -ne 240) { throw 'Fixed-step GPU run did not advance by 240 ticks' }

$axiomArguments = '--db "{0}" play --port {1} --smoke-frames 6000' -f $axiomDb, $Port
$axiomPortProbe = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, $Port)
try { $axiomPortProbe.Start() } finally { $axiomPortProbe.Stop() }
$axiomProcess = Start-Process -FilePath $axiomExe -ArgumentList $axiomArguments -WorkingDirectory $PWD -WindowStyle Hidden -PassThru -RedirectStandardOutput "$axiomArtifacts/bridge.log" -RedirectStandardError "$axiomArtifacts/bridge-errors.log"
try {
    $axiomAddress = "127.0.0.1:$Port"
    $axiomReady = $false
    for ($axiomAttempt = 0; $axiomAttempt -lt 50; $axiomAttempt++) {
        $axiomProbe = & $axiomExe --connect $axiomAddress query 2>$null
        if ($LASTEXITCODE -eq 0) { $axiomReady = $true; break }
        if ($axiomProcess.HasExited) { throw 'Bridge process exited during startup' }
        Start-Sleep -Milliseconds 200
    }
    if (-not $axiomReady) { throw 'Bridge did not become ready' }
    $axiomApplied = Invoke-Axiom @('--connect', $axiomAddress, 'apply', 'examples/more-enemies.json')
    $axiomSynchronized = $false
    for ($axiomAttempt = 0; $axiomAttempt -lt 50; $axiomAttempt++) {
        $axiomObserved = Invoke-Axiom @('--connect', $axiomAddress, 'observe')
        if ($axiomObserved.observation.rev -eq $axiomApplied.rev -and $axiomObserved.observation.enemies_total -eq 12) { $axiomSynchronized = $true; break }
        Start-Sleep -Milliseconds 100
    }
    if (-not $axiomSynchronized) { throw 'Runtime did not synchronize committed revision' }
    $axiomUndo = Invoke-Axiom @('--connect', $axiomAddress, 'undo', '1', '--expected-rev', "$($axiomApplied.rev)")
    $axiomFinal = Invoke-Axiom @('--db', $axiomDb, 'verify')
    $axiomReport = [ordered]@{ ok = $true; headless = $axiomHeadless; menu = $axiomMenu; gameplay = $axiomGame; apply = $axiomApplied; synchronized = $axiomObserved; undo = $axiomUndo; final = $axiomFinal }
    $axiomReport | ConvertTo-Json -Depth 20 | Set-Content -Encoding utf8 "$axiomArtifacts/report.json"
    Write-Host "PASS: GPU screenshots, headless simulation, live bridge apply/reconcile/observe/undo. Report: $axiomArtifacts/report.json"
} finally {
    if (-not $axiomProcess.HasExited) { Stop-Process -Id $axiomProcess.Id }
}
