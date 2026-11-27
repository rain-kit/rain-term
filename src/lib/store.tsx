import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { connApi, errorMessage, events, hostKeyApi, settingsApi, updateApi } from "./api";
import { setLanguage, t } from "./i18n";
import type { ConnView, HostKeyPrompt, Settings, SettingsInput, UpdateInfo } from "./types";

// ---------- 提示 ----------

export interface Toast {
  id: number;
  kind: "success" | "error" | "info";
  text: string;
}

interface ToastCtx {
  toasts: Toast[];
  success: (text: string) => void;
  error: (e: unknown) => void;
  info: (text: string) => void;
  dismiss: (id: number) => void;
}

const ToastContext = createContext<ToastCtx>(null!);
export const useToast = () => useContext(ToastContext);

function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const seq = useRef(0);
  const dismiss = useCallback((id: number) => setToasts((l) => l.filter((t) => t.id !== id)), []);
  const push = useCallback(
    (kind: Toast["kind"], text: string) => {
      const id = ++seq.current;
      setToasts((l) => [...l.slice(-3), { id, kind, text }]);
      setTimeout(() => dismiss(id), kind === "error" ? 6000 : 3000);
    },
    [dismiss],
  );
  const value = useMemo<ToastCtx>(
    () => ({
      toasts,
      dismiss,
      success: (t) => push("success", t),
      info: (t) => push("info", t),
      error: (e) => push("error", errorMessage(e)),
    }),
    [toasts, push, dismiss],
  );
  return <ToastContext.Provider value={value}>{children}</ToastContext.Provider>;
}

// ---------- 设置与主题 ----------

interface SettingsCtx {
  settings: Settings | null;
  dark: boolean;
  save: (patch: Partial<SettingsInput>) => Promise<void>;
}

const SettingsContext = createContext<SettingsCtx>(null!);
export const useSettings = () => useContext(SettingsContext);

function useSystemDark() {
  const [dark, setDark] = useState(() => matchMedia("(prefers-color-scheme: dark)").matches);
  useEffect(() => {
    const mq = matchMedia("(prefers-color-scheme: dark)");
    const fn = () => setDark(mq.matches);
    mq.addEventListener("change", fn);
    return () => mq.removeEventListener("change", fn);
  }, []);
  return dark;
}

function SettingsProvider({ children }: { children: ReactNode }) {
  const toast = useToast();
  const [settings, setSettings] = useState<Settings | null>(null);
  const systemDark = useSystemDark();
  const theme = settings?.theme ?? "system";
  const dark = theme === "dark" || (theme === "system" && systemDark);

  useEffect(() => {
    const load = () => settingsApi.get().then(setSettings).catch(toast.error);
    load();
    return events.onSettingsChanged(load);
  }, []);

  useEffect(() => {
    if (settings) setLanguage(settings.language, settings.systemLanguage);
  }, [settings?.language, settings?.systemLanguage]);

  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
    document.documentElement.style.colorScheme = dark ? "dark" : "light";
  }, [dark]);

  const save = useCallback(
    async (patch: Partial<SettingsInput>) => {
      if (!settings) return;
      const input: SettingsInput = {
        logLevel: settings.logLevel,
        theme: settings.theme,
        language: settings.language,
        ...patch,
      };
      setSettings({ ...settings, ...patch });
      try {
        setSettings(await settingsApi.save(input));
      } catch (e) {
        setSettings(settings);
        throw e;
      }
    },
    [settings],
  );

  const value = useMemo(() => ({ settings, dark, save }), [settings, dark, save]);
  return <SettingsContext.Provider value={value}>{children}</SettingsContext.Provider>;
}

// ---------- 连接 ----------

interface ConnsCtx {
  conns: ConnView[];
  loaded: boolean;
  refresh: () => Promise<void>;
  /** 本地先调整顺序再提交，拖动时界面不闪动。 */
  reorder: (ids: string[]) => Promise<void>;
}

const ConnsContext = createContext<ConnsCtx>(null!);
export const useConns = () => useContext(ConnsContext);

function ConnsProvider({ children }: { children: ReactNode }) {
  const toast = useToast();
  const [conns, setConns] = useState<ConnView[]>([]);
  const [loaded, setLoaded] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setConns(await connApi.list());
    } catch (e) {
      toast.error(e);
    } finally {
      setLoaded(true);
    }
  }, []);

  useEffect(() => {
    refresh();
    return events.onConnChanged(refresh);
  }, [refresh]);

  const reorder = useCallback(
    async (ids: string[]) => {
      setConns((list) => {
        const rank = new Map(ids.map((id, i) => [id, i]));
        return [...list].sort((a, b) => (rank.get(a.id) ?? Infinity) - (rank.get(b.id) ?? Infinity));
      });
      try {
        await connApi.reorder(ids);
      } catch (e) {
        toast.error(e);
        refresh();
      }
    },
    [refresh],
  );

  const value = useMemo(() => ({ conns, loaded, refresh, reorder }), [conns, loaded, refresh, reorder]);
  return <ConnsContext.Provider value={value}>{children}</ConnsContext.Provider>;
}

// ---------- 主机指纹确认 ----------

interface HostKeyCtx {
  prompts: HostKeyPrompt[];
  answer: (p: HostKeyPrompt, accept: boolean) => Promise<void>;
}

const HostKeyContext = createContext<HostKeyCtx>(null!);
export const useHostKeys = () => useContext(HostKeyContext);

function HostKeyProvider({ children }: { children: ReactNode }) {
  const toast = useToast();
  const [prompts, setPrompts] = useState<HostKeyPrompt[]>([]);
  useEffect(() => {
    hostKeyApi
      .pending()
      .then((l) => setPrompts((cur) => merge(cur, l)))
      .catch(() => {});
    const offPrompt = events.onHostKey((p) => setPrompts((cur) => merge(cur, [p])));
    const offDone = events.onHostKeyDone((id) => setPrompts((cur) => cur.filter((p) => p.requestId !== id)));
    return () => {
      offPrompt();
      offDone();
    };
  }, []);
  const answer = useCallback(async (p: HostKeyPrompt, accept: boolean) => {
    setPrompts((cur) => cur.filter((x) => x.requestId !== p.requestId));
    try {
      await hostKeyApi.answer(p.requestId, accept);
    } catch (e) {
      toast.error(e);
    }
  }, []);
  const value = useMemo(() => ({ prompts, answer }), [prompts, answer]);
  return <HostKeyContext.Provider value={value}>{children}</HostKeyContext.Provider>;
}

function merge(cur: HostKeyPrompt[], add: HostKeyPrompt[]) {
  const ids = new Set(cur.map((p) => p.requestId));
  return [...cur, ...add.filter((p) => !ids.has(p.requestId))];
}

// ---------- 版本更新 ----------

interface UpdateCtx {
  info: UpdateInfo | null;
  checking: boolean;
  /** 主动检查，结果用提示告知。 */
  check: () => Promise<void>;
  installing: boolean;
  /** 下载进度百分比，大小未知时为 null。 */
  progress: number | null;
  install: () => Promise<void>;
  /** 更新窗口（新版本的更新内容和更新按钮） */
  dialogOpen: boolean;
  setDialogOpen: (open: boolean) => void;
}

const UpdateContext = createContext<UpdateCtx>(null!);
export const useUpdate = () => useContext(UpdateContext);

function UpdateProvider({ children }: { children: ReactNode }) {
  const toast = useToast();
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);
  const [dialogOpen, setDialogOpen] = useState(false);
  const seen = useRef<string | undefined>(undefined);
  const manual = useRef(false);

  // 托盘点“发现新版本”时打开更新窗口：主窗口新建时在加载后取请求，已打开时收到事件再取。
  useEffect(() => {
    const take = () =>
      updateApi
        .takeOpenRequest()
        .then((yes) => yes && setDialogOpen(true))
        .catch(() => {});
    take();
    return events.onUpdateOpen(take);
  }, []);

  useEffect(() => {
    updateApi
      .status()
      .then((u) => {
        seen.current = u.latest;
        setInfo(u);
      })
      .catch(() => {});
    // 后台定时检查发现新版本时提示一次，主动检查的结果由 check 自己提示。
    return events.onUpdate((u) => {
      if (!manual.current && u.hasUpdate && u.latest !== seen.current) toast.info(t("update.found", { version: u.latest ?? "" }));
      seen.current = u.latest;
      setInfo(u);
    });
  }, []);

  const check = useCallback(async () => {
    manual.current = true;
    setChecking(true);
    try {
      const u = await updateApi.check();
      setInfo(u);
      if (u.hasUpdate) setDialogOpen(true);
      else toast.success(t("update.upToDate"));
    } catch (e) {
      toast.error(e);
    } finally {
      manual.current = false;
      setChecking(false);
    }
  }, []);

  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<number | null>(null);
  useEffect(() => events.onUpdateProgress((p) => setProgress(p.total ? Math.floor((p.downloaded / p.total) * 100) : null)), []);

  const install = useCallback(async () => {
    setInstalling(true);
    setProgress(0);
    try {
      await updateApi.install();
    } catch (e) {
      toast.error(e);
    } finally {
      setInstalling(false);
      setProgress(null);
    }
  }, []);

  const value = useMemo(
    () => ({ info, checking, check, installing, progress, install, dialogOpen, setDialogOpen }),
    [info, checking, check, installing, progress, install, dialogOpen],
  );
  return <UpdateContext.Provider value={value}>{children}</UpdateContext.Provider>;
}

export function AppProvider({ children }: { children: ReactNode }) {
  return (
    <ToastProvider>
      <SettingsProvider>
        <UpdateProvider>
          <ConnsProvider>
            <HostKeyProvider>{children}</HostKeyProvider>
          </ConnsProvider>
        </UpdateProvider>
      </SettingsProvider>
    </ToastProvider>
  );
}
