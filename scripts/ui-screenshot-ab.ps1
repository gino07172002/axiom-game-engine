param(
    [Parameter(Mandatory = $true)][string]$Old,
    [string]$New = 'target/debug/axiom.exe',
    [string[]]$Packs = @('adv', 'animation', 'animator-lab', 'axiom-town', 'character-lab', 'duet-lab', 'rts-bench', 'rts', 'street', 'style-lab'),
    [int]$Frames = 120
)
# UI regression A/B (docs/plan-ui-styling.zh-TW.md, R5): every pack with a ui.document is
# captured with an old and a new executable, each on its own fresh database, and the PNGs
# are compared byte for byte. Same GPU, same window size, same frame count; the first old run
# is repeated to show the capture itself is stable. Never touches data\.
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
$Old = (Resolve-Path $Old).Path
$New = (Resolve-Path $New).Path
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$dir = Join-Path $PWD "artifacts/ui-ab-$stamp"
New-Item -ItemType Directory -Force $dir | Out-Null
function Capture([string]$Exe, [string]$Pack, [string]$Tag) {
    $db = Join-Path $dir "$Pack-$Tag.sqlite"
    $png = Join-Path $dir "$Pack-$Tag.png"
    $raw = & $Exe --db $db --pack "content/$Pack" play --start --port 0 --smoke-frames $Frames --screenshot $png 2>$null
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path $png)) { throw "$Tag capture of $Pack failed" }
    (Get-FileHash -LiteralPath $png).Hash
}
$rows = foreach ($pack in $Packs) {
    $a = Capture $Old $pack 'old'
    $a2 = if ($pack -eq $Packs[0]) { Capture $Old $pack 'old2' } else { $a }
    $b = Capture $New $pack 'new'
    [pscustomobject]@{ pack = $pack; stable = ($a -eq $a2); identical = ($a -eq $b) }
}
$rows | Format-Table -AutoSize | Out-String | Write-Host
$report = @{ old = $Old; new = $New; frames = $Frames; packs = $rows } | ConvertTo-Json -Depth 4
$report | Set-Content (Join-Path $dir 'report.json') -Encoding utf8
if ($rows | Where-Object { -not $_.stable }) { throw "the old capture is not stable; the A/B is inconclusive ($dir)" }
$changed = @($rows | Where-Object { -not $_.identical } | ForEach-Object { $_.pack })
Write-Host ("changed: " + ($(if ($changed.Count) { $changed -join ', ' } else { 'none' })) + " (report: $dir)")
