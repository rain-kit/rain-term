import { LoaderCircle, RotateCw, VenetianMask } from "lucide-react";
import { createContext, useContext, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { useI18n } from "../lib/i18n";
import type { TabStatus } from "./TabBar";

const SlotContext = createContext<HTMLElement | null>(null);

/** 标签区域底部的状态栏，当前标签用 ConnStatus 把自己的连接状态放进来。 */
export function StatusBarHost({ children }: { children: ReactNode }) {
  const [el, setEl] = useState<HTMLElement | null>(null);
  return (
    <SlotContext.Provider value={el}>
      {children}
      <footer ref={setEl} className="flex h-7 shrink-0 items-center gap-2 border-t border-line bg-surface/60 px-3 text-xs" />
    </SlotContext.Provider>
  );
}

/** 连接中显示进度，已连接显示连接名，失败或断开时显示原因和重新连接按钮。 */
export function ConnStatus({
  status,
  name,
  incognito = false,
  connectingText,
  message,
  extra,
  onReconnect,
}: {
  status: TabStatus;
  name: string;
  /** 无痕模式：已连接时在状态栏显示标记。 */
  incognito?: boolean;
  connectingText: string;
  message?: string;
  /** 已连接时显示在右侧。 */
  extra?: ReactNode;
  onReconnect: () => void;
}) {
  const { t } = useI18n();
  const el = useContext(SlotContext);
  if (!el || status === "idle") return null;
  return createPortal(
    status === "connecting" ? (
      <>
        <LoaderCircle size={13} className="shrink-0 animate-spin text-muted" />
        <span className="truncate text-muted">{connectingText}</span>
      </>
    ) : status === "connected" ? (
      <>
        <span className="size-2 shrink-0 rounded-full bg-emerald-500" />
        <span className="truncate text-muted">{t("status.connected", { name })}</span>
        {incognito && (
          <span
            title={t("conn.incognitoHint")}
            className="flex shrink-0 items-center gap-1 rounded bg-indigo-500/12 px-1.5 font-medium text-indigo-600 dark:bg-indigo-500/20 dark:text-indigo-400"
          >
            <VenetianMask size={12} />
            {t("status.incognito")}
          </span>
        )}
        {extra && <span className="ml-auto shrink-0 text-muted tabular-nums">{extra}</span>}
      </>
    ) : (
      <>
        <span className="size-2 shrink-0 rounded-full bg-red-500" />
        <span className="min-w-0 flex-1 truncate" title={message}>
          {message}
        </span>
        <button
          type="button"
          onClick={onReconnect}
          className="flex h-5 shrink-0 items-center gap-1 rounded px-1.5 font-medium text-accent transition-colors hover:bg-subtle"
        >
          <RotateCw size={12} />
          {t("terminal.reconnect")}
        </button>
      </>
    ),
    el,
  );
}
