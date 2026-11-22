@echo off
rem Removes Choppy: Ctrl+V and the right-click menu go back to Windows' own copy (removes only what build.ps1 added).
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0build.ps1" -NoBuild -Shell Revert
pause
