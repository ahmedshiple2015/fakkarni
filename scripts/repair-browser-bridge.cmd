@echo off
setlocal
title Fakkarni browser bridge setup
echo Install the Fakkarni bridge for this Windows desktop session.
echo This does not read or change your vault, passwords, or browser profile.
echo.
echo Run this file from File Explorer as your normal Windows user.
echo Do not choose Run as administrator or launch it from an isolated terminal.
echo.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0register-brave.ps1"
if errorlevel 1 goto failed
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0register-edge.ps1"
if errorlevel 1 goto failed
if not exist "%~dp0..\artifacts\bridge-diag" mkdir "%~dp0..\artifacts\bridge-diag"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0test-browser-bridge.ps1" > "%~dp0..\artifacts\bridge-diag\desktop-install.json"
if errorlevel 1 goto failed
echo.
echo Installation finished. Fully exit Brave and Edge, including background tasks.
echo Reopen them from your usual shortcuts and refresh the Fakkarni extension.
echo Closing browser windows alone can leave background processes running.
echo.
pause
exit /b 0
:failed
echo.
echo Installation failed. Keep the error above for troubleshooting.
echo If a bridge executable is in use, fully exit both browsers and retry.
echo.
pause
exit /b 1
