#!/bin/sh

basedir="$(realpath "$0" | xargs dirname)"
cd "$basedir"

cargo clean
rm -f transfer-desktop-linux-x64
rm -f transfer-desktop-windows-x64.exe
rm -f transfer-aarch64-unsigned.apk
rm -f transfer-aarch64.apk
rm -f transfer-aarch64.apk.idsig
#rm -f debug.jks
