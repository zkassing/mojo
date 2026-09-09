# MiRemote Desktop

小米蓝牙语音遥控器的跨平台可视化控制面板，基于 **Tauri 2 + React + TypeScript**。

它是既有 Swift 守护进程（`../Sources`）的图形前端：读写 `config.json`、控制后台服务、查看实时日志、测试火山 ASR 凭证。

## 功能

- **🎛 按键映射可视化**：以遥控器布局图展示 12 个键，点选后分别配置单击 / 长按 / 双击动作，支持按住连发
- **🗂 方案管理**：多 profile（terminal / video / default），按前台 App 的 Bundle ID / 应用名匹配
- **🎤 语音识别设置**：填写火山引擎 AppID / AccessToken / ResourceID，一键**实时测试连接**，流式出字 / 术语纠正 / 自动回车等开关
- **⚙️ 服务与状态**：查看守护进程运行状态，一键重启 / 停止
- **📜 实时日志**：内嵌日志流，彩色分级，跟随底部

## 技术结构

```
desktop/
├── src/                     React + TS 前端
│   ├── pages/               五个页面
│   ├── components/          动作编辑器、Toast
│   └── lib/                 类型、Tauri API 封装、配置 Context
└── src-tauri/
    └── src/
        ├── lib.rs           Tauri 命令层
        ├── config.rs        配置模型（与 Swift schema 1:1，含单元测试）
        ├── store.rs         配置文件读写（unix 600 权限）
        ├── service.rs       launchd 守护进程控制（macOS）
        ├── asr.rs           火山流式 ASR 二进制协议 + 凭证连通性测试
        └── engine.rs        平台内核 trait 抽象（跨平台接缝）
```

配置路径：

| 平台 | 路径 |
|---|---|
| macOS / Linux | `~/.config/miremote/config.json` |
| Windows | `%APPDATA%\miremote\config.json` |

## 开发

```bash
pnpm install
pnpm tauri dev        # 开发模式（前端 HMR + Rust 热编译）
pnpm tauri build      # 打包 .app / .exe / 对应安装包
```

仅检查前端：`pnpm build`
仅检查 Rust + 跑配置测试：`cd src-tauri && cargo test`

## 跨平台内核状态

UI、配置、火山 ASR 协议、方案逻辑全部跨平台。只有**原始按键拦截 / 注入**和 **BLE 遥控器连接**与操作系统强相关，定义在 `engine.rs` 的 `PlatformEngine` trait 后分平台实现：

| 平台 | 状态 | 实现路径 |
|---|---|---|
| macOS | ✅ 完整 | 复用现有 Swift 守护进程（sidecar / launchd） |
| Windows | 🚧 移植中（优先） | btleplug(BLE) + Low-Level Hook / SendInput + WinRT |
| Linux | 🚧 移植中 | btleplug(BLE) + evdev / uinput，处理 X11/Wayland |

Windows / Linux 上当前版本提供完整的配置编辑与凭证测试能力，原生按键引擎在 trait 就位后接入，UI 无需改动。
