# Rust 内核迁移计划

目标：把 Swift 守护进程（`Sources/`，约 3200 行）移植为 Tauri 面板内置的 Rust 引擎，
最终三平台（macOS / Windows / Linux）共享同一套核心逻辑，消灭 Swift/Rust 双份实现。

## 已确认的决策

| 决策点 | 选择 |
|---|---|
| 架构形态 | **并入 Tauri 面板**：面板即守护进程，单产物常驻后台 |
| BLE 库 | **btleplug**（跨平台，ATVV 协议在其上实现） |
| 语音识别 | sherpa-onnx（本地）+ 火山（云端）**都保留** |
| 迁移节奏 | **并存验证后切换**：Swift 版保留到 Rust 版真机验证通过 |

## 线程模型

```
主线程          Tauri/NSApplication 事件循环（UI）
引擎线程        独立 CFRunLoop：CGEventTap 回调、IOHIDManager 回调、
                按键状态机、长按/双击/连发定时器
tokio 运行时    btleplug ATVV 客户端、火山 WebSocket ASR 会话
sherpa 线程池   本地识别推理（全局一个引擎实例，会话间 reset）
```

通道：ATVV(tokio) → PCM 帧 → ASR 会话(tokio/sherpa) → 识别文本 → 引擎线程（LiveTyper 上屏）。

## 模块映射（Swift → Rust）

| Swift | Rust（`desktop/src-tauri/src/`） | 状态 |
|---|---|---|
| Config.swift | `config.rs`（schema 已有，补 `app`/`url` 字段） | ✅ 已有 |
| —（配置反查/方案匹配） | `config.rs` impl（`raw_to_button` / `resolve_profile`） | ✅ 阶段 0 |
| RemapEngine 状态机 | `engine/state.rs`（纯逻辑 + 单测） | ✅ 阶段 0 |
| Action 归一化（简写/对象） | `engine/action.rs` + 单测 | ✅ 阶段 0 |
| ADPCMDecoder | `engine/adpcm.rs` + 单测 | ✅ 阶段 0 |
| TermFixer | `engine/termfix.rs` + 单测 | ✅ 阶段 0 |
| LiveTyper 差异逻辑 | `engine/livetype.rs` + 单测 | ✅ 阶段 0 |
| CGEventTap 拦截 | `engine/macos/mod.rs`（raw FFI，掩码含 systemDefined） | ✅ 阶段 1 |
| DeviceWatcher/HIDWatcher | `engine/macos/hid.rs`（IOKit FFI） | ✅ 阶段 1 |
| Emitter + NXPoster | `engine/macos/emit.rs`（CGEvent + IOHIDPostEvent FFI） | ✅ 阶段 1 |
| PowerButtonGuard | `engine/macos/power.rs`（IOKit pwr_mgt FFI） | ✅ 阶段 1 |
| frontmostApp (NSWorkspace) | `engine/macos/frontmost.rs`（objc2-app-kit） | ✅ 阶段 1 |
| ATVVClient | `engine/atvv.rs`（btleplug，retrieve_peripherals 找已连接设备） | ✅ 阶段 2 |
| VolcASRClient 完整流式 | `engine/asr/volc.rs`（WebSocket 二进制帧全协议） | ✅ 阶段 2 |
| SherpaASRClient | `engine/asr/sherpa.rs`（**复用 bridge.c**，cc 编译 + 链接预编译库） | ✅ 阶段 2 |
| 语音编排（按住-松开/上屏） | `engine/voice.rs`（私有 tokio runtime + 串行上屏线程） | ✅ 阶段 2 |
| 面板服务页 / 托盘 / Info.plist | `lib.rs` + 前端（托盘菜单、关窗常驻） | ✅ 阶段 3（部分） |
| Swift 守护进程 + CI daemon job | 已删除（launchd 已卸载、仓库与 CI 已清） | ✅ 阶段 4 |

## 关键实现要点

- **sherpa**：`Sources/SherpaBridge/bridge.c` 用 `cc` crate 原样编译，链接
  `third_party/sherpa-onnx` 预编译库（与 Package.swift 同一套），行为与 Swift 完全一致。
- **按键注入**：优先 IOHIDPostEvent（驱动层、无 PID、微信等热键可见），CGEvent 兜底 ——
  与 Swift 版 NXPoster/Emitter 策略一致。
- **TCC 权限**：面板 bundle id 已是 `com.zyk.mojo`（与守护进程相同）。若用同一证书签名，
  辅助功能/输入监控授权可平滑继承；Info.plist 需补
  `NSBluetoothAlwaysUsageDescription`、`NSMicrophoneUsageDescription`。
- **配置文件**：路径与 schema 不变（`~/.config/mojo/config.json`），Swift/Rust 可交叉读写。
- **事件 tap 放引擎线程自有 CFRunLoop**，不占主线程，避免 UI 卡顿吞事件（tap 超时禁用）。
- **btleplug 找设备**：遥控器已连接时不广播，扫描找不到 —— 用
  `retrieve_peripherals`（底层 `retrieveConnectedPeripheralsWithServices`）
  按 ATVV 服务 UUID 过滤，兜底按名字 + 电池/设备信息服务过滤。
- **ASR 会话形状统一**：`SessionIn`（Pcm/Finish/Cancel）进、`AsrOut`
  （Partial/Final）出，通道用 tokio unbounded —— volc 任务 `.recv().await`，
  sherpa 线程 `.blocking_recv()`，两端顺手。

## 阶段与验收

- **阶段 0 纯逻辑核心**（本轮）：状态机、动作归一化、ADPCM、术语修正、
  LiveTyper 差异 —— 全部跨平台纯逻辑，cargo test 验证。
- **阶段 1 macOS 输入输出**（已完成）：拦截/注入/设备监视/电源键守卫/前台 App，
  面板服务页新增「内置引擎（Rust · 试验）」卡片可启停，配置保存即热重载。
  验收：真机按遥控器，映射、长按、双击、连发、去抖行为与 Swift 版一致。
  注意：与 Swift 守护进程互斥，测试前先停掉守护进程；首次启动需给面板
  应用授权「辅助功能」「输入监控」。
  已知差异：`watch`/`learn` CLI 子命令未移植（面板无对应 UI，阶段 3 再议）。
- **阶段 2 语音链路**（已完成，待真机验证）：ATVV(btleplug) + 火山流式全协议 +
  sherpa（cc 编译 bridge.c，库缺失时自动 stub）+ LiveTyper 上屏线程。
  语音跑引擎私有 tokio runtime（2 线程）+ sherpa 专用线程 + mojo-voice-out
  串行上屏线程，全部与 CFRunLoop 解耦。
  Info.plist 已补蓝牙/麦克风权限描述。
  验收：按住语音键说话，边说边出字、术语纠正、自动回车与 Swift 版一致。
  注意：打包进 .app 时需把 sherpa dylib 收进 Frameworks 并补
  `@executable_path/../Frameworks` rpath（当前 dev 用绝对路径 rpath）。
- **阶段 3 面板整合**：引擎启停接管服务页、权限引导、开机自启、托盘常驻、
  配置保存热重载。
- **阶段 4 切换**（已完成）：launchd 守护进程已卸载（bootout + 删 plist +
  删 `~/Library/Application Support/mojo`）；`Sources/`、`Package.swift`、
  `build-app.sh`、`install.sh`、`probe/`、CI daemon job 已删；
  SherpaBridge 移至 `desktop/src-tauri/native/sherpa-bridge/`；
  面板 service.rs / 服务页守护进程卡片同步移除；
  sherpa 下载步骤并入 CI panel job（仅 macOS）。
  遗留：`watch`/`learn` CLI 子命令未移植。
  开机自启已做（tauri-plugin-autostart，LaunchAgent，托盘菜单勾选开关）；
  应用启动时若有配置文件会自动拉起引擎。
  签名已解决：`desktop/scripts/build-signed.sh`（`pnpm build:signed`）
  自动检测本机开发证书经 APPLE_SIGNING_IDENTITY 注入（不落配置，CI 不受影响），
  与旧 Swift 版同证书同 bundle id → TCC 授权继承实测通过；
  sherpa dylib 已收进 .app Frameworks 并随包重签名，rpath 已规范化，可分发。
  下一步：Windows/Linux 内核。

## 阶段 5 · Windows / Linux 内核（已接入，待真机验证）

- `engine/windows/`：`WH_KEYBOARD_LL` 钩子 + Win32 消息泵引擎线程；信号 id
  `win:vk:0x{vk}`（verbose 日志可发现）；SendInput 注入（Unicode 直输，
  LiveTyping 完整可用）；SetupAPI 轮询遥控器在线状态；GetForegroundWindow
  前台 App。已知取舍：LL 钩子拿不到设备来源，映射键对所有键盘生效
  （PowerToys 同款）；SendInput 对管理员进程无效（UIPI）。
- `engine/linux/`：evdev 独占抓取（grab）+ uinput 虚拟设备转发未映射事件
  （真·按设备过滤）；信号 id `linux:key:{name}`；读线程 + 代数定时器；
  文本注入 ASCII 直打、非 ASCII 走剪贴板粘贴（v1 建议 liveTyping=false）；
  前台 App 暂恒默认方案；需 input 组/udev 权限（引擎启动自检并给出提示）。
- sherpa 三平台：build.rs 按 CARGO_CFG_TARGET_OS 选库目录
  （third_party/sherpa-onnx{,-win-x64,-linux-x64}），CI 各自下载
  （Windows 用 MD-Release）。Windows/Linux 产物运行时需 DLL/.so 与
  可执行文件同目录（CI 产物打包事项，真机验证时处理）。
- TLS 从 native-tls 换 rustls（tokio-tungstenite rustls-tls-native-roots），
  免去 Linux 的 openssl-sys 交叉编译依赖。
- 本地交叉检查：`MOJO_SKIP_TAURI_BUILD=1 cargo check --target
  x86_64-pc-windows-msvc`（Linux 需 pkg-config 桩，见会话记录）。
