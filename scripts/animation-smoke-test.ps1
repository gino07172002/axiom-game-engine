param([Parameter(Mandatory)][string]$Model, [int]$Port=4382, [switch]$SkipEarlier)
$ErrorActionPreference='Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
if((Get-Item -LiteralPath $Model).PSIsContainer){
    $pmxFiles=@(Get-ChildItem -LiteralPath $Model -File -Filter '*.pmx')
    if($pmxFiles.Count -ne 1){throw 'Supply a PMX file or a directory containing exactly one PMX file'}
    $Model=$pmxFiles[0].FullName
}
$animExe=(Resolve-Path 'target/debug/axiom.exe').Path
$animHash=(Get-FileHash -LiteralPath $animExe -Algorithm SHA256).Hash
$modelHash=(Get-FileHash -LiteralPath $Model -Algorithm SHA256).Hash
$stamp=Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$runDir=Join-Path $PWD "artifacts/animation-$stamp"
$db=Join-Path $PWD "data/animation-qa-$stamp.sqlite"
$pack=Join-Path $PWD 'data/animation-reference/pack'
New-Item -ItemType Directory -Force $runDir|Out-Null
function Run-Axiom([string[]]$Arguments){
    $raw=& $animExe @Arguments
    if($LASTEXITCODE -ne 0){throw "Axiom failed: $Arguments"}
    $raw|ConvertFrom-Json
}
function Send-Animation($Request){
    $client=[Net.Sockets.TcpClient]::new()
    try{
        $client.Connect('127.0.0.1',$Port)
        $client.ReceiveTimeout=5000
        $stream=$client.GetStream()
        $writer=[IO.StreamWriter]::new($stream,[Text.UTF8Encoding]::new($false),1024,$true)
        $writer.AutoFlush=$true
        $writer.WriteLine(($Request|ConvertTo-Json -Compress -Depth 64))
        [IO.StreamReader]::new($stream).ReadLine()|ConvertFrom-Json
    }finally{$client.Dispose()}
}
if(-not $SkipEarlier){& "$PSScriptRoot/m15-smoke-test.ps1" -Port ($Port+1)}
$demo=Run-Axiom @('animation-demo',$Model,'--source','content/animation/rigify.glb','--output',$pack)
$numericRaw=& .\target\debug\examples\validate_animation.exe $pack "$runDir/numeric.json"
if($LASTEXITCODE -ne 0){throw 'Numerical rig validation failed'}
$numeric=$numericRaw|ConvertFrom-Json
$bake=Run-Axiom @('animation-retarget','content/animation/rigify.glb',$Model,'--clip','rigify_wave','--output',"$runDir/retargeted.clip.json")
$rest=Run-Axiom @('--db',$db,'--pack',$pack,'play','--start','--port','0','--smoke-frames','2','--screenshot',"$runDir/rest.png")
$pose=Run-Axiom @('--db',$db,'play','--start','--port','0','--smoke-frames','32','--screenshot',"$runDir/retarget.png")
if(-not $pose.observation.animation.players.character.retargeted -or $pose.observation.animation.players.character.fault){throw 'Native animation did not activate'}
$process=Start-Process -FilePath $animExe -ArgumentList ('--db "{0}" play --port {1} --smoke-frames 240 --screenshot "{2}"' -f $db,$Port,"$runDir/ik.png") -WorkingDirectory $PWD -WindowStyle Hidden -PassThru -RedirectStandardOutput "$runDir/native.log" -RedirectStandardError "$runDir/native-errors.log"
try{
    $ready=$false
    for($attempt=0;$attempt -lt 100;$attempt++){
        try{$initial=Send-Animation @{op='observe'};if($initial.ok -and $initial.observation.gpu){$ready=$true;break}}catch{}
        if($process.HasExited){throw 'Animation window exited early'}
        Start-Sleep -Milliseconds 100
    }
    if(-not $ready){throw 'Animation renderer did not become ready'}
    $seek=Send-Animation @{op='command';command=@{kind='call';capability='animation';action='seek';args=@{entity='character';seconds=0.5333333;paused=$true}}}
    if(-not $seek.ok){throw ($seek|ConvertTo-Json -Compress -Depth 20)}
    $request=@{op='execute';request_id='ik-target';runtime_session=$initial.observation.runtime_session;request=@{op='command';command=@{kind='call';capability='animation';action='ik';args=@{entity='character';id='left_hand';target=$numeric.ik_target;pole=$numeric.ik_pole;weight=1.0}}}}
    $ik=Send-Animation $request
    $retry=Send-Animation $request
    if(-not $ik.ok -or $ik.result.ik.left_hand.residual -gt 0.002 -or ($ik|ConvertTo-Json -Compress -Depth 20) -ne ($retry|ConvertTo-Json -Compress -Depth 20)){throw 'Native IK/retry failed'}
    $delta=Send-Animation @{op='observe_delta';fields=@('/animation/players/character')}
    if(-not $delta.ok){throw 'Projected observation failed'}
    if(-not $process.WaitForExit(60000) -or $process.ExitCode -ne 0){throw 'Native animation rendering failed'}
}finally{if(-not $process.HasExited){Stop-Process -Id $process.Id;[void]$process.WaitForExit(5000)}}
$rtsDb=Join-Path $PWD "data/animation-rts-regression-$stamp.sqlite"
$rts=Run-Axiom @('--db',$rtsDb,'--pack','content/rts-bench','scenario','examples/rts-functional.scenario.json')
$rtsGpu=Run-Axiom @('--db',$rtsDb,'play','--start','--port','0','--smoke-frames','8','--screenshot',"$runDir/rts.png")
if($rts.observation.rts.summary.stats.completed_moves -ne 32 -or $rtsGpu.observation.rts.summary.units_total -ne 64){throw 'RTS regression'}
$bundled=Run-Axiom @('--db',"data/animation-bundled-$stamp.sqlite",'--pack','content/animation','play','--start','--port','0','--smoke-frames','32','--screenshot',"$runDir/bundled.png")
if($bundled.observation.animation.players.character.clip -ne 'rigify_wave'){throw 'Bundled animation fallback did not play'}
if((Get-FileHash -LiteralPath $animExe -Algorithm SHA256).Hash -ne $animHash -or (Get-FileHash -LiteralPath $Model -Algorithm SHA256).Hash -ne $modelHash){throw 'Executable or source model changed during QA'}
$earlier=@{}
foreach($m in @('m0','m1','m15')){
    $previous=Get-Content "artifacts/$m-validation.json" -Raw|ConvertFrom-Json
    $earlier[$m]=$previous.ok -and $previous.executable_sha256 -eq $animHash
}
if(-not $SkipEarlier -and $earlier.Values -contains $false){throw 'Earlier regression used a different executable'}
$report=[ordered]@{ok=$true;executable_sha256=$animHash;source_model_sha256=$modelHash;numeric=$numeric;bake=$bake;gpu=$pose.gpu;rest=$rest.observation.animation;retarget=$pose.observation.animation;ik=$ik;delta=$delta;rts=$rts.observation.rts.summary;earlier=$earlier;screenshots=@{rest="$runDir/rest.png";retarget="$runDir/retarget.png";ik="$runDir/ik.png";rts="$runDir/rts.png";bundled="$runDir/bundled.png"}}
$report|ConvertTo-Json -Depth 75|Set-Content -Encoding utf8 "$runDir/report.json"
Copy-Item -LiteralPath "$runDir/report.json" -Destination artifacts/animation-validation.json
Write-Host "PASS animation: Rigify / PMX / retarget / twist / IK / native rendering / RTS regression. Report: $runDir/report.json"
