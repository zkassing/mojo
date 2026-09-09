#!/bin/bash
set -e
cd "$(dirname "$0")"

BIN_DIR="$HOME/.local/bin"
CFG_DIR="$HOME/.config/mojo"

echo "▸ 编译 (release)…"
swift build -c release

echo "▸ 安装到 $BIN_DIR/mojo"
mkdir -p "$BIN_DIR"
cp -f .build/release/mojo "$BIN_DIR/mojo"

if [ ! -f "$CFG_DIR/config.json" ]; then
  echo "▸ 写入初始配置"
  mkdir -p "$CFG_DIR"
  cp config.sample.json "$CFG_DIR/config.json"
  echo "  → $CFG_DIR/config.json"
else
  echo "▸ 配置已存在，跳过（示例见 config.sample.json）"
fi

echo
echo "▸ 检查辅助功能权限…"
if ! "$BIN_DIR/mojo" status | grep -q "辅助功能权限: ✅"; then
  cat <<'MSG'
  ❌ 未授权。请执行：
     1. 打开 系统设置 › 隐私与安全性 › 辅助功能
     2. 点 + 号，按 Cmd-Shift-G 输入路径：~/.local/bin
     3. 选中 mojo 并勾选开关
     4. 然后重新运行 ./install.sh
MSG
  open "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility" 2>/dev/null || true
  exit 1
fi
echo "  ✅ 已授权"

echo
echo "▸ 注册开机自启…"
"$BIN_DIR/mojo" install

echo
"$BIN_DIR/mojo" status
echo
echo "✅ 完成。把 $BIN_DIR 加进 PATH 即可直接用 mojo 命令："
echo "   echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.zshrc"
