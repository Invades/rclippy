# rclippy

`rclippy` is a small native Rust tray app for encrypted text clipboard sharing
between two computers on the same LAN or VPN.

## Scope

- Text clipboard sync only.
- One paired peer.
- Manual `ip:port` configuration.
- End-to-end encrypted paired transport using TLS 1.3 with pinned self-signed
  device certificates.
- Pairing uses a short one-time code to authenticate the first exchange.
- Secrets are stored in the OS keychain.

No file sharing, image clipboard, cloud relay, account system, clipboard
history, or LAN discovery is included in v1.

## Development

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Linux tray builds need GTK/AppIndicator development packages required by the
`tray-icon` crate.

