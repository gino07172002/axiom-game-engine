@echo off
cd /d "%~dp0"
rem Effects Atlas is GPU/CPU heavy: use the optimized build (incremental, fast when up to date).
powershell.exe -NoProfile -Command ". .\scripts\env.ps1; Write-Host 'Preparing optimized build (first time can take several minutes)...'; cargo build --release -p axiom; exit $LASTEXITCODE"
if errorlevel 1 (
    pause
    exit /b 1
)
if exist "data\effects-atlas.sqlite" (
    "target\release\axiom.exe" --db data\effects-atlas.sqlite play --start
) else (
    "target\release\axiom.exe" --db data\effects-atlas.sqlite --pack content\effects-tour play --start
)
if errorlevel 1 pause