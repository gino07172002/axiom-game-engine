param([string]$Executable='target/release/axiom.exe',[int]$Ticks=2400,[int]$Runs=3)
$ErrorActionPreference='Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
if($Runs -lt 1 -or $Runs -gt 10){throw 'Runs must be 1..10'}
$m2StressExe=(Resolve-Path -LiteralPath $Executable).Path
$m2StressHash=(Get-FileHash -LiteralPath $m2StressExe -Algorithm SHA256).Hash
$m2StressStamp=Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$m2StressDir=Join-Path $PWD "artifacts/m2-stress-$m2StressStamp"
New-Item -ItemType Directory -Force $m2StressDir|Out-Null
$m2StressResults=@()
foreach($m2Population in @(64,128,256)){
 for($m2Run=1;$m2Run -le $Runs;$m2Run++){
  $m2Case=& $m2StressExe rts-stress --units $m2Population --ticks $Ticks --output "$m2StressDir/$m2Population-$m2Run.json"
  if($LASTEXITCODE -ne 0){throw "Stress failed: $m2Population / $m2Run"}
  $m2Case=$m2Case|ConvertFrom-Json
  if(-not $m2Case.ok -or $m2Case.state.units_alive -ne $m2Population -or $m2Case.overlaps -ne 0){throw 'Stress invariants failed'}
  $m2StressResults+=@{population=$m2Population;run=$m2Run;result=$m2Case}
  Write-Host ("Measured {0} units run {1}: step p95 {2:N3} ms, p99 {3:N3} ms" -f $m2Population,$m2Run,$m2Case.step_ms.p95,$m2Case.step_ms.p99)
 }
}
if((Get-FileHash -LiteralPath $m2StressExe -Algorithm SHA256).Hash -ne $m2StressHash){throw 'Stress executable changed during measurements'}
$m2Cpu=Get-CimInstance Win32_Processor|Select-Object Name,NumberOfCores,NumberOfLogicalProcessors
$m2Os=Get-CimInstance Win32_OperatingSystem|Select-Object Caption,Version,TotalVisibleMemorySize
$m2StressSummary=@()
foreach($m2Population in @(64,128,256)){
 $m2Rows=@($m2StressResults|Where-Object population -EQ $m2Population)
 $m2P95=@($m2Rows|ForEach-Object {$_.result.step_ms.p95}|Sort-Object)
 $m2Median=$m2P95[[int][Math]::Floor($m2P95.Count/2)]
 if($m2P95.Count % 2 -eq 0){$m2Median=($m2Median+$m2P95[[int]($m2P95.Count/2)-1])/2}
 $m2Peak=@($m2Rows|ForEach-Object {$_.result.memory.peak_working_set_bytes}|Measure-Object -Maximum).Maximum
 $m2StressSummary+=@{population=$m2Population;runs=$Runs;p95_median_ms=$m2Median;p95_min_ms=$m2P95[0];p95_max_ms=$m2P95[-1];peak_process_working_set_bytes=$m2Peak}
}
$m2StressReport=[ordered]@{ok=$true;executable=$m2StressExe;executable_sha256=$m2StressHash;timestamp=[DateTimeOffset]::Now.ToString('o');cpu=$m2Cpu;os=$m2Os;summary=$m2StressSummary;cases=$m2StressResults;scope='Fresh process per case; headless CPU simulation only; both factions moving; combat disabled for fixed population; commands and invariant checks excluded from step samples. No GPU timing or rendered FPS claim.'}
$m2StressReport|ConvertTo-Json -Depth 35|Set-Content -Encoding utf8 "$m2StressDir/report.json"
Copy-Item -LiteralPath "$m2StressDir/report.json" -Destination artifacts/m2-stress.json
Write-Host "PASS M2 measured stress: $m2StressDir/report.json"
