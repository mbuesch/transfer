#!/bin/sh
set -e

basedir="$(dirname "$(realpath "$0")")"
cd "$basedir"

dx build --desktop --release

cp ./target/dx/transfer/release/linux/app/transfer \
   ./transfer-desktop-linux-x64
