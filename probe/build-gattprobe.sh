#!/bin/zsh
# 把 gattprobe 编译并打成 .app bundle —— macOS 的蓝牙 TCC 只认真实 bundle 的 Info.plist，
# 裸命令行 binary（哪怕用 -sectcreate 嵌了 __TEXT,__info_plist）会被 TCC 直接 SIGABRT。
set -e
cd "$(dirname "$0")"

APP="GattProbe.app"
BIN="$APP/Contents/MacOS/GattProbe"

# 1. 先编译成裸 binary（满足 `swiftc -O gattprobe.swift -o gattprobe` 的编译验证）
swiftc -O -target arm64-apple-macos13.0 gattprobe.swift -o gattprobe

# 2. 打 bundle
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp gattprobe "$BIN"
sed 's|<string>gattprobe</string>|<string>GattProbe</string>|' gattprobe-Info.plist \
  > "$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Add :CFBundlePackageType string APPL' \
  -c 'Add :CFBundleShortVersionString string 1.0' \
  -c 'Add :CFBundleVersion string 1' \
  -c 'Add :LSMinimumSystemVersion string 13.0' \
  -c 'Add :LSUIElement bool true' \
  "$APP/Contents/Info.plist" >/dev/null

# 3. ad-hoc 签名（TCC 需要稳定标识；ad-hoc 够用于本机调试）
xattr -cr "$APP" 2>/dev/null || true
codesign --force --sign - "$APP"

echo "✅ built: $APP"
echo "   运行: ./$BIN --timeout 30"
echo "   握手: ./$BIN --handshake --timeout 60"
