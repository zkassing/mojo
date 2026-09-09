import { useEffect, useState } from "react";
import {
  Plus,
  X,
  Trash2,
  Settings2,
  AppWindow,
  ChevronsUpDown,
  FolderOpen,
  Loader2,
} from "lucide-react";
import { toast } from "sonner";
import { useConfig } from "../lib/config-context";
import { api } from "../lib/api";
import type { AppInfo, Profile } from "../lib/types";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";

interface Props {
  /** 当前选中的方案名 */
  active: string;
  /** 切换当前方案（改名 / 选中其它方案时通知映射页） */
  onActiveChange: (name: string) => void;
  /** 弹窗内修改后通知保存条刷新脏状态用的触发器 */
  onDirtyChange?: () => void;
  trigger?: React.ReactNode;
}

export default function ProfileDialog({
  active,
  onActiveChange,
  onDirtyChange,
  trigger,
}: Props) {
  const { config, setConfig } = useConfig();
  const [open, setOpen] = useState(false);
  const [sel, setSel] = useState(0);
  const [idInput, setIdInput] = useState("");
  const [nameInput, setNameInput] = useState("");
  const [platform, setPlatform] = useState("macos");
  const [picking, setPicking] = useState(false);

  useEffect(() => {
    api.platform().then(setPlatform).catch(() => {});
  }, []);

  // 打开时同步到当前方案
  useEffect(() => {
    if (open && config) {
      const i = Math.max(
        0,
        config.profiles.findIndex((p) => p.name === active)
      );
      setSel(i);
    }
  }, [open, config, active]);

  if (!config) return null;

  const profile: Profile | undefined = config.profiles[sel];

  const updateProfile = (fn: (p: Profile) => void) => {
    setConfig((c) => {
      fn(c.profiles[sel]);
      return c;
    });
    onDirtyChange?.();
  };

  const addProfile = () => {
    const name = `方案${config.profiles.length + 1}`;
    setConfig((c) => {
      c.profiles.push({ name, bindings: {} });
      return c;
    });
    setSel(config.profiles.length);
    onActiveChange(name);
    onDirtyChange?.();
  };

  const removeProfile = () => {
    if (config.profiles.length <= 1) {
      toast.error("至少保留一个方案");
      return;
    }
    setConfig((c) => {
      c.profiles.splice(sel, 1);
      return c;
    });
    const next = Math.max(0, sel - 1);
    setSel(next);
    onActiveChange(config.profiles[next]?.name ?? "");
    onDirtyChange?.();
  };

  const rename = (name: string) => {
    updateProfile((p) => (p.name = name));
    onActiveChange(name);
  };

  const bundleIds = profile?.match?.bundleIds ?? [];
  const appNames = profile?.match?.appNames ?? [];
  const isFallback =
    profile && sel === config.profiles.length - 1 && !profile.match;

  /// 把选中的应用加进匹配列表（优先 Bundle ID，没有就退用应用名）
  const addAppInfo = (info: AppInfo) => {
    let added: string | null = null;
    updateProfile((p) => {
      p.match = p.match ?? {};
      if (info.bundleId) {
        const list = p.match.bundleIds ?? [];
        if (!list.includes(info.bundleId!)) {
          p.match.bundleIds = [...list, info.bundleId!];
          added = info.bundleId;
        }
      } else if (info.appName) {
        const list = p.match.appNames ?? [];
        if (!list.includes(info.appName!)) {
          p.match.appNames = [...list, info.appName!];
          added = info.appName;
        }
      }
    });
    if (added) {
      toast.success(`已添加：${info.appName ?? added}`);
    } else {
      toast.info("该应用已在列表中");
    }
  };

  /// 文件选择器兜底（应用没安装在标准目录时用）
  const browseApp = async () => {
    setPicking(true);
    try {
      const info = await api.pickApp(platform);
      if (info) addAppInfo(info);
    } catch (e) {
      toast.error(String(e));
    } finally {
      setPicking(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        {trigger ?? (
          <Button variant="outline" size="sm">
            <Settings2 className="h-4 w-4" />
            方案设置
          </Button>
        )}
      </DialogTrigger>

      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle>方案管理</DialogTitle>
          <DialogDescription>
            按当前前台应用自动切换方案；没有匹配时使用最后的兜底方案。
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-wrap items-center gap-2">
          {config.profiles.map((p, i) => (
            <Button
              key={i}
              variant={i === sel ? "default" : "outline"}
              size="sm"
              onClick={() => setSel(i)}
            >
              {p.name}
            </Button>
          ))}
          <Button variant="ghost" size="sm" onClick={addProfile}>
            <Plus className="h-4 w-4" />
            新建
          </Button>
        </div>

        {profile && (
          <Card>
            <CardContent className="space-y-5 pt-6">
              <div className="space-y-1.5">
                <Label>方案名称</Label>
                <Input
                  value={profile.name}
                  onChange={(e) => rename(e.target.value)}
                />
              </div>

              <div className="space-y-2">
                <Label>匹配的应用</Label>
                <AppPicker onPick={addAppInfo} />
                <div className="flex flex-wrap gap-1.5">
                  {bundleIds.map((v, i) => (
                    <span
                      key={v + i}
                      className="inline-flex items-center gap-1.5 rounded-full bg-secondary px-2.5 py-1 text-[12px] text-secondary-foreground"
                    >
                      <span className="font-mono text-[11px]">{v}</span>
                      <button
                        onClick={() =>
                          updateProfile((p) => p.match?.bundleIds?.splice(i, 1))
                        }
                        className="text-muted-foreground hover:text-foreground"
                      >
                        <X className="h-3 w-3" />
                      </button>
                    </span>
                  ))}
                  {bundleIds.length === 0 && (
                    <span className="text-[12px] text-muted-foreground">
                      （空 — 此方案不匹配特定应用，可作兜底）
                    </span>
                  )}
                </div>
                <div className="flex gap-2">
                  <Input
                    className="h-8 flex-1 font-mono text-[12px]"
                    placeholder="手动输入应用标识（一般不用）"
                    value={idInput}
                    onChange={(e) => setIdInput(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && idInput.trim()) {
                        updateProfile((p) => {
                          p.match = p.match ?? {};
                          p.match.bundleIds = [
                            ...(p.match.bundleIds ?? []),
                            idInput.trim(),
                          ];
                        });
                        setIdInput("");
                      }
                    }}
                  />
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() => {
                      if (!idInput.trim()) return;
                      updateProfile((p) => {
                        p.match = p.match ?? {};
                        p.match.bundleIds = [
                          ...(p.match.bundleIds ?? []),
                          idInput.trim(),
                        ];
                      });
                      setIdInput("");
                    }}
                  >
                    <Plus className="h-3.5 w-3.5" />
                    添加
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={picking}
                    onClick={browseApp}
                    title="从磁盘浏览 .app 文件"
                  >
                    {picking ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <FolderOpen className="h-3.5 w-3.5" />
                    )}
                    浏览…
                  </Button>
                </div>
              </div>

              <TagList
                title="按进程名匹配（高级，通常留空）"
                hint="如 stable（Warp 的进程名）"
                values={appNames}
                input={nameInput}
                setInput={setNameInput}
                onAdd={(v) =>
                  updateProfile((p) => {
                    p.match = p.match ?? {};
                    p.match.appNames = [...(p.match.appNames ?? []), v];
                  })
                }
                onRemove={(i) =>
                  updateProfile((p) => p.match?.appNames?.splice(i, 1))
                }
              />

              {isFallback && (
                <p className="rounded-md bg-muted px-3 py-2 text-[12.5px] text-muted-foreground">
                  此方案没有匹配条件，将作为所有未命中应用的兜底方案。
                </p>
              )}

              <Button variant="destructive" onClick={removeProfile}>
                <Trash2 className="h-4 w-4" />
                删除此方案
              </Button>
            </CardContent>
          </Card>
        )}

        <DialogFooter className="text-[12px] text-muted-foreground">
          修改会随映射页一起「保存并热重载」。
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/// 已安装应用搜索选择器：按名字点选，Bundle ID 在背后自动填
function AppPicker({ onPick }: { onPick: (app: AppInfo) => void }) {
  const [open, setOpen] = useState(false);
  const [apps, setApps] = useState<AppInfo[] | null>(null);

  useEffect(() => {
    if (open && apps === null) {
      api
        .listApps()
        .then(setApps)
        .catch(() => setApps([]));
    }
  }, [open, apps]);

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          size="sm"
          className="w-full justify-between font-normal"
        >
          <span className="flex items-center gap-2">
            <AppWindow className="h-4 w-4" />
            选择应用…
          </span>
          <ChevronsUpDown className="h-3.5 w-3.5 opacity-50" />
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-[380px] p-0" align="start">
        <Command>
          <CommandInput placeholder="搜索应用名…" />
          <CommandList>
            <CommandEmpty>
              {apps === null ? "正在扫描已安装应用…" : "没有匹配的应用"}
            </CommandEmpty>
            <CommandGroup>
              {(apps ?? []).map((a) => (
                <CommandItem
                  key={a.path}
                  value={`${a.appName ?? ""} ${a.bundleId ?? ""}`}
                  onSelect={() => {
                    onPick(a);
                    setOpen(false);
                  }}
                >
                  <div className="flex min-w-0 flex-col">
                    <span className="truncate text-[13px]">
                      {a.appName ?? a.bundleId ?? a.path}
                    </span>
                    {a.bundleId && (
                      <span className="truncate font-mono text-[10.5px] text-muted-foreground">
                        {a.bundleId}
                      </span>
                    )}
                  </div>
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}

function TagList({
  title,
  hint,
  values,
  input,
  setInput,
  onAdd,
  onRemove,
}: {
  title: string;
  hint: string;
  values: string[];
  input: string;
  setInput: (s: string) => void;
  onAdd: (v: string) => void;
  onRemove: (i: number) => void;
}) {
  const commit = () => {
    if (input.trim()) {
      onAdd(input.trim());
      setInput("");
    }
  };
  return (
    <div className="space-y-2">
      <Label>{title}</Label>
      <div className="flex flex-wrap gap-1.5">
        {values.map((v, i) => (
          <span
            key={v + i}
            className="inline-flex items-center gap-1.5 rounded-full bg-secondary px-2.5 py-1 text-[12px] text-secondary-foreground"
          >
            <span className="font-mono text-[11px]">{v}</span>
            <button
              onClick={() => onRemove(i)}
              className="text-muted-foreground hover:text-foreground"
            >
              <X className="h-3 w-3" />
            </button>
          </span>
        ))}
        {values.length === 0 && (
          <span className="text-[12px] text-muted-foreground">（空）</span>
        )}
      </div>
      <div className="flex gap-2">
        <Input
          className="h-8 flex-1 font-mono text-[12px]"
          placeholder={hint}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && commit()}
        />
        <Button size="sm" variant="secondary" onClick={commit}>
          <Plus className="h-3.5 w-3.5" />
          添加
        </Button>
      </div>
    </div>
  );
}
