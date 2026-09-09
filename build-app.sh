#!/bin/bash
# 打包 mojo 为 .app bundle（蓝牙权限需要真实 .app + Info.plist）
set -e
cd "$(dirname "$0")"
BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/Library/Application Support/mojo"
APP="$APP_DIR/mojo.app"
PLIST="$APP/Contents/Info.plist"

echo "▸ 编译 (release)…"
swift build -c release

echo "▸ 安装到 $BIN_DIR/mojo"
mkdir -p "$BIN_DIR"
cp -f .build/release/mojo "$BIN_DIR/mojo"

echo "▸ 打包 .app: $APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# Info.plist — 声明蓝牙和语音识别权限
cat > "$PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>mojo</string>
    <key>CFBundleIdentifier</key>
    <string>com.zyk.mojo</string>
    <key>CFBundleName</key>
    <string>mojo</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSBluetoothAlwaysUsageDescription</key>
    <string>用于连接遥控器麦克风（语音转文字）</string>
    <key>NSMicrophoneUsageDescription</key>
    <string>用于语音输入</string>
</dict>
</plist>
PLIST

cp -f "$BIN_DIR/mojo" "$APP/Contents/MacOS/mojo"

# sherpa-onnx 动态库（本地免费 ASR 引擎）→ .app/Contents/Frameworks
echo "▸ 拷贝 sherpa-onnx 动态库"
mkdir -p "$APP/Contents/Frameworks"
cp -f third_party/sherpa-onnx/lib/libsherpa-onnx-c-api.dylib "$APP/Contents/Frameworks/"
cp -f third_party/sherpa-onnx/lib/libonnxruntime.dylib "$APP/Contents/Frameworks/"

# 签名：macOS 的 TCC 权限绑定「路径 + 代码签名标识」。
# ad-hoc 签名(-)每次重建都会变，导致每次都要重新勾选辅助功能/输入监控。
# 用真实开发证书签名后标识稳定，权限一次授权长期有效。
IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null \
           | grep -oE '[0-9A-F]{40}' | head -1)
if [ -n "$IDENTITY" ]; then
  DESC=$(security find-identity -v -p codesigning | grep "$IDENTITY" | sed 's/.*"\(.*\)".*/\1/')
  echo "▸ 用开发证书签名: $DESC"
  codesign --force --sign "$IDENTITY" --timestamp=none "$APP/Contents/Frameworks/"*.dylib
  codesign --force --sign "$IDENTITY" --timestamp=none "$APP"
else
  echo "▸ ad-hoc 签名（未找到开发证书）"
  echo "  ⚠️  每次重建后需重新授权辅助功能/输入监控"
  codesign --force --sign - --timestamp=none "$APP" 2>/dev/null || true
fi

echo "✅ $APP 已生成"
echo "   运行: open $APP --args run"
echo "   或: $APP/Contents/MacOS/mojo run"
echo ""

# 安装 launchd 自启时指向 .app 内的二进制
"$BIN_DIR/mojo" status 2>/dev/null | grep -q "开机自启: ✅" && {
  echo "⚠️  已安装开机自启，需要重新安装以指向新路径"
  echo "   $BIN_DIR/mojo uninstall"
  echo "   $BIN_DIR/mojo install (需先编辑 launchAgent 指向 .app 内二进制)"
}