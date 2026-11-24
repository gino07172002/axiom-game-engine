$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $root
$tools = Join-Path $root '.tools'
New-Item -ItemType Directory -Force -Path $tools, (Join-Path $tools 'tmp') | Out-Null
$env:TEMP = Join-Path $tools 'tmp'
$env:TMP = $env:TEMP
$env:CARGO_HOME = Join-Path $tools 'cargo'
$env:RUSTUP_HOME = Join-Path $tools 'rustup'

# Pinned downloads and their trusted SHA-256. A file is used (extracted or executed) only after
# its hash matches; to move to a new version, change the URL and the hash together.
# - winlibs: SHA-256 published for the release asset (GitHub release digest).
# - rustup-init 1.29.3 (x86_64-pc-windows-gnu): the .sha256 published next to it on
#   static.rust-lang.org/rustup/archive/1.29.3/. rustup then verifies each toolchain component.
$winlibsUrl = 'https://github.com/brechtsanders/winlibs_mingw/releases/download/16.2.0posix-14.0.0-msvcrt-r1/winlibs-x86_64-posix-seh-gcc-16.2.0-mingw-w64msvcrt-14.0.0-r1.7z'
$winlibsSha256 = '983f680245a3200f9a3b1ced23ed56581d752b1b7cbcc6a0cace924edf9bca0d'
$rustupUrl = 'https://static.rust-lang.org/rustup/archive/1.29.3/x86_64-pc-windows-gnu/rustup-init.exe'
$rustupSha256 = 'f6289656ce42e1099f3cbc6f10ef6321dfde54773faf599dd36fd3b1f07d9103'
# Cache names carry the version and target, so an older unpinned download kept under the old
# names (winlibs.7z, rustup-init.exe) is neither reused nor deleted: the pinned file is fetched
# beside it. Change these names together with the URL and hash.
$winlibsCache = 'winlibs-16.2.0posix-14.0.0-msvcrt-r1.7z'
$rustupCache = 'rustup-init-1.29.3-x86_64-pc-windows-gnu.exe'

# Downloads $Url to $Path through a temporary file and keeps it only when its SHA-256 equals
# $Sha256; otherwise deletes the temporary file and stops with the file name and both hashes.
# An existing $Path (an earlier download) is checked the same way before it is reused.
function Get-VerifiedDownload([string]$Url, [string]$Path, [string]$Sha256) {
    if (Test-Path -LiteralPath $Path) {
        $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
        if ($actual -eq $Sha256) { return }
        throw "existing $Path has SHA-256 $actual, expected $Sha256. Delete it yourself if it is a stale download, then rerun."
    }
    $partial = "$Path.partial"
    if (Test-Path -LiteralPath $partial) { Remove-Item -LiteralPath $partial }
    & curl.exe --fail --location --output $partial $Url
    if ($LASTEXITCODE -ne 0) {
        if (Test-Path -LiteralPath $partial) { Remove-Item -LiteralPath $partial }
        throw "download failed: $Url"
    }
    $actual = (Get-FileHash -LiteralPath $partial -Algorithm SHA256).Hash
    if ($actual -ne $Sha256) {
        Remove-Item -LiteralPath $partial
        throw "SHA-256 mismatch for $(Split-Path -Leaf $Path) from ${Url}: expected $Sha256, got $actual. Nothing was extracted or run."
    }
    Move-Item -LiteralPath $partial -Destination $Path
}

if (-not (Test-Path (Join-Path $tools 'mingw64/bin/gcc.exe'))) {
    Write-Host 'Downloading portable GNU build tools to this workspace...'
    $archive = Join-Path $tools $winlibsCache
    Get-VerifiedDownload $winlibsUrl $archive $winlibsSha256
    & tar.exe -xf $archive -C $tools
    if ($LASTEXITCODE -ne 0) { throw 'GNU toolchain extraction failed' }
}
if (-not (Test-Path (Join-Path $tools 'cargo/bin/rustup.exe'))) {
    $installer = Join-Path $tools $rustupCache
    Get-VerifiedDownload $rustupUrl $installer $rustupSha256
    & $installer -y --profile minimal --default-host x86_64-pc-windows-gnu --default-toolchain stable --no-modify-path
    if ($LASTEXITCODE -ne 0) { throw 'Rust installation failed' }
}
. "$PSScriptRoot/env.ps1"
& cargo build --workspace
if ($LASTEXITCODE -ne 0) { throw 'Build failed' }
Write-Host 'Ready. Run scripts/play.ps1 or target/debug/axiom.exe play.'
