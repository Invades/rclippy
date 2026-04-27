#!/usr/bin/env sh
set -eu

app_name="rclippy"
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
appimage="${1:-}"

if [ -z "$appimage" ]; then
    appimage=$(find "$script_dir" -maxdepth 1 -name "${app_name}*.AppImage" -print | head -n 1)
fi

if [ -z "$appimage" ] || [ ! -f "$appimage" ]; then
    echo "Usage: $0 [path/to/rclippy.AppImage]" >&2
    exit 1
fi

if [ "$(id -u)" -eq 0 ]; then
    bin_dir="/usr/local/bin"
    desktop_dir="/usr/share/applications"
else
    bin_dir="${HOME}/.local/bin"
    desktop_dir="${HOME}/.local/share/applications"
fi

mkdir -p "$bin_dir" "$desktop_dir"
install -m 755 "$appimage" "${bin_dir}/${app_name}.AppImage"

cat > "${desktop_dir}/${app_name}.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=rclippy
Comment=Encrypted LAN/VPN text clipboard sharing
Exec=${bin_dir}/${app_name}.AppImage
Icon=${bin_dir}/${app_name}.AppImage
Terminal=false
Categories=Utility;
StartupNotify=false
EOF

chmod 644 "${desktop_dir}/${app_name}.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$desktop_dir" >/dev/null 2>&1 || true
fi
