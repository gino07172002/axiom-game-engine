param([int]$Port=4282,[switch]$SkipM1)
$ErrorActionPreference='Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
$m15Exe=(Resolve-Path target/debug/axiom.exe).Path
$m15Hash=(Get-FileHash $m15Exe -Algorithm SHA256).Hash
$m15Stamp=Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$m15Dir=Join-Path $PWD "artifacts/m15-$m15Stamp"
$m15Db=Join-Path $PWD "data/m15-$m15Stamp.sqlite"
New-Item -ItemType Directory -Force $m15Dir | Out-Null
function Invoke-M15([string[]]$M15Args){$m15Raw=& $m15Exe @M15Args;if($LASTEXITCODE -ne 0){throw "axiom failed: $M15Args"};$m15Raw|ConvertFrom-Json}
function Wire-M15($M15Request){$m15Client=[Net.Sockets.TcpClient]::new();try{$m15Client.Connect('127.0.0.1',$Port);$m15Client.ReceiveTimeout=5000;$m15Stream=$m15Client.GetStream();$m15Writer=[IO.StreamWriter]::new($m15Stream,[Text.UTF8Encoding]::new($false),1024,$true);$m15Writer.AutoFlush=$true;$m15Writer.WriteLine(($M15Request|ConvertTo-Json -Compress -Depth 60));$m15Reader=[IO.StreamReader]::new($m15Stream);$m15Reader.ReadLine()|ConvertFrom-Json}finally{$m15Client.Dispose()}}
if(-not $SkipM1){& "$PSScriptRoot/m1-smoke-test.ps1" -Port ($Port+1)}
$m15Bench=Invoke-M15 @('benchmark','benchmarks/m15.json','--output',"$m15Dir/benchmark.json")
if($m15Bench.summary.local.total_model_tokens -ne $null){throw 'Replay benchmark fabricated model usage'}
Copy-Item "$m15Dir/benchmark.json" artifacts/m15-benchmark.json
Invoke-M15 @('--db',$m15Db,'--pack','content/adv','verify')|Out-Null
function Start-M15 {
 $m15Probe=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,$Port);try{$m15Probe.Start()}finally{$m15Probe.Stop()}
 $script:m15Process=Start-Process -FilePath $m15Exe -ArgumentList ('--db "{0}" play --port {1} --smoke-frames 6000' -f $m15Db,$Port) -WorkingDirectory $PWD -WindowStyle Hidden -PassThru -RedirectStandardOutput "$m15Dir/live.log" -RedirectStandardError "$m15Dir/live-errors.log"
 for($m15Try=0;$m15Try -lt 50;$m15Try++){try{$m15Ready=Wire-M15 @{op='observe'};if($m15Ready.ok){return $m15Ready}}catch{};if($script:m15Process.HasExited){throw 'Native helper exited'};Start-Sleep -Milliseconds 200};throw 'Native helper not ready'
}
try {
 $m15Ready=Start-M15
 $m15Schema=Wire-M15 @{op='discover';topic='local'}
 $m15Cached=Wire-M15 @{op='discover';topic='local';if_hash=$m15Schema.hash}
 if(-not $m15Cached.not_modified -or $m15Cached.schema){throw 'Schema cache failed'}
 $m15Select=Wire-M15 @{op='select';query=@{from='choices';parent='workshop';ids=@('open');fields=@('id','when','next');max_bytes=2048}}
 if(-not $m15Select.ok -or $m15Select.items[0].target.id -ne 'open'){throw 'Local query failed'}
 $m15Edit=Get-Content examples/m15-edit-choice.json -Raw|ConvertFrom-Json
 $m15Envelope=@{op='execute';request_id='authoring-1';request=$m15Edit}
 $m15Applied=Wire-M15 $m15Envelope
 $m15Replay=Wire-M15 $m15Envelope
 if(-not $m15Applied.ok -or ($m15Applied|ConvertTo-Json -Compress -Depth 50) -ne ($m15Replay|ConvertTo-Json -Compress -Depth 50)){throw 'Durable replay changed result'}
 $m15Changes=Wire-M15 @{op='diff';from_rev=1;to_rev=2}
 if($m15Changes.total_changes -lt 1){throw 'Resource diff empty'}
 $m15Bad=Wire-M15 @{op='preview';expected_rev=2;edits=@(@{kind='set_field';target=@{kind='choice';node='workshop';id='open'};field='next';value='missing'})}
 if($m15Bad.ok -or $m15Bad.diagnostics[0].path -ne '/resources/narrative.graph/nodes/@workshop/choices/@open/next'){throw 'Diagnostic lost stable field location'}
 $m15Fields=@('/gameplay/node','/gameplay/variables')
 $m15First=Wire-M15 @{op='observe_delta';fields=$m15Fields;trace_limit=16}
 $m15Same=Wire-M15 @{op='observe_delta';fields=$m15Fields;trace_limit=16;cursor=$m15First.cursor}
 if(-not $m15Same.unchanged){throw 'Paused observation repeated snapshot'}
 $m15Runtime=@{op='execute';request_id='choose-help';runtime_session=$m15Ready.observation.runtime_session;request=@{op='command';command=@{kind='choose';node='arrival';choice='help'}}}
 $m15Command=Wire-M15 $m15Runtime
 $m15CommandReplay=Wire-M15 $m15Runtime
 if(-not $m15Command.ok -or ($m15Command|ConvertTo-Json -Compress -Depth 50) -ne ($m15CommandReplay|ConvertTo-Json -Compress -Depth 50)){throw 'Runtime command executed twice'}
 $m15Delta=Wire-M15 @{op='observe_delta';fields=$m15Fields;trace_limit=16;cursor=$m15First.cursor}
 if($m15Delta.reset -or -not ($m15Delta.changes.path -contains '/gameplay/node')){throw 'Incremental observation did not contain node change'}
 if(@($m15Delta.trace|Where-Object {$_.kind -eq 'choice' -and $_.id -eq 'help'}).Count -ne 1){throw 'Duplicate runtime choice trace'}
 Stop-Process -Id $script:m15Process.Id
 if(-not $script:m15Process.WaitForExit(5000)){throw 'Native helper did not exit before restart'}
 $m15Restart=Start-M15
 $m15OldSession=Wire-M15 $m15Runtime
 if($m15OldSession.ok){throw 'Foreign runtime session accepted after restart'}
 $m15Persistent=Wire-M15 $m15Envelope
 if(-not $m15Persistent.ok -or $m15Persistent.rev -ne 2){throw 'Receipt lost across process restart'}
 $m15Final=Wire-M15 @{op='query'}
 if($m15Final.rev -ne 2){throw 'Duplicate authoring revision after retries'}
}finally{if($script:m15Process -and -not $script:m15Process.HasExited){Stop-Process -Id $script:m15Process.Id}}
if((Get-FileHash $m15Exe -Algorithm SHA256).Hash -ne $m15Hash){throw 'Executable changed during QA'}
$m15Report=[ordered]@{ok=$true;same_executable=$true;executable_sha256=$m15Hash;m1_included=(-not $SkipM1);benchmark=$m15Bench;select=$m15Select;schema_cache=$m15Cached;apply=$m15Applied;replay=$m15Replay;diff=$m15Changes;diagnostic=$m15Bad;first=$m15First;unchanged=$m15Same;command=$m15Command;command_replay=$m15CommandReplay;delta=$m15Delta;old_session=$m15OldSession;durable_restart=$m15Persistent;final=$m15Final}
$m15Report|ConvertTo-Json -Depth 70|Set-Content -Encoding utf8 "$m15Dir/report.json"
Copy-Item "$m15Dir/report.json" artifacts/m15-validation.json
Write-Host "PASS M1.5: local IDs / precise diff / diagnostics / cursor / durable and session retries / benchmark. Report: $m15Dir/report.json"
