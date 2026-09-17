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
      desc="内置引擎负责蓝牙连接、按键拦截和语音识别，常驻托盘运行。"
    >
      <div className="max-w-2xl space-y-5">
        {(noInput || noAx) && (
          <Card className="border-red-500/40 bg-red-500/5">
            <CardContent className="space-y-3 pt-5">
              {noAx && (
                <PermissionWarn
                  title="缺少「辅助功能」权限"
                  desc="无法拦截和注入按键，方向键/确定键等映射不会生效。"
                  onClick={() => api.openPermissionSettings("accessibility")}
                />
              )}
              {noInput && (
                <PermissionWarn
                  title="缺少「输入监控」权限"
                  desc="返回键（back，usage 0xF1）只能通过 HID 直读通道接收，未授权时该键完全无反应。"
                  onClick={() => api.openPermissionSettings("input")}
                />
              )}
              <p className="text-[12px] leading-relaxed text-muted-foreground">
                授权后请在本页点「停止引擎」再「启动引擎」（或退出重开）。
                若列表里已有 Mojo 且开关已打开却仍提示：说明旧授权条目已失效
                （未签名的 App 每次更新都会这样），需删掉旧条目重新授权 ——
                可直接点下方按钮一键重置。
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
                当前平台的原生按键/蓝牙内核正在移植中（计划：btleplug +
                平台钩子）。配置、方案、火山连接测试可正常使用，保存的配置将在引擎就绪后生效。
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
              按键映射 + 语音转文字均由内置引擎处理。首次启动需在
              「辅助功能」「输入监控」「蓝牙」中授权本应用。
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
              启动时会自动检查；新版本下载完成后需重启应用生效。
            </p>
          </CardContent>
        </Card>
      </div>
    </PageShell>
  );
}
