# Packaging smoke test (docs/plan-packaging.zh-TW.md, P5).
# For each pack: `axiom package` -> copy the folder elsewhere -> run the windowed player from the
# copy with an isolated APPDATA -> check exit code, screenshot, APPDATA database, that the game
# folder stays unchanged and that nothing in the player's database points back at the repo.
# Negative cases: an external animation asset and --force on a non-game folder are refused and
# leave nothing behind. Never touches data/ or the real %APPDATA%.
param(
    [switch]$Release,
    [string[]]$Packs = @('plaza', 'adv', 'fps', 'action'),
    [int]$Frames = 120,
    [switch]$Keep
)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot\env.ps1"
$repo = (Get-Location).Path
$build = if ($Release) { 'release' } else { 'debug' }
$cli = Join-Path $repo "target\$build\axiom.exe"
$player = Join-Path $repo "target\$build\axiom-player.exe"
foreach ($exe in $cli, $player) {
    if (-not (Test-Path $exe)) {
        $flag = if ($Release) { '--release ' } else { '' }
        throw "$exe missing; build first: cargo build ${flag}-p axiom -p axiom-player"
    }
}
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$root = (New-Item -ItemType Directory -Force (Join-Path $repo "artifacts\package-smoke-$stamp")).FullName

function Invoke-Package([string]$pack, [string]$out, [string[]]$extra = @()) {
    # stderr carries warnings (e.g. output inside the repo) and recoverable errors as JSON.
    $ErrorActionPreference = 'Continue'
    $lines = & $cli --pack $pack package --out $out @extra 2>&1 | ForEach-Object { "$_" }
    $json = $lines | Where-Object { $_.StartsWith('{') } | Select-Object -Last 1
    if (-not $json) { throw "no JSON from package ${pack}: $($lines -join ' ')" }
    $json | ConvertFrom-Json
}

$results = @()
foreach ($name in $Packs) {
    $out = Join-Path $root "dist\$name"
    $report = Invoke-Package "content\$name" $out
    if (-not $report.ok) { throw "package $name failed: $($report.error)" }
    $moved = Join-Path $root "moved\$name copy"
    New-Item -ItemType Directory -Force (Split-Path $moved) | Out-Null
    Copy-Item -Recurse $out $moved
    $before = (Get-ChildItem -Recurse -File $moved | ForEach-Object { $_.FullName } | Sort-Object) -join '|'
    $appdata = Join-Path $root "appdata\$name"
    New-Item -ItemType Directory -Force $appdata | Out-Null
    $shot = Join-Path $root "$name.png"
    $saved = $env:APPDATA
    $env:APPDATA = $appdata
    $env:AXIOM_PLAYER_NO_DIALOG = '1'
    try {
        $watch = [Diagnostics.Stopwatch]::StartNew()
        $proc = Start-Process -FilePath (Join-Path $moved $report.exe) -WorkingDirectory $root -Wait -PassThru `
            -ArgumentList @('--smoke-frames', "$Frames", '--screenshot', "`"$shot`"") `
            -RedirectStandardOutput (Join-Path $root "$name.out.json") -RedirectStandardError (Join-Path $root "$name.err.txt")
        $watch.Stop()
    } finally {
        $env:APPDATA = $saved
        Remove-Item Env:AXIOM_PLAYER_NO_DIALOG -ErrorAction SilentlyContinue
    }
    if ($proc.ExitCode -ne 0) { throw "player $name exited $($proc.ExitCode): $(Get-Content -Raw (Join-Path $root "$name.err.txt"))" }
    if (-not (Test-Path $shot)) { throw "player $name wrote no screenshot" }
    $db = Join-Path $appdata "Axiom\$($report.id)\world.sqlite"
    if (-not (Test-Path $db)) { throw "player $name did not create $db" }
    $after = (Get-ChildItem -Recurse -File $moved | ForEach-Object { $_.FullName } | Sort-Object) -join '|'
    if ($after -ne $before) { throw "player $name wrote into its game folder" }
    # Asset paths are resolved at import: they must point into the moved copy, never the repo.
    $bytes = [IO.File]::ReadAllText($db, [Text.Encoding]::UTF8)
    foreach ($wal in "$db-wal") { if (Test-Path $wal) { $bytes += [IO.File]::ReadAllText($wal, [Text.Encoding]::UTF8) } }
    $source = (Join-Path $repo 'content').Replace('\', '\\')
    if ($bytes.Contains($source) -or $bytes.Contains((Join-Path $repo 'content'))) { throw "player $name database references the repo content" }
    $results += [ordered]@{
        pack = $name; id = $report.id; exe = $report.exe; files = $report.files.Count; bytes = $report.bytes
        package_ms = $report.ms; unreferenced = @($report.unreferenced); player_ms = $watch.ElapsedMilliseconds
        screenshot = $shot
    }
}

# Negative 1: an external animation asset is refused before anything is written.
$ext = Join-Path $root 'external'
Copy-Item -Recurse 'content\animation' (Join-Path $ext 'pack')
New-Item -ItemType Directory -Force (Join-Path $ext 'outside') | Out-Null
Move-Item (Join-Path $ext 'pack\rigify.glb') (Join-Path $ext 'outside\rigify.glb')
$worldPath = Join-Path $ext 'pack\world.json'
$world = Get-Content -Raw -Encoding UTF8 $worldPath | ConvertFrom-Json
$asset = $world.resources.'animation.library'.assets.character
$asset | Add-Member -Force -NotePropertyName external -NotePropertyValue $true
$asset.path = (Join-Path $ext 'outside\rigify.glb')
[IO.File]::WriteAllText($worldPath, ($world | ConvertTo-Json -Depth 100 -Compress), (New-Object Text.UTF8Encoding $false))
$refused = Invoke-Package (Join-Path $ext 'pack') (Join-Path $ext 'out\Game')
if ($refused.ok -or "$($refused.error)" -notlike '*external animation asset*') { throw "external asset was not refused: $($refused | ConvertTo-Json -Compress)" }
if ((Test-Path (Join-Path $ext 'out')) ) { throw 'refused external package left output behind' }

# Negative 2: --force never replaces a folder that is not a packaged game.
$foreign = Join-Path $root 'foreign'
New-Item -ItemType Directory -Force $foreign | Out-Null
Set-Content -Path (Join-Path $foreign 'keep.txt') -Value 'keep'
$denied = Invoke-Package 'content\adv' $foreign @('--force')
if ($denied.ok -or -not (Test-Path (Join-Path $foreign 'keep.txt'))) { throw '--force replaced a non-game folder' }

$summary = [ordered]@{ok = $true; build = $build; packs = $results; external_refused = $refused.error; force_guard = $denied.error; root = $root }
$summary | ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 (Join-Path $root 'report.json')
if (-not $Keep) {
    # Keep only the screenshots and the report; the copies hold 100 MB executables each.
    foreach ($dir in 'dist', 'moved', 'appdata', 'external', 'foreign') { Remove-Item -Recurse -Force (Join-Path $root $dir) }
}
$summary | ConvertTo-Json -Depth 6 -Compress
$global:LASTEXITCODE = 0
Write-Host "PASS package smoke ($build): $($Packs -join ', ') packaged, relocated and played; external asset and --force guard refused. Report: $root\report.json"
exit 0
