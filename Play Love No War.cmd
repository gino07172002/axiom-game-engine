@echo off
cd /d "%~dp0"
if not exist "target\debug\axiom.exe" (
    powershell.exe -NoProfile -Command ". .\scripts\env.ps1; cargo build -p axiom; exit $LASTEXITCODE"
    if errorlevel 1 exit /b 1
)
if exist "data\love-no-war.sqlite" (
    "target\debug\axiom.exe" --db data\love-no-war.sqlite play --start
) else (
    "target\debug\axiom.exe" --db data\love-no-war.sqlite --pack content\love-no-war play --start
)
if errorlevel 1 pause
