<div align="center">
  <img src="desktop/app-icon.png" width="128" alt="Mojo 图标" />
  <h1>Mojo</h1>
  <p><b>小米蓝牙语音遥控器 → 按键映射 + 语音转文字</b></p>
  <p>macOS 菜单栏常驻的 Tauri 面板应用，内置完整 Rust 引擎，无独立守护进程。</p>
</div>

---

## 它做什么

把一支小米蓝牙语音遥控器（VID `0x2717` / PID `0x32b8`，BLE HID）变成 Mac 的趁手外设：

- **🎛 按键映射**：12 个按键，单击 / 双击 / 长按分别配置动作，支持按住连发
- **🗂 方案（Profile）**：多套映射方案，按前台 App 自动切换（Bundle ID / 应用名匹配）
- **🎤 语音转文字**：按住语音键说话，识别结果直接上屏到当前光标处
  - 本地 [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) 识别，**不联网**
  - 可选火山引擎云端大模型（流式出字 / 术语纠正 / 自动回车）
- **📜 实时日志**：面板内嵌彩色分级日志流

**关键设计**：通过 HID 设备指纹（`senderID`）精确识别遥控器发出的事件，
**只拦截遥控器的按键，不影响笔记本自带键盘和其他外设**。

## 快速开始（macOS）

### 下载安装

从 [Releases](https://github.com/zkassing/mojo/releases/latest) 下载 `mojo-macos-arm64.app.tar.gz`，
解压后拖进 `/Applications`，首次**右键 → 打开**（未做代码签名/公证，Gatekeeper 会拦一下）。

> 已安装的版本启动时会自动检测更新（也可在「服务与状态」页手动检查）。

### 授权（系统设置 › 隐私与安全性）

1. **辅助功能** —— 拦截与注入按键
2. **输入监控** —— 返回键等事件需要
3. **蓝牙** —— 连接遥控器麦克风（语音链路）

### 使用

1. 应用常驻菜单栏托盘（胖遥控器图标），托盘菜单可启停引擎 / 打开面板 / 退出
2. 首次使用在面板的「语音识别」页选引擎：`sherpa`（本地，需下载模型）或 `火山`（填凭证，可一键测试连接）

### 从源码构建

```bash
cd desktop
pnpm install
pnpm tauri build        # 产出 src-tauri/target/release/bundle/macos/Mojo.app
```

## 配置与日志

| 内容 | 路径 |
|---|---|
| 配置文件（面板编辑，保存即热重载） | `~/.config/mojo/config.json`（Windows: `%APPDATA%\mojo\config.json`） |
| 运行日志 | `~/.config/mojo/logs/mojo.log` |

## 平台支持

UI、配置、方案逻辑、火山 ASR 协议全部跨平台；只有**按键拦截/注入**和 **BLE 遥控器连接**与操作系统强相关：

| 平台 | 状态 | 实现 |
|---|---|---|
| macOS | ✅ 完整支持 | CGEventTap / IOKit HID / btleplug / sherpa-onnx |
| Windows | ⚠️ 内核已接入 | Low-Level Hook / SendInput / btleplug |
| Linux | ⚠️ 内核已接入 | evdev / uinput / btleplug（X11/Wayland 行为有差异） |

## 开发

```bash
cd desktop
pnpm tauri dev               # 开发模式（前端 HMR + Rust 热编译）
cd src-tauri && cargo test   # 引擎纯逻辑单测
```

> **dev 模式的蓝牙权限坑**：macOS TCC 把裸二进制的隐私权限归属到父进程终端。
> dev 模式下需把蓝牙权限授给你的终端（系统设置 › 蓝牙），否则语音链路不会启动
>（引擎会写日志提示，不会崩溃）。按键映射不受此限制，打包的 `.app` 无此问题。

## 发版

同步 `desktop/package.json` 与 `desktop/src-tauri/tauri.conf.json` 的版本号后打 tag 推送，
GitHub Actions 自动完成三平台构建、更新包签名与 Release 发布：

```bash
git tag v0.2.0 && git push origin v0.2.0
```

细节（更新机制、签名密钥、预发布 tag）见 [AGENTS.md](AGENTS.md)。

## 仓库结构

```
desktop/                Tauri 面板 + Rust 引擎（唯一产物）
  src/                      React + TypeScript 前端（Tailwind / shadcn）
  src-tauri/src/engine/     引擎核心：状态机 / 平台驱动 / ATVV 蓝牙协议 / 语音链路
    engine/macos|windows|linux/  平台内核（按键拦截注入、BLE、前台 App 感知）
    engine/asr/             sherpa-onnx（本地）与火山（云端）识别
third_party/            sherpa-onnx 预编译库（体积大不入 git，CI 自动下载）
docs/                   设计与迁移文档
```

引擎最初是独立 Swift 守护进程，已整体用 Rust 重写并并入面板（详见
[docs/rust-migration.md](docs/rust-migration.md)）。

## 技术栈

Rust（引擎）· Tauri 2 · React 18 + TypeScript + Vite · Tailwind CSS + shadcn/ui ·
btleplug（BLE/ATVV）· sherpa-onnx + ONNX Runtime · CGEventTap / IOKit HID
