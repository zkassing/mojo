# mojo

小米蓝牙语音遥控器 → 按键映射 + 语音转文字。

通过 HID 设备指纹（`senderID`）精确识别遥控器发出的事件，
**只拦截遥控器的按键，不影响笔记本自带键盘和其他外设**。
按住语音键说话转文字，直接输入到当前光标处（本地 sherpa-onnx 识别，不联网；
也可选火山引擎云端大模型）。

- 设备：小米蓝牙语音遥控器（VID `0x2717` / PID `0x32b8`，BLE HID）
- 形态：Tauri 面板应用（macOS 菜单栏常驻，托盘运行），内置完整引擎，无独立守护进程
- 实现：Rust（引擎）+ CGEventTap / IOKit HID / btleplug(BLE) / sherpa-onnx
- 平台：macOS 完整支持；Windows / Linux 内核已接入（evdev/uinput、LL 钩子/SendInput），行为差异见下文

## 快速开始（macOS）

```bash
cd desktop
pnpm install
pnpm tauri build        # 产出 src-tauri/target/release/bundle/macos/Mojo.app
```

1. 把 `Mojo.app` 拖进 `/Applications`，首次**右键 → 打开**（未签名，Gatekeeper 会拦一下）
2. 按提示授权（系统设置 › 隐私与安全性）：
   - ① 辅助功能
   - ② 输入监控（返回键等需要）
   - ③ 蓝牙（连接遥控器麦克风）
3. 应用常驻菜单栏托盘，在托盘菜单启动引擎 / 打开面板
4. 首次使用在面板的「语音识别」页选引擎：sherpa（本地，需下载模型）或火山（填凭证）

配置文件：`~/.config/mojo/config.json`（面板编辑，保存即热重载）
日志：`~/.config/mojo/logs/mojo.log`

## 开发

```bash
cd desktop
pnpm tauri dev          # 开发模式（见下方 dev 模式蓝牙权限说明）
cd src-tauri && cargo test   # 引擎纯逻辑单测
```

### dev 模式的蓝牙权限坑

macOS TCC 把裸二进制的隐私权限**归属到父进程终端**：dev 模式下蓝牙权限
需要授给你的终端（系统设置 › 蓝牙），否则语音链路不会启动（引擎会写日志提示，
不会崩溃）。按键映射不受此限制。打包的 `.app` 无此问题。

## 仓库结构

```
desktop/            Tauri 面板 + Rust 引擎（唯一产物）
  src-tauri/src/engine/   引擎核心（状态机/平台驱动/语音链路）
  src-tauri/native/       sherpa C 桥（bridge.c）
third_party/        sherpa-onnx 预编译库（体积大不入 git，CI 自动下载）
docs/               设计与迁移文档
```

历史：引擎最初是独立 Swift 守护进程，已整体用 Rust 重写并并入面板
（详见 `docs/rust-migration.md`）。
