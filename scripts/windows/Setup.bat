@echo off
REM Ghal Bol - per-user install (no admin rights). Double-click to install.
setlocal
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1"
pause
