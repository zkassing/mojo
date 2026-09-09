import { useCallback, useEffect, useState } from "react";
import { RefreshCw, Square, RotateCw, AlertTriangle } from "lucide-react";
import { toast } from "sonner";
import { api } from "../lib/api";
import type { ServiceStatus } from "../lib/types";
import { PageShell } from "./MappingPage";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";

export default function ServicePage() {
  const [st, setSt] = useState<ServiceStatus | null>(null);
  const [platform, setPlatform] = useState("macos");
  const [supported, setSupported] = useState(true);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    setSt(await api.serviceStatus().catch(() => null));
  }, []);

  useEffect(() => {
    api.platform().then(setPlatform).catch(() => {});
    api.engineSupported().then(setSupported).catch(() => {});
    refresh();
    const t = setInterval(refresh, 2500);
    return () => clearInterval(t);
  }, [refresh]);

  const restart = async () => {
    setBusy(true);
    try {
      await api.serviceRestart();
      toast.success("守护进程已重启");
      setTimeout(refresh, 800);
    } catch (e) {
      toast.error(String(e));
    } finally {
      setBusy(false);
    }
  };

  const stop = async () => {
    setBusy(true);
    try {
      await api.serviceStop();
      toast("已发送停止信号");
      setTimeout(refresh, 800);
    } catch (e) {
      toast.error(String(e));
    } finally {
      setBusy(false);
    }
  };

  const statusBadge = st?.running ? (
    <Badge variant="secondary" className="gap-1.5">
      <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
      运行中
    </Badge>
  ) : st?.installed ? (
    <Badge variant="secondary" className="gap-1.5">
      <span className="h-1.5 w-1.5 rounded-full bg-amber-500" />
      已安装未运行
    </Badge>
  ) : (
    <Badge variant="destructive" className="gap-1.5">
      <span className="h-1.5 w-1.5 rounded-full" />
      未运行
    </Badge>
  );

  return (
    <PageShell
      title="服务与状态"
      desc="后台守护进程负责蓝牙连接、按键拦截和语音识别。"
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
              守护进程
              {statusBadge}
            </CardTitle>
          </CardHeader>
          <CardContent>
            <div className="flex gap-2.5">
              <Button onClick={restart} disabled={busy}>
                <RotateCw className="h-4 w-4" />
                重启守护进程
              </Button>
              <Button variant="outline" onClick={stop} disabled={busy}>
                <Square className="h-4 w-4" />
                停止
              </Button>
              <Button variant="ghost" onClick={refresh}>
                <RefreshCw className="h-4 w-4" />
                刷新状态
              </Button>
            </div>
          </CardContent>
        </Card>
      </div>
    </PageShell>
  );
}
