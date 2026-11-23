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
cd desktop
bun install --frozen-lockfile
