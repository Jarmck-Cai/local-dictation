#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
if [[ "$(uname -s)" != Darwin ]]; then
    printf '%s\n' "Run this script on macOS." >&2
    exit 1
fi

models=desktop/src-tauri/resources/models
mkdir -p "$models"
staging=$(mktemp -d)
trap 'rm -rf -- "$staging"' EXIT

download_verified() {
    local source=$1 target=$2 digest=$3
    curl --fail --silent --show-error --location --retry 3 "$source" -o "$target"
    printf '%s  %s\n' "$digest" "$target" | shasum -a 256 --check --status
}

download_verified https://blob.handy.computer/silero_vad_v4.onnx "$staging/silero_vad_v4.onnx" \
    a35ebf52fd3ce5f1469b2a36158dba761bc47b973ea3382b3186ca15b1f5af28
download_verified https://blob.handy.computer/sense-voice-int8.tar.gz "$staging/sense-voice-int8.tar.gz" \
    171d611fe5d353a50bbb741b6f3ef42559b1565685684e9aa888ef563ba3e8a4
tar -xzf "$staging/sense-voice-int8.tar.gz" -C "$staging" \
    sense-voice-int8/model.int8.onnx sense-voice-int8/tokens.txt
(
    cd "$staging"
    shasum -a 256 --check <<'CHECKSUMS'
c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51  sense-voice-int8/model.int8.onnx
f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc  sense-voice-int8/tokens.txt
CHECKSUMS
)
install -m 644 "$staging/silero_vad_v4.onnx" "$models/silero_vad_v4.onnx"
mkdir -p "$models/sense-voice-int8"
install -m 644 "$staging"/sense-voice-int8/{model.int8.onnx,tokens.txt} "$models/sense-voice-int8"
# Reuse the pinned upstream AEC3 and Abseil sources used by the Windows beta.
arch=$(uname -m)
case "$arch" in
    arm64) rust_arch=aarch64 ;;
    x86_64) rust_arch=x86_64 ;;
    *) printf '%s\n' "Unsupported macOS architecture: $arch" >&2; exit 1 ;;
esac
apm=".cache/native/webrtc-apm-2.1-macos-$rust_arch"
if [[ ! -f "$apm/libwebrtc_apm.sha256" ]]; then
    build="$staging/apm"
    mkdir -p "$build"
    download_verified https://freedesktop.org/software/pulseaudio/webrtc-audio-processing/webrtc-audio-processing-2.1.tar.gz \
        "$build/source.tgz" 35e86b986d02ea15f3d04741a1a5a735ba399bc0fac0ee089c39480e35fc3253
    tar -xzf "$build/source.tgz" -C "$build"
    source="$build/webrtc-audio-processing-2.1"
    packages="$source/subprojects/packagecache"
    mkdir -p "$packages"
    download_verified https://github.com/abseil/abseil-cpp/releases/download/20240722.0/abseil-cpp-20240722.0.tar.gz \
        "$packages/abseil-cpp-20240722.0.tar.gz" f50e5ac311a81382da7fa75b97310e4b9006474f9560ac46f54a9967f07d4ae3
    download_verified https://wrapdb.mesonbuild.com/v2/abseil-cpp_20240722.0-3/get_patch \
        "$packages/abseil-cpp_20240722.0-3_patch.zip" 12dd8df1488a314c53e3751abd2750cf233b830651d168b6a9f15e7d0cf71f7b
    export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
    meson setup "$build/out" "$source" --buildtype=release --default-library=static \
        -Dcpp_std=c++20 -Dabseil-cpp:cpp_std=c++20 --wrap-mode=forcefallback
    meson compile -C "$build/out" webrtc-audio-processing-2
    archive="$build/out/webrtc/modules/audio_processing/libwebrtc-audio-processing-2.a"
    test "$(lipo -archs "$archive")" = "$arch"
    mkdir -p "$apm/source/subprojects"
    cp -R "$source/webrtc" "$apm/source/"
    cp -R "$source/subprojects/abseil-cpp-20240722.0" "$apm/source/subprojects/"
    install -m 644 "$archive" "$apm/libwebrtc_apm.a"
    shasum -a 256 "$apm/libwebrtc_apm.a" | cut -d ' ' -f 1 > "$apm/libwebrtc_apm.sha256"
fi

cd desktop
bun install --frozen-lockfile
