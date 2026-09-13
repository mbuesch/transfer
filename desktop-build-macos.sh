#!/bin/sh
set -e

basedir="$(dirname "$(realpath "$0")")"
cd "$basedir"

dx build --desktop --release

cp -R ./target/dx/transfer/release/macos/Transfer.app \
   ./Transfer.app
