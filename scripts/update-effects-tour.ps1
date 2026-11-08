param([string]$SourceCommit='5f762a1a0862268c595ba2b8d6281b2379c79767')
$ErrorActionPreference='Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
if($SourceCommit -notmatch '^[0-9a-f]{40}$'){throw 'Use an exact upstream commit SHA'}
$headers=@{'User-Agent'='Axiom-effects-inventory'}
$base="https://raw.githubusercontent.com/FlaxEngine/FlaxSamples/$SourceCommit"
$commit=Invoke-RestMethod -Headers $headers -Uri "https://api.github.com/repos/FlaxEngine/FlaxSamples/commits/$SourceCommit"
$tree=Invoke-RestMethod -Headers $headers -Uri "https://api.github.com/repos/FlaxEngine/FlaxSamples/git/trees/$SourceCommit`?recursive=1"
$tree|ConvertTo-Json -Depth 16|Set-Content '.tools/tmp/flax-samples-tree.json' -Encoding utf8
[ordered]@{commit=$commit.sha;date=$commit.commit.committer.date}|ConvertTo-Json -Compress|Set-Content '.tools/tmp/flax-samples-version.json' -Encoding utf8
foreach($name in @('MaterialsFeaturesTour','ParticlesFeaturesTour')) {
    Invoke-WebRequest "$base/$name/Content/Scenes/Main.scene" -OutFile ".tools/tmp/$name.scene"
}
node "$PSScriptRoot/build-effects-tour.cjs"
if($LASTEXITCODE -ne 0){throw 'Effects tour inventory/content generation failed'}
