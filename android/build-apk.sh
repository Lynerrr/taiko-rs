#!/bin/sh
# Builds the Android app: target/android/taiko-rs.apk.
#
# The game is the crate's library built for Android (it starts in `android_main`, src/lib.rs),
# packaged as a NativeActivity with one small Java class (android/java, for importing .osz files
# opened with the game). Plain SDK tools, no Gradle.
#
# Needs: the Android SDK (build-tools and a platform) with the NDK, a JDK,
#   cargo install cargo-ndk
#   rustup target add aarch64-linux-android   (and x86_64-linux-android for an emulator)
#
# Usage: android/build-apk.sh [--install]
#   --install            adb install the result on the connected device
# Environment:
#   ANDROID_HOME         the SDK (default ~/Android/Sdk); ANDROID_NDK_HOME: the NDK (default:
#                        the newest one in the SDK)
#   TAIKO_ABIS           ABIs to include (default "arm64-v8a"; e.g. "arm64-v8a x86_64")
#   TAIKO_VERSION_CODE   versionCode (default: the number of commits)
#   ANDROID_KEYSTORE, ANDROID_KEYSTORE_PASS, ANDROID_KEY_ALIAS
#                        signing key. Default: the release key ~/.android/taiko-rs-release.jks
#                        (password in taiko-rs-release.pass next to it, alias taiko-rs) if it is
#                        there, the one CI signs with; else the SDK debug key
#                        ~/.android/debug.keystore, created if missing. Updates only install
#                        over an APK signed with the same key.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
install=false
for arg in "$@"; do
    case $arg in
        --install) install=true ;;
        *) echo "unknown option $arg" >&2; exit 2 ;;
    esac
done

sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}
[ -d "$sdk" ] || { echo "Android SDK not found (set ANDROID_HOME)" >&2; exit 1; }
newest() { ls -1 "$1" 2>/dev/null | sort -V | tail -n 1; }
tools="$sdk/build-tools/$(newest "$sdk/build-tools")"
# The newest release platform ("android-36"; previews like "android-37.0" would make the APK
# installable only on that preview).
platform=""
for p in $(ls -1 "$sdk/platforms" | grep -E '^android-[0-9]+$' | sort -V); do
    [ -f "$sdk/platforms/$p/android.jar" ] && platform="$sdk/platforms/$p"
done
[ -n "$platform" ] || { echo "no Android release platform with android.jar in $sdk/platforms" >&2; exit 1; }
ndk=${ANDROID_NDK_HOME:-$sdk/ndk/$(newest "$sdk/ndk")}
[ -d "$ndk" ] || { echo "Android NDK not found (set ANDROID_NDK_HOME)" >&2; exit 1; }
export ANDROID_HOME="$sdk" ANDROID_NDK_HOME="$ndk"
android_jar="$platform/android.jar"
target_sdk=${platform##*android-}
# AAudio (the audio output) arrived in Android 8.0.
min_sdk=26
strip="$ndk/toolchains/llvm/prebuilt/$(ls "$ndk/toolchains/llvm/prebuilt" | head -n 1)/bin/llvm-strip"

abis=${TAIKO_ABIS:-arm64-v8a}
version_name=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
version_code=${TAIKO_VERSION_CODE:-$(git rev-list --count HEAD 2>/dev/null || echo 1)}

out="$root/target/android"
stage="$out/stage"
rm -rf "$stage"
mkdir -p "$stage/lib" "$stage/classes" "$stage/res/mipmap-xxxhdpi"

echo "== native library ($abis)"
for abi in $abis; do
    case $abi in
        arm64-v8a) triple=aarch64-linux-android ;;
        armeabi-v7a) triple=armv7-linux-androideabi ;;
        x86_64) triple=x86_64-linux-android ;;
        x86) triple=i686-linux-android ;;
        *) echo "unknown ABI $abi" >&2; exit 1 ;;
    esac
    cargo ndk -t "$abi" -P "$min_sdk" rustc --lib --release --locked --crate-type cdylib
    mkdir -p "$stage/lib/$abi"
    "$strip" -o "$stage/lib/$abi/libtaiko_rs.so" "target/$triple/release/libtaiko_rs.so"
done

echo "== java"
javac -nowarn --release 11 -classpath "$android_jar" -d "$stage/classes" $(find android/java -name '*.java')
"$tools/d8" --min-api "$min_sdk" --lib "$android_jar" --output "$stage" $(find "$stage/classes" -name '*.class')

echo "== resources and manifest"
cp assets/taiko-rs.png "$stage/res/mipmap-xxxhdpi/ic_launcher.png"
"$tools/aapt2" compile --dir "$stage/res" -o "$stage/res.zip"
"$tools/aapt2" link -o "$stage/unsigned.apk" -I "$android_jar" --manifest android/AndroidManifest.xml \
    --min-sdk-version "$min_sdk" --target-sdk-version "$target_sdk" \
    --version-code "$version_code" --version-name "$version_name" "$stage/res.zip"

# aapt2 doesn't add code; put classes.dex and the libraries in with Python's zipfile (no zip
# tool needed).
python3 - "$stage" <<'EOF'
import os, sys, zipfile
stage = sys.argv[1]
with zipfile.ZipFile(os.path.join(stage, "unsigned.apk"), "a", zipfile.ZIP_DEFLATED) as apk:
    apk.write(os.path.join(stage, "classes.dex"), "classes.dex")
    for abi in sorted(os.listdir(os.path.join(stage, "lib"))):
        apk.write(os.path.join(stage, "lib", abi, "libtaiko_rs.so"), f"lib/{abi}/libtaiko_rs.so")
EOF

echo "== align and sign"
"$tools/zipalign" -f -p 4 "$stage/unsigned.apk" "$stage/aligned.apk"
release="$HOME/.android/taiko-rs-release.jks"
if [ -z "${ANDROID_KEYSTORE:-}" ] && [ -f "$release" ] && [ -f "${release%.jks}.pass" ]; then
    # The release key (as in CI), so local builds install over published ones and back.
    keystore=$release
    pass=$(cat "${release%.jks}.pass")
    alias=taiko-rs
else
    keystore=${ANDROID_KEYSTORE:-$HOME/.android/debug.keystore}
    pass=${ANDROID_KEYSTORE_PASS:-android}
    alias=${ANDROID_KEY_ALIAS:-androiddebugkey}
fi
if [ ! -f "$keystore" ] && [ -z "${ANDROID_KEYSTORE:-}" ]; then
    mkdir -p "$(dirname "$keystore")"
    keytool -genkeypair -keystore "$keystore" -storepass android -keypass android -alias androiddebugkey \
        -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=Android Debug,O=Android,C=US" >/dev/null 2>&1
fi
"$tools/apksigner" sign --ks "$keystore" --ks-pass "pass:$pass" --ks-key-alias "$alias" \
    --out "$out/taiko-rs.apk" "$stage/aligned.apk"
rm -f "$out/taiko-rs.apk.idsig"
echo "built $out/taiko-rs.apk (version $version_name, code $version_code, $abis)"

if $install; then
    adb install -r "$out/taiko-rs.apk"
fi
