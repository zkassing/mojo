#!/bin/bash
# 用本机开发证书签名打包 Mojo.app。
#
# 为什么需要：macOS TCC 权限绑定「签名标识」。ad-hoc 签名每次重建都变，
# 辅助功能/输入监控/蓝牙每次都要重新授权；用固定开发证书签名后标识稳定，
# 授权一次长期有效 —— 且与旧 Swift 版同证书同 bundle id（com.zyk.mojo），
# 已有的 TCC 授权直接继承。
#
# 不把证书写进 tauri.conf.json 的原因：CI/其他机器没有该证书时打包会失败；
# 用环境变量 APPLE_SIGNING_IDENTITY 按需注入，无证书自动退回 ad-hoc。
set -e
cd "$(dirname "$0")/.."

IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null \
           | grep -oE '[0-9A-F]{40}' | head -1)
if [ -z "$IDENTITY" ]; then
  echo "⚠️  未找到开发证书，退回 ad-hoc 签名（每次重建需重新授权）"
else
  DESC=$(security find-identity -v -p codesigning | grep "$IDENTITY" | sed 's/.*"\(.*\)".*/\1/')
  echo "▸ 签名证书: $DESC"
  export APPLE_SIGNING_IDENTITY="$IDENTITY"
fi

exec pnpm tauri build "$@"
