# Rebuilds content/street from its authoring script (content/street/build) through the public
# protocol: a fresh database from content/empty, then every ACL stage in order (00-seed.acl
# first; one revision each), verify, and export. The database is a throwaway under target/.
# Usage: powershell -NoProfile -File scripts/build-street.ps1 [-Exe target\release\axiom.exe] [-NoExport]
param(
    [string]$Exe = "target\debug\axiom.exe",
    # A database path that does not exist yet. Default: a new per-run folder under
    # target\street-build. An existing database or SQLite sidecar is refused, never deleted.
    [string]$Db,
    [switch]$NoExport
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
if (-not (Test-Path $Exe)) { throw "missing $Exe (build it first: cargo build -p axiom)" }
$fresh = -not $Db
if ($fresh) { $Db = "target\street-build\run-{0}-{1}\street.sqlite" -f (Get-Date -Format 'yyyyMMdd-HHmmss-fff'), $PID }
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
    [IO.File]::WriteAllText((Join-Path $root "content\street\manifest.json"), $manifest)
    Write-Host "exported content\street\world.json + manifest.json"
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
