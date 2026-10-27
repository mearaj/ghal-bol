@echo off
REM Ghal Bol - uninstall. Leaves the install folder first and runs on one line, because
REM uninstall.ps1 deletes this folder (and this file) while it runs.
cd /d "%TEMP%" & powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0uninstall.ps1" & pause
