#!/bin/sh

srcdir="$(realpath "$0" | xargs dirname)"
srcdir="$srcdir/.."

# Import the makerelease.lib
# https://bues.ch/cgit/misc.git/tree/makerelease.lib
die() { echo "$*"; exit 1; }
for path in $(echo "$PATH" | tr ':' ' '); do
    [ -f "$MAKERELEASE_LIB" ] && break
    MAKERELEASE_LIB="$path/makerelease.lib"
done
[ -f "$MAKERELEASE_LIB" ] && . "$MAKERELEASE_LIB" || die "makerelease.lib not found."

hook_get_version()
{
    version="$(cargo_local_pkg_version transfer)"
}

hook_regression_tests()
{
    true
}

hook_testbuild()
{
    true
}

hook_post_archives()
{
    local archive_dir="$1"
    local checkout_dir="$2"

    cd "$checkout_dir"

    ./android-build.sh
    local android="transfer-android-aarch64-$version"
    mkdir "./$android"
    cp "./transfer-release-aarch64-signed.apk" "./$android/"
    cp "./android-install.sh" "./$android/"
    tar cJf "./$android.tar.xz" \
        --numeric-owner --owner=0 --group=0 \
        --mtime='1970-01-01 00:00Z' \
        --sort=name \
        "./$android"
    cp "./$android.tar.xz" "$archive_dir/"

    ./desktop-build.sh
    local desktop="transfer-desktop-x64-$version"
    mkdir "./$desktop"
    cp "./transfer-desktop-linux-x64" "./$desktop/"
    tar cJf "./$desktop.tar.xz" \
        --numeric-owner --owner=0 --group=0 \
        --mtime='1970-01-01 00:00Z' \
        --sort=name \
        "./$desktop"
    cp "./$desktop.tar.xz" "$archive_dir/"
}

project=transfer
makerelease "$@"
