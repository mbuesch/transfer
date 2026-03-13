#!/bin/sh
set -e

basedir="$(realpath "$0" | xargs dirname)"
cd "$basedir"

adb uninstall ch.bues.Transfer 2>/dev/null || true
adb install ./transfer-release-aarch64-signed.apk
