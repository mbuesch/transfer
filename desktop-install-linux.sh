#!/bin/sh
# -*- coding: utf-8 -*-

basedir="$(realpath "$0" | xargs dirname)"

. "$basedir/scripts/lib.sh"

install_entry_checks()
{
    [ -f "$bin" ] || die "transfer is not built! Run ./desktop-build-linux.sh"
    [ "$(id -u)" = "0" ] || die "Must be root to install transfer."
}

install_dirs()
{
    do_install \
        -o root -g root -m 0755 \
        -d /opt/transfer/bin
}

install_transfer()
{
    do_install \
        -o root -g root -m 0755 \
        "$bin" \
        /opt/transfer/bin/transfer
}

bin="$basedir/transfer-desktop-linux-x64"

install_entry_checks
install_dirs
install_transfer
