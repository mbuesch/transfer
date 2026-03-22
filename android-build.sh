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

    # Sign the APK.  A self-signed debug key is sufficient for sideloading;
    # generate one once at ~/.android/transfer-debug.keystore if it doesn't exist.
    KEYSTORE="$HOME/.android/transfer-debug.keystore"
    if [ ! -f "$KEYSTORE" ]; then
        mkdir -p "$HOME/.android"
        keytool -genkeypair -v \
            -keystore "$KEYSTORE" \
            -alias transfer-debug \
            -keyalg RSA -keysize 2048 -validity 10000 \
            -storepass android -keypass android \
            -dname "CN=Transfer Debug, O=Transfer, C=US"
    fi
    apksigner sign \
        --ks "$KEYSTORE" \
        --ks-key-alias transfer-debug \
        --ks-pass pass:android \
        --key-pass pass:android \
        --out app/build/outputs/apk/release/app-release-signed.apk \
        app/build/outputs/apk/release/app-release-unsigned.apk
)

cp ./target/dx/transfer/release/android/app/app/build/outputs/apk/release/app-release-signed.apk \
   ./transfer-release-aarch64-signed.apk
