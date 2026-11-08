$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
New-Item -ItemType Directory -Force -Path '.tools', '.tools/tmp' | Out-Null
$env:TEMP = (Resolve-Path '.tools/tmp').Path
$env:TMP = $env:TEMP
$env:CARGO_HOME = Join-Path $PWD '.tools/cargo'
$env:RUSTUP_HOME = Join-Path $PWD '.tools/rustup'

if (-not (Test-Path '.tools/mingw64/bin/gcc.exe')) {
    Write-Host 'Downloading portable GNU build tools to this workspace...'
    $axiomUrl = 'https://github.com/brechtsanders/winlibs_mingw/releases/download/16.2.0posix-14.0.0-msvcrt-r1/winlibs-x86_64-posix-seh-gcc-16.2.0-mingw-w64msvcrt-14.0.0-r1.7z'
    & curl.exe --fail --location --output '.tools/winlibs.7z' $axiomUrl
    if ($LASTEXITCODE -ne 0) { throw 'GNU toolchain download failed' }
    & tar.exe -xf '.tools/winlibs.7z' -C '.tools'
    if ($LASTEXITCODE -ne 0) { throw 'GNU toolchain extraction failed' }
}
if (-not (Test-Path '.tools/cargo/bin/rustup.exe')) {
    & curl.exe --fail --location --output '.tools/rustup-init.exe' 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-gnu/rustup-init.exe'
    if ($LASTEXITCODE -ne 0) { throw 'Rust installer download failed' }
    & ./.tools/rustup-init.exe -y --profile minimal --default-host x86_64-pc-windows-gnu --default-toolchain stable --no-modify-path
    if ($LASTEXITCODE -ne 0) { throw 'Rust installation failed' }
}
. "$PSScriptRoot/env.ps1"
& cargo build --workspace
if ($LASTEXITCODE -ne 0) { throw 'Build failed' }
Write-Host 'Ready. Run scripts/play.ps1 or target/debug/axiom.exe play.'
