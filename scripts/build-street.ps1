# Rebuilds content/street from its authoring script (content/street/build) through the public
# protocol: a fresh database from content/empty, the typed seed request, then every ACL stage in
# order (one revision each), verify, and export. The database is a throwaway under target/.
# Usage: powershell -NoProfile -File scripts/build-street.ps1 [-Exe target\release\axiom.exe] [-NoExport]
param(
    [string]$Exe = "target\debug\axiom.exe",
    [string]$Db = "target\street-build\street.sqlite",
    [switch]$NoExport
)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)
if (-not (Test-Path $Exe)) { throw "missing $Exe (build it first: cargo build -p axiom)" }
$dir = Split-Path -Parent $Db
New-Item -ItemType Directory -Force $dir | Out-Null
Remove-Item "$Db*" -ErrorAction SilentlyContinue

function Invoke-Axiom([string[]]$Arguments) {
    $out = & $Exe --db $Db @Arguments 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "axiom $($Arguments -join ' ') failed:`n$out" }
    return $out
}

Invoke-Axiom @("--pack", "content\empty", "query") | Out-Null
$seed = Invoke-Axiom @("apply", "content\street\build\00-seed.apply.json")
Write-Host "seed: rev $(($seed | ConvertFrom-Json).rev)"
foreach ($stage in Get-ChildItem content\street\build\*.acl | Sort-Object Name) {
    $out = Invoke-Axiom @("acl", $stage.FullName)
    Write-Host "$($stage.Name): $(($out -split "`n")[0].Trim())"
}
$verify = Invoke-Axiom @("verify") | ConvertFrom-Json
if (-not $verify.ok) { throw "verify failed: $($verify | ConvertTo-Json -Depth 8)" }
Write-Host "verify: ok rev=$($verify.rev)"
if (-not $NoExport) {
    Invoke-Axiom @("export", "content\street\world.json") | Out-Null
    $world = Get-Content content\street\world.json -Raw -Encoding utf8 | ConvertFrom-Json
    $caps = ($world.capabilities.PSObject.Properties | ForEach-Object { "`"$($_.Name)`":$($_.Value)" }) -join ","
    $manifest = "{`"id`":`"axiom.street`",`"version`":1,`"world`":`"world.json`",`"capabilities`":{$caps}}`n"
    [IO.File]::WriteAllText((Join-Path (Get-Location) "content\street\manifest.json"), $manifest)
    Write-Host "exported content\street\world.json + manifest.json"
}
