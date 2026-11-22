# Rebuilds content/plaza ("Sakura Plaza", the first-party editor demo scene) from its ACL stages
# (content/plaza/build/*.acl) through the public protocol: a fresh database from content/empty,
# every stage in order (one revision each), verify, and export. Model paths are written relative
# to the working directory while building and rewritten relative to the package on export.
# Usage: powershell -NoProfile -File scripts/build-plaza.ps1 [-Exe target\release\axiom.exe] [-NoExport]
param(
    [string]$Exe = "target\debug\axiom.exe",
    # A database path that does not exist yet. Default: a new per-run folder under
    # target\plaza-build. An existing database or SQLite sidecar is refused, never deleted.
    [string]$Db,
    [switch]$NoExport
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
if (-not (Test-Path $Exe)) { throw "missing $Exe (build it first: cargo build -p axiom)" }
$fresh = -not $Db
if ($fresh) { $Db = "target\plaza-build\run-{0}-{1}\plaza.sqlite" -f (Get-Date -Format 'yyyyMMdd-HHmmss-fff'), $PID }
$Db = [IO.Path]::GetFullPath([IO.Path]::Combine($root, $Db))
$created = @('', '-wal', '-shm', '-journal') | ForEach-Object { "$Db$_" }
foreach ($path in $created) {
    if (Test-Path -LiteralPath $path) { throw "refusing to build into an existing database: $path exists (nothing was deleted). Pass a new -Db path." }
}
New-Item -ItemType Directory -Force (Split-Path -Parent $Db) | Out-Null

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
    $world = Join-Path $root "content\plaza\world.json"
    Invoke-Axiom @("export", "content\plaza\world.json") | Out-Null
    $text = [IO.File]::ReadAllText($world).Replace('"content/plaza/', '"')
    [IO.File]::WriteAllText($world, $text)
    $parsed = $text | ConvertFrom-Json
    $caps = ($parsed.capabilities.PSObject.Properties | ForEach-Object { "`"$($_.Name)`":$($_.Value)" }) -join ","
    $manifest = "{`"id`":`"axiom.plaza`",`"version`":1,`"world`":`"world.json`",`"capabilities`":{$caps}}`n"
    [IO.File]::WriteAllText((Join-Path $root "content\plaza\manifest.json"), $manifest)
    Write-Host "exported content\plaza\world.json + manifest.json"
}
# A default (per-run) database is removed after a successful build: only the exact files this
# run created, then its own folder if it is empty. A database passed with -Db is kept.
if ($fresh) {
    foreach ($path in $created) { if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path } }
    $runDir = Split-Path -Parent $Db
    if (-not (Get-ChildItem -LiteralPath $runDir -Force)) { Remove-Item -LiteralPath $runDir }
} else {
    Write-Host "database: $Db"
}
