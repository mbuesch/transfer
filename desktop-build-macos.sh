#!/bin/sh
set -e

basedir="$(cd "$(dirname "$0")" && pwd)"
cd "$basedir"

dx build --desktop --release

cp ./target/dx/transfer/release/macos/app/transfer \
   ./transfer-desktop-macos-aarch64
