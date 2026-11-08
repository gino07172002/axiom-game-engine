@echo off
cd /d "%~dp0"
if not exist "target\debug\axiom.exe" (
    powershell.exe -NoProfile -Command ". .\scripts\env.ps1; cargo build --workspace; exit $LASTEXITCODE"
    if errorlevel 1 exit /b 1
)
if exist "data\frontier.sqlite" (
    "target\debug\axiom.exe" --db data\frontier.sqlite play --start
) else (
    "target\debug\axiom.exe" --db data\frontier.sqlite --pack content\rts play --start
)
if errorlevel 1 pause
