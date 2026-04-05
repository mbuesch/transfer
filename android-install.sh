#!/bin/sh
set -e

basedir="$(realpath "$0" | xargs dirname)"
cd "$basedir"

if [ -z "$APK_SIGNED" ]; then APK_SIGNED="transfer-aarch64.apk"; fi

adb uninstall ch.bues.Transfer 2>/dev/null || true
adb install "$APK_SIGNED"
