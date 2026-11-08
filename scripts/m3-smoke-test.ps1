param([int]$Port=4492)
$ErrorActionPreference='Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
$exe=(Resolve-Path 'target/debug/axiom.exe').Path
$hash=(Get-FileHash -LiteralPath $exe).Hash
$stamp=Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$dir=Join-Path $PWD "artifacts/m3-$stamp"
$db=Join-Path $PWD "data/m3-$stamp.sqlite"
New-Item -ItemType Directory -Force $dir | Out-Null
function Invoke-M3([string[]]$Arguments) {
    $raw=& $exe @Arguments
    if($LASTEXITCODE -ne 0){throw "Axiom failed: $Arguments"}
    $raw | ConvertFrom-Json
}
function Send-M3($Request) {
    $client=[Net.Sockets.TcpClient]::new()
    try {
        $client.Connect('127.0.0.1',$Port)
        $client.ReceiveTimeout=5000
        $stream=$client.GetStream()
        $writer=[IO.StreamWriter]::new($stream,[Text.UTF8Encoding]::new($false),1024,$true)
        $writer.AutoFlush=$true
        $writer.WriteLine(($Request|ConvertTo-Json -Compress -Depth 30))
        [IO.StreamReader]::new($stream).ReadLine() | ConvertFrom-Json
    } finally { $client.Dispose() }
}
function Wait-M3([scriptblock]$Predicate) {
    for($i=0;$i -lt 1500;$i++){
        if($process.HasExited){throw "Action helper exited early. See $dir/live-errors.log"}
        try{
            $reply=Send-M3 @{op='observe'}
            if($reply.ok -and (& $Predicate $reply.observation)){return $reply.observation}
        }catch{}
        Start-Sleep -Milliseconds 15
    }
    throw 'Action observation timed out'
}
function Command-M3([string]$Action){
    $result=Send-M3 @{op='command';command=@{kind='call';capability='action';action=$Action;args=@{}}}
    if(-not $result.ok){throw ($result|ConvertTo-Json -Depth 20)}
    $result
}

$functional=Invoke-M3 @('--db',$db,'--pack','content/action','scenario','examples/action-functional.scenario.json')
if(-not $functional.ok){throw 'Action functional scenario failed'}
$baseline=Invoke-M3 @('--db',$db,'play','--start','--port','0','--smoke-frames','24','--screenshot',"$dir/initial.png")

# Controlled targets give the screenshot and RPC test reproducible timing.
$pack=Join-Path $dir 'pack'
New-Item -ItemType Directory -Force $pack | Out-Null
Copy-Item -LiteralPath 'content/action/fighter.glb' -Destination "$pack/fighter.glb"
Copy-Item -LiteralPath 'content/action/fighter-alt.glb' -Destination "$pack/fighter-alt.glb"
Copy-Item -LiteralPath 'content/action/manifest.json' -Destination "$pack/manifest.json"
$world=Get-Content 'content/action/world.json' -Raw | ConvertFrom-Json -AsHashtable
$world.prefabs.sentinel.components['action.actor'].ai=$false
$world.prefabs.sentinel.components['core.health']=@{value=700;max=700}
($world.entities|Where-Object{$_.id -eq 'enemy/guard'}).components['core.transform'].position=@(0,0,0.9)
[IO.File]::WriteAllText("$pack/world.json",($world|ConvertTo-Json -Depth 50))
$liveDb=Join-Path $PWD "data/m3-live-$stamp.sqlite"
$process=Start-Process -FilePath $exe -ArgumentList ('--db "{0}" --pack "{1}" play --start --port {2} --smoke-frames 420 --screenshot "{3}"' -f $liveDb,$pack,$Port,"$dir/combat.png") -WorkingDirectory $PWD -WindowStyle Hidden -PassThru -RedirectStandardOutput "$dir/live.log" -RedirectStandardError "$dir/live-errors.log"
try{
    $initial=Wait-M3{param($o)$o.gpu -and $o.action.hero.health -eq 100}
    $bad=Send-M3 @{op='preview';expected_rev=$initial.rev;edits=@(@{kind='set_field';target=@{kind='resource';id='action.abilities'};field='/abilities/slash2/dodge_window';value=@{start=25;end=99}})}
    if($bad.ok -or $bad.diagnostics[0].path -ne '/resources/action.abilities/abilities/slash2/dodge_window'){throw 'Timeline diagnostic is missing'}
    $edits=@(@{kind='set_field';target=@{kind='resource';id='action.abilities'};field='/abilities/slash2/dodge_window';value=@{start=25;end=70}})
    $preview=Send-M3 @{op='preview';expected_rev=$initial.rev;edits=$edits}
    if(-not $preview.ok){throw 'Valid cancellation edit was rejected'}
    $envelope=@{op='execute';request_id='action-cancel-edit';request=@{op='apply';expected_rev=$initial.rev;edits=$edits}}
    $applied=Send-M3 $envelope
    $replay=Send-M3 $envelope
    if(-not $applied.ok -or ($applied|ConvertTo-Json -Compress -Depth 20) -ne ($replay|ConvertTo-Json -Compress -Depth 20)){throw 'Action authoring retry failed'}
    $active=Wait-M3{param($o)$o.rev -eq $applied.rev}
    $lock=Command-M3 lock
    $first=Command-M3 attack
    $chain=Wait-M3{param($o)$o.action.hero.ability -eq 'slash1' -and $o.action.hero.elapsed -ge 35}
    $second=Command-M3 attack
    $cancel=Wait-M3{param($o)$o.action.hero.ability -eq 'slash2' -and $o.action.hero.elapsed -ge 26}
    $roll=Command-M3 dodge
    $rolled=Wait-M3{param($o)$o.action.hero.ability -eq 'roll'}
    if($rolled.action.stats.chains -ne 1 -or $rolled.action.stats.rolls -ne 1){throw 'Second attack did not cancel into a roll'}
    # Near the final fixed tick, trigger a fresh slash for the captured native frame.
    $ready=Wait-M3{param($o)$o.tick -ge 814}
    $pose=Command-M3 attack
    if(-not $process.WaitForExit(90000) -or $process.ExitCode -ne 0){throw 'Action GPU capture did not finish'}
}finally{
    if(-not $process.HasExited){Stop-Process -Id $process.Id;[void]$process.WaitForExit(5000)}
}
$capture=Get-Content "$dir/combat.json" -Raw | ConvertFrom-Json
if($capture.audio.emitted -lt 3 -or $capture.animation.players.hero.fault){throw 'Animation/audio timeline did not run'}
if((Get-FileHash -LiteralPath $exe).Hash -ne $hash){throw 'Executable changed during QA'}
$report=@{
    ok=$true;exe_sha256=$hash;gpu=$baseline.gpu;functional=$functional.observation.action;
    timeline_diagnostic=$bad;preview=$preview;applied=$applied;cancelled=$rolled.action;
    audio=$capture.audio;animation=$capture.animation;initial="$dir/initial.png";combat="$dir/combat.png"
}
$report|ConvertTo-Json -Depth 40|Set-Content "$dir/report.json" -Encoding utf8
Copy-Item -LiteralPath "$dir/report.json" -Destination 'artifacts/m3-validation.json'
Write-Host "PASS M3 functional combo, cancellation, authored windows, durable retry, animation and native audio. Report: $dir/report.json"
