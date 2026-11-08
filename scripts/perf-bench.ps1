# Headless performance baseline: one fresh process per pack and repeat (see docs/perf-baseline.zh-TW.md).
#   powershell -File scripts/perf-bench.ps1 [-Profile release] [-Repeats 3] [-Exe path\perf_bench.exe] [-Out file.jsonl]
# Build first: cargo build --release -p axiom-runtime --example perf_bench -j1
# Repeats are interleaved across packs (pack A, B, ..., then A, B, ... again) so machine
# noise spreads over all packs. Each line of the output file is one perf_bench JSON result.
param(
    [ValidateSet('debug', 'release')][string]$Profile = 'release',
    [int]$Repeats = 3,
    [string]$Exe = '',
    [string[]]$Packs = @('content/fps', 'content/rts-bench', 'content/rts', 'content/axiom-town', 'content/effects-tour', 'content/light-lab', 'generated:4096'),
    [int]$Ticks = 1200,
    [int]$Edits = 15,
    [string]$Out = ''
)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot/env.ps1"
if (-not $Exe) { $Exe = "target/$Profile/examples/perf_bench.exe" }
$exePath = (Resolve-Path $Exe).Path
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
if (-not $Out) { $Out = "artifacts/perf-scratch/bench-$Profile-$stamp.jsonl" }
$scratch = "artifacts/perf-scratch/db-$stamp"
New-Item -ItemType Directory -Force $scratch | Out-Null
$lines = [Collections.Generic.List[string]]::new()
for ($r = 0; $r -lt $Repeats; $r++) {
    foreach ($pack in $Packs) {
        $target = if ($pack -like 'generated:*') { @('--generated', $pack.Split(':')[1]) } else { @('--pack', $pack) }
        $raw = & $exePath @target --ticks $Ticks --edits $Edits --scratch $scratch
        if ($LASTEXITCODE -ne 0) { throw "perf_bench failed for $pack" }
        $lines.Add(($raw | Select-Object -Last 1))
        Write-Host ("repeat {0} {1} done" -f $r, $pack)
    }
}
[IO.File]::WriteAllLines((Join-Path $PWD $Out), $lines)
Remove-Item -Recurse -Force $scratch
Write-Output $Out
