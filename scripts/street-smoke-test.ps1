# Street demo GPU smoke test: launches the real game window (release build by default) on an isolated
# database and captures four screenshots, driving the player through the live bridge with the same
# runtime actions the mouse and the E key use (nav.go = click on the ground, interact.use = E):
#   street.png  the street at dusk (cars, pedestrians, HUD)
#   talk.png    next to Ming: the interaction prompt and the dialogue box
#   door.png    in front of the cafe after pressing E: the door swinging open
#   room.png    inside the cafe: HUD "你進入了咖啡廳", camera inside the room
# Each capture is a separate process (`play --smoke-frames N --screenshot`); in smoke mode every frame advances the
# simulation by exactly 1/60 s, so N frames = N/60 s of game time (wall time depends on the GPU). Report: artifacts/street-*/report.json
param([int]$Port = 4472, [string]$Exe = "target\release\axiom.exe")
$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)
if (-not (Test-Path $Exe)) { throw "missing $Exe (cargo build --release -p axiom)" }
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$dir = Join-Path $PWD "artifacts\street-$stamp"
New-Item -ItemType Directory -Force $dir | Out-Null
$db = Join-Path $dir "street.sqlite"
& $Exe --db $db --pack content\street verify | Out-Null
if ($LASTEXITCODE -ne 0) { throw "verify failed" }

function Send-Game($Request) {
    $client = [Net.Sockets.TcpClient]::new()
    try {
        $client.Connect('127.0.0.1', $Port)
        $client.ReceiveTimeout = 30000
        $stream = $client.GetStream()
        $writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false), 1024, $true)
        $writer.AutoFlush = $true
        $writer.WriteLine(($Request | ConvertTo-Json -Compress -Depth 30))
        [IO.StreamReader]::new($stream, [Text.UTF8Encoding]::new($false)).ReadLine() | ConvertFrom-Json
    } finally { $client.Dispose() }
}
function Wait-Game([scriptblock]$Predicate, [int]$Seconds = 120) {
    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        if ($script:game.HasExited) { throw "game exited early (see $dir)" }
        try {
            $reply = Send-Game @{ op = 'observe' }
            if ($reply.ok -and (& $Predicate $reply.observation)) { return $reply.observation }
        } catch {}
        Start-Sleep -Milliseconds 150
    }
    throw "timed out waiting for the game"
}
function Call-Game([string]$Capability, [string]$Action, $Arguments) {
    $reply = Send-Game @{ op = 'command'; command = @{ kind = 'call'; capability = $Capability; action = $Action; args = $Arguments } }
    if (-not $reply.ok) { throw "$Capability.$Action failed: $($reply | ConvertTo-Json -Depth 10 -Compress)" }
    $reply
}
function Walk([double[]]$Point) {
    $null = Call-Game 'nav' 'go' @{ id = 'hero'; point = $Point }
    $null = Wait-Game { param($o) $o.nav.at.hero[0] -eq 'arrived' -and [math]::Abs($o.nav.at.hero[1] - $Point[0]) -lt 0.6 -and [math]::Abs($o.nav.at.hero[2] - $Point[2]) -lt 0.6 }
}
function Capture([string]$Name, [int]$Frames, [scriptblock]$Drive) {
    $png = Join-Path $dir "$Name.png"
    $script:game = Start-Process -FilePath $Exe -ArgumentList ('--db "{0}" play --start --port {1} --smoke-frames {2} --screenshot "{3}"' -f $db, $Port, $Frames, $png) -WorkingDirectory $PWD -PassThru -RedirectStandardOutput "$dir\$Name.log" -RedirectStandardError "$dir\$Name-errors.log"
    $null = $script:game.Handle
    $clock = [Diagnostics.Stopwatch]::StartNew()
    try {
        $null = Wait-Game { param($o) $o.gpu -and $o.tick -gt 0 }
        & $Drive
        if (-not $script:game.WaitForExit(600000)) { throw "$Name did not finish" }
        if ($script:game.ExitCode -ne 0) { throw "$Name exited with $($script:game.ExitCode)" }
    } finally {
        if (-not $script:game.HasExited) { Stop-Process -Id $script:game.Id; [void]$script:game.WaitForExit(10000) }
    }
    if (-not (Test-Path $png)) { throw "missing $png" }
    # play --screenshot writes the final observation next to the image.
    $o = (Get-Content ([IO.Path]::ChangeExtension($png, ".json")) -Raw -Encoding utf8 | ConvertFrom-Json)
    [ordered]@{
        png = $png; seconds = [math]::Round($clock.Elapsed.TotalSeconds, 1); tick = $o.tick
        node = $o.gameplay.node; prompt = $o.gameplay.bindings.prompt; location = $o.gameplay.variables.location
        focus = $o.interact.focus.hero; camera = $o.camera.follow; frame = $o.frame
    }
}

$report = [ordered]@{ ok = $false; exe = $Exe }
$report.street = Capture 'street' 150 {}
$report.talk = Capture 'talk' 300 {
    Walk @(-9, 0, 6.4)
    $null = Wait-Game { param($o) $o.interact.focus.hero -eq 'npc/ming' }
    $null = Call-Game 'interact' 'use' @{}
    $null = Wait-Game { param($o) $o.gameplay.node -eq 'ming' }
    # Orbit to the street side (what Z / C does) so the bus stop does not fill the view.
    $null = Call-Game 'camera' 'follow.set' @{ id = 'camera'; yaw = 200; distance = 5 }
}
$report.door = Capture 'door' 820 {
    Walk @(-0.6, 0, 5.0); Walk @(0.6, 0, -5.0); Walk @(11, 0, -6.4)
    $null = Wait-Game { param($o) $o.interact.focus.hero -eq 'cafe/door' }
    $null = Call-Game 'interact' 'use' @{}
    $null = Wait-Game { param($o) $o.gameplay.variables.door_open -eq $true }
}
$report.room = Capture 'room' 1150 {
    Walk @(-0.6, 0, 5.0); Walk @(0.6, 0, -5.0); Walk @(11, 0, -6.4)
    $null = Call-Game 'interact' 'use' @{}
    Start-Sleep -Milliseconds 1500
    Walk @(11, 0, -13.3)
    $null = Wait-Game { param($o) $o.gameplay.variables.location -eq 'loc_cafe' }
}
foreach ($k in 'street', 'talk', 'door', 'room') { if (-not $report[$k].png) { throw "missing $k" } }
if ($report.talk.node -ne 'ming' -or $report.door.focus -ne 'cafe/door' -or $report.room.location -ne 'loc_cafe') { throw "unexpected state: $($report | ConvertTo-Json -Depth 6 -Compress)" }
$report.ok = $true
$report | ConvertTo-Json -Depth 8 | Set-Content -Encoding utf8 (Join-Path $dir 'report.json')
Write-Host "PASS street GPU captures: $dir"
