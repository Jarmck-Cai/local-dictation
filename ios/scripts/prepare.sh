#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
if ! command -v xcodegen >/dev/null; then
  echo '请先安装 XcodeGen：brew install xcodegen'
  exit 1
fi
test -f Config/Developer.xcconfig || cp Config/Developer.xcconfig.example Config/Developer.xcconfig
test -f Config/Service.plist || cp Config/Service.plist.example Config/Service.plist
xcodegen generate
echo '打开 ios/LocalDictation.xcodeproj，选择自己的签名团队和已连接的 iPhone。'
