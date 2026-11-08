param([int]$Port=4532)
$ErrorActionPreference='Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
$exe=(Resolve-Path 'target/debug/axiom.exe').Path
$hash=(Get-FileHash -LiteralPath $exe).Hash
$stamp=Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$dir=Join-Path $PWD "artifacts/m4-$stamp"
$db=Join-Path $PWD "data/m4-$stamp.sqlite"
New-Item -ItemType Directory -Force $dir|Out-Null
function Invoke-M4([string[]]$Arguments) {
    $raw=& $exe @Arguments
    if($LASTEXITCODE -ne 0){throw "Axiom failed: $Arguments"}
    $raw|ConvertFrom-Json
}
function Send-M4($Request) {
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
function Wait-M4([scriptblock]$Predicate) {
    for($i=0;$i -lt 1200;$i++){
        if($process.HasExited){throw "Alliance exited early. See $dir/live-errors.log"}
        try{
            $response=Send-M4 @{op='observe'}
            if($response.ok -and (& $Predicate $response.observation)){return $response.observation}
        }catch{}
        Start-Sleep -Milliseconds 20
    }
    throw 'Alliance observation timed out'
}
function Command-M4($Command) {
    $response=Send-M4 @{op='command';command=$Command}
    if(-not $response.ok){throw ($response|ConvertTo-Json -Depth 30)}
    $response
}
function Call-M4([string]$Capability,[string]$Action,$ArgsValue=@{}) {
    Command-M4 @{kind='call';capability=$Capability;action=$Action;args=$ArgsValue}
}
$scenario=Invoke-M4 @('--db',$db,'--pack','content/m4','scenario','examples/m4-joint.scenario.json')
if($scenario.observation.gameplay.ending -ne 'joined_victory'){throw 'Joint scenario did not finish'}
$graph=Invoke-M4 @('--db',$db,'query','packages')
if($graph.packages.packages.Count -ne 4){throw 'Expected four resolved packages'}
$baseline=Invoke-M4 @('--db',$db,'play','--start','--port','0','--smoke-frames','12','--screenshot',"$dir/squad.png")
$process=Start-Process -FilePath $exe -ArgumentList ('--db "{0}" play --start --port {1} --smoke-frames 800 --screenshot "{2}"' -f $db,$Port,"$dir/joined-victory.png") -WorkingDirectory $PWD -WindowStyle Hidden -PassThru -RedirectStandardOutput "$dir/live.log" -RedirectStandardError "$dir/live-errors.log"
$null=$process.Handle
try{
    $ready=Wait-M4 {param($o)$o.gpu -and $o.input.context -eq 'squad'}
    $fields=@('/gameplay/variables','/input','/events/delivered','/rev')
    $initialDelta=Send-M4 @{op='observe_delta';fields=$fields}
    $accepted=Command-M4 @{kind='choose';node='quest.arrival';choice='accept'}
    $gathered=Call-M4 rts gather @{units=@('squad/worker/0','squad/worker/1','squad/worker/2','squad/worker/3');target='squad/ore'}
    $production=@{op='execute';request_id='alliance-production';runtime_session=$ready.runtime_session;request=@{
        op='command';command=@{kind='call';capability='rts';action='train';args=@{building='squad/base';recipe='soldier';count=1}}
    }}
    $trained=Send-M4 $production
    $trainedReplay=Send-M4 $production
    if(-not $trained.ok -or ($trained|ConvertTo-Json -Compress -Depth 30) -ne ($trainedReplay|ConvertTo-Json -Compress -Depth 30)){throw 'Production retry changed the runtime receipt'}
    $edits=@(
        @{kind='set_field';target=@{kind='rule';id='quest.complete'};field='/when/conditions/1/right/value';value=4},
        @{kind='set_field';target=@{kind='choice';node='quest.active';id='assault'};field='/when/right/value';value=4}
    )
    $preview=Send-M4 @{op='preview';expected_rev=$ready.rev;edits=$edits}
    $envelope=@{op='execute';request_id='alliance-goal';request=@{op='apply';expected_rev=$ready.rev;edits=$edits}}
    $applied=Send-M4 $envelope
    $replay=Send-M4 $envelope
    if(-not $preview.ok -or -not $applied.ok -or ($applied|ConvertTo-Json -Compress -Depth 30) -ne ($replay|ConvertTo-Json -Compress -Depth 30)){throw 'Mission edit retry failed'}
    $supplied=Wait-M4 {param($o)$o.rev -eq $applied.rev -and $o.gameplay.variables.'quest.supplies' -ge 8 -and $o.gameplay.variables.'quest.trained' -eq 1}
    $assault=Command-M4 @{kind='choose';node='quest.active';choice='assault'}
    $hero=Call-M4 input activate @{context='hero'}
    $locked=Call-M4 action lock
    $first=Call-M4 action attack
    $window=Wait-M4 {param($o)$o.action.hero.ability -eq 'slash1' -and $o.action.hero.elapsed -ge 35}
    $second=Call-M4 action attack
    $window=Wait-M4 {param($o)$o.action.hero.ability -eq 'slash2' -and $o.action.hero.elapsed -ge 38}
    $third=Call-M4 action attack
    $finished=Wait-M4 {param($o)$o.gameplay.ending -eq 'joined_victory'}
    if($finished.events.error -or $finished.events.pending -ne 0 -or $finished.gameplay.variables.'quest.trained' -ne 1){throw 'Event delivery or production count is inconsistent'}
    $delta=Send-M4 @{op='observe_delta';fields=$fields;cursor=$initialDelta.cursor}
    if($delta.reset -or '/input/context' -notin $delta.changes.path){throw 'Context switch was absent from incremental observation'}
    $spoof=Send-M4 @{op='command';command=@{kind='emit';event='action.victory';fields=@{}}}
    if($spoof.ok){throw 'Owned native event could be impersonated'}
    if(-not $process.WaitForExit(90000) -or $process.ExitCode -ne 0){throw 'Alliance GPU capture did not finish'}
}finally{
    if(-not $process.HasExited){Stop-Process -Id $process.Id;[void]$process.WaitForExit(5000)}
}
$capture=Get-Content "$dir/joined-victory.json" -Raw -Encoding utf8|ConvertFrom-Json
if($capture.gameplay.ending -ne 'joined_victory' -or $capture.input.context -ne 'hero'){throw 'Final screenshot is not the completed joint operation'}
$benchmark=Invoke-M4 @('benchmark','benchmarks/m4.json','--output',"$dir/benchmark.json")
if(-not $benchmark.ok){throw 'M4 AI authoring benchmark failed'}
$restored=Invoke-M4 @('--db',$db,'undo',"$($ready.rev)")
if((Get-FileHash -LiteralPath $exe).Hash -ne $hash){throw 'Executable changed during validation'}
$report=@{
    ok=$true;exe_sha256=$hash;gpu=$baseline.gpu;graph=$graph.packages;headless=$scenario.observation;
    supplied=$supplied;completed=$capture;authoring=$applied;runtime_receipt=$trained;delta=$delta;spoof_rejection=$spoof;
    benchmark=$benchmark;restored=$restored;squad="$dir/squad.png";victory="$dir/joined-victory.png"
}
$report|ConvertTo-Json -Depth 65|Set-Content "$dir/report.json" -Encoding utf8
Copy-Item -LiteralPath "$dir/report.json" -Destination artifacts/m4-validation.json
Write-Host "PASS M4 composition, pinned versions, joint mission, event contracts, contexts, retries and benchmark. Report: $dir/report.json"
