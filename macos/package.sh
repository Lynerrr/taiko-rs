#!/bin/sh
# Builds taiko-rs-macos.tar.gz: taiko-rs.app as one universal binary (Apple silicon and Intel).
# Needs both Rust targets, aarch64-apple-darwin and x86_64-apple-darwin.
#
# There is no Apple Developer ID, so the app is signed ad hoc: Apple silicon won't run a binary
# without a signature, and macOS only stops one that came through a browser (see install.sh).
set -eu
cd "$(dirname "$0")/.."

# The same as LSMinimumSystemVersion in Info.plist.
export MACOSX_DEPLOYMENT_TARGET=11.0

for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release --locked --target "$target"
done

dist=target/macos
app="$dist/taiko-rs.app"
binary="$app/Contents/MacOS/taiko-rs"
rm -rf "$dist"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

lipo -create -output "$binary" target/aarch64-apple-darwin/release/taiko-rs target/x86_64-apple-darwin/release/taiko-rs
lipo -verify_arch arm64 x86_64 "$binary"
strip -x "$binary"

version="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
sed "s/@VERSION@/$version/" macos/Info.plist > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"

# The icon, up to the 256 pixels of assets/taiko-rs.png.
iconset="$dist/taiko-rs.iconset"
mkdir "$iconset"
for size in 16 32 64 128 256; do
    half=$((size / 2))
    sips -z "$size" "$size" assets/taiko-rs.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
    if [ "$size" -gt 16 ]; then
        cp "$iconset/icon_${size}x${size}.png" "$iconset/icon_${half}x${half}@2x.png"
    fi
done
# iconutil only takes the sizes Apple lists.
rm "$iconset/icon_64x64.png"
iconutil -c icns -o "$app/Contents/Resources/taiko-rs.icns" "$iconset"
rm -rf "$iconset"

codesign --force --sign - "$app"
codesign --verify --strict --verbose=2 "$app"

# ustar, and no ._ files for Finder's metadata: the updater's reader (src/update.rs) wants plain
# entries.
COPYFILE_DISABLE=1 tar --format ustar -czf taiko-rs-macos.tar.gz -C "$dist" taiko-rs.app
tar -tzf taiko-rs-macos.tar.gz | grep -qx 'taiko-rs.app/Contents/MacOS/taiko-rs'
