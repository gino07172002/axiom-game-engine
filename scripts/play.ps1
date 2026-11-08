param([switch]$Release)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
. "$PSScriptRoot\env.ps1"
if ($Release) { cargo run --release -p axiom -- play } else { cargo run -p axiom -- play }
exit $LASTEXITCODE
