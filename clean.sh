#!/bin/sh

basedir="$(realpath "$0" | xargs dirname)"
cd "$basedir"

cargo clean
rm -f transfer-desktop-linux-x64
rm -f transfer-release-aarch64-signed.apk
