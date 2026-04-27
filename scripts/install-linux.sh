#!/usr/bin/env sh
set -eu

app_name="rclippy"
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
appimage="${1:-}"

run_root() {
    if [ "$(id -u)" -eq 0 ]; then
        "$@"
    elif command -v sudo >/dev/null 2>&1; then
        sudo "$@"
    else
        echo "Missing sudo; install dependencies manually, or run as root." >&2
        return 1
    fi
}

apt_package() {
    apt-cache show "$1" >/dev/null 2>&1
}

install_dependencies() {
    if [ ! -r /etc/os-release ]; then
        echo "Could not detect distro; skipping dependency install." >&2
        return 0
    fi

    # shellcheck disable=SC1091
    . /etc/os-release
    distro_ids="${ID:-} ${ID_LIKE:-}"

    case " $distro_ids " in
        *" debian "*|*" ubuntu "*)
            fuse_pkg="libfuse2"
            if apt_package libfuse2t64; then
                fuse_pkg="libfuse2t64"
            fi
            run_root apt-get update
            run_root apt-get install -y \
                "$fuse_pkg" \
                desktop-file-utils \
                libayatana-appindicator3-1 \
                libgtk-3-0 \
                libsecret-1-0 \
                xdg-utils
            ;;
        *" fedora "*|*" rhel "*|*" centos "*)
            pm="dnf"
            if ! command -v dnf >/dev/null 2>&1 && command -v yum >/dev/null 2>&1; then
                pm="yum"
            fi
            run_root "$pm" install -y \
                desktop-file-utils \
                fuse-libs \
                gtk3 \
                libappindicator-gtk3 \
                libsecret \
                xdg-utils
            ;;
        *" arch "*)
            run_root pacman -Sy --needed --noconfirm \
                desktop-file-utils \
                fuse2 \
                gtk3 \
                libayatana-appindicator \
                libsecret \
                xdg-utils
            ;;
        *" opensuse "*|*" suse "*)
            run_root zypper --non-interactive install \
                desktop-file-utils \
                fuse \
                gtk3 \
                libayatana-appindicator3-1 \
                libsecret-1-0 \
                xdg-utils
            ;;
        *)
            echo "Unsupported distro '${ID:-unknown}'; skipping dependency install." >&2
            ;;
    esac
}

if [ -z "$appimage" ]; then
    appimage=$(find "$script_dir" -maxdepth 1 -name "${app_name}*.AppImage" -print | head -n 1)
fi

if [ -z "$appimage" ] || [ ! -f "$appimage" ]; then
    echo "Usage: $0 [path/to/rclippy.AppImage]" >&2
    exit 1
fi

install_dependencies

if [ "$(id -u)" -eq 0 ]; then
    bin_dir="/usr/local/bin"
    desktop_dir="/usr/share/applications"
    icon_base_dir="/usr/share/icons/hicolor"
else
    bin_dir="${HOME}/.local/bin"
    desktop_dir="${HOME}/.local/share/applications"
    icon_base_dir="${HOME}/.local/share/icons/hicolor"
fi

mkdir -p "$bin_dir" "$desktop_dir"
install -m 755 "$appimage" "${bin_dir}/${app_name}.AppImage"

extract_dir=$(mktemp -d)
cleanup() {
    rm -rf "$extract_dir"
}
trap cleanup EXIT

(
    cd "$extract_dir"
    "$appimage" --appimage-extract >/dev/null
)

icons_installed=false
for icon_source in "${extract_dir}/squashfs-root"/usr/share/icons/hicolor/*/apps/"${app_name}.png"; do
    if [ -f "$icon_source" ]; then
        icon_size=$(basename "$(dirname "$(dirname "$icon_source")")")
        icon_dir="${icon_base_dir}/${icon_size}/apps"
        mkdir -p "$icon_dir"
        install -m 644 "$icon_source" "${icon_dir}/${app_name}.png"
        icons_installed=true
    fi
done

if [ "$icons_installed" = false ]; then
    echo "Could not find ${app_name}.png icons inside AppImage; shortcut may use fallback icon." >&2
fi

cat > "${desktop_dir}/${app_name}.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=rclippy
Comment=Encrypted LAN/VPN text clipboard sharing
Exec=${bin_dir}/${app_name}.AppImage
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
    gtk-update-icon-cache "$icon_base_dir" >/dev/null 2>&1 || true
fi
