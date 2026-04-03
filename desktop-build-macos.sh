#!/bin/sh
set -e

basedir="$(cd "$(dirname "$0")" && pwd)"
cd "$basedir"

dx build --desktop --release

cp -R ./target/dx/transfer/release/macos/Transfer.app \
   ./Transfer.app
