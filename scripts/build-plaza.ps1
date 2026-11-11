# Rebuilds content/plaza ("Sakura Plaza", the first-party editor demo scene) from its ACL stages
# (content/plaza/build/*.acl) through the public protocol: a fresh database from content/empty,
# every stage in order (one revision each), verify, and export. Model paths are written relative
# to the working directory while building and rewritten relative to the package on export.
# Usage: powershell -NoProfile -File scripts/build-plaza.ps1 [-Exe target\release\axiom.exe] [-NoExport]
param(
    [string]$Exe = "target\debug\axiom.exe",
    [string]$Db = "target\plaza-build\plaza.sqlite",
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
foreach ($stage in Get-ChildItem content\plaza\build\*.acl | Sort-Object Name) {
    $out = Invoke-Axiom @("acl", $stage.FullName)
    Write-Host "$($stage.Name): $(($out -split "`n")[0].Trim())"
}
$verify = Invoke-Axiom @("verify") | ConvertFrom-Json
if (-not $verify.ok) { throw "verify failed: $($verify | ConvertTo-Json -Depth 8)" }
Write-Host "verify: ok rev=$($verify.rev)"
if (-not $NoExport) {
    $world = Join-Path (Get-Location) "content\plaza\world.json"
    Invoke-Axiom @("export", "content\plaza\world.json") | Out-Null
    $text = [IO.File]::ReadAllText($world).Replace('"content/plaza/', '"')
    [IO.File]::WriteAllText($world, $text)
    $parsed = $text | ConvertFrom-Json
    $caps = ($parsed.capabilities.PSObject.Properties | ForEach-Object { "`"$($_.Name)`":$($_.Value)" }) -join ","
    $manifest = "{`"id`":`"axiom.plaza`",`"version`":1,`"world`":`"world.json`",`"capabilities`":{$caps}}`n"
    [IO.File]::WriteAllText((Join-Path (Get-Location) "content\plaza\manifest.json"), $manifest)
    Write-Host "exported content\plaza\world.json + manifest.json"
}
