@echo off
setlocal enabledelayedexpansion

set "BASEDIR=%~dp0"
cd /d "%BASEDIR%"

dx build --desktop --release
if errorlevel 1 (
  echo Failed to build desktop release.
  exit /b 1
)

copy target\dx\transfer\release\windows\app\transfer.exe ^
     "%BASEDIR%transfer-desktop-windows-x64.exe" /Y
if errorlevel 1 (
  echo Failed to copy built binary.
  exit /b 1
)

echo Build and copy completed SUCCESSFULLY.
pause
