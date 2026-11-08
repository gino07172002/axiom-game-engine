param([int]$Port = 4472)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
$exe = (Resolve-Path 'target/debug/axiom.exe').Path
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$dir = Join-Path $PWD "artifacts/duet-$stamp"
$db = Join-Path $PWD "data/duet-smoke-$stamp.sqlite"
New-Item -ItemType Directory -Force $dir | Out-Null

function Invoke-DuetAxiom([string[]]$Arguments) {
    $raw = & $exe @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Axiom failed: $Arguments" }
    $raw | ConvertFrom-Json
}
function Send-Duet($Request) {
    $client = [Net.Sockets.TcpClient]::new()
    try {
        $client.Connect('127.0.0.1', $Port)
        $client.ReceiveTimeout = 5000
        $stream = $client.GetStream()
        $writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false), 1024, $true)
        $writer.AutoFlush = $true
        $writer.WriteLine(($Request | ConvertTo-Json -Compress -Depth 20))
        [IO.StreamReader]::new($stream).ReadLine() | ConvertFrom-Json
    } finally { $client.Dispose() }
}
function Wait-Duet([scriptblock]$Predicate) {
    for ($i = 0; $i -lt 200; $i++) {
        if ($process.HasExited) { throw "Duet exited early. See $dir/live-errors.log" }
        try {
            $reply = Send-Duet @{op='observe'}
            if ($reply.ok -and (& $Predicate $reply.observation)) { return $reply.observation }
        } catch {}
        Start-Sleep -Milliseconds 12
    }
    throw 'Duet observation timed out'
}
function Call-Duet([string]$Action, $ArgsValue = @{}) {
    $result = Send-Duet @{op='command';command=@{kind='call';capability='rhythm';action=$Action;args=$ArgsValue}}
    if (-not $result.ok) { throw ($result | ConvertTo-Json -Depth 15) }
    $result.result
}

$scenario = Invoke-DuetAxiom @('--db',$db,'--pack','content/love-no-war','scenario','examples/love-no-war.scenario.json')
if (-not $scenario.ok -or $scenario.observation.rhythm.hits -ne 8) { throw 'Headless duet did not score eight beats' }
$stage = Invoke-DuetAxiom @('--db',$db,'play','--start','--port','0','--smoke-frames','8','--screenshot',"$dir/stage.png")
if ($stage.observation.rhythm.phase -ne 'waiting') { throw 'Duet must wait for an invitation' }
$process = Start-Process -FilePath $exe -ArgumentList ('--db "{0}" play --start --port {1} --smoke-frames 760 --screenshot "{2}"' -f $db,$Port,"$dir/finale.png") -WorkingDirectory $PWD -WindowStyle Hidden -PassThru -RedirectStandardOutput "$dir/live.log" -RedirectStandardError "$dir/live-errors.log"
try {
    $start = Wait-Duet { param($o) $o.gpu -and $o.rhythm.phase -eq 'waiting' }
    $invite = Call-Duet invite
    if ($invite.phase -ne 'ready') { throw 'Partner did not accept the invitation' }
    $begin = Call-Duet begin
    if ($begin.phase -ne 'dancing') { throw 'Both participants were ready but the duet did not start' }
    for ($index = 0; $index -lt $start.rhythm.beats; $index++) {
        $cue = Wait-Duet { param($o)
            $o.rhythm.phase -eq 'dancing' -and $o.rhythm.index -eq $index -and
            ($o.rhythm.target_tick - $o.tick) -le ($o.rhythm.window_ticks - 3)
        }
        $hit = Call-Duet press @{input=$cue.rhythm.input}
        if (-not $hit.hit) { throw "Live cue $index missed its hit window" }
    }
    $final = Wait-Duet { param($o) $o.rhythm.phase -eq 'finale' -and $o.animation.players.'cast/ren'.clip -eq 'finale' }
    if ($final.rhythm.hits -ne 8 -or $final.rhythm.misses -ne 0) { throw 'Native duet did not finish with full score' }
    if (-not $process.WaitForExit(90000) -or $process.ExitCode -ne 0) { throw 'Duet did not finish GPU capture' }
} finally {
    if (-not $process.HasExited) { Stop-Process -Id $process.Id; [void]$process.WaitForExit(5000) }
}
$capture = Get-Content "$dir/finale.json" -Raw | ConvertFrom-Json
if ($capture.rhythm.phase -ne 'finale' -or $capture.animation.players.'cast/valentina'.clip -ne 'finale') { throw 'Screenshot did not capture the final pose' }
$report = @{
    ok=$true; exe_sha256=(Get-FileHash -LiteralPath $exe).Hash; gpu=$capture.gpu;
    headless=$scenario.observation.rhythm; native=$capture.rhythm; animation=$capture.animation;
    stage="$dir/stage.png"; finale="$dir/finale.png"
}
$report | ConvertTo-Json -Depth 20 | Set-Content "$dir/report.json" -Encoding utf8
Write-Host "PASS duet invitation, eight timed cues, two imported finales and GPU captures. Report: $dir/report.json"
