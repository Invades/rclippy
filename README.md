# rclippy (remote clippy)

Simple encrypted text clipboard sharing for two computers on the same LAN or VPN.

<p align="center">
  <img src="./assets/rclippy.svg" width="180" alt="rclippy logo">
</p>

An experimental native tray app for Windows, macOS, and Linux. It syncs
clipboard contents directly between two paired peers.

## Features

- Text clipboard sync only.
- Manual `ip:port` configuration for LAN, WireGuard, Tailscale, or similar VPNs.
- Direct TCP transport.
- TLS 1.3 transport encryption with pinned self-signed device certificates.
- One-time pairing code to authenticate first connection.
- Secrets stored in the OS keychain.

## Install

Download the latest release from the GitHub Releases page.

### Windows

Download and run:

```text
rclippy-windows-x64-installer.exe
```

The installer is unattended. It installs for the current user by default, or
system-wide when run as administrator.

### macOS

Download:

```text
rclippy-macos-universal.dmg
```

Open the DMG and drag `rclippy.app` into Applications.

> Current builds are unsigned. On macOS this may require opening the system settings to explicitly allo the app.

### Linux

Download and extract:

```text
rclippy-linux-x64.zip
```

Install:

```sh
chmod +x rclippy-linux-install.sh
./rclippy-linux-install.sh
```

The script installs the AppImage and desktop entry for the current user. When
run as root, it installs system-wide. It also installs common runtime
dependencies based on `/etc/os-release` and extracts the desktop icon from the
AppImage.

You can also run the AppImage directly:

```sh
chmod +x rclippy-linux-x64.AppImage
./rclippy-linux-x64.AppImage
```

Linux notes:

- Secret storage depends on Secret Service / a compatible keychain provider.
- The app defaults to X11/XWayland when available for tray-window behavior.
  To force native Wayland:

```sh
WINIT_UNIX_BACKEND=wayland ./rclippy-linux-x64.AppImage
```

## Usage

1. Install and open rclippy on both devices.
2. Put both devices on the same reachable network, LAN, or VPN.
3. On the host device, choose `Host` and click `Show pairing code`.
4. On the client device, choose `Client`, enter the host `ip:port`, enter the
   pairing code, and click `Pair`.
5. After pairing, changing the text clipboard on either side syncs it to the
   other side.

Default listen address:

```toml
listen_addr = "0.0.0.0:38765"
```

If using a firewall, allow inbound TCP on the configured port.

## Development

Install Rust stable, then run:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Run locally:

```sh
cargo run
```

### Linux development dependencies

Ubuntu/Debian:

```sh
sudo apt-get update
sudo apt-get install -y \
  build-essential \
  pkg-config \
  libasound2-dev \
  libatk1.0-dev \
  libayatana-appindicator3-dev \
  libcairo2-dev \
  libfuse2 \
  libgdk-pixbuf-2.0-dev \
  libgtk-3-dev \
  libpango1.0-dev \
  librsvg2-dev \
  libsecret-1-dev \
  libx11-dev \
  libxdo-dev \
  libxi-dev \
  libxkbcommon-dev \
  libxrandr-dev \
  patchelf
```

### Packaging

Install `cargo-packager` for macOS/Linux packaging:

```sh
cargo install cargo-packager --locked
```

Linux AppImage:

```sh
cargo build --release
cargo packager --release --formats appimage --out-dir dist/linux-x64
```

macOS DMG:

```sh
rustup target add x86_64-apple-darwin aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
```

Windows installer:

```powershell
cargo build --release --bin rclippy-uninstaller
$env:RCLIPPY_INSTALLER_PAYLOAD = (Resolve-Path target/release/rclippy.exe).Path
$env:RCLIPPY_UNINSTALLER_PAYLOAD = (Resolve-Path target/release/rclippy-uninstaller.exe).Path
cargo build --release --bin rclippy-installer
```

## Configuration

Config is stored in the platform config directory:

```
# config.toml
listen_addr = "0.0.0.0:38765"
peer_addr = "100.64.0.2:38765"
start_on_login = false
poll_ms = 500
max_text_bytes = 1048576
```

These secrets are stored in the OS keychain:

- local TLS private key
- local device id
- pinned peer certificate/fingerprint

## Roadmap

- [x] Base app scaffolding
- [ ] LAN discovery
- [ ] Image clipboard sync
- [ ] File clipboard sync
- [ ] Document distro-specific Linux requirements more precisely
- [ ] Sign and notarize macOS releases
- [ ] Sign Windows releases

## License

rclippy is licensed under GPL-3.0-or-later. See [LICENSE](./LICENSE).
