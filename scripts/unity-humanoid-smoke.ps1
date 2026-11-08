param([Parameter(Mandatory)][string]$Editor,[Parameter(Mandatory)][string]$HumanoidFbx,[string]$UnityVersion='6000.6.0f1',[string]$Model)
$ErrorActionPreference='Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
$unityProject=Join-Path $PWD ('data/unity-animation-qa-'+(Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
$unityProject=Join-Path $unityProject 'project'
New-Item -ItemType Directory -Force "$unityProject/Assets/Editor","$unityProject/Packages","$unityProject/ProjectSettings" | Out-Null
'{"dependencies":{"com.unity.modules.animation":"1.0.0","com.unity.modules.imgui":"1.0.0","com.unity.modules.imageconversion":"1.0.0"}}' | Set-Content -Encoding utf8 "$unityProject/Packages/manifest.json"
"m_EditorVersion: $UnityVersion" | Set-Content -Encoding utf8 "$unityProject/ProjectSettings/ProjectVersion.txt"
Copy-Item -LiteralPath $HumanoidFbx -Destination "$unityProject/Assets/fixture.fbx"
Copy-Item -LiteralPath "$HumanoidFbx.meta" -Destination "$unityProject/Assets/fixture.fbx.meta"
Copy-Item -LiteralPath 'tools/unity/Editor/AxiomHumanoidExport.cs' -Destination "$unityProject/Assets/Editor/AxiomHumanoidExport.cs"
Copy-Item -LiteralPath 'scripts/unity-humanoid-smoke.cs' -Destination "$unityProject/Assets/Editor/AxiomUnitySmoke.cs"
$unityLog=Join-Path (Split-Path $unityProject) 'editor.log'
$unityArgs='-batchmode -quit --burst-disable-compilation -projectPath "{0}" -executeMethod AxiomUnitySmoke.Run -logFile "{1}"' -f $unityProject,$unityLog
$unityProcess=Start-Process -FilePath $Editor -ArgumentList $unityArgs -WindowStyle Hidden -PassThru
if(-not $unityProcess.WaitForExit(180000)){Stop-Process -Id $unityProcess.Id;throw "Unity export timed out. See $unityLog"}
if($unityProcess.ExitCode -ne 0 -or -not(Select-String -LiteralPath $unityLog -SimpleMatch 'AXIOM_UNITY_HUMANOID_EXPORT_OK' -Quiet)){
    Get-Content -LiteralPath $unityLog -Tail 38
    throw "Unity export failed. See $unityLog"
}
$bundle=Join-Path (Split-Path $unityProject) 'unity-walk.axrig'
Write-Host "PASS Unity Humanoid exporter. Asset: $bundle"
if($Model){& "$PSScriptRoot/animation-unity-verify.ps1" -Bundle $bundle -Model $Model -UnityVersion $UnityVersion -EditorLog $unityLog}
