import { useCallback, useEffect, useState } from "react";
import { Square, AlertTriangle, Play, Cpu, RefreshCw } from "lucide-react";
import { toast } from "sonner";
import { api } from "../lib/api";
import type { EngineStatus } from "../lib/types";
import { PageShell } from "./MappingPage";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";

export default function ServicePage() {
  const [engine, setEngine] = useState<EngineStatus | null>(null);
  const [platform, setPlatform] = useState("macos");
  const [supported, setSupported] = useState(true);
  const [busy, setBusy] = useState(false);

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
      </div>
    </PageShell>
  );
}
