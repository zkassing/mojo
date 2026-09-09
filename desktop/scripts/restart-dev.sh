#!/bin/bash
# 干净重启 Tauri 开发实例。
# 之前的手工重启只杀应用二进制，留下 pnpm/tauri-dev 包装链，
# 叠加 tauri dev 自身的文件监听重启，导致多个应用进程并存、日志重复。
# 本脚本按「先杀整树、确认清零、再启动」的顺序执行。

set -u

# 1. 杀包装链（tauri CLI / pnpm / 旧的 nohup bash 包装）
pkill -f "tauri dev" 2>/dev/null

# 2. 杀应用二进制（target/debug 下的真实进程）
pkill -f "target/debug/mojo-desktop" 2>/dev/null

# 3. 杀 vite 开发服务器（占 1420 端口的）
lsof -ti:1420 2>/dev/null | xargs kill -9 2>/dev/null

sleep 1

# 4. 兜底强杀任何残留
pkill -9 -f "target/debug/mojo-desktop" 2>/dev/null

leftover=$(pgrep -f "target/debug/mojo-desktop" | wc -l | tr -d ' ')
if [ "$leftover" != "0" ]; then
  echo "警告：仍有 $leftover 个残留进程"
  pgrep -fl "target/debug/mojo-desktop"
fi

# 5. 启动（single-instance 插件保证不会再叠加）
cd "$(dirname "$0")/.."
nohup pnpm tauri dev > /tmp/mojo-tauri.log 2>&1 &
echo "已重启（pid $!），日志: /tmp/mojo-tauri.log"
