@echo off
rem Installs Choppy for you and makes it the default file manager (Ctrl+V of files in Explorer uses Choppy).
rem Builds first if needed. Undo with "Revert to Windows Copy.cmd".
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0build.ps1" -Shell Apply
pause
