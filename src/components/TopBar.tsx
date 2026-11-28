import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Copy, KeyRound, Minus, Settings, Square, Wrench, X } from "lucide-react";
import { useEffect, useState, type ComponentType, type ReactNode } from "react";
import { isMac, isWindows } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { useUpdate } from "../lib/store";
import KeysDialog from "./KeysDialog";
import ToolsDialog from "./ToolsDialog";
import { cx, IconButton } from "./ui";

export type View = "terminal" | "settings";

function WindowControls() {
  const { t } = useI18n();
  const win = getCurrentWindow();
  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    win.isMaximized().then(setMaximized);
    const un = win.onResized(() => win.isMaximized().then(setMaximized));
    return () => {
      un.then((f) => f());
    };
  }, [win]);
  const btn = "flex h-full w-11.5 items-center justify-center text-muted transition-colors hover:text-ink";
  return (
    <div className="flex h-full">
      <button className={cx(btn, "hover:bg-subtle")} onClick={() => win.minimize()} title={t("window.minimize")}>
        <Minus size={16} strokeWidth={1.5} />
      </button>
      <button className={cx(btn, "hover:bg-subtle")} onClick={() => win.toggleMaximize()} title={t(maximized ? "window.restore" : "window.maximize")}>
        {maximized ? <Copy size={12} strokeWidth={1.5} className="-scale-x-100" /> : <Square size={12} strokeWidth={1.5} />}
      </button>
      <button className={cx(btn, "hover:bg-red-600 hover:text-white")} onClick={() => win.close()} title={t("common.close")}>
        <X size={17} strokeWidth={1.5} />
      </button>
    </div>
  );
}

/** 顶栏带文字的按钮，点开一个窗口。 */
function DialogButton({
  icon,
  label,
  dialog: Dlg,
}: {
  icon: ReactNode;
  label: string;
  dialog: ComponentType<{ open: boolean; onClose: () => void }>;
}) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        onClick={() => setOpen(true)}
        className={cx(
          "inline-flex items-center gap-1.5 transition-colors hover:bg-subtle hover:text-ink",
          isMac ? "h-7 rounded-md px-2 text-[13px]" : "h-8 rounded-lg px-2.5 text-sm",
          open ? "bg-subtle text-ink" : "text-muted",
        )}
      >
        {icon}
        {label}
      </button>
      <Dlg open={open} onClose={() => setOpen(false)} />
    </>
  );
}

/**
 * 窗口顶栏，同时作为拖动区域（data-tauri-drag-region="deep" 让非按钮区域都能拖动和双击缩放）：
 * - macOS：左侧留给系统红绿灯按钮，中间显示窗口标题（设置页点击回到终端），右侧是密钥/用户、工具和设置；
 * - Windows / Linux：左侧是名称（点击回到终端）、密钥/用户和工具，右侧是设置和 Windows 的窗口按钮。
 */
export default function TopBar({ view, onChange }: { view: View; onChange: (v: View) => void }) {
  const { t } = useI18n();
  const update = useUpdate().info;
  const settingsLabel =
    view === "settings"
      ? t("nav.backToTerminal")
      : update?.hasUpdate
        ? t("update.found", { version: update.latest ?? "" })
        : t("nav.settings");

  const dialogs = (
    <div className={cx("flex items-center", isMac ? "gap-0.5" : "-ml-2 gap-1")}>
      <DialogButton icon={<KeyRound size={isMac ? 14 : 15} />} label={t("keys.title")} dialog={KeysDialog} />
      <DialogButton icon={<Wrench size={isMac ? 14 : 15} />} label={t("tools.title")} dialog={ToolsDialog} />
    </div>
  );

  const settingsBtn = (
    <IconButton
      size={isMac ? "sm" : "md"}
      label={settingsLabel}
      onClick={() => onChange(view === "settings" ? "terminal" : "settings")}
      className={cx("relative", view === "settings" && "bg-subtle text-ink")}
    >
      {view === "settings" ? <X size={isMac ? 16 : 17} /> : <Settings size={isMac ? 16 : 17} />}
      {update?.hasUpdate && view !== "settings" && (
        <span className={cx("absolute size-2 rounded-full bg-red-500", isMac ? "top-1 right-1" : "top-1.5 right-1.5")} />
      )}
    </IconButton>
  );

  if (isMac) {
    return (
      <header
        data-tauri-drag-region="deep"
        className="relative flex h-10 shrink-0 items-center justify-end border-b border-line bg-surface/60 pr-2.5 pl-20"
      >
        {view === "settings" ? (
          <button
            onClick={() => onChange("terminal")}
            title={t("nav.backToTerminal")}
            className="absolute left-1/2 -translate-x-1/2 rounded-md px-2 py-0.5 text-[13px] font-medium text-muted transition-colors hover:bg-subtle hover:text-ink"
          >
            Rain Term
          </button>
        ) : (
          <span className="pointer-events-none absolute left-1/2 -translate-x-1/2 text-[13px] font-medium text-muted">
            Rain Term
          </span>
        )}
        {dialogs}
        <div className="mx-1.5 h-3.5 w-px bg-line" />
        {settingsBtn}
      </header>
    );
  }

  return (
    <header data-tauri-drag-region="deep" className="flex h-12 shrink-0 items-center gap-5 border-b border-line bg-surface/60">
      <div className="flex items-center pl-4">
        <button
          onClick={() => onChange("terminal")}
          title={t("nav.backToTerminal")}
          className="-ml-1.5 flex h-8 items-center gap-2.5 rounded-lg px-1.5 transition-colors hover:bg-subtle"
        >
          <img src="/logo.svg" alt="" className="pointer-events-none size-6" />
          <span className="font-bold tracking-tight">Rain Term</span>
        </button>
      </div>

      {dialogs}

      <div className="flex-1 self-stretch" />

      <div className="flex items-center gap-1">{settingsBtn}</div>

      {isWindows && isTauri() ? (
        <div className="h-full">
          <WindowControls />
        </div>
      ) : (
        <div className="w-1" />
      )}
    </header>
  );
}
