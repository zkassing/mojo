import { useMemo, useState } from "react";
import {
  Power,
  Mic,
  ChevronUp,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Menu,
  Home,
  Plus,
  Minus,
  Tv,
  Save,
  Loader2,
  TriangleAlert,
} from "lucide-react";
import { toast } from "sonner";
import { useConfig } from "../lib/config-context";
import { BUTTON_LABELS } from "../lib/types";
import type { Action, ButtonName, Profile } from "../lib/types";
import { describeAction } from "../lib/actions";
import ActionEditor from "../components/ActionEditor";
import ProfileDialog from "../components/ProfileDialog";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";

type Slot = "tap" | "long" | "double";
const SLOT_LABELS: Record<Slot, string> = {
  tap: "单击",
  long: "长按",
  double: "双击",
};

export default function MappingPage() {
  const { config, setConfig, save, dirty, saving } = useConfig();
  const [profileName, setProfileName] = useState("默认配置");
  const [selected, setSelected] = useState<ButtonName | null>(null);

  const profile = useMemo(
    () => config?.profiles.find((p) => p.name === profileName),
    [config, profileName]
  );

  if (!config || !profile) {
    return <PageShell title="按键映射">加载中…</PageShell>;
  }

  const bindingFor = (b: string) => profile.bindings[b] ?? {};
  const hasAny = (b: string) => {
    const x = profile.bindings[b];
    return x && (x.tap || x.long || x.double);
  };

  const updateBinding = (
    button: string,
    slot: Slot | "repeat",
    value: Action | undefined | boolean
  ) => {
    setConfig((c) => {
      const pf = mustProfile(c, profileName);
      const cur = pf.bindings[button] ?? {};
      const next = { ...cur };
      if (slot === "repeat") {
        next.repeat = value ? true : undefined;
      } else {
        next[slot] = (value as Action) ?? undefined;
      }
      if (
        !next.tap &&
        !next.long &&
        !next.double &&
        next.repeat === undefined
      ) {
        delete pf.bindings[button];
      } else {
        pf.bindings[button] = next;
      }
      return c;
    });
  };

  const doSave = async () => {
    try {
      await save();
      toast.success("配置已保存，守护进程会自动热重载");
    } catch (e) {
      toast.error(String(e));
    }
  };

  const selBinding = selected ? bindingFor(selected) : null;

  return (
    <PageShell
      title="按键映射"
      desc="点击遥控器上的按键，分别配置单击、长按、双击的动作。"
    >
      <div className="mb-5 flex flex-wrap items-center gap-2">
        {config.profiles.map((p) => (
          <Button
            key={p.name}
            variant={p.name === profileName ? "default" : "outline"}
            size="sm"
            onClick={() => {
              setProfileName(p.name);
              setSelected(null);
            }}
          >
            {p.name}
          </Button>
        ))}
        <ProfileDialog
          active={profileName}
          onActiveChange={(name) => {
            setProfileName(name);
            setSelected(null);
          }}
        />
      </div>

      <div className="grid grid-cols-[248px_1fr] gap-8">
        {/* 遥控器（按实物布局：银机身 / 黑键） */}
        <div className="mx-auto w-[224px] self-start rounded-[30px] border border-zinc-300 bg-gradient-to-b from-zinc-100 to-zinc-300 p-4 shadow-md dark:border-zinc-700">
          {/* 顶部：电源 / 麦克风（圆形描边） */}
          <div className="flex items-start justify-between px-1">
            <RoundGhost
              icon={Power}
              bound={!!hasAny("power")}
              selected={selected === "power"}
              onClick={() => setSelected("power")}
            />
            <RoundGhost
              icon={Mic}
              bound={!!hasAny("voice")}
              selected={selected === "voice"}
              onClick={() => setSelected("voice")}
            />
          </div>

          {/* 大圆形方向盘 */}
          <div className="relative mx-auto my-4 h-[172px] w-[172px] overflow-hidden rounded-full bg-zinc-800 shadow-inner">
            <DpadQuad
              icon={ChevronUp}
              bound={!!hasAny("up")}
              selected={selected === "up"}
              onClick={() => setSelected("up")}
              className="left-1/4 top-0 h-1/2 w-1/2 items-start pt-1.5"
            />
            <DpadQuad
              icon={ChevronDown}
              bound={!!hasAny("down")}
              selected={selected === "down"}
              onClick={() => setSelected("down")}
              className="bottom-0 left-1/4 h-1/2 w-1/2 items-end pb-1.5"
            />
            <DpadQuad
              icon={ChevronLeft}
              bound={!!hasAny("left")}
              selected={selected === "left"}
              onClick={() => setSelected("left")}
              className="left-0 top-1/4 h-1/2 w-1/2 items-center justify-start pl-2.5"
            />
            <DpadQuad
              icon={ChevronRight}
              bound={!!hasAny("right")}
              selected={selected === "right"}
              onClick={() => setSelected("right")}
              className="right-0 top-1/4 h-1/2 w-1/2 items-center justify-end pr-2.5"
            />
            <button
              onClick={() => setSelected("ok")}
              className={cn(
                "absolute left-1/2 top-1/2 z-10 flex h-[88px] w-[88px] -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded-full border border-zinc-600 bg-zinc-700 text-[13px] font-semibold text-zinc-100 transition-colors hover:bg-zinc-600",
                selected === "ok" && SelRing
              )}
            >
              OK
              {hasAny("ok") && <BoundDot />}
            </button>
          </div>

          {/* 下方两列 */}
          <div className="grid grid-cols-2 gap-x-3 gap-y-3">
            {/* 左列：返回 / 主页 / 菜单 */}
            <div className="flex flex-col items-center gap-3">
              <RoundDark
                icon={ChevronLeft}
                bound={!!hasAny("back")}
                selected={selected === "back"}
                onClick={() => setSelected("back")}
              />
              <RoundDark
                icon={Home}
                bound={!!hasAny("home")}
                selected={selected === "home"}
                onClick={() => setSelected("home")}
              />
              <RoundDark
                icon={Menu}
                bound={!!hasAny("menu")}
                selected={selected === "menu"}
                onClick={() => setSelected("menu")}
              />
            </div>

            {/* 右列：连体音量键 + TV */}
            <div className="flex flex-col gap-3">
              <div className="mx-auto w-[52px] overflow-hidden rounded-[22px] bg-zinc-800 shadow-inner">
                <RockerHalf
                  icon={Plus}
                  bound={!!hasAny("volup")}
                  selected={selected === "volup"}
                  onClick={() => setSelected("volup")}
                  className="h-[56px] rounded-none"
                />
                <div className="h-px bg-zinc-700" />
                <RockerHalf
                  icon={Minus}
                  bound={!!hasAny("voldown")}
                  selected={selected === "voldown"}
                  onClick={() => setSelected("voldown")}
                  className="h-[56px] rounded-none"
                />
              </div>
              <div className="flex justify-center">
                <RoundDark
                  icon={Tv}
                  bound={!!hasAny("tv")}
                  selected={selected === "tv"}
                  onClick={() => setSelected("tv")}
                />
              </div>
            </div>
          </div>
        </div>

        {/* 编辑面板 */}
        <div>
          {!selected || !selBinding ? (
            <div className="rounded-xl border border-dashed border-border p-10 text-center text-sm text-muted-foreground">
              在左侧选择一个按键开始配置
            </div>
          ) : (
            <div className="rounded-xl border border-border bg-card p-5">
              <div className="mb-4 flex items-center justify-between">
                <h2 className="text-base font-semibold">
                  {BUTTON_LABELS[selected]} 键
                </h2>
                <Badge variant="outline" className="font-mono text-[10.5px]">
                  {(config.buttons[selected] ?? []).join(", ")}
                </Badge>
              </div>

              <div className="space-y-3">
                {(["tap", "long", "double"] as Slot[]).map((slot) => (
                  <SlotRow
                    key={slot}
                    label={SLOT_LABELS[slot]}
                    action={selBinding[slot]}
                    onChange={(a) => updateBinding(selected, slot, a)}
                  />
                ))}
              </div>

              <div className="mt-4 flex flex-wrap items-center gap-3 border-t border-border pt-4">
                <Switch
                  id="repeat"
                  checked={selBinding.repeat === true}
                  disabled={!!(selBinding.long || selBinding.double)}
                  onCheckedChange={(v) =>
                    updateBinding(selected, "repeat", v)
                  }
                />
                <Label
                  htmlFor="repeat"
                  className={cn(
                    "text-[13px]",
                    (selBinding.long || selBinding.double) &&
                      "text-muted-foreground"
                  )}
                >
                  按住连发（方向键等）
                </Label>
                {(selBinding.long || selBinding.double) && (
                  <Badge variant="secondary" className="gap-1 text-[11px]">
                    <TriangleAlert className="h-3 w-3" />
                    配了长按/双击时连发失效
                  </Badge>
                )}
              </div>
            </div>
          )}
        </div>
      </div>

      <SaveBar dirty={dirty} saving={saving} onSave={doSave} />
    </PageShell>
  );
}

function SlotRow({
  label,
  action,
  onChange,
}: {
  label: string;
  action: Action | undefined;
  onChange: (a: Action | undefined) => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div className="rounded-lg border border-border bg-background/40 p-3">
      <div className="flex items-center justify-between gap-3">
        <span className="w-10 shrink-0 text-[13px] font-medium">{label}</span>
        <span className="flex-1 truncate text-[12.5px] text-muted-foreground">
          {describeAction(action)}
        </span>
        <Button
          variant="outline"
          size="sm"
          onClick={() => setOpen((o) => !o)}
        >
          {open ? "收起" : action ? "编辑" : "添加"}
        </Button>
      </div>
      {open && (
        <div className="mt-2.5">
          <ActionEditor value={action} onChange={onChange} compact />
        </div>
      )}
    </div>
  );
}

// 选中时的高亮环（黑键上用青色，亮/暗主题都清晰）
const SelRing =
  "ring-2 ring-sky-400 ring-offset-2 ring-offset-zinc-800 outline-none";

// 已配置动作的小圆点
function BoundDot() {
  return (
    <span className="absolute right-1.5 top-1.5 h-2 w-2 rounded-full bg-emerald-400 shadow" />
  );
}

// 顶部圆形描边键：电源 / 麦克风
function RoundGhost({
  icon: Icon,
  selected,
  bound,
  onClick,
}: {
  icon: typeof Power;
  selected: boolean;
  bound: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "relative flex h-12 w-12 items-center justify-center rounded-full border-2 border-zinc-400 text-zinc-600 transition-colors hover:border-zinc-600 hover:bg-black/5",
        selected && "border-sky-500 text-sky-600 ring-2 ring-sky-400/40"
      )}
    >
      <Icon className="h-5 w-5" />
      {bound && (
        <span
          className={cn(
            "absolute -right-0.5 -top-0.5 h-2.5 w-2.5 rounded-full bg-emerald-500",
            selected ? "" : "border-2 border-zinc-200"
          )}
        />
      )}
    </button>
  );
}

// 黑色圆形实体键：返回 / 主页 / 菜单
function RoundDark({
  icon: Icon,
  selected,
  bound,
  onClick,
}: {
  icon: typeof Power;
  selected: boolean;
  bound: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "relative flex h-[52px] w-[52px] items-center justify-center rounded-full bg-zinc-800 text-zinc-100 shadow transition-colors hover:bg-zinc-700",
        selected && SelRing
      )}
    >
      <Icon className="h-5 w-5" />
      {bound && <BoundDot />}
    </button>
  );
}

// 方向盘四分之一热区
function DpadQuad({
  icon: Icon,
  selected,
  bound,
  onClick,
  className,
}: {
  icon: typeof Power;
  selected: boolean;
  bound: boolean;
  onClick: () => void;
  className?: string;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "absolute flex justify-center text-zinc-300 transition-colors hover:bg-white/10 hover:text-white",
        selected && "bg-sky-500/25 text-white",
        className
      )}
    >
      <Icon className="h-5 w-5" />
      {bound && (
        <span
          className={cn(
            "absolute h-1.5 w-1.5 rounded-full bg-emerald-400",
            className?.includes("items-start") && "top-1",
            className?.includes("items-end") && "bottom-1",
            className?.includes("justify-start") &&
              "left-1 top-1/2 -translate-y-1/2",
            className?.includes("justify-end") &&
              "right-1 top-1/2 -translate-y-1/2",
            // 垂直方向的两个点居中
            !className?.includes("justify-start") &&
              !className?.includes("justify-end") &&
              "left-1/2 -translate-x-1/2"
          )}
        />
      )}
    </button>
  );
}

// 连体音量键的一半
function RockerHalf({
  icon: Icon,
  selected,
  bound,
  onClick,
  className,
}: {
  icon: typeof Power;
  selected: boolean;
  bound: boolean;
  onClick: () => void;
  className?: string;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "relative flex w-full items-center justify-center text-zinc-200 transition-colors hover:bg-zinc-700",
        selected && SelRing,
        className
      )}
    >
      <Icon className="h-5 w-5" />
      {bound && <BoundDot />}
    </button>
  );
}

export function PageShell({
  title,
  desc,
  children,
}: {
  title: string;
  desc?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="px-9 py-7">
      <div className="mb-6">
        <h1 className="text-[21px] font-semibold">{title}</h1>
        {desc && (
          <p className="mt-1 text-[13px] text-muted-foreground">{desc}</p>
        )}
      </div>
      {children}
    </div>
  );
}

export function SaveBar({
  dirty,
  saving,
  onSave,
}: {
  dirty: boolean;
  saving: boolean;
  onSave: () => void;
}) {
  return (
    <div className="sticky bottom-0 -mx-9 mt-6 flex items-center gap-3 border-t border-border bg-background/90 px-9 py-3 backdrop-blur">
      <Button onClick={onSave} disabled={!dirty || saving}>
        {saving ? (
          <Loader2 className="h-4 w-4 animate-spin" />
        ) : (
          <Save className="h-4 w-4" />
        )}
        保存并热重载
      </Button>
      <span className="text-[12.5px] text-muted-foreground">
        {dirty ? "有未保存的修改" : "所有修改已保存"}
      </span>
    </div>
  );
}

function mustProfile(c: import("../lib/types").Config, name: string): Profile {
  const p = c.profiles.find((x) => x.name === name);
  if (!p) throw new Error("方案不存在: " + name);
  return p;
}
