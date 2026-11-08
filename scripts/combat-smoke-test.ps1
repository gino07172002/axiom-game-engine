param([int]$Port=4562)
$ErrorActionPreference='Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
$exe=(Resolve-Path 'target/debug/axiom.exe').Path
$hash=(Get-FileHash -LiteralPath $exe).Hash
$stamp=Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$dir=Join-Path $PWD "artifacts/combat-$stamp"
$db=Join-Path $PWD "data/combat-$stamp.sqlite"
New-Item -ItemType Directory -Force $dir|Out-Null
function Invoke-Combat([string[]]$Arguments) {
    $raw=& $exe @Arguments
    if($LASTEXITCODE -ne 0){throw "Axiom failed: $Arguments"}
    $raw|ConvertFrom-Json
}
function Send-Combat($Request) {
    $client=[Net.Sockets.TcpClient]::new()
    try{
        $client.Connect('127.0.0.1',$Port)
        $client.ReceiveTimeout=5000
        $stream=$client.GetStream()
        $writer=[IO.StreamWriter]::new($stream,[Text.UTF8Encoding]::new($false),1024,$true)
        $writer.AutoFlush=$true
        $writer.WriteLine(($Request|ConvertTo-Json -Compress -Depth 40))
        [IO.StreamReader]::new($stream).ReadLine()|ConvertFrom-Json
    }finally{$client.Dispose()}
}
function Wait-Combat([scriptblock]$Predicate) {
    for($i=0;$i -lt 1200;$i++){
        if($process.HasExited){throw "Alliance exited early. See $dir/live-errors.log"}
        try{
            $response=Send-Combat @{op='observe'}
            if($response.ok -and (& $Predicate $response.observation)){return $response.observation}
        }catch{}
        Start-Sleep -Milliseconds 20
    }
    throw 'Alliance observation timed out'
}
function Command-Combat($Command) {
    $response=Send-Combat @{op='command';command=$Command}
    if(-not $response.ok){throw ($response|ConvertTo-Json -Depth 30)}
    $response
}
function Call-Combat([string]$Capability,[string]$Action,$ArgsValue=@{}) {
    Command-Combat @{kind='call';capability=$Capability;action=$Action;args=$ArgsValue}
}

$scenario=Invoke-Combat @('--db',$db,'--pack','content/m4','scenario','examples/m4-support.scenario.json')
if($scenario.observation.gameplay.ending -ne 'joined_victory' -or $scenario.observation.rts.summary.stats.kills -ne 1){throw 'Headless mutual support failed'}
$baseline=Invoke-Combat @('--db',$db,'play','--start','--port','0','--smoke-frames','12','--screenshot',"$dir/initial.png")
$process=Start-Process -FilePath $exe -ArgumentList ('--db "{0}" play --start --port {1} --smoke-frames 1200 --screenshot "{2}"' -f $db,$Port,"$dir/support.png") -WorkingDirectory $PWD -WindowStyle Hidden -PassThru -RedirectStandardOutput "$dir/live.log" -RedirectStandardError "$dir/live-errors.log"
try {
    $ready=Wait-Combat {param($o)$o.gpu -and $o.combat.bodies -eq 9}
    $initialDelta=Send-Combat @{op='observe_delta';fields=@('/combat','/gameplay/ending')}
    $accepted=Command-Combat @{kind='choose';node='quest.arrival';choice='accept'}
    $gathered=Call-Combat rts gather @{units=@('squad/worker/0','squad/worker/1','squad/worker/2','squad/worker/3');target='squad/ore'}
    $attack=@{op='execute';request_id='support-attack';runtime_session=$ready.runtime_session;request=@{op='command';command=@{kind='call';capability='rts';action='attack';args=@{units=@('squad/soldier/4','squad/soldier/5');target='enemy/guard'}}}}
    $ordered=Send-Combat $attack
    $replayed=Send-Combat $attack
    if(-not $ordered.ok -or ($ordered|ConvertTo-Json -Compress -Depth 30) -ne ($replayed|ConvertTo-Json -Compress -Depth 30)){throw 'Cross-controller attack retry failed'}
    $locked=Call-Combat action lock
    $heroAttack=Call-Combat action attack
    $heroHit=Wait-Combat {param($o)@($o.combat.recent|Where-Object {$_.receipt.source -eq 'hero' -and $_.receipt.outcome -eq 'hit'}).Count -gt 0}
    $inspection=Call-Combat combat inspect @{ids=@('hero','enemy/guard','squad/soldier/4')}
    $patch=@{op='execute';request_id='combat-bounds';request=@{op='apply';expected_rev=$ready.rev;edits=@(@{kind='set_field';target=@{kind='resource';id='combat.session'};field='/bounds/min/0';value=-11.8})}}
    $applied=Send-Combat $patch
    $again=Send-Combat $patch
    if(-not $applied.ok -or ($applied|ConvertTo-Json -Compress -Depth 30) -ne ($again|ConvertTo-Json -Compress -Depth 30)){throw 'Shared collider authoring retry failed'}
    $completed=Wait-Combat {param($o)$o.gameplay.ending -eq 'joined_victory'}
    if($completed.combat.deaths -ne 1 -or $completed.rts.summary.stats.kills -ne 1 -or $completed.events.error){throw 'Shared native damage/death failed'}
    $delta=Send-Combat @{op='observe_delta';fields=@('/combat','/gameplay/ending');cursor=$initialDelta.cursor}
    $hero=Call-Combat input activate @{context='hero'}
    if(-not $process.WaitForExit(60000) -or $process.ExitCode -ne 0){throw 'Native shared combat capture failed'}
} finally {if(-not $process.HasExited){Stop-Process -Id $process.Id;[void]$process.WaitForExit(5000)}}
$capture=Get-Content "$dir/support.json" -Raw|ConvertFrom-Json
if($capture.gameplay.ending -ne 'joined_victory' -or $capture.combat.deaths -ne 1){throw 'Native capture did not preserve the joint ending'}
if((Get-FileHash -LiteralPath $exe).Hash -ne $hash){throw 'Executable changed during combat QA'}
$report=@{ok=$true;exe_sha256=$hash;scenario=$scenario;initial=$baseline;hero_hit=$heroHit;inspection=$inspection;attack_receipt=$ordered;authoring=$applied;completed=$capture;delta=$delta;screenshot="$dir/support.png"}
$report|ConvertTo-Json -Depth 70|Set-Content "$dir/report.json" -Encoding utf8
Copy-Item -LiteralPath "$dir/report.json" -Destination artifacts/combat-validation.json
Write-Host "PASS shared collision / hero and squad focus fire / native enemy death / joint ending / hot reload / projected observe / idempotent retries. Report: $dir/report.json"
