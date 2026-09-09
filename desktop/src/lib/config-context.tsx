import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";
import type { ReactNode } from "react";
import type { Config } from "./types";
import { api } from "./api";

interface ConfigCtx {
  config: Config | null;
  setConfig: (updater: (c: Config) => Config) => void;
  save: () => Promise<void>;
  reload: () => Promise<void>;
  dirty: boolean;
  saving: boolean;
}

const Ctx = createContext<ConfigCtx | null>(null);

export function useConfig() {
  const c = useContext(Ctx);
  if (!c) throw new Error("useConfig 必须在 ConfigProvider 内使用");
  return c;
}

export function ConfigProvider({ children }: { children: ReactNode }) {
  const [config, setConfigState] = useState<Config | null>(null);
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const loaded = useRef<string>("");

  const reload = useCallback(async () => {
    const c = await api.loadConfig();
    loaded.current = JSON.stringify(c);
    setConfigState(c);
    setDirty(false);
  }, []);

  useEffect(() => {
    reload().catch(console.error);
  }, [reload]);

  const setConfig = useCallback(
    (updater: (c: Config) => Config) => {
      setConfigState((prev) => {
        if (!prev) return prev;
        const next = updater(structuredClone(prev));
        setDirty(JSON.stringify(next) !== loaded.current);
        return next;
      });
    },
    []
  );

  const save = useCallback(async () => {
    if (!config) return;
    setSaving(true);
    try {
      await api.saveConfig(config);
      loaded.current = JSON.stringify(config);
      setDirty(false);
    } finally {
      setSaving(false);
    }
  }, [config]);

  return (
    <Ctx.Provider value={{ config, setConfig, save, reload, dirty, saving }}>
      {children}
    </Ctx.Provider>
  );
}
