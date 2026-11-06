tch-gw-GUI-admin.bat
@echo off
setlocal

:: --- Relaunch this .bat as Administrator if it isn't already ---
net session >nul 2>&1
if %errorlevel% neq 0 (
    echo Requesting administrator rights...
    powershell -NoProfile -ExecutionPolicy Bypass -Command "Start-Process -FilePath '%~f0' -Verb RunAs"
    exit /b
)

:: Elevated processes start in System32, so go back to this script's folder
cd /d "%~dp0"

:: --- Pick a Python launcher ---
set "PYEXE="
where py >nul 2>&1 && set "PYEXE=py"
if not defined PYEXE (
    where python >nul 2>&1 && set "PYEXE=python"
)
if not defined PYEXE (
    echo Python was not found on PATH. Install Python or add it to PATH.
    pause
    exit /b 1
)

:: --- Run the patcher ---
%PYEXE% "%~dp0patch-gw-GUI.py"

:: Keep the window open if something went wrong so you can read the error
if %errorlevel% neq 0 (
    echo.
    echo patch-gw-GUI.py exited with error code %errorlevel%.
    pause
)

endlocal
 