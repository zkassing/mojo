import { useState, useEffect } from "react";
import {
  Save,
  Loader2,
  PlugZap,
  Eye,
  EyeOff,
  CheckCircle2,
  XCircle,
  KeyRound,
  Fingerprint,
  Layers,
} from "lucide-react";
import { toast } from "sonner";
import { useConfig } from "../lib/config-context";
import { api } from "../lib/api";
import type { AsrTestResult } from "../lib/types";
import { PageShell, SaveBar } from "./MappingPage";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

export default function VoicePage() {
  const { config, setConfig, save, dirty, saving } = useConfig();
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<AsrTestResult | null>(null);
  const [showKey, setShowKey] = useState(false);
  const [sherpa, setSherpa] = useState<{ installed: boolean; dir: string } | null>(null);
  const [downloading, setDownloading] = useState(false);
  const [progress, setProgress] = useState(0);

  useEffect(() => {
    api.sherpaModelStatus().then(setSherpa).catch(() => setSherpa(null));
    const un = api.onSherpaProgress((p) => {
      if (p.error) {
        toast(p.error, { duration: 4000 });
        return;
      }
      if (p.total > 0) setProgress(Math.min(99, Math.round((p.downloaded / p.total) * 100)));
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  const download = async () => {
    setDownloading(true);
    setProgress(0);
    try {
      await api.sherpaModelDownload();
      setProgress(100);
      toast.success("模型下载完成");
      setSherpa(await api.sherpaModelStatus());
    } catch (e) {
      toast.error(String(e));
    } finally {
      setDownloading(false);
    }
  };

  if (!config)
    return (
      <PageShell title="语音识别">
        加载中…
      </PageShell>
    );
  const v = config.voice;

  const setV = (patch: Partial<typeof v>) =>
    setConfig((c) => {
      c.voice = { ...c.voice, ...patch };
      return c;
    });

  const doSave = async () => {
    try {
      await save();
      toast.success("语音设置已保存");
    } catch (e) {
      toast.error(String(e));
    }
  };

  const test = async () => {
    setTesting(true);
    setResult(null);
    try {
      const r = await api.testVolc(
        v.volcAppId.trim(),
        v.volcAccessToken.trim(),
        v.volcResourceId.trim()
      );
      setResult(r);
      toast[r.ok ? "success" : "error"](
        r.ok ? "连接成功，凭证有效" : "连接失败"
      );
    } catch (e) {
      setResult({ ok: false, message: String(e), latencyMs: 0 });
    } finally {
      setTesting(false);
    }
  };

  return (
    <PageShell
      title="语音识别"
      desc="选择引擎并配置凭证，按住语音键说话。"
    >
      <div className="max-w-2xl space-y-5">
        <Card>
          <CardHeader>
            <CardTitle className="text-[15px]">识别引擎</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-1.5">
              <Label>引擎</Label>
              <Select value={v.engine} onValueChange={(t) => setV({ engine: t })}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="volc">
                    火山引擎流式大模型（推荐 · 边说边出字）
                  </SelectItem>
                  <SelectItem value="sherpa">
                    sherpa-onnx 本地流式（免费 · 离线 · 无需凭证）
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>

            <div className="grid gap-3 pt-1">
              <ToggleRow
                checked={v.liveTyping}
                onChange={(x) => setV({ liveTyping: x })}
                title="边说边出字"
                desc="中间结果实时上屏"
              />
              <ToggleRow
                checked={v.fixTerms}
                onChange={(x) => setV({ fixTerms: x })}
                title="纠正编程术语谐音"
                desc="如 main→闷、diff→地府"
              />
              <ToggleRow
                checked={v.stripPunctuation}
                onChange={(x) => setV({ stripPunctuation: x })}
                title="去除中文标点"
                desc="命令行输入更友好"
              />
              <ToggleRow
                checked={v.output === "typeEnter"}
                onChange={(x) => setV({ output: x ? "typeEnter" : "type" })}
                title="识别后自动回车"
                desc="说完直接执行命令"
              />
            </div>
          </CardContent>
        </Card>

        {v.engine === "sherpa" && (
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2 text-[15px]">
                <Layers className="h-4 w-4 text-primary" />
                本地模型
              </CardTitle>
              <CardDescription>
                sherpa-onnx zipformer 中英双语流式模型，识别在本机完成，音频不出机。
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-3">
              <div className="flex items-center gap-2">
                <span className="text-sm">模型状态</span>
                {sherpa === null ? (
                  <Badge variant="secondary">检查中…</Badge>
                ) : sherpa.installed ? (
                  <Badge className="border-emerald-600/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400">
                    已安装
                  </Badge>
                ) : (
                  <Badge variant="destructive">未找到模型文件</Badge>
                )}
              </div>
              {sherpa && (
                <p className="text-xs text-muted-foreground break-all">
                  {sherpa.dir}
                </p>
              )}
              {downloading ? (
                <div className="space-y-1.5">
                  <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                    <div
                      className="h-full rounded-full bg-primary transition-all"
                      style={{ width: `${progress}%` }}
                    />
                  </div>
                  <p className="text-xs text-muted-foreground">
                    下载中… {progress}%
                  </p>
                </div>
              ) : (
                <Button
                  variant={sherpa?.installed ? "outline" : "default"}
                  size="sm"
                  onClick={download}
                >
                  {sherpa?.installed ? "重新下载" : "下载模型（约 130MB）"}
                </Button>
              )}
            </CardContent>
          </Card>
        )}

{v.engine === "volc" && (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-[15px]">
              <KeyRound className="h-4 w-4 text-primary" />
              火山引擎凭证
            </CardTitle>
            <CardDescription>
              在语音技术控制台获取；方舟 ARK 的 API Key 不适用。
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-1.5">
              <Label className="flex items-center gap-1.5">
                <Fingerprint className="h-3.5 w-3.5 text-muted-foreground" />
                App ID
              </Label>
              <Input
                className="font-mono text-[12.5px]"
                placeholder="在语音技术控制台查看"
                value={v.volcAppId}
                onChange={(e) => setV({ volcAppId: e.target.value })}
              />
            </div>

            <div className="space-y-1.5">
              <Label className="flex items-center gap-1.5">
                <KeyRound className="h-3.5 w-3.5 text-muted-foreground" />
                Access Token
              </Label>
              <div className="flex gap-2">
                <Input
                  className="font-mono text-[12.5px]"
                  type={showKey ? "text" : "password"}
                  placeholder="Access Token"
                  value={v.volcAccessToken}
                  onChange={(e) => setV({ volcAccessToken: e.target.value })}
                />
                <Button
                  variant="outline"
                  size="icon"
                  onClick={() => setShowKey((s) => !s)}
                >
                  {showKey ? (
                    <EyeOff className="h-4 w-4" />
                  ) : (
                    <Eye className="h-4 w-4" />
                  )}
                </Button>
              </div>
            </div>

            <div className="space-y-1.5">
              <Label className="flex items-center gap-1.5">
                <Layers className="h-3.5 w-3.5 text-muted-foreground" />
                Resource ID
              </Label>
              <Input
                className="font-mono text-[12.5px]"
                placeholder="volc.bigasr.sauc.duration"
                value={v.volcResourceId}
                onChange={(e) => setV({ volcResourceId: e.target.value })}
              />
              <p className="text-[11.5px] text-muted-foreground">
                小时版 volc.bigasr.sauc.duration · 并发版
                volc.bigasr.sauc.concurrent
              </p>
            </div>

            <div className="flex items-center gap-3 pt-1">
              <Button onClick={test} disabled={testing}>
                {testing ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <PlugZap className="h-4 w-4" />
                )}
                测试连接
              </Button>
              {result &&
                (result.ok ? (
                  <Badge variant="secondary" className="gap-1.5">
                    <CheckCircle2 className="h-3.5 w-3.5" />
                    {result.message}（{result.latencyMs}ms）
                  </Badge>
                ) : (
                  <Badge variant="destructive" className="gap-1.5">
                    <XCircle className="h-3.5 w-3.5" />
                    {result.message}
                  </Badge>
                ))}
            </div>
          </CardContent>
        </Card>
        )}
      </div>

      <SaveBar dirty={dirty} saving={saving} onSave={doSave} />
    </PageShell>
  );
}

function ToggleRow({
  checked,
  onChange,
  title,
  desc,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  title: string;
  desc: string;
}) {
  return (
    <label className="flex cursor-pointer items-center justify-between gap-4 rounded-lg border border-border bg-background/40 px-3.5 py-3">
      <div>
        <div className="text-[13px] font-medium">{title}</div>
        <div className="text-[11.5px] text-muted-foreground">{desc}</div>
      </div>
      <Switch checked={checked} onCheckedChange={onChange} />
    </label>
  );
}
