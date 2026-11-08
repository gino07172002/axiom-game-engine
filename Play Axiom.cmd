@echo off
cd /d "%~dp0"
if exist "target\debug\axiom.exe" (
    "target\debug\axiom.exe" play
) else (
    powershell.exe -NoProfile -File "scripts\play.ps1"
)
if errorlevel 1 pause
