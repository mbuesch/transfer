#!/bin/sh
set -e

basedir="$(realpath "$0" | xargs dirname)"
cd "$basedir"

dx run --desktop --release
