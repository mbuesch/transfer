#!/bin/sh
# Regenerate all icon assets from assets/icon.svg.
# Requires: rsvg-convert (librsvg), convert (ImageMagick)
set -e

basedir="$(realpath "$0" | xargs dirname)"
cd "$basedir/.."

# Desktop icon: 512x512 PNG embedded by main.rs
rsvg-convert -w 512 -h 512 assets/icon.svg -o assets/icon.png

# Android mipmap WebP (legacy raster icons)
for size_dir in "48:mipmap-mdpi" "72:mipmap-hdpi" "96:mipmap-xhdpi" "144:mipmap-xxhdpi" "192:mipmap-xxxhdpi"; do
    size="${size_dir%%:*}"
    dir="${size_dir##*:}"
    rsvg-convert -w "$size" -h "$size" assets/icon.svg -o /tmp/ic_launcher_${size}.png
    convert /tmp/ic_launcher_${size}.png -define webp:lossless=true android/res/${dir}/ic_launcher.webp
done
