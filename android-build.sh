#!/bin/sh
set -e

basedir="$(realpath "$0" | xargs dirname)"
cd "$basedir"

export CFLAGS= CXXFLAGS= CPPFLAGS= LDFLAGS= RUSTFLAGS=

dx build --android --target aarch64-linux-android --release

ANDROID_APP="target/dx/transfer/release/android/app/app"
ANDROID_RES="$ANDROID_APP/src/main/res"

# The release build has isMinifyEnabled = true, so R8 would rename the
# @JvmStatic methods called by name from Rust via JNI (pickFile, pickFolder,
# getSharedFiles, clearSharedFiles), silently breaking the file pickers.
# Inject our keep rules before invoking Gradle.
cp android/proguard-jni.pro "$ANDROID_APP/proguard-jni.pro"

# dx hardcodes default launcher icons into the Android project and doesn't
# honour [bundle] icon or [android] icon for Android builds.  Work around
# this by overwriting the generated resources and re-running gradle.
cp android/res/drawable/ic_launcher_background.xml         "$ANDROID_RES/drawable/"
cp android/res/drawable-v24/ic_launcher_foreground.xml     "$ANDROID_RES/drawable-v24/"
cp android/res/mipmap-anydpi-v26/ic_launcher.xml           "$ANDROID_RES/mipmap-anydpi-v26/"
cp android/res/mipmap-mdpi/ic_launcher.webp                "$ANDROID_RES/mipmap-mdpi/"
cp android/res/mipmap-hdpi/ic_launcher.webp                "$ANDROID_RES/mipmap-hdpi/"
cp android/res/mipmap-xhdpi/ic_launcher.webp               "$ANDROID_RES/mipmap-xhdpi/"
cp android/res/mipmap-xxhdpi/ic_launcher.webp              "$ANDROID_RES/mipmap-xxhdpi/"
cp android/res/mipmap-xxxhdpi/ic_launcher.webp             "$ANDROID_RES/mipmap-xxxhdpi/"

# Rebuild the release APK with the updated icons.
(
    cd target/dx/transfer/release/android/app
    ./gradlew packageRelease
)

cp ./target/dx/transfer/release/android/app/app/build/outputs/apk/release/app-release-unsigned.apk \
   ./transfer-aarch64-unsigned.apk
