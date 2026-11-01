#!/bin/sh
# Installs or updates taiko.rs for the current user on Linux or macOS (no root needed).
#
#   curl -fsSL https://raw.githubusercontent.com/Lynerrr/taiko-rs/worktree-taiko-client/install.sh | sh
#
# Options (after `sh -s --` when piped): --no-launch (don't start it afterwards), --uninstall.
# TAIKO_RS_CHANNEL=dev installs the latest test build instead (`| TAIKO_RS_CHANNEL=dev sh`).
# TAIKO_RS_URL overrides the download (any URL curl understands, e.g. file:// for testing).
set -eu

case "${TAIKO_RS_CHANNEL:-}" in
    dev) tag=dev ;;
    *) tag=latest ;;
esac
system="$(uname -s)"
case "$system" in
    Darwin) asset=taiko-rs-macos.tar.gz ;;
    *) asset=taiko-rs-linux-x64.tar.gz ;;
esac
url="${TAIKO_RS_URL:-https://github.com/Lynerrr/taiko-rs/releases/download/$tag/$asset}"
bin_dir="$HOME/.local/bin"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
exe="$bin_dir/taiko-rs"
desktop="$data_dir/applications/taiko-rs.desktop"
# The hidden entry that opens taiko-rs:// links (a score's "Watch replay"), as the game writes it.
link_desktop="$data_dir/applications/taiko-rs-link.desktop"
icon="$data_dir/icons/hicolor/256x256/apps/taiko-rs.png"
# Earlier installs put an SVG icon here.
old_icon="$data_dir/icons/hicolor/scalable/apps/taiko-rs.svg"
# macOS: Applications if this user may write there (an admin may), else their own.
apps=/Applications
[ -w "$apps" ] || apps="$HOME/Applications"
app="$apps/taiko-rs.app"

launch=1
for arg in "$@"; do
    case "$arg" in
        --no-launch) launch=0 ;;
        --uninstall)
            if [ "$system" = Darwin ]; then
                rm -rf "$app"
                echo "Removed taiko.rs. Settings ($HOME/.config/taiko-rs) and maps ($data_dir/taiko-rs) are kept."
                exit 0
            fi
            rm -f "$exe" "$desktop" "$link_desktop" "$icon" "$old_icon"
            echo "Removed taiko.rs. Settings ($HOME/.config/taiko-rs) and maps ($data_dir/taiko-rs) are kept."
            exit 0
            ;;
        *) echo "unknown option: $arg" >&2; exit 1 ;;
    esac
done

# The macOS build is universal.
if [ "$system" != Darwin ] && [ "$(uname -m)" != "x86_64" ]; then
    echo "There is only an x86_64 build. Build from source instead: https://github.com/Lynerrr/taiko-rs" >&2
    exit 1
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading taiko.rs..."
if command -v curl >/dev/null 2>&1; then
    curl -fL --progress-bar "$url" -o "$tmp/taiko-rs.tar.gz"
elif command -v wget >/dev/null 2>&1; then
    wget -q --show-progress "$url" -O "$tmp/taiko-rs.tar.gz"
else
    echo "Needs curl or wget." >&2
    exit 1
fi
tar -xzf "$tmp/taiko-rs.tar.gz" -C "$tmp"

# Files that curl saves aren't quarantined, so Gatekeeper lets this unnotarized app start. A
# running game keeps going from the old files it has open.
if [ "$system" = Darwin ]; then
    mkdir -p "$apps"
    rm -rf "$app"
    mv "$tmp/taiko-rs.app" "$app"
    echo "Installed $app."
    echo "Maps: point it at your osu! Songs folder in Settings -> Library, or put .osz files in $data_dir/taiko-rs/Songs"
    if [ "$launch" = 1 ]; then
        open "$app"
    fi
    exit 0
fi

# Updating works while the old version runs: the new binary is a new file renamed over it.
mkdir -p "$bin_dir" "$(dirname "$desktop")" "$(dirname "$icon")"
install -m 755 "$tmp/taiko-rs/taiko-rs" "$exe.new"
mv -f "$exe.new" "$exe"
install -m 644 "$tmp/taiko-rs/taiko-rs.png" "$icon"
rm -f "$old_icon"

cat > "$desktop" <<EOF
[Desktop Entry]
Type=Application
Name=taiko.rs
Comment=Fast osu!taiko client
Exec="$exe" %F
Icon=taiko-rs
Terminal=false
Categories=Game;
EOF
cat > "$link_desktop" <<EOF
[Desktop Entry]
Type=Application
Name=taiko.rs
Comment=Opens taiko.rs links (scores to watch)
Exec="$exe" %u
Icon=taiko-rs
Terminal=false
NoDisplay=true
MimeType=x-scheme-handler/taiko-rs;
EOF
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$(dirname "$desktop")" 2>/dev/null || true
command -v xdg-mime >/dev/null 2>&1 && xdg-mime default taiko-rs-link.desktop x-scheme-handler/taiko-rs 2>/dev/null || true

echo "Installed $exe (added to the app menu)."

# The build links the system's PipeWire and ALSA libraries; say which package is missing.
missing="$(ldd "$exe" 2>/dev/null | awk '/not found/ { print $1 }')"
if [ -n "$missing" ]; then
    echo
    echo "Missing libraries: $(echo "$missing" | tr '\n' ' ')"
    echo "Install them, then run taiko.rs:"
    echo "  Arch:          sudo pacman -S pipewire alsa-lib"
    echo "  Debian/Ubuntu: sudo apt install libpipewire-0.3-0 libasound2"
    echo "  Fedora:        sudo dnf install pipewire-libs alsa-lib"
    launch=0
fi

case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) echo "Note: $bin_dir is not in your PATH; start it from the app menu or as $exe" ;;
esac
echo "Maps: taiko.rs reads your osu! Songs folder (osu-winello / Wine installs are found on their own)."
echo "No osu!? Put .osz files in $data_dir/taiko-rs/Songs"

if [ "$launch" = 1 ] && { [ -n "${WAYLAND_DISPLAY:-}" ] || [ -n "${DISPLAY:-}" ]; }; then
    nohup "$exe" >/dev/null 2>&1 &
fi
