import { useCallback, useEffect, useState } from "react";
import { Square, AlertTriangle, Play, Cpu, RefreshCw, Download, ShieldAlert } from "lucide-react";
import { toast } from "sonner";
import { relaunch } from "@tauri-apps/plugin-process";
import { api } from "../lib/api";
import { checkForUpdate } from "../lib/updater";
import type { EngineStatus } from "../lib/types";
import { PageShell } from "./MappingPage";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";

function PermissionWarn({
  title,
  desc,
  onClick,
}: {
  title: string;
  desc: string;
  onClick: () => void;
}) {
  return (
    <div className="flex items-start gap-3">
      <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0 text-red-400" />
      <div className="flex-1">
        <div className="text-[13px] font-medium text-red-300">{title}</div>
        <p className="mt-0.5 text-[12px] leading-relaxed text-muted-foreground">
          {desc}
        </p>
      </div>
      <Button size="sm" variant="outline" onClick={onClick}>
        去授权
      </Button>
    </div>
  );
}

export default function ServicePage() {
  const [engine, setEngine] = useState<EngineStatus | null>(null);
  const [platform, setPlatform] = useState("macos");
  const [supported, setSupported] = useState(true);
  const [busy, setBusy] = useState(false);
  const [checking, setChecking] = useState(false);
  const [perms, setPerms] = useState<{
    accessibility: boolean | null;
    inputMonitoring: boolean | null;
  } | null>(null);

  const refresh = useCallback(async () => {
    setEngine(await api.engineRuntimeStatus().catch(() => null));
  }, []);

  useEffect(() => {
    api.platform().then(setPlatform).catch(() => {});
    api.engineSupported().then(setSupported).catch(() => {});
    refresh();
    const t = setInterval(refresh, 2500);
    return () => clearInterval(t);
  }, [refresh]);

  // macOS 权限状态轮询（授权/重构建 App 后权限可能变化）
  useEffect(() => {
    if (platform !== "macos") return;
    const query = () =>
      api
        .permissionStatus()
        .then(setPerms)
        .catch(() => setPerms(null));
    query();
    const t = setInterval(query, 3000);
    return () => clearInterval(t);
  }, [platform]);

  const noInput = perms?.inputMonitoring === false;
  const noAx = perms?.accessibility === false;

  const engineRunning = engine?.kind === "running";

  const engineToggle = async () => {
    setBusy(true);
    try {
      if (engineRunning) {
        await api.engineStop();
        toast("引擎已停止");
      } else {
        await api.engineStart();
        toast.success("引擎已启动");
      }
      setTimeout(refresh, 500);
    } catch (e) {
      toast.error(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <PageShell
      title="服务与状态"
      desc="引擎、权限与更新。"
    >
      <div className="max-w-2xl space-y-5">
        {(noInput || noAx) && (
          <Card className="border-red-500/40 bg-red-500/5">
            <CardContent className="space-y-3 pt-5">
              {noAx && (
                <PermissionWarn
                  title="缺少「辅助功能」权限"
                  desc="按键映射不会生效。"
                  onClick={() => api.openPermissionSettings("accessibility")}
                />
              )}
              {noInput && (
                <PermissionWarn
                  title="缺少「输入监控」权限"
                  desc="返回键（back）需要此权限，否则无反应。"
                  onClick={() => api.openPermissionSettings("input")}
                />
              )}
              <p className="text-[12px] leading-relaxed text-muted-foreground">
                授权后请重启引擎生效。开关已打开却仍提示：授权条目已失效，需重置。
              </p>
              <Button
                size="sm"
                variant="outline"
                onClick={async () => {
                  try {
                    await api.resetPermissions();
                    toast.success("已清除失效的授权条目，应用即将重启，请重新授权");
                    setTimeout(() => relaunch(), 1200);
                  } catch (e) {
                    toast.error(String(e));
                  }
                }}
              >
                重置授权并重启
              </Button>
            </CardContent>
          </Card>
        )}
        {!supported && (
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2 text-[15px]">
                <AlertTriangle className="h-4 w-4 text-muted-foreground" />
                {platform} 支持状态
              </CardTitle>
            </CardHeader>
            <CardContent>
              <p className="text-[13px] leading-relaxed text-muted-foreground">
                该平台引擎尚未就绪，可正常编辑配置，引擎支持后生效。
              </p>
            </CardContent>
          </Card>
        )}

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center justify-between text-[15px]">
              <span className="flex items-center gap-2">
                <Cpu className="h-4 w-4 text-muted-foreground" />
                引擎
              </span>
              {engineRunning ? (
                <Badge variant="secondary" className="gap-1.5">
                  <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
                  运行中
                  {engine.kind === "running" && engine.remoteConnected
                    ? " · 遥控器已连接"
                    : ""}
                </Badge>
              ) : (
                <Badge variant="secondary" className="gap-1.5">
                  <span className="h-1.5 w-1.5 rounded-full bg-zinc-400" />
                  未运行
                </Badge>
              )}
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="flex gap-2.5">
              <Button
                variant={engineRunning ? "outline" : "default"}
                onClick={engineToggle}
                disabled={busy}
              >
                {engineRunning ? (
                  <Square className="h-4 w-4" />
                ) : (
                  <Play className="h-4 w-4" />
                )}
                {engineRunning ? "停止引擎" : "启动引擎"}
              </Button>
              <Button variant="ghost" onClick={refresh}>
                <RefreshCw className="h-4 w-4" />
                刷新状态
              </Button>
            </div>
            <p className="text-[12px] leading-relaxed text-muted-foreground">
              首次使用需在「辅助功能 / 输入监控 / 蓝牙」中授权。
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-[15px]">
              <Download className="h-4 w-4 text-muted-foreground" />
              应用更新
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="flex gap-2.5">
              <Button
                variant="outline"
                onClick={() => {
                  setChecking(true);
                  checkForUpdate(false).finally(() => setChecking(false));
                }}
                disabled={checking}
              >
                <RefreshCw
                  className={`h-4 w-4 ${checking ? "animate-spin" : ""}`}
                />
                {checking ? "检查中…" : "检查更新"}
              </Button>
            </div>
            <p className="text-[12px] leading-relaxed text-muted-foreground">
              启动时自动检查；更新需重启生效。
            </p>
          </CardContent>
        </Card>
      </div>
    </PageShell>
  );
}
