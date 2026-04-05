@echo off
setlocal enabledelayedexpansion

set "BASEDIR=%~dp0"
cd /d "%BASEDIR%"

cargo clean
del /F /Q "%BASEDIR%transfer-desktop-linux-x64" 2>nul
del /F /Q "%BASEDIR%transfer-desktop-windows-x64.exe" 2>nul
del /F /Q "%BASEDIR%transfer-aarch64-unsigned.apk" 2>nul
del /F /Q "%BASEDIR%transfer-aarch64.apk" 2>nul
del /F /Q "%BASEDIR%transfer-aarch64.apk.idsig" 2>nul
del /F /Q "%BASEDIR%transfer-aarch64-release.apk" 2>nul
del /F /Q "%BASEDIR%transfer-aarch64-release.apk.idsig" 2>nul
del /F /Q "%BASEDIR%transfer-aarch64-unsigned.aab" 2>nul
del /F /Q "%BASEDIR%transfer-aarch64.aab" 2>nul
del /F /Q "%BASEDIR%transfer-aarch64-release.aab" 2>nul
rem del /F /Q "%BASEDIR%debug.jks" 2>nul
