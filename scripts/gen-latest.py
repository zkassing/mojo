#!/usr/bin/env python3
"""根据 release/ 目录里已扁平化的更新器包和 .sig 生成 Tauri updater 的 latest.json。

CI 用法：
    VERSION=0.1.0 BASE=https://github.com/zkassing/mojo/releases/download/v0.1.0 \
        python3 scripts/gen-latest.py release

release/ 内应包含固定文件名（由工作流从各平台 artifact 拷入）：
    mojo-macos-arm64.app.tar.gz(.sig)
    mojo-windows-x64-setup.nsis.zip(.sig)
    mojo-linux-x86_64.AppImage.tar.gz(.sig)
存在哪些平台就写哪些；至少要有 macOS（本项目主平台），否则以非零码失败，
避免发布出一个无法用于更新的空清单。
"""
import json
import pathlib
import sys

PLATFORMS = {
    "darwin-aarch64": "mojo-macos-arm64.app.tar.gz",
    "windows-x86_64": "mojo-windows-x64-setup.nsis.zip",
    "linux-x86_64": "mojo-linux-x86_64.AppImage.tar.gz",
}


def main() -> int:
    if len(sys.argv) != 3:
        print("用法: gen-latest.py <release目录> <输出latest.json路径>", file=sys.stderr)
        return 2
    release_dir = pathlib.Path(sys.argv[1])
    out_path = pathlib.Path(sys.argv[2])

    version = required_env("VERSION")
    base = required_env("BASE").rstrip("/")

    platforms = {}
    for target, filename in PLATFORMS.items():
        pkg = release_dir / filename
        sig = release_dir / (filename + ".sig")
        if not pkg.exists():
            print(f"跳过 {target}：缺少 {filename}", file=sys.stderr)
            continue
        if not sig.exists():
            print(f"错误：{filename} 存在但缺少签名 {sig.name}（检查 TAURI_SIGNING_PRIVATE_KEY 是否配置）",
                  file=sys.stderr)
            return 1
        platforms[target] = {
            "signature": sig.read_text().strip(),
            "url": f"{base}/{filename}",
        }

    if "darwin-aarch64" not in platforms:
        print("错误：没有 macOS 更新包，无法发布（至少需要 mojo-macos-arm64.app.tar.gz[.sig]）",
              file=sys.stderr)
        return 1

    manifest = {
        "version": version,
        "notes": "详见本 Release 的更新说明",
        "platforms": platforms,
    }
    out_path.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(out_path.read_text(encoding="utf-8"))
    return 0


def required_env(name: str) -> str:
    import os
    v = os.environ.get(name, "").strip()
    if not v:
        print(f"错误：缺少环境变量 {name}", file=sys.stderr)
        sys.exit(1)
    return v


if __name__ == "__main__":
    raise SystemExit(main())
