#!/bin/sh
set -e

basedir="$(realpath "$0" | xargs dirname)"
cd "$basedir"

dx build --desktop --release

cp ./target/dx/transfer/release/linux/app/transfer \
   ./transfer-desktop-linux-x64
