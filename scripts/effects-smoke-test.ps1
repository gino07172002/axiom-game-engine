param([int]$Port=4572,[switch]$SkipCaptures)
$ErrorActionPreference='Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
$exe=(Resolve-Path 'target/debug/axiom.exe').Path
$hash=(Get-FileHash -LiteralPath $exe).Hash
$stamp=Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$dir=Join-Path $PWD "artifacts/effects-$stamp"
$db=Join-Path $PWD "data/effects-$stamp.sqlite"
New-Item -ItemType Directory -Force $dir|Out-Null
function Invoke-Effects([string[]]$Arguments) {
    $raw=& $exe @Arguments
    if($LASTEXITCODE -ne 0){throw "Axiom failed: $Arguments"}
    $raw|ConvertFrom-Json
}
function Send-Effects($Request) {
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
function Wait-Effects([scriptblock]$Predicate) {
    for($i=0;$i -lt 1200;$i++){
        if($process.HasExited){throw "Alliance exited early. See $dir/live-errors.log"}
        try{
            $response=Send-Effects @{op='observe'}
            if($response.ok -and (& $Predicate $response.observation)){return $response.observation}
        }catch{}
        Start-Sleep -Milliseconds 20
    }
    throw 'Alliance observation timed out'
}
function Command-Effects($Command) {
    $response=Send-Effects @{op='command';command=$Command}
    if(-not $response.ok){throw ($response|ConvertTo-Json -Depth 30)}
    $response
}
function Call-Effects([string]$Capability,[string]$Action,$ArgsValue=@{}) {
    Command-Effects @{kind='call';capability=$Capability;action=$Action;args=$ArgsValue}
}

$inventory=Get-Content content/effects-tour/inventory.json -Raw -Encoding utf8|ConvertFrom-Json
if($inventory.stations.Count -ne 36){throw 'Official tour inventory must contain 36 actual stations'}
$verified=Invoke-Effects @('--db',$db,'--pack','content/effects-tour','verify','--ticks','120')
$revision=(Invoke-Effects @('--db',$db,'query')).rev
function Offline-Effects($Request) {
    # A request file avoids Windows PowerShell 5.1 stdin piping into native programs.
    $requestFile=Join-Path $dir 'rpc-request.json'
    [IO.File]::WriteAllText($requestFile,($Request|ConvertTo-Json -Compress -Depth 30))
    $raw=& $exe --db $db rpc $requestFile
    if($LASTEXITCODE -ne 0){throw 'Offline authoring command failed'}
    $response=$raw|ConvertFrom-Json
    if(-not $response.ok){throw ($response|ConvertTo-Json -Depth 20)}
    $response
}
$captures=@()
$overview=Invoke-Effects @('--db',$db,'play','--start','--port','0','--smoke-frames','80','--screenshot',"$dir/overview.png")
if(-not $SkipCaptures) {
    foreach($station in $inventory.stations) {
        $changed=Offline-Effects @{op='apply';expected_rev=$revision;edits=@(@{kind='set_field';target=@{kind='resource';id='showcase.tour'};field='/initial';value=$station.id})}
        $revision=$changed.rev
        $file=$station.id.Replace('/','-')
        $capture=Invoke-Effects @('--db',$db,'play','--start','--port','0','--smoke-frames','80','--screenshot',"$dir/$file.png")
        if($capture.observation.showcase.active -ne $station.id -or $capture.observation.showcase.stations -ne 36){throw "Station capture incorrect: $($station.id)"}
        $captures+=@{id=$station.id;title=$station.title;screenshot="$dir/$file.png";implementation=$station.implementation;observation=$capture.observation}
        Write-Host "Captured $($station.id)"
    }
}
$changed=Offline-Effects @{op='apply';expected_rev=$revision;edits=@(@{kind='set_field';target=@{kind='resource';id='showcase.tour'};field='/initial';value=$null})}
$revision=$changed.rev
$process=Start-Process -FilePath $exe -ArgumentList ('--db "{0}" play --start --port {1} --smoke-frames 900 --screenshot "{2}"' -f $db,$Port,"$dir/live-repaired.png") -WorkingDirectory $PWD -WindowStyle Hidden -PassThru -RedirectStandardOutput "$dir/live.log" -RedirectStandardError "$dir/live-errors.log"
$null=$process.Handle
try {
    $ready=Wait-Effects {param($o)$o.gpu -and $o.showcase.stations -eq 36}
    $firstDelta=Send-Effects @{op='observe_delta';fields=@('/showcase','/shader','/activation','/rev')}
    $request=@{op='execute';request_id='focus-metal';runtime_session=$ready.runtime_session;request=@{op='command';command=@{kind='call';capability='showcase';action='focus';args=@{station='mat/03-metalness'}}}}
    $focused=Send-Effects $request
    $focusedReplay=Send-Effects $request
    if(-not $focused.ok -or ($focused|ConvertTo-Json -Compress -Depth 20) -ne ($focusedReplay|ConvertTo-Json -Compress -Depth 20)){throw 'Focus command retry failed'}
    $effectBefore=Wait-Effects {param($o)$o.showcase.active -eq 'mat/03-metalness'}
    $edit=@{op='apply';expected_rev=$ready.rev;edits=@(@{kind='set_field';target=@{kind='resource';id='render.shaders'};field='/materials/metal/parameters/metalness';value=0.15})}
    $preview=$edit.Clone();$preview.op='preview'
    $previewed=Send-Effects $preview
    $applyRequest=@{op='execute';request_id='metal-instance';request=$edit}
    $applied=Send-Effects $applyRequest
    $applyReplay=Send-Effects $applyRequest
    if(-not $previewed.ok -or -not $applied.ok -or ($applied|ConvertTo-Json -Compress -Depth 20) -ne ($applyReplay|ConvertTo-Json -Compress -Depth 20)){throw 'Material local edit/receipt failed'}
    $active=Wait-Effects {param($o)$o.rev -eq $applied.rev -and $o.activation -eq 'applied'}
    if($active.vfx.emitters.'fx/02-gpu'.time -lt $effectBefore.vfx.emitters.'fx/02-gpu'.time){throw 'Unchanged GPU emitter restarted after material edit'}
    $paused=Command-Effects @{kind='call';capability='vfx';action='pause';args=@{id='fx/02-gpu';paused=$true}}
    $inspected=Command-Effects @{kind='call';capability='vfx';action='inspect';args=@{id='fx/02-gpu'}}
    $restart=Command-Effects @{kind='call';capability='vfx';action='restart';args=@{id='fx/06-attached'}}
    $bad=@{op='preview';expected_rev=$applied.rev;edits=@(@{kind='set_field';target=@{kind='resource';id='vfx.library'};field='/emitters/fx~102-gpu/count';value=1000000})}
    $rejected=Send-Effects $bad
    if($rejected.ok -or -not $rejected.diagnostics){throw 'Invalid particle budget did not produce diagnostics'}
    $vertexBad='fn vertex_offset(p:vec3<f32>,n:vec3<f32>,uv:vec2<f32>,t:f32,v:vec4<f32>)->vec3<f32>{return p+dpdx(p);} fn surface(s:SurfaceInput)->vec4<f32>{return s.base_color;}'
    $failure=Send-Effects @{op='apply';expected_rev=$applied.rev;edits=@(@{kind='set_field';target=@{kind='resource';id='render.shaders'};field='/shaders/vertex/wgsl';value=$vertexBad})}
    if(-not $failure.ok){throw 'Vertex pipeline failure fixture did not pass the CPU signature check'}
    $retained=Wait-Effects {param($o)$o.requested_rev -eq $failure.rev -and $o.activation -eq 'activation_failed'}
    if($retained.rev -ne $applied.rev){throw 'GPU vertex stage failure replaced the usable revision'}
    $repair=Send-Effects @{op='undo';rev=$applied.rev}
    if(-not $repair.ok){throw 'Vertex shader restoration failed'}
    $restored=Wait-Effects {param($o)$o.rev -eq $repair.rev -and $o.activation -eq 'applied'}
    $delta=Send-Effects @{op='observe_delta';fields=@('/showcase','/shader','/activation','/rev');cursor=$firstDelta.cursor}
    if(-not $delta.ok){throw 'Projected effects observation failed'}
    if(-not $process.WaitForExit(60000) -or $process.ExitCode -ne 0){throw 'Effects native helper timed out or failed'}
} finally {if(-not $process.HasExited){Stop-Process -Id $process.Id;[void]$process.WaitForExit(5000)}}
$final=Get-Content "$dir/live-repaired.json" -Raw -Encoding utf8|ConvertFrom-Json
if($final.activation -ne 'applied' -or $final.showcase.active -ne 'mat/03-metalness'){throw 'Final native screenshot is not the repaired material instance'}
if((Get-FileHash -LiteralPath $exe).Hash -ne $hash){throw 'Executable changed during effects QA'}
$report=@{ok=$true;exe_sha256=$hash;source_commit=$inventory.commit;inventory_count=$inventory.stations.Count;captures=$captures;overview="$dir/overview.png";verified=$verified;live=@{focused=$focused;applied=$applied;preview=$previewed;delta=$delta;budget_rejection=$rejected;gpu_stage_failure=$retained;repair=$restored;gpu_inspection=$inspected};completed=$final;final_screenshot="$dir/live-repaired.png"}
$report|ConvertTo-Json -Depth 70|Set-Content "$dir/report.json" -Encoding utf8
Copy-Item -LiteralPath "$dir/report.json" -Destination artifacts/effects-validation.json
Write-Host "PASS 24 materials / 12 particle exhibits / native GPU captures / local edits / retries / emitter state / vertex-stage recovery. Report: $dir/report.json"
