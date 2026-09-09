import { useEffect, useState } from "react";
import { Command, X } from "lucide-react";
import KeyCapture from "./KeyCapture";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { Action, FullAction } from "../lib/types";

const ACTION_TYPES: { value: string; label: string }[] = [
  { value: "key", label: "模拟按键 / 快捷键" },
  { value: "media", label: "系统媒体控制" },
  { value: "shell", label: "执行 Shell 命令" },
  { value: "open", label: "打开 App / URL" },
  { value: "mouseMove", label: "鼠标移动" },
  { value: "mouseClick", label: "鼠标点击" },
  { value: "mouseScroll", label: "鼠标滚轮" },
  { value: "dictate", label: "语音转文字" },
  { value: "sequence", label: "动作序列" },
  { value: "none", label: "忽略（吞掉）" },
  { value: "passthrough", label: "放行原键" },
];

const MODS = ["cmd", "ctrl", "alt", "shift", "fn"];
const MEDIA_KEYS = [
  "playpause",
  "volup",
  "voldown",
  "next",
  "previous",
  "mute",
];

function toFull(a: Action | undefined): FullAction {
  if (!a) return { type: "none" };
  if (typeof a === "string") {
    if (["dictate", "voice", "speech", "asr"].includes(a.toLowerCase()))
      return { type: "dictate" };
    const parts = a.split("+").map((p) => p.trim());
    const key = parts.pop() ?? "";
    const mods = parts.map((p) => p.toLowerCase());
    return { type: "key", key, mods };
  }
  return a;
}

export function actionToStorable(a: FullAction): Action {
  // 归一：只有修饰键、没有主键时，把最后一个修饰键当作主键（单按修饰键）
  // 例：mods=["fn"] 无 key → "fn"；mods=["ctrl","cmd"] 无 key → "ctrl+cmd"
  if (a.type === "key" && !a.key && (a.mods ?? []).length > 0) {
    const mods = [...(a.mods ?? [])];
    const key = mods.pop()!;
    a = { ...a, key, mods };
  }
  if (
    a.type === "key" &&
    a.key &&
    !(a.mods ?? []).some((m) => !MODS.includes(m))
  ) {
    const joined = [...(a.mods ?? []), a.key].join("+");
    return joined;
  }
  const out: FullAction = { type: a.type };
  for (const k of [
    "key",
    "mods",
    "command",
    "target",
    "dx",
    "dy",
    "button",
    "count",
    "actions",
  ] as const) {
    const v = a[k];
    if (v !== undefined && v !== "" && !(Array.isArray(v) && v.length === 0)) {
      // @ts-expect-error 动态拷贝
      out[k] = v;
    }
  }
  return out;
}

interface Props {
  value: Action | undefined;
  onChange: (a: Action | undefined) => void;
  compact?: boolean;
}

export default function ActionEditor({ value, onChange, compact }: Props) {
  const [a, setA] = useState<FullAction>(toFull(value));
  const [manual, setManual] = useState(false);

  useEffect(() => setA(toFull(value)), [value]);

  const patch = (p: Partial<FullAction>) => {
    const next = { ...a, ...p };
    setA(next);
    onChange(actionToStorable(next));
  };

  const isKey = ["key", "keyboard", "hotkey"].includes(a.type);
  const isMouseMove = ["mouseMove", "move"].includes(a.type);

  return (
    <div
      className="space-y-2.5 rounded-lg border border-border bg-background/50 p-3"
      onClick={(e) => e.stopPropagation()}
    >
      <div className="flex items-center gap-2">
        <Select value={a.type} onValueChange={(t) => patch({ type: t })}>
          <SelectTrigger className="h-8 flex-1 text-[13px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {ACTION_TYPES.map((t) => (
              <SelectItem key={t.value} value={t.value}>
                {t.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {value !== undefined && (
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8 shrink-0 text-muted-foreground"
            onClick={() => onChange(undefined)}
          >
            <X className="h-3.5 w-3.5" />
          </Button>
        )}
      </div>

      {isKey && (
        <div className="space-y-2">
          <KeyCapture
            keyName={a.key}
            mods={a.mods}
            onCapture={(key, mods) => patch({ key, mods })}
            onClear={() => patch({ key: undefined, mods: undefined })}
          />
          <p className="text-[11.5px] leading-relaxed text-muted-foreground">
            Cmd+Tab、Cmd+Q 等被系统吃掉的组合键录不上，
            <button
              type="button"
              className="underline underline-offset-2 hover:text-foreground"
              onClick={() => setManual((m) => !m)}
            >
              {manual ? "收起手动输入" : "手动输入"}
            </button>
          </p>
          {manual && (
            <div className="space-y-2 rounded-md border border-dashed border-border p-2.5">
              <Input
                className="h-8 font-mono text-[12px]"
                placeholder="按键名：return / tab / c / up"
                value={a.key ?? ""}
                onChange={(e) => patch({ key: e.target.value })}
              />
              <div className="flex flex-wrap gap-1.5">
                {MODS.map((m) => {
                  const on = (a.mods ?? []).includes(m);
                  return (
                    <button
                      key={m}
                      onClick={() => {
                        const cur = new Set(a.mods ?? []);
                        on ? cur.delete(m) : cur.add(m);
                        patch({ mods: [...cur] });
                      }}
                      className={cn(
                        "flex items-center gap-1 rounded-full border px-2.5 py-1 text-[11px] transition-colors",
                        on
                          ? "border-primary bg-primary text-primary-foreground"
                          : "border-input bg-background text-muted-foreground hover:bg-accent"
                      )}
                    >
                      {m === "cmd" && <Command className="h-3 w-3" />}
                      {m}
                    </button>
                  );
                })}
              </div>
            </div>
          )}
        </div>
      )}

      {a.type === "media" && (
        <Select
          value={a.key ?? "playpause"}
          onValueChange={(t) => patch({ key: t })}
        >
          <SelectTrigger className="h-8 text-[13px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {MEDIA_KEYS.map((k) => (
              <SelectItem key={k} value={k}>
                {k}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )}

      {a.type === "shell" && (
        <Input
          className="h-8 font-mono text-[12px]"
          placeholder='osascript -e "tell app …"'
          value={a.command ?? ""}
          onChange={(e) => patch({ command: e.target.value })}
        />
      )}

      {a.type === "open" && (
        <Input
          className="h-8 font-mono text-[12px]"
          placeholder="App 名 / bundle id / https://… / 路径"
          value={a.target ?? ""}
          onChange={(e) => patch({ target: e.target.value })}
        />
      )}

      {(isMouseMove || a.type === "mouseScroll") && (
        <div className="grid grid-cols-2 gap-2">
          <NumField
            label={compact ? "X" : "X 位移"}
            value={a.dx ?? 0}
            onChange={(v) => patch({ dx: v })}
          />
          <NumField
            label={compact ? "Y" : "Y 位移"}
            value={a.dy ?? 0}
            onChange={(v) => patch({ dy: v })}
          />
        </div>
      )}

      {a.type === "mouseClick" && (
        <div className="grid grid-cols-2 gap-2">
          <Select
            value={a.button ?? "left"}
            onValueChange={(t) => patch({ button: t })}
          >
            <SelectTrigger className="h-8 text-[13px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="left">左键</SelectItem>
              <SelectItem value="right">右键</SelectItem>
              <SelectItem value="center">中键</SelectItem>
            </SelectContent>
          </Select>
          <NumField
            label="次数"
            value={a.count ?? 1}
            onChange={(v) => patch({ count: v })}
          />
        </div>
      )}

      {(a.type === "dictate" ||
        a.type === "none" ||
        a.type === "passthrough") && (
        <p className="text-[12px] leading-relaxed text-muted-foreground">
          {a.type === "dictate" &&
            "按住开麦说话，松开发送识别（需在「语音识别」配置火山凭证）"}
          {a.type === "none" && "按键被完全吞掉，不产生任何效果"}
          {a.type === "passthrough" && "不拦截，把原始按键交给系统"}
        </p>
      )}
    </div>
  );
}

function NumField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: number;
  onChange: (v: number) => void;
}) {
  return (
    <label className="flex flex-col gap-1 text-[11px] text-muted-foreground">
      {label}
      <Input
        type="number"
        className="h-8 text-[13px]"
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
      />
    </label>
  );
}
