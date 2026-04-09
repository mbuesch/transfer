# File Transfer App

![img](android/res/mipmap-hdpi/ic_launcher.webp)

A *minimal* and fast cross‑platform file transfer tool for **Android** and **Desktop** on the same local network.

## Key Features

- **Automatic peer discovery**: No manual IP entry or QR codes
- **Cross‑platform**: Android, Windows desktop, Linux desktop, MacOS desktop
- Integrates into the **Android share menu / send menu** for easy file sending.
  The file transfer app will appear as an option when sharing files from other apps
- **Integrity protection**: The transferred file data and metadata is protected with a strong checksum to ensure integrity and detect corruption during network transfer.
- **Encryption**: File transfer traffic is always automatically encrypted.
  This happens transparently in the background without any user interaction or configuration needed.
  The user can *optionally* set a password for an additional layer of security, which is mixed into the encryption key.

Transfer over the Internet is not supported and will never be supported, as this app is designed for local network file transfers only.

## Why not use one of the many other solutions?

Many other solutions are either very complex, require manual setup, are slow, or are complicated to use.
This project aims to provide a simple, fast, user-friendly alternative that works seamlessly across platforms without the need for manual configuration.

If you need features other than simple file transfer between two devices, there are many other great apps available that may suit your needs better. This project is focused on simplicity and ease of use for basic file transfers on local networks.

### Alternative solutions include:

- **KDE Connect**: File sharing, remote control, and much more.
- **SMB / Windows File Sharing**: Network file sharing protocol, but requires manual setup and is not user-friendly for non-technical users.
  And typically not usable with the Android share menu.
- **Bluetooth file transfer**: Built into most devices, but can be slow and unreliable for large files.

## Building and installing

### Install Rust

Get and install the latest stable Rust version from [https://rust-lang.org/](https://rust-lang.org/).

### Desktop (Linux)

Build the application for desktop Linux:

```sh
./desktop-build.sh
```

The built application executable binary is `transfer-desktop-linux-x64`.
You can run it directly; typically by double-clicking it.
Or you can put it anywhere you like, e.g. in your home directory or in `/usr/local/bin` for system-wide access.

### Desktop (Windows)

Build the application for desktop Windows:

Double click on `desktop-build.cmd` to run the build script.
The built application executable binary is `transfer-desktop-windows-x64.exe`.
You can run it directly; typically by double-clicking it.

### Desktop (MacOS)

Build the application for desktop MacOS:

Double click on `desktop-build-macos.sh` to run the build script.
The built application executable binary is `Transfer.app`.
You can run it directly; typically by double-clicking it.

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

## Firewall / Ports

If you have a firewall or router that restricts incoming and/or outgoing connections on your network, you may need to allow the application to communicate through the firewall.

Open the following ports (incoming and outgoing) on each device to allow operation on a local network:

- **UDP port 42300** (for discovery): Used for automatic peer discovery.
  - Allow inbound UDP 42300 to receive discovery packets.
  - Allow outbound UDP to 42300 for sending broadcasts/multicasts.

- **TCP port 42301** (for file transfer): Used for actual file transfers.
  - Allow inbound TCP 42301 to receive incoming transfers.
  - Outbound connections use an ephemeral port on the sender side. Typically, your firewall will already allow that.

Open these ports for IPv4 and/or IPv6 as needed.
This app can operate on both IPv4 or IPv6 or dual-stack networks.

IPv4 discovery uses broadcast; IPv6 discovery uses link-local multicast.
Some networks block broadcast or multicast.
Ensure that these are allowed on your LAN and router if discovery fails.

## Encryption

The app implements strong encryption for all file transfer traffic.
This encryption is automatic and transparent to the user by default, requiring no configuration or setup.
Devices remember each other after the first approved connection - on subsequent connections, the stored key is verified against the presented one.

For users who want an additional layer of security, a password can be set, which is mixed into the encryption key.
The password must be the same on both devices.
Click on the Key icon in the app to set or change the password.
It is generally not necessary to set a password, as the automatic encryption already provides strong security, since peers must be explicitly approved on first contact.

For more details on the encryption protocol, see the [crypto protocol documentation](CRYPTO-PROTOCOL.md).

## Future Plans

- Make the app available in **Play Store**.
  I need your help for that.
  Please get in contact with me, if you are interested in becoming a tester for the app to get it registered in the Play Store.
- Provide a built **Android APK** for manual installation (sideloading).
- Desktop: Add the option for this app to be always-on and sit in the **system tray** waiting for new connections and popping-up on new connections.
  Provide systemd user service files for that.

## License

This application is AI generated with heavy manual modifications by
**Michael Büsch** <m@bues.ch>

Licensed under the Apache License version 2.0 or alternatively MIT or alternatively feel free to do whatever you want with this software without restriction.

If you want to redistribute this software, I would like to ask you kindly to remove my name from the source code, the documentation, the build scripts and any other files beforehand.
