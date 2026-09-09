import type { Action, FullAction } from "./types";

export const MOD_LABELS: Record<string, string> = {
  cmd: "Cmd",
  command: "Cmd",
  ctrl: "Ctrl",
  control: "Ctrl",
  alt: "Alt",
  option: "Alt",
  shift: "Shift",
  fn: "Fn",
};

export const KEY_LABELS: Record<string, string> = {
  return: "回车",
  enter: "回车",
  escape: "Esc",
  delete: "Delete",
  backspace: "Delete",
  forwarddelete: "Fwd Delete",
  tab: "Tab",
  space: "空格",
  up: "上",
  down: "下",
  left: "左",
  right: "右",
  pageup: "PgUp",
  pagedown: "PgDn",
  home: "Home",
  end: "End",
  capslock: "Caps Lock",
  // 修饰键单独作为主键时
  fn: "Fn",
  cmd: "Cmd",
  command: "Cmd",
  ctrl: "Ctrl",
  control: "Ctrl",
  alt: "Alt",
  option: "Alt",
  shift: "Shift",
};

export function isFull(a: Action): a is FullAction {
  return typeof a === "object" && a !== null;
}

/** 把一个动作渲染成中文短描述 */
export function describeAction(a: Action | undefined | null): string {
  if (a === undefined || a === null) return "未绑定";
  if (!isFull(a)) return describeShorthand(a);

  switch (a.type) {
    case "key":
    case "keyboard":
    case "hotkey": {
      const mods = (a.mods ?? []).map((m) => MOD_LABELS[m] ?? m);
      // 还没录/填键名时给明确提示，而不是一个看不懂的 "?"
      const key = a.key ? (KEY_LABELS[a.key] ?? a.key) : "未设置按键";
      return [...mods, key].join(" + ");
    }
    case "media":
      return `媒体 · ${mediaLabel(a.key ?? "")}`;
    case "shell":
    case "command":
    case "exec":
      return `命令 · ${truncate(a.command ?? "", 22)}`;
    case "open":
    case "app":
    case "url":
    case "launch":
      return `打开 · ${truncate(a.target ?? "", 22)}`;
    case "mouseMove":
    case "move":
      return `鼠标移动 ${a.dx ?? 0},${a.dy ?? 0}`;
    case "mouseClick":
    case "click":
      return `鼠标${a.button === "right" ? "右" : "左"}键${
        (a.count ?? 1) > 1 ? ` ×${a.count}` : ""
      }`;
    case "mouseScroll":
    case "scroll":
      return `滚轮 ${a.dx ?? 0},${a.dy ?? 0}`;
    case "sequence":
    case "seq":
      return `序列 · ${a.actions?.length ?? 0} 步`;
    case "dictate":
    case "voice":
      return "语音转文字";
    case "none":
      return "忽略";
    case "passthrough":
      return "放行原键";
    default:
      return a.type;
  }
}

function describeShorthand(s: string): string {
  if (["dictate", "voice", "speech", "asr"].includes(s.toLowerCase()))
    return "语音转文字";
  const parts = s.split("+").map((p) => p.trim());
  const key = parts.pop() ?? "";
  const mods = parts.map((m) => MOD_LABELS[m.toLowerCase()] ?? m);
  const keyName = key ? (KEY_LABELS[key.toLowerCase()] ?? key) : "未设置按键";
  return [...mods, keyName].join(" + ");
}

function mediaLabel(k: string): string {
  return (
    {
      playpause: "播放/暂停",
      volup: "音量+",
      voldown: "音量−",
      next: "下一首",
      previous: "上一首",
    }[k] ?? k
  );
}

function truncate(s: string, n: number): string {
  return s.length > n ? s.slice(0, n) + "…" : s;
}

/** 把对象动作转回简写（仅 key 类型可逆），否则返回 null */
export function toShorthand(a: FullAction): string | null {
  if (a.type === "key" || a.type === "keyboard" || a.type === "hotkey") {
    const mods = a.mods ?? [];
    return [...mods, a.key ?? ""].join("+");
  }
  return null;
}
