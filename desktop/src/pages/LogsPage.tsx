import { useEffect, useRef, useState } from "react";
import {
  ArrowDownToLine,
  Trash2,
  XCircle,
  TriangleAlert,
  CheckCircle2,
  Mic,
} from "lucide-react";
import { api } from "../lib/api";
import { PageShell } from "./MappingPage";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";

export default function LogsPage() {
  const [lines, setLines] = useState<string[]>([]);
  const [following, setFollowing] = useState(true);
  const boxRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let alive = true;

    api.logTail(400).then((tail) => {
      if (alive && tail) setLines(tail.split("\n"));
    });
    api.logFollow();
    api.onLogLine((l) => {
      setLines((prev) => {
        const next = [...prev, l];
        return next.length > 5000 ? next.slice(next.length - 5000) : next;
      });
    }).then((u) => (unlisten = u));

    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (following && boxRef.current) {
      boxRef.current.scrollTop = boxRef.current.scrollHeight;
    }
  }, [lines, following]);

  const classify = (l: string): { icon?: typeof XCircle; cls: string } => {
    if (/错误|失败|panic|error/i.test(l))
      return { icon: XCircle, cls: "text-red-400" };
    if (/警告|warn/i.test(l))
      return { icon: TriangleAlert, cls: "text-amber-400" };
    if (/识别结果|就绪|已连接|成功/.test(l))
      return { icon: CheckCircle2, cls: "text-emerald-400" };
    if (/开始录音|松开/.test(l)) return { icon: Mic, cls: "text-sky-400" };
    if (l.startsWith("·") || /debug/i.test(l))
      return { cls: "text-zinc-600" };
    return { cls: "text-zinc-400" };
  };

  return (
    <PageShell
      title="实时日志"
      desc="守护进程输出，排查按键、蓝牙、识别问题用。"
    >
      <div className="mb-3 flex items-center justify-between">
        <label className="flex items-center gap-2 text-[13px] text-muted-foreground">
          <Switch
            checked={following}
            onCheckedChange={setFollowing}
            className="scale-90"
          />
          <span className="flex items-center gap-1.5">
            <ArrowDownToLine className="h-3.5 w-3.5" />
            跟随底部
          </span>
        </label>
        <Button variant="outline" size="sm" onClick={() => setLines([])}>
          <Trash2 className="h-3.5 w-3.5" />
          清空显示
        </Button>
      </div>

      <div
        ref={boxRef}
        className="h-[calc(100vh-220px)] overflow-y-auto whitespace-pre-wrap break-all rounded-xl border border-border bg-[#050507] p-4 font-mono text-[12px] leading-relaxed"
      >
        {lines.length === 0 && (
          <span className="text-zinc-600">
            暂无日志（守护进程可能未运行）
          </span>
        )}
        {lines.map((l, i) => {
          const { icon: Icon, cls } = classify(l);
          return (
            <div key={i} className={`flex items-start gap-1.5 ${cls}`}>
              <span className="mt-[3px] inline-flex w-3.5 shrink-0 justify-center">
                {Icon && <Icon className="h-3 w-3" />}
              </span>
              <span className="flex-1">{l || " "}</span>
            </div>
          );
        })}
      </div>
    </PageShell>
  );
}
