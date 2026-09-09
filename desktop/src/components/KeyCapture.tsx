import { useEffect, useState } from "react";
import { Keyboard, X } from "lucide-react";
import { cn } from "@/lib/utils";

/// DOM KeyboardEvent.key → 守护进程 KeyMap 键名
const DOM_TO_DAEMON: Record<string, string> = {
  Enter: "return",
  Escape: "escape",
  Tab: "tab",
  " ": "space",
  Backspace: "delete",
  Delete: "forwarddelete",
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowLeft: "left",
  ArrowRight: "right",
  PageUp: "pageup",
  PageDown: "pagedown",
  Home: "home",
  End: "end",
  CapsLock: "capslock",
  Fn: "fn",
};

/// 守护进程键名 → 展示用名字
const PRETTY: Record<string, string> = {
  return: "Return",
  escape: "Esc",
  tab: "Tab",
  space: "Space",
  delete: "Delete",
  forwarddelete: "Fwd Delete",
  pageup: "Page Up",
  pagedown: "Page Down",
  home: "Home",
  end: "End",
  capslock: "Caps Lock",
  fn: "Fn",
  ctrl: "Ctrl",
  alt: "Alt",
  shift: "Shift",
  cmd: "Cmd",
};

const MOD_LABEL: Record<string, string> = {
  ctrl: "Ctrl",
  alt: "Alt",
  shift: "Shift",
  cmd: "Cmd",
  fn: "Fn",
};

const MOD_ORDER = ["ctrl", "alt", "shift", "cmd", "fn"];
const MODIFIER_KEYS = new Set(["Shift", "Control", "Alt", "Meta"]);
/// DOM 修饰键 key 值 → 守护进程键名
const MOD_DOM: Record<string, string> = {
  Control: "ctrl",
  Alt: "alt",
  Shift: "shift",
  Meta: "cmd",
};

/// Shift+符号键 产生的字符 → 基础键（守护进程键码表认基础键，
/// Shift 会由 collectMods 记录，组合出正确快捷键）
const SHIFTED_CHAR: Record<string, string> = {
  "?": "/",
  "+": "=",
  _: "-",
  ':': ";",
  '"': "'",
  "<": ",",
  ">": ".",
  "|": "\\",
  "~": "`",
  "!": "1",
  "@": "2",
  "#": "3",
  $: "4",
  "%": "5",
  "^": "6",
  "&": "7",
  "*": "8",
  "(": "9",
  ")": "0",
  "{": "[",
  "}": "]",
};

function domKeyToDaemon(e: KeyboardEvent): string | null {
  if (DOM_TO_DAEMON[e.key]) return DOM_TO_DAEMON[e.key];
  if (/^F\d{1,2}$/i.test(e.key)) return e.key.toLowerCase();
  if (SHIFTED_CHAR[e.key]) return SHIFTED_CHAR[e.key];
  if (e.key.length === 1) return e.key.toLowerCase();
  return null;
}

function prettyKey(k: string): string {
  return PRETTY[k] ?? (k.length === 1 ? k.toUpperCase() : k);
}

interface Props {
  /** 当前键名 + 修饰键（守护进程格式） */
  keyName?: string;
  mods?: string[];
  onCapture: (key: string, mods: string[]) => void;
  onClear: () => void;
}

/**
 * 快捷键录制框：点击后按下组合键即录入。
 * 注意：WKWebView 里点击 button 不会自动聚焦（Chrome 才会），
 * 所以录制期间监听挂在 window 捕获阶段，不依赖按钮焦点。
 * Cmd+Tab / Cmd+Q 等系统级组合键到不了 webview，那种场景走手动输入。
 */
export default function KeyCapture({ keyName, mods, onCapture, onClear }: Props) {
  const [recording, setRecording] = useState(false);

  useEffect(() => {
    if (!recording) return;

    // 本次录制按过的修饰键种类数：>1 说明用户在按组合，松开不抢录
    let modKinds = 0;

    const collectMods = (e: KeyboardEvent): string[] => {
      const m: string[] = [];
      if (e.ctrlKey) m.push("ctrl");
      if (e.altKey) m.push("alt");
      if (e.shiftKey) m.push("shift");
      if (e.metaKey) m.push("cmd");
      return m;
    };

    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (MODIFIER_KEYS.has(e.key)) {
        // 同个修饰键重复 keydown（长按自动重复）只计一次
        if (!e.repeat) modKinds += 1;
        return; // 修饰键按下不处理，等松开或组合键
      }
      const key = domKeyToDaemon(e);
      if (!key) return; // 认不出的键，忽略
      onCapture(key, collectMods(e));
      setRecording(false);
    };

    // 修饰键松开：只有「本次录制只按下过这一种修饰键」时，才说明
    // 用户想录的就是这个修饰键本身。按过多个修饰键时松开属于组合
    // 过程中的犹豫/调整，继续等主键，不抢录。
    const onUp = (e: KeyboardEvent) => {
      const k = MOD_DOM[e.key];
      if (!k || modKinds > 1) return;
      e.preventDefault();
      e.stopPropagation();
      onCapture(k, collectMods(e));
      setRecording(false);
    };

    // 捕获阶段 + preventDefault：拦住 Tab 焦点切换、输入框抢键等
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("keyup", onUp, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("keyup", onUp, true);
    };
  }, [recording, onCapture]);

  const chips = keyName
    ? [
        ...MOD_ORDER.filter((m) => (mods ?? []).includes(m)).map(
          (m) => MOD_LABEL[m] ?? m
        ),
        prettyKey(keyName),
      ]
    : [];

  return (
    <div className="flex items-center gap-2">
      <button
        type="button"
        onClick={() => setRecording((r) => !r)}
        className={cn(
          "flex h-9 flex-1 items-center gap-1.5 rounded-md border px-3 text-left transition-colors",
          recording
            ? "border-primary bg-accent animate-pulse"
            : "border-input bg-background hover:bg-accent/50"
        )}
      >
        <Keyboard className="h-4 w-4 shrink-0 text-muted-foreground" />
        {recording ? (
          <span className="text-[12.5px] text-muted-foreground">
            按住修饰键再点主键；单录修饰键：按一下松开
          </span>
        ) : chips.length > 0 ? (
          <span className="flex flex-wrap items-center gap-1">
            {chips.map((c, i) => (
              <span key={i} className="flex items-center gap-1">
                {i > 0 && (
                  <span className="text-[11px] text-muted-foreground">+</span>
                )}
                <span className="rounded border border-border bg-muted px-1.5 py-0.5 text-[11.5px] font-medium">
                  {c}
                </span>
              </span>
            ))}
          </span>
        ) : (
          <span className="text-[12.5px] text-muted-foreground">
            点击录制快捷键
          </span>
        )}
      </button>
      {keyName && !recording && (
        <button
          type="button"
          onClick={onClear}
          className="text-muted-foreground hover:text-foreground"
          title="清除"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      )}
    </div>
  );
}
