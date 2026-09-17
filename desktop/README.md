# Mojo Desktop

小米蓝牙语音遥控器的跨平台可视化面板，**Tauri 2 + React + TypeScript**。
面板即引擎：Rust 引擎内置于应用中，无独立守护进程。

> 项目说明、安装与使用见 [根目录 README](../README.md)。

## 页面

- **按键映射**：遥控器布局图，12 键 × 单击/双击/长按动作配置，支持按住连发
- **方案管理**：多 profile，按前台 App（Bundle ID / 应用名）自动切换
- **语音识别**：sherpa（本地）/ 火山（云端）引擎选择与凭证测试
- **服务与状态**：引擎运行状态、遥控器连接、更新检查
- **实时日志**：内嵌彩色分级日志流

## 技术结构

```
src/                     React + TS 前端
  pages/                   五个页面
  components/              动作编辑器、Toast 等
  lib/                     类型、Tauri API 封装、配置 Context
src-tauri/src/
  lib.rs                 Tauri 命令层 + 托盘
  config.rs              配置模型（含单元测试）
  store.rs               配置文件读写（unix 600 权限）
  asr.rs                 火山流式 ASR 协议 + 凭证连通性测试
  engine/                引擎核心：状态机 / ATVV 蓝牙 / 语音链路 / 平台内核
    macos|windows|linux/   平台实现（按键拦截注入、前台 App 感知）
    asr/                   sherpa-onnx（本地）/ 火山（云端）
```

配置路径：macOS/Linux `~/.config/mojo/config.json`，Windows `%APPDATA%\mojo\config.json`。

## 开发

```bash
pnpm install
pnpm tauri dev        # 开发模式（前端 HMR + Rust 热编译）
pnpm tauri build      # 打包
cd src-tauri && cargo test   # Rust 单测
```

dev 模式蓝牙权限注意：macOS TCC 把权限归属到父进程终端，需给终端授蓝牙权限，
语音链路才会启动（打包的 `.app` 无此问题）。
