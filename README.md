# miremote

小米蓝牙语音遥控器 → macOS 按键映射 + 语音转文字工具。

用户级后台服务，通过 HID 设备指纹（`senderID`）精确识别遥控器发出的事件，
**只拦截遥控器的按键，不影响笔记本自带键盘和其他外设**。
按住语音键可说话转文字，直接输入到当前光标处（本地识别，不联网）。

- 设备：小米蓝牙语音遥控器（VID `0x2717` / PID `0x32b8`，BLE HID）
- 系统：macOS 13+（已在 macOS 27 / Apple Silicon 实测）
- 实现：Swift + CGEventTap + IOKit HID + CoreBluetooth + Speech，无第三方依赖

## 快速开始

```bash
# 1. 编译并打包成 .app（蓝牙/语音权限必需）
./build-app.sh

# 2. 写入配置
cp config.sample.json ~/.config/miremote/config.json

# 3. 授权（系统设置 › 隐私与安全性）
#    ① 辅助功能    → 添加 ~/Library/Application Support/miremote/miremote.app
#    ② 输入监控    → 同上（返回键需要）
#    ③ 蓝牙 / 语音识别 → 首次按语音键时自动弹窗

# 4. 试跑
open ~/Library/Application\ Support/miremote/miremote.app --args run -v

# 5. 满意后装成开机自启
~/.local/bin/miremote install
```

> 授权注意：macOS 的 TCC 权限绑定「路径 + 代码签名标识」。
> `build-app.sh` 优先用本机 Apple 开发证书签名，标识固定，重建后权限不失效。
> 若只有 ad-hoc 签名，每次重建都需重新授权（先 `tccutil reset Accessibility com.zyk.miremote` 再添加）。

## 命令

| 命令 | 说明 |
|---|---|
| `miremote init [--force]` | 生成默认配置，自动识别设备 VID/PID |
| `miremote devices` | 列出所有已连接 HID 设备（查 VID/PID/registryID） |
| `miremote watch` | 实时打印遥控器原始信号，**不做映射**，用于调试 |
| `miremote learn` | 交互式学习按键：按一下键 → 输入名字 → 自动写入配置 |
| `miremote preset` | 根据已学按键生成一套基础映射 |
| `miremote run [-v]` | 前台运行映射服务 |
| `miremote install` | 装成 launchd 用户级服务，开机自启 |
| `miremote uninstall` | 卸载开机自启 |
| `miremote status` | 查看配置 / 权限 / 运行状态 / 设备连接 |
| `miremote reload` | 让运行中的服务重读配置（也支持保存文件后自动热重载） |

## 当前映射（terminal 方案）

前台是终端 / VSCode 时生效：

| 按键 | 功能 | 按住 |
|---|---|---|
| **OK** | 回车 | 连发 |
| **返回** | 退格删除 | 连续删 |
| **← →** | 光标左右移动 | 连续移 |
| **↑ ↓** | 上下（终端里翻命令历史，AI 菜单里切选项） | 连发 |
| 菜单 | Ctrl-C 中断 | — |
| 主页 | Cmd-Tab 切窗口；双击新建标签 | — |
| **语音** | **按住说话 → 松开转文字输入** | — |
| 电源 | 睡眠锁屏 | — |
| 音量 ± | 放行给系统 | — |

另有 `video`（IINA/VLC/QuickTime）和 `default` 两套方案，按前台 App 自动切换。

## 本机遥控器实测键码

| 按键 | 原始信号 | 通道 | 说明 |
|---|---|---|---|
| 上 / 下 / 左 / 右 | `kc:126` / `kc:125` / `kc:123` / `kc:124` | CGEvent | 标准方向键 |
| OK（确认） | `kc:36` | CGEvent | 实际发 Return |
| **返回** | **`hid:7:241`** | **HID 直读** | 见下方说明 |
| 菜单 | `kc:110` | CGEvent | Menu 键 |
| 主页 | `kc:115` | CGEvent | Home |
| 电源 | `aux:6` | CGEvent | NX_POWER_KEY |
| 语音（话筒） | `kc:96` | CGEvent | 实际发 F5 |
| 音量 + / − | `aux:0` / `aux:1` | CGEvent | 媒体键通道 |

> 遥控器同时走三条通道：
> 1. 普通按键 → keyDown/keyUp（配置写 `kc:<CGKeyCode>`）
> 2. 音量和电源 → `NX_SYSDEFINED` 媒体键（配置写 `aux:<NX_KEYTYPE>`）
> 3. **返回键 → HID Keyboard usage 0xF1（241）**，这是个非标准 usage，
>    macOS 键盘驱动不认识它，**永远不会产生任何 CGEvent**，
>    CGEventTap 拦不到。程序为此单开了一条 HID element 直读通道，
>    配置写 `hid:<usagePage>:<usage>`（十进制），即 `hid:7:241`。

### 三种信号写法

| 前缀 | 含义 | 例子 |
|---|---|---|
| `kc:` | CGEvent 键码 | `kc:126`（方向上） |
| `aux:` | 媒体键 NX_KEYTYPE | `aux:0`（音量+） |
| `hid:` | HID usagePage:usage 直读 | `hid:7:241`（返回键） |

用 `miremote watch` 可以看到所有三种通道的信号。

## 配置文件

位置：`~/.config/miremote/config.json`（支持 `//` 注释）

```jsonc
{
  "device": {
    "vendorId": "0x2717",     // 也可写十进制 10007
    "productId": "0x32b8",
    "name": "小米蓝牙语音遥控器"
  },
  "options": {
    "swallowOriginal": true,  // 是否吞掉遥控器原始按键
    "longPressMs": 450,       // 长按判定阈值
    "doublePressMs": 280,     // 双击判定窗口
    "verbose": false
  },
  "buttons": {
    "ok": ["kc:96"],          // 逻辑名 → 原始信号（可多个）
    "up": ["kc:126"]
  },
  "profiles": [
    {
      "name": "terminal",
      "match": { "bundleIds": ["com.apple.Terminal"] },
      "bindings": {
        "ok": { "tap": "return", "long": { "type": "key", "key": "c", "mods": ["ctrl"] } },
        "up": { "tap": "up", "repeat": true }
      }
    },
    { "name": "default", "bindings": { "ok": { "tap": { "type": "media", "key": "playpause" } } } }
  ]
}
```

### 方案（profile）匹配规则

按数组顺序，第一个 `match` 命中前台 App 的 profile 生效；
都不命中时用名为 `default` 的方案。`match` 支持 `bundleIds` 和 `appNames`。

查前台 App 的 bundle id：

```bash
osascript -e 'id of app "iTerm"'
```

### 绑定（binding）

每个按键可分别设置四种触发：

```jsonc
{
  "tap":    { ... },   // 单击
  "long":   { ... },   // 长按（超过 longPressMs）
  "double": { ... },   // 双击（两次间隔小于 doublePressMs）
  "repeat": true       // 按住时连续触发 tap（仅在没有 long/double 时生效）
}
```

> 注意：设了 `long` 或 `double` 后，单击动作会延迟到判定结束才发出。
> 方向键这类要求即时响应的，建议只用 `tap` + `repeat`。

### 动作（action）类型

| type | 参数 | 例子 |
|---|---|---|
| `key` | `key`, `mods` | `{"type":"key","key":"t","mods":["cmd","shift"]}` |
| `media` | `key` | `{"type":"media","key":"playpause"}` |
| `shell` | `command` | `{"type":"shell","command":"open -a Terminal"}` |
| `open` | `target` | `{"type":"open","target":"com.apple.finder"}` |
| `mouseMove` | `dx`, `dy` | `{"type":"mouseMove","dx":0,"dy":-30}` |
| `mouseClick` | `button`, `count` | `{"type":"mouseClick","button":"left","count":2}` |
| `mouseScroll` | `dx`, `dy` | `{"type":"mouseScroll","dy":-60}` |
| `sequence` | `actions` | `{"type":"sequence","actions":["cmd+a","delete"]}` |
| `dictate` | — | `"dictate"` — 按住说话转文字 |
| `none` | — | 吞掉按键，什么都不做 |
| `passthrough` | — | 放行原始按键（不受 `swallowOriginal` 影响） |

**简写**：动作可以直接写成字符串，等价于 `key` 类型。
`"up"` → 方向上键；`"cmd+shift+t"` → 组合键。

可用 `mods`：`cmd` `shift` `opt` `ctrl` `fn`

可用 `media` key：
`playpause` `next` `previous` `volup` `voldown` `mute`
`brightnessup` `brightnessdown` `fast` `rewind` `eject`
`illuminationup` `illuminationdown`

## 语音转文字（已打通）

按住语音键 → 遥控器开麦 → 松开 → 本地识别 → 文字敲到当前光标处。
用 `SFSpeechRecognizer` 的**离线模式**（`requiresOnDeviceRecognition`），不联网。

配置：

```jsonc
"voice": {
  "locale": "zh-CN",          // 识别语言
  "output": "type",           // type = 敲到光标处
                              // typeEnter = 敲完自动回车（vibecoding 适用）
                              // clipboard = 放剪贴板
  "stripPunctuation": true    // 去掉中文标点，命令行里更干净
}
```

绑定方式：`"voice": { "tap": "dictate" }`

### 调试工具

```bash
cd probe
./capture.sh 30     # 抓原始音频 → 自动解码 → 播放（不经过识别）
```

产物：`/tmp/atvv_raw.bin`（原始 ADPCM）、`/tmp/atvv_voice.wav`。

### 协议实测结论

遥控器麦克风**不走 HID**，而走 Google ATVV（Android TV Voice over BLE）的
独立 GATT 服务。CoreBluetooth 可以和系统 HID 驱动**同时**持有这条 BLE 链路，
无需在蓝牙设置里断开配对。

服务 `AB5E0001-5A21-4F05-BC7D-AF01F617B664`：

| 特征 | 属性 | 用途 |
|---|---|---|
| `AB5E0002-…` | write | TX，主机发指令 |
| `AB5E0003-…` | notify | AUDIO，ADPCM 音频帧 |
| `AB5E0004-…` | notify | CTL，控制消息 |

流程：`GET_CAPS 0x0A` → `CAPS_RESP 0x0B` → 按住语音键
→ 遥控器**自发** `AUDIO_START 0x04` → 音频帧流 → 松开 → `AUDIO_STOP 0x00`。

> ⚠️ **坑：遥控器是 hold-to-talk 硬件行为，按下就自己开流**。
> 如果你再主动发一次 `MIC_OPEN`，会触发第二次 `AUDIO_START`，
> 把已经录到的帧清空 —— 表现为识别结果**开头缺字**。
> 代码里用 `isStreaming` 守卫位避开了这个竞态。

**帧格式与 Google spec 不一致，以下为本机固件（2671）实测：**

| 项 | spec / CAPS_RESP 宣称 | 实测真相 |
|---|---|---|
| 帧长 | 134 字节 | **120 字节** |
| 帧头 | 3B 序号 + 3B DVI 状态 | **无头，120 字节全是 ADPCM 数据** |
| 解码器状态 | 逐帧重置 | **跨包连续，不能重置** |
| nibble 顺序 | 低 4 位先 | **高 4 位先** |

编码：IMA/DVI ADPCM 4-bit、16 kHz、16-bit、单声道。120 字节 = 240 样本 = 15 ms。

按错误结构解码会得到削波噪声（peak 顶满 32768、时长只有实际的一半）；
按实测结构解码得到正常人声（过零率 ~0.07，有清晰的静音-说话-静音能量包络）。

### 为何必须打成 .app

macOS 的蓝牙 TCC 只认真实 .app bundle 的 Info.plist，
用 `-Xlinker -sectcreate` 嵌 `__TEXT,__info_plist` **无效**，会被 SIGABRT。
所以 `build-app.sh` 会生成带 `NSBluetoothAlwaysUsageDescription` /
`NSSpeechRecognitionUsageDescription` 的 .app，`launchd` 也指向它。

## 常见问题

**改了配置要重启服务吗？**
不用。保存文件后会自动热重载，也可以手动 `miremote reload`。

**提示无法创建事件拦截 / 按键没反应？**
到 系统设置 › 隐私与安全性 › 辅助功能 检查 miremote 是否勾选。
换过可执行文件路径（比如重新编译到别处、跑了 `install`）需要**移除旧条目再重新添加**，
macOS 是按路径 + 签名授权的。

**音量键想保留系统原本行为？**
绑成 `{"tap": {"type": "passthrough"}}`。

**怎么知道某个键的原始码？**
`miremote watch` 然后按键，或用 `miremote learn` 交互式录入。

**日志在哪？**
后台服务：`~/.config/miremote/logs/miremote.log`

## 已知限制

- **无法独占设备**：`IOHIDDeviceOpen` 加 `kIOHIDOptionsTypeSeizeDevice` 需要 root，
  而 root 进程又无法向用户会话注入事件。所以采用 CGEventTap 拦截方案，
  副作用是按键在被吞掉前会先进入系统事件流（实测无感知延迟）。
- 密码输入框等安全输入场景（EnableSecureEventInput）下事件拦截会被系统挂起，
  程序会自动恢复。
- 遥控器闲置会休眠并断开 BLE，此时按语音键第一下可能只是唤醒。
  程序会自动重连（断开后 3s 重试）。
