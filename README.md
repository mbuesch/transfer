# File Transfer App

![img](android/res/mipmap-hdpi/ic_launcher.webp)

A *minimal*, cross‑platform file transfer tool for **Android** and **Desktop** on the same local network.

## Key Features

- **Automatic peer discovery**: No manual IP entry or QR codes
- **Cross‑platform**: Currently: Linux desktop + Android
- Integrates into the **Android share menu / send menu** for easy file sending.
  The file transfer app will appear as an option when sharing files from other apps

Please note that currently **no encryption** is implemented by design - use only on trusted local networks.
If you want encryption, consider encrypting files before sending (e.g. encrypted Zip/7z archives).

The transferred file data and metadata is protected with a strong checksum to ensure integrity and detect corruption during network transfer.

## Why not use one of the many existing solutions?

Many existing solutions are either complex, require manual setup or are complicated to use.
This project aims to provide a simple, user-friendly alternative that works seamlessly across platforms without the need for manual configuration.

If you need features other than simple file transfer between two devices, there are many other great apps available that may suit your needs better. This project is focused on simplicity and ease of use for basic file transfers on local networks.

### Alternative solutions include:

- **KDE Connect**: File sharing, remote control, encryption, and much more.
- **SMB / Windows File Sharing**: Network file sharing protocol, but requires manual setup and is not user-friendly for non-technical users.
  And typically not usable with the Android share menu.
- **Bluetooth file transfer**: Built into most devices, but can be slow and unreliable for large files.

## Building and installing

### Install Rust

Get and install the latest stable Rust version from [https://rust-lang.org/](https://rust-lang.org/).

### Desktop (Linux/x86_64)

Build the application for desktop Linux:

```sh
./desktop-build.sh
```

The built application executable binary is `transfer-desktop-linux-x64`.
You can run it directly; typically by double-clicking it.
Or you can put it anywhere you like, e.g. in your home directory or in `/usr/local/bin` for system-wide access.

### Android

Before running the Android build script, ensure you have the Android NDK and SDK installed and properly configured.
The easiest way to get them is to install [Android Studio](https://developer.android.com/studio), which includes both.
For the build script to work, you need to set the some environment variables to point to your Android NDK and SDK installations.

```sh
# Set this to the path of your Android SDK installation.
export ANDROID_HOME="$HOME/Android/Sdk"

# Set this to the path of your Android NDK installation.
# Adjust the VERSION part to match the installed NDK version.
export ANDROID_NDK_HOME="$HOME/Android/Sdk/ndk/VERSION"

# Add Android SDK platform-tools to PATH for ADB access.
export PATH="$HOME/Android/Sdk/platform-tools/:$PATH"
```

Use the provided script to build the Android packages:

```sh
./android-build.sh
```

Install the generated APK on your Android device (via ADB).
Plug in your Android device, ensure Developer Mode, USB debugging and Sideloading are enabled, and run:

```sh
./android-install.sh
```

## Security Notes

- This project deliberately does **not** encrypt traffic.
- Only run on trusted, private networks (e.g., home LAN).
- Do **not** expose it directly to the internet.

## Future Plans

- It might be a good idea to add an option for a builtin simple **password based encryption** in the future. No complicated (albeit more secure) public key cryptography, just a simple password that both sender and receiver enter to encrypt/decrypt the file data.
- It shall be made possible to **transfer whole directories at once**, not just single files.
  This could be done by using an existing archive format and use that internally (always; also for single files).
- Make the app available in **Play Store**.
  I need your help for that.
  Please get in contact with me, if you are interested in becoming an tester for the app to get it registered in the Play Store.
- Provide a built **Android APK** for manual installation (sideloading).

## License

This application is AI generated with heavy manual modifications.

Licensed under the Apache License version 2.0 or alternatively feel free to do whatever you want with this software without restriction.
