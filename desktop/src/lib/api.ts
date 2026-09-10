import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  Config,
  EngineStatus,
  AsrTestResult,
  AppInfo,
} from "./types";

export const api = {
  loadConfig: () => invoke<Config>("config_load"),
  saveConfig: (cfg: Config) => invoke<void>("config_save", { cfg }),
  defaultConfig: () => invoke<Config>("config_default"),
  configExists: () => invoke<boolean>("config_exists"),
  configPath: () => invoke<string>("config_path_string"),
  platform: () => invoke<string>("platform_name"),
  engineSupported: () => invoke<boolean>("engine_supported"),

  /** 弹出系统文件选择器选一个应用，返回它的 Bundle ID / 名字 */
  pickApp: async (platform: string): Promise<AppInfo | null> => {
    const isWin = platform === "windows";
    const isMac = platform === "macos";
    const selected = await open({
      multiple: false,
      directory: false,
      title: "选择要匹配的应用",
      defaultPath: isMac ? "/Applications" : undefined,
      filters: isWin
        ? [{ name: "应用程序", extensions: ["exe"] }]
        : isMac
          ? [{ name: "应用程序", extensions: ["app"] }]
          : [{ name: "可执行文件", extensions: ["*"] }],
    });
    if (!selected || typeof selected !== "string") return null;
    return invoke<AppInfo>("resolve_app", { path: selected });
  },

  /** 列出本机已安装的应用（macOS 扫描 Applications 目录） */
  listApps: () => invoke<AppInfo[]>("list_apps"),

  /** 内置引擎（按键映射 + 语音转文字） */
  engineStart: () => invoke<void>("engine_start"),
  engineStop: () => invoke<void>("engine_stop"),
  engineRuntimeStatus: () => invoke<EngineStatus>("engine_runtime_status"),

  logTail: (lines = 300) => invoke<string>("log_tail", { lines }),
  logFollow: () => invoke<void>("log_follow"),
  onLogLine: (cb: (line: string) => void) =>
    listen<string>("log-line", (e) => cb(e.payload)),

  testVolc: (
    appId: string,
    accessToken: string,
    resourceId: string
  ) =>
    invoke<AsrTestResult>("volc_test", {
      appId,
      accessToken,
      resourceId,
    }),

  sherpaModelStatus: (customDir?: string) =>
    invoke<{ installed: boolean; dir: string }>("sherpa_model_status", {
      customDir: customDir ?? null,
    }),

  sherpaModelDownload: () => invoke<string>("sherpa_model_download"),

  onSherpaProgress: (cb: (p: { downloaded: number; total: number }) => void) =>
    listen<{ downloaded: number; total: number }>(
      "sherpa-download-progress",
      (e) => cb(e.payload)
    ),
};
