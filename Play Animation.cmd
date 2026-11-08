@echo off
cd /d "%~dp0"
if not exist "target\debug\axiom.exe" (
    powershell.exe -NoProfile -Command ". .\scripts\env.ps1; cargo build --workspace; exit $LASTEXITCODE"
    if errorlevel 1 exit /b 1
)
if exist "data\animation.sqlite" (
    "target\debug\axiom.exe" --db data\animation.sqlite play --start
) else if exist "data\animation-reference\pack\world.json" (
    "target\debug\axiom.exe" --db data\animation.sqlite --pack data\animation-reference\pack play --start
) else (
    "target\debug\axiom.exe" --db data\animation.sqlite --pack content\animation play --start
)
if errorlevel 1 pause
