# File Transfer App

A *minimal*, cross‑platform file transfer tool for **Android** and **Desktop** on the same local network.

## Key Features

- **Peer discovery** no manual IP entry or QR codes.
- **Cross‑platform** (Currently: Linux desktop + Android).
- Integrates into the **Android share menu / send menu** for easy file sending.
  The file transfer app will appear as an option when sharing files from other apps.

Please note that **no encryption** is implemented by design - use only on trusted local networks.
If you want encryption, consider encrypting files before sending (e.g. encrypted Zip/7z archives).

## Why not use one of the many existing solutions?

Many existing solutions are either complex, require manual setup or are complicated to use.
This project aims to provide a simple, user-friendly alternative that works seamlessly across platforms without the need for manual configuration.

If you need features other than simple file transfer between two devices, there are many other great apps available that may suit your needs better. This project is focused on simplicity and ease of use for basic file transfers on local networks.

### Alternative solutions include:

- **KDE Connect**: File sharing, remote control, encryption, and much more.
- **SMB / Windows File Sharing**: Network file sharing protocol, but requires manual setup and is not user-friendly for non-technical users.
  And typically not usable with the Android share menu.
- **Bluetooth file transfer**: Built into most devices, but can be slow and unreliable for large files.

## Quick Start

### Desktop (Linux/x86_64)

```sh
./desktop-build.sh
./desktop-run.sh
```

### Android

Use the provided scripts to build and install on a connected device:

```sh
./android-build.sh
./android-install.sh
```

## Building from Source

1. Install Rust (stable) via `rustup`.
2. Ensure Android NDK + SDK are installed for Android builds.
3. Run desktop/Android build script (see **Quick Start**).

## Security Notes

- This project deliberately does **not** encrypt traffic.
- Only run on trusted, private networks (e.g., home LAN).
- Do **not** expose it directly to the internet.

## License

This application is AI generated with heavy manual modifications.

Licensed under the Apache License version 2.0 or alternatively feel free to do whatever you want with this software without restriction.
