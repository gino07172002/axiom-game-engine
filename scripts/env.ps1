$env:CARGO_HOME = Join-Path $PSScriptRoot '..\.tools\cargo'
$env:RUSTUP_HOME = Join-Path $PSScriptRoot '..\.tools\rustup'
$env:TEMP = Join-Path $PSScriptRoot '..\.tools\tmp'
$env:TMP = $env:TEMP
New-Item -ItemType Directory -Force -Path $env:TEMP | Out-Null
$axiomLinker = Join-Path $PSScriptRoot '..\.tools\mingw64\bin'
if (-not (Test-Path (Join-Path $axiomLinker 'gcc.exe'))) { $axiomLinker = 'D:\w64devkit\bin' }
$env:PATH = "$env:CARGO_HOME\bin;$axiomLinker;$env:PATH"

