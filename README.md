# File Transfer App

A minimal, cross‑platform file transfer tool for **Android - Desktop** (and **Android - Android** / **Desktop - Desktop**) on the same local network.

## Key Features

- **Peer discovery** no manual IP entry or QR codes.
- **Cross‑platform** (Linux desktop + Android) built with **Rust + Dioxus**.
- **Simple UI** focused on fast transfer.
- **No encryption by design** – use only on trusted local networks.

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

This application is AI generated with manual modifications.

Licensed under the Apache License version 2.0 or alternatively feel free to do whatever you want with this software without restriction.
