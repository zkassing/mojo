// 与 Rust config.rs / Swift Config.swift 对齐的配置类型

export interface DeviceConfig {
  vendorId: string;
  productId: string;
  name?: string | null;
}

export interface Options {
  swallowOriginal: boolean;
  longPressMs: number;
  doublePressMs: number;
  debounceMs: number;
  verbose: boolean;
}

export interface VoiceConfig {
  locale: string;
  output: string;
  stripPunctuation: boolean;
  engine: string;
  volcAppId: string;
  volcAccessToken: string;
  volcResourceId: string;
  liveTyping: boolean;
  fixTerms: boolean;
  sherpaModelDir: string;
}

export interface FullAction {
  type: string;
  key?: string;
  mods?: string[];
  command?: string;
  target?: string;
  dx?: number;
  dy?: number;
  button?: string;
  count?: number;
  actions?: Action[];
}

// 简写 "cmd+tab" 或完整对象
export type Action = string | FullAction;

export interface Binding {
  tap?: Action;
  long?: Action;
  double?: Action;
  repeat?: boolean;
}

export interface ProfileMatch {
  bundleIds?: string[];
  appNames?: string[];
}

export interface Profile {
  name: string;
  match?: ProfileMatch;
  bindings: Record<string, Binding>;
}

export interface Config {
  device: DeviceConfig;
  options: Options;
  voice: VoiceConfig;
  buttons: Record<string, string[]>;
  profiles: Profile[];
}

/** 内置 Rust 引擎运行状态（tag = kind） */
export type EngineStatus =
  | { kind: "stopped" }
  | { kind: "running"; daemonConnected: boolean; remoteConnected: boolean }
  | { kind: "unsupported"; reason: string };

export interface AsrTestResult {
  ok: boolean;
  message: string;
  latencyMs: number;
}

export interface AppInfo {
  bundleId?: string | null;
  appName?: string | null;
  path: string;
}

// 遥控器 12 个逻辑键，顺序用于布局
export const BUTTONS = [
  "power",
  "voice",
  "up",
  "down",
  "left",
  "right",
  "ok",
  "back",
  "menu",
  "home",
  "volup",
  "voldown",
  "tv",
] as const;

export type ButtonName = (typeof BUTTONS)[number];

export const BUTTON_LABELS: Record<string, string> = {
  power: "电源",
  voice: "语音",
  up: "上",
  down: "下",
  left: "左",
  right: "右",
  ok: "OK",
  back: "返回",
  menu: "菜单",
  home: "主页",
  volup: "音量 +",
  voldown: "音量 −",
  tv: "TV",
};
