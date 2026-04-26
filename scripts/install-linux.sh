#!/usr/bin/env sh
set -eu

app_name="rclippy"
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

if [ "$(id -u)" -eq 0 ]; then
    bin_dir="/usr/local/bin"
    app_dir="/usr/local/lib/${app_name}"
    desktop_dir="/usr/share/applications"
    icon_dir="/usr/share/icons/hicolor/256x256/apps"
else
    bin_dir="${HOME}/.local/bin"
    app_dir="${HOME}/.local/lib/${app_name}"
    desktop_dir="${HOME}/.local/share/applications"
    icon_dir="${HOME}/.local/share/icons/hicolor/256x256/apps"
fi

mkdir -p "$bin_dir" "$app_dir" "$desktop_dir" "$icon_dir"
install -m 755 "${script_dir}/${app_name}" "${app_dir}/${app_name}"
ln -sf "${app_dir}/${app_name}" "${bin_dir}/${app_name}"

if [ -f "${script_dir}/${app_name}.png" ]; then
    install -m 644 "${script_dir}/${app_name}.png" "${icon_dir}/${app_name}.png"
fi

cat > "${desktop_dir}/${app_name}.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=rclippy
Comment=Encrypted LAN/VPN text clipboard sharing
Exec=${app_dir}/${app_name}
Icon=${app_name}
Terminal=false
Categories=Utility;
StartupNotify=false
EOF

chmod 644 "${desktop_dir}/${app_name}.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$desktop_dir" >/dev/null 2>&1 || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache "$(dirname "$(dirname "$icon_dir")")" >/dev/null 2>&1 || true
fi
