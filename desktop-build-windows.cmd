@echo off
SETLOCAL ENABLEDELAYEDEXPANSION

set "BASEDIR=%~dp0"
cd /d "%BASEDIR%"

dx build --desktop --release
if ERRORLEVEL 1 (
  echo Failed to build desktop release.
  exit /b 1
)

copy target\dx\transfer\release\windows\app\transfer.exe ^
     "%BASEDIR%transfer-desktop-windows-x64.exe" /Y
if ERRORLEVEL 1 (
  echo Failed to copy built binary.
  exit /b 1
)

echo Build and copy completed SUCCESSFULLY.
if %GITHUB_ACTIONS% == "" pause
exit /b 0
