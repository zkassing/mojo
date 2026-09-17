import { useEffect, useState } from "react";
import {
  Keyboard,
  AudioLines,
  HeartPulse,
  ScrollText,
  AlertTriangle,
} from "lucide-react";
import appIcon from "./assets/app-icon.png";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { Badge } from "@/components/ui/badge";
import { ConfigProvider } from "./lib/config-context";
import { api } from "./lib/api";
import { checkForUpdate } from "./lib/updater";
import MappingPage from "./pages/MappingPage";
import VoicePage from "./pages/VoicePage";
import ServicePage from "./pages/ServicePage";
import LogsPage from "./pages/LogsPage";
import { cn } from "./lib/utils";

type PageKey = "mapping" | "voice" | "service" | "logs";

const NAV: { key: PageKey; label: string; icon: typeof Keyboard }[] = [
  { key: "mapping", label: "按键映射", icon: Keyboard },
  { key: "voice", label: "语音识别", icon: AudioLines },
  { key: "service", label: "服务与状态", icon: HeartPulse },
  { key: "logs", label: "实时日志", icon: ScrollText },
];

export default function App() {
  const [page, setPage] = useState<PageKey>("mapping");
  const [platform, setPlatform] = useState("macos");
  const [supported, setSupported] = useState(true);

  useEffect(() => {
    api.platform().then(setPlatform).catch(() => {});
    api.engineSupported().then(setSupported).catch(() => {});

    // 跟随系统亮/暗主题，在根节点加 .dark 命中 shadcn 暗色变量
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () =>
      document.documentElement.classList.toggle("dark", mq.matches);
    apply();
    mq.addEventListener("change", apply);
    // 启动 5 秒后静默检查更新（无更新/网络失败均不打扰）
    const updateTimer = setTimeout(() => {
      void checkForUpdate(true);
    }, 5000);
    return () => {
      mq.removeEventListener("change", apply);
      clearTimeout(updateTimer);
    };
  }, []);

  return (
    <TooltipProvider delayDuration={300}>
      <div className="grid h-screen grid-cols-[224px_1fr] bg-background">
        <aside className="flex flex-col gap-1 border-r border-border bg-card p-3">
          <div className="flex items-center gap-2.5 px-2 pb-5 pt-2">
            <img
              src={appIcon}
              alt="Mojo"
              className="h-9 w-9 rounded-lg"
            />
            <div className="leading-tight">
              <div className="text-sm font-semibold">Mojo</div>
              <div className="text-[11px] text-muted-foreground">
                小米语音遥控器
              </div>
            </div>
          </div>

          {NAV.map((n) => {
            const Icon = n.icon;
            const active = page === n.key;
            return (
              <button
                key={n.key}
                onClick={() => setPage(n.key)}
                className={cn(
                  "flex items-center gap-3 rounded-md px-3 py-2 text-left text-[13px] transition-colors",
                  active
                    ? "bg-accent font-medium text-foreground"
                    : "text-muted-foreground hover:bg-accent/60 hover:text-foreground"
                )}
              >
                <Icon className="h-4 w-4" />
                {n.label}
              </button>
            );
          })}

          <div className="mt-auto space-y-2 px-2 pb-1 text-[11px] text-muted-foreground">
            {!supported && (
              <Badge variant="secondary" className="gap-1.5">
                <AlertTriangle className="h-3 w-3" />
                {platform} 引擎移植中
              </Badge>
            )}
            <div>跨平台控制面板 · v0.1</div>
          </div>
        </aside>

        <main className="overflow-y-auto">
          <ConfigProvider>
            {page === "mapping" && <MappingPage />}
            {page === "voice" && <VoicePage />}
            {page === "service" && <ServicePage />}
            {page === "logs" && <LogsPage />}
          </ConfigProvider>
        </main>
      </div>

      <Toaster position="bottom-right" richColors closeButton />
    </TooltipProvider>
  );
}
