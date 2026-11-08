@echo off
cd /d "%~dp0"
if not exist "target\debug\axiom.exe" (
    powershell.exe -NoProfile -Command ". .\scripts\env.ps1; cargo build -p axiom; exit $LASTEXITCODE"
    if errorlevel 1 exit /b 1
)
if exist "data\light-lab.sqlite" (
    "target\debug\axiom.exe" --db data\light-lab.sqlite play --start
) else (
    "target\debug\axiom.exe" --db data\light-lab.sqlite --pack content\light-lab play --start
)
if errorlevel 1 pause
