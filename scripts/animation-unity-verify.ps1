param([Parameter(Mandatory)][string]$Bundle,[Parameter(Mandatory)][string]$Model,[string]$UnityVersion,[string]$EditorLog)
$ErrorActionPreference='Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
if((Get-Item -LiteralPath $Model).PSIsContainer){
    $pmx=@(Get-ChildItem -LiteralPath $Model -Filter '*.pmx' -File)
    if($pmx.Count -ne 1){throw 'Select one PMX model'}
    $Model=$pmx[0].FullName
}
$exe=(Resolve-Path 'target/debug/axiom.exe').Path
$reportDir=Join-Path $PWD ('artifacts/unity-animation-'+(Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
New-Item -ItemType Directory -Force $reportDir|Out-Null
function Invoke-UnityAxiom([string[]]$Arguments){
    $result=& $exe @Arguments
    if($LASTEXITCODE -ne 0){throw "Axiom failed: $Arguments"}
    $result|ConvertFrom-Json
}
$pack='data/animation-reference/unity-pack'
$demo=Invoke-UnityAxiom @('animation-demo',$Model,'--source',$Bundle,'--output',$pack)
$numerical=& .\target\debug\examples\validate_animation.exe $pack "$reportDir/numeric.json"
if($LASTEXITCODE -ne 0){throw 'Unity weighted-mesh validation failed'}
$numerical=$numerical|ConvertFrom-Json
$baked=Invoke-UnityAxiom @('animation-retarget',$Bundle,$Model,'--clip','Walking','--output',"$reportDir/walking.clip.json")
$clip=Get-Content -LiteralPath $baked.output -Raw|ConvertFrom-Json
$animated=@($clip.tracks|Where-Object {$_.property -eq 'rotation' -and $_.values.Count -gt 4}).Count
if($animated -lt 10){throw "Unity clip has too few animated rotation tracks: $animated"}
$db='data/unity-animation-preview-'+(Get-Date -Format 'yyyyMMdd-HHmmss-fff')+'.sqlite'
$capture=Invoke-UnityAxiom @('--db',$db,'--pack',$pack,'play','--start','--port','0','--smoke-frames','26','--screenshot',"$reportDir/walking.png")
$player=$capture.observation.animation.players.character
if($player.clip -ne 'Walking' -or -not $player.retargeted -or $player.fault){throw 'Native Unity playback failed'}
$verifiedEditor=$EditorLog -and (Select-String -LiteralPath $EditorLog -SimpleMatch 'AXIOM_UNITY_HUMANOID_EXPORT_OK' -Quiet)
[ordered]@{ok=$true;editor=$UnityVersion;editor_executed=$verifiedEditor;editor_log=$EditorLog;numerical=$numerical;bake=$baked;animated_rotation_tracks=$animated;capture=$capture;executable_sha256=(Get-FileHash -LiteralPath $exe).Hash}|ConvertTo-Json -Depth 40|Set-Content -Encoding utf8 "$reportDir/report.json"
Copy-Item -LiteralPath "$reportDir/report.json" -Destination artifacts/animation-unity-validation.json
Copy-Item -LiteralPath "$reportDir/walking.png" -Destination artifacts/animation-unity.png
Write-Host "PASS Unity Walking → PMX. Weighted foot displacement: $($numerical.reference_weighted_foot_delta_m) m. Report: $reportDir/report.json"
