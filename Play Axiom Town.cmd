@echo off
cd /d "%~dp0"
if not exist "target\debug\axiom.exe" (
    powershell.exe -NoProfile -Command ". .\scripts\env.ps1; cargo build -p axiom; exit $LASTEXITCODE"
    if errorlevel 1 exit /b 1
)
if exist "data\axiom-town.sqlite" (
    "target\debug\axiom.exe" --db data\axiom-town.sqlite play --start
) else (
    "target\debug\axiom.exe" --db data\axiom-town.sqlite --pack content\axiom-town play --start
)
if errorlevel 1 pause
