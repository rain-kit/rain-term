import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Copy, Minus, Settings, Square, Wrench, X } from "lucide-react";
import { useEffect, useState } from "react";
import { isMac, isWindows } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { useUpdate } from "../lib/store";
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

function ToolsButton() {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        onClick={() => setOpen(true)}
        className={cx(
          "inline-flex h-8 items-center gap-1.5 rounded-lg px-2.5 text-sm transition-colors hover:bg-subtle hover:text-ink",
          open ? "bg-subtle text-ink" : "text-muted",
        )}
      >
        <Wrench size={15} />
        {t("tools.title")}
      </button>
      <ToolsDialog open={open} onClose={() => setOpen(false)} />
    </>
  );
}

/** 窗口顶栏，同时作为拖动区域：左侧是名称（点击回到终端）和工具，右侧是设置（再点一次回到终端）和 Windows 的窗口按钮。 */
export default function TopBar({ view, onChange }: { view: View; onChange: (v: View) => void }) {
  const { t } = useI18n();
  const update = useUpdate().info;
  return (
    <header data-tauri-drag-region className="flex h-12 shrink-0 items-center gap-5 border-b border-line bg-surface/60">
      {/* macOS 左侧留出红绿灯按钮的位置。 */}
      <div data-tauri-drag-region className={cx("flex items-center", isMac ? "pl-20" : "pl-4")}>
        {/* 不能放进拖动区域，否则按下鼠标就开始拖窗口，点击不生效。 */}
        <button
          onClick={() => onChange("terminal")}
          title={t("nav.backToTerminal")}
          className="-ml-1.5 flex h-8 items-center gap-2.5 rounded-lg px-1.5 transition-colors hover:bg-subtle"
        >
          <img src="/logo.svg" alt="" className="pointer-events-none size-6" />
          <span className="font-bold tracking-tight">Rain Term</span>
        </button>
      </div>

      <div className="-ml-2 flex items-center gap-1">
        <ToolsButton />
      </div>

      <div data-tauri-drag-region className="flex-1 self-stretch" />

      <div className="flex items-center gap-1">
        <IconButton
          label={
            view === "settings"
              ? t("nav.backToTerminal")
              : update?.hasUpdate
                ? t("update.found", { version: update.latest ?? "" })
                : t("nav.settings")
          }
          onClick={() => onChange(view === "settings" ? "terminal" : "settings")}
          className={cx("relative", view === "settings" && "bg-subtle text-ink")}
        >
          {view === "settings" ? <X size={17} /> : <Settings size={17} />}
          {update?.hasUpdate && view !== "settings" && <span className="absolute top-1.5 right-1.5 size-2 rounded-full bg-red-500" />}
        </IconButton>
      </div>

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
