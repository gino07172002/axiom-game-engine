# -TimeoutSec: wait for each GPU smoke process to finish (default 60 s; readiness polls scale with it).
# Raise it on machines whose present wait is long (e.g. MX150/Optimus: 200-475 ms per frame, 900 frames).
param([int]$Port=4302,[switch]$SkipEarlier,[switch]$BuildRelease,[string]$StressExecutable='target/release/axiom.exe',[int]$StressRuns=3,[int]$TimeoutSec=60)
$ErrorActionPreference='Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
$m2Exe=(Resolve-Path target/debug/axiom.exe).Path
$m2Hash=(Get-FileHash -LiteralPath $m2Exe -Algorithm SHA256).Hash
$m2Stamp=Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$m2Dir=Join-Path $PWD "artifacts/m2-$m2Stamp"
$m2Db=Join-Path $PWD "data/m2-$m2Stamp.sqlite"
New-Item -ItemType Directory -Force $m2Dir|Out-Null
$m2WaitMs=$TimeoutSec*1000
$m2ReadyTries=[math]::Max(100,[int](100*$TimeoutSec/60))
function Invoke-M2([string[]]$M2Args){$m2Raw=& $m2Exe @M2Args;if($LASTEXITCODE -ne 0){throw "axiom failed: $M2Args"};$m2Raw|ConvertFrom-Json}
function Wire-M2($M2Request,[int]$M2WirePort=$Port){$m2Client=[Net.Sockets.TcpClient]::new();try{$m2Client.Connect('127.0.0.1',$M2WirePort);$m2Client.ReceiveTimeout=5000;$m2Stream=$m2Client.GetStream();$m2Writer=[IO.StreamWriter]::new($m2Stream,[Text.UTF8Encoding]::new($false),1024,$true);$m2Writer.AutoFlush=$true;$m2Writer.WriteLine(($M2Request|ConvertTo-Json -Compress -Depth 70));$m2Reader=[IO.StreamReader]::new($m2Stream);$m2Reader.ReadLine()|ConvertFrom-Json}finally{$m2Client.Dispose()}}
if(-not $SkipEarlier){& "$PSScriptRoot/m15-smoke-test.ps1" -Port ($Port+1)}
$m2Verified=Invoke-M2 @('--db',$m2Db,'--pack','content/rts-bench','verify','--ticks','120')
$m2Functional=Invoke-M2 @('--db',$m2Db,'scenario','examples/rts-functional.scenario.json')
if($m2Functional.observation.rts.summary.stats.produced -ne 2 -or $m2Functional.observation.rts.summary.stats.completed_moves -ne 32){throw '64-unit function scenario incomplete'}
$m2Initial=Invoke-M2 @('--db',$m2Db,'play','--start','--port','0','--smoke-frames','8','--screenshot',"$m2Dir/initial.png")
if($m2Initial.observation.rts.summary.units_total -ne 64){throw 'Native RTS did not start with 64 units'}
$m2Probe=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,$Port);try{$m2Probe.Start()}finally{$m2Probe.Stop()}
$m2Process=Start-Process -FilePath $m2Exe -ArgumentList ('--db "{0}" play --start --port {1} --smoke-frames 900 --screenshot "{2}"' -f $m2Db,$Port,"$m2Dir/gameplay.png") -WorkingDirectory $PWD -WindowStyle Hidden -PassThru -RedirectStandardOutput "$m2Dir/live.log" -RedirectStandardError "$m2Dir/live-errors.log"
$null=$m2Process.Handle
try{
 $m2Ready=$false
 for($m2Try=0;$m2Try -lt $m2ReadyTries;$m2Try++){try{$m2Observation=Wire-M2 @{op='observe'};if($m2Observation.ok -and $m2Observation.observation.gpu){$m2Ready=$true;break}}catch{};if($m2Process.HasExited){throw 'RTS helper exited during startup'};Start-Sleep -Milliseconds 200}
 if(-not $m2Ready){throw 'RTS renderer did not become ready'}
 $m2Selected=Invoke-M2 @('--connect',"127.0.0.1:$Port",'command','examples/rts-select.command.json')
 if(@($m2Selected.result.selected).Count -ne 32){throw 'Live box selection failed'}
 $m2Moved=Invoke-M2 @('--connect',"127.0.0.1:$Port",'command','examples/rts-move.command.json')
 $m2Gather=Invoke-M2 @('--connect',"127.0.0.1:$Port",'command','examples/rts-gather.command.json')
 $m2TrainRequest=@{op='execute';request_id='worker-production';runtime_session=$m2Observation.observation.runtime_session;request=@{op='command';command=@{kind='call';capability='rts';action='train';args=@{building='base/0';recipe='worker';count=1}}}}
 $m2Train=Wire-M2 $m2TrainRequest
 $m2TrainReplay=Wire-M2 $m2TrainRequest
 if(-not $m2Train.ok -or ($m2Train|ConvertTo-Json -Compress -Depth 50) -ne ($m2TrainReplay|ConvertTo-Json -Compress -Depth 50)){throw 'RTS production retry duplicated the order'}
 $m2Soldier=Wire-M2 @{op='command';command=@{kind='call';capability='rts';action='train';args=@{building='base/0';recipe='soldier';count=1}}}
 if(-not $m2Soldier.ok){throw 'Soldier production failed'}
 $m2Denied=Wire-M2 @{op='command';command=@{kind='call';capability='rts';action='move';args=@{units=@('unit/1/0');destination=@(-10,0)}}}
 if($m2Denied.ok){throw 'Enemy ownership guard failed'}
 $m2First=Wire-M2 @{op='observe_delta';fields=@('/rts/summary','/rts/selected')}
 $m2Delta=Wire-M2 @{op='observe_delta';fields=@('/rts/summary','/rts/selected');cursor=$m2First.cursor}
 if(-not $m2Delta.ok){throw 'Projected RTS observation failed'}
 if(-not $m2Process.WaitForExit($m2WaitMs)){throw "RTS GPU gameplay timed out after $TimeoutSec s (raise -TimeoutSec)"}
 if($m2Process.ExitCode -ne 0){throw 'RTS GPU gameplay failed'}
 $m2Gameplay=Get-Content "$m2Dir/gameplay.json" -Raw -Encoding utf8|ConvertFrom-Json
 if($m2Gameplay.rts.summary.units_total -ne 66 -or $m2Gameplay.rts.summary.stats.produced -ne 2 -or $m2Gameplay.rts.summary.stats.delivered -lt 8){throw 'Live gathering/production failed'}
}finally{if(-not $m2Process.HasExited){Stop-Process -Id $m2Process.Id;[void]$m2Process.WaitForExit(5000)}}
$m2GpuCases=@()
foreach($m2Population in @(128,256)){
 $m2Pack=Join-Path $m2Dir "pack-$m2Population"
 & .\target\debug\examples\export_rts_pack.exe $m2Population $m2Pack
 if($LASTEXITCODE -ne 0){throw 'RTS stress content export failed'}
 $m2CasePort=$Port+10+[int]($m2Population/128)
 $m2CaseDb=Join-Path $PWD "data/m2-$m2Stamp-$m2Population.sqlite"
 $m2CaseProbe=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,$m2CasePort);try{$m2CaseProbe.Start()}finally{$m2CaseProbe.Stop()}
 $m2CaseProcess=Start-Process -FilePath $m2Exe -ArgumentList ('--db "{0}" --pack "{1}" play --start --port {2} --smoke-frames 240 --screenshot "{3}"' -f $m2CaseDb,$m2Pack,$m2CasePort,"$m2Dir/gpu-$m2Population.png") -WorkingDirectory $PWD -WindowStyle Hidden -PassThru -RedirectStandardOutput "$m2Dir/gpu-$m2Population.log" -RedirectStandardError "$m2Dir/gpu-$m2Population-errors.log"
 $null=$m2CaseProcess.Handle
 try{
  $m2CaseReady=$false
  for($m2Try=0;$m2Try -lt $m2ReadyTries;$m2Try++){try{$m2CaseObserved=Wire-M2 @{op='observe'} $m2CasePort;if($m2CaseObserved.ok -and $m2CaseObserved.observation.gpu){$m2CaseReady=$true;break}}catch{};if($m2CaseProcess.HasExited){throw 'GPU case exited early'};Start-Sleep -Milliseconds 100}
  if(-not $m2CaseReady){throw 'GPU case not ready'}
  $m2CaseSelect=Wire-M2 @{op='command';command=@{kind='call';capability='rts';action='select_rect';args=@{min=@(-45,-25);max=@(-10,5)}}} $m2CasePort
  $m2CaseMove=Wire-M2 @{op='command';command=@{kind='call';capability='rts';action='move';args=@{destination=@(-20.5,12.5)}}} $m2CasePort
  if(-not $m2CaseSelect.ok -or -not $m2CaseMove.ok -or @($m2CaseSelect.result.selected).Count -ne ($m2Population/2)){throw 'GPU group input failed'}
  if(-not $m2CaseProcess.WaitForExit($m2WaitMs) -or $m2CaseProcess.ExitCode -ne 0){throw "GPU case failed or timed out after $TimeoutSec s (raise -TimeoutSec)"}
  $m2Case=Get-Content "$m2Dir/gpu-$m2Population.json" -Raw -Encoding utf8|ConvertFrom-Json
  if($m2Case.rts.summary.units_alive -ne $m2Population -or $m2Case.rts.summary.stats.path_requests -lt ($m2Population/2)){throw 'GPU case did not simulate/render expected population'}
  $m2GpuCases+=@{population=$m2Population;observation=$m2Case;screenshot="$m2Dir/gpu-$m2Population.png";scope='240 native GPU frames with friendly group movement; validates rendering, not GPU timestamp performance'}
  Copy-Item -LiteralPath "$m2Dir/gpu-$m2Population.png" -Destination "artifacts/m2-gpu-$m2Population.png"
 }finally{if(-not $m2CaseProcess.HasExited){Stop-Process -Id $m2CaseProcess.Id;[void]$m2CaseProcess.WaitForExit(5000)}}
}
if($BuildRelease){
 cargo build --release -p axiom --offline -j1
 if($LASTEXITCODE -ne 0){throw 'Release build failed'}
}
& "$PSScriptRoot/m2-stress-test.ps1" -Executable $StressExecutable -Runs $StressRuns
$m2Stress=Get-Content artifacts/m2-stress.json -Raw -Encoding utf8|ConvertFrom-Json
if((Get-FileHash -LiteralPath $m2Exe -Algorithm SHA256).Hash -ne $m2Hash){throw 'Gameplay executable changed during QA'}
Copy-Item -LiteralPath "$m2Dir/initial.png" -Destination artifacts/m2-rts.png
Copy-Item -LiteralPath "$m2Dir/gameplay.png" -Destination artifacts/m2-gameplay.png
$m2EarlierReports=@{}
foreach($m2Milestone in @('m0','m1','m15')){
 $m2EarlierPath="artifacts/$m2Milestone-validation.json"
 if(Test-Path -LiteralPath $m2EarlierPath){
  $m2Earlier=Get-Content -LiteralPath $m2EarlierPath -Raw -Encoding utf8|ConvertFrom-Json
  if($m2Earlier.ok -and $m2Earlier.executable_sha256 -eq $m2Hash){$m2EarlierReports[$m2Milestone]=@{path=$m2EarlierPath;sha256=(Get-FileHash -LiteralPath $m2EarlierPath -Algorithm SHA256).Hash}}
 }
}
if(-not $SkipEarlier -and $m2EarlierReports.Count -ne 3){throw 'Earlier regression reports do not match this executable'}
$m2Report=[ordered]@{ok=$true;same_gameplay_executable=$true;gameplay_executable_sha256=$m2Hash;earlier_regressions_included=(-not $SkipEarlier);earlier_regressions_verified_same_build=($m2EarlierReports.Count -eq 3);earlier_reports=$m2EarlierReports;verify=$m2Verified;functional=$m2Functional;initial=$m2Initial;native_gameplay=$m2Gameplay;live=@{selection=$m2Selected;move=$m2Moved;gather=$m2Gather;train=$m2Train;train_replay=$m2TrainReplay;soldier=$m2Soldier;ownership_rejection=$m2Denied;delta=$m2Delta};gpu_cases=$m2GpuCases;stress=$m2Stress}
$m2Report|ConvertTo-Json -Depth 85|Set-Content -Encoding utf8 "$m2Dir/report.json"
Copy-Item -LiteralPath "$m2Dir/report.json" -Destination artifacts/m2-validation.json
Write-Host "PASS M2: 64-unit functions / native RTS / selection / group orders / mining / production / 128-256 measured stress / FPS-ADV regression. Report: $m2Dir/report.json"
