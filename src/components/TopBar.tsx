import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Copy, Minus, Settings, Square, Wrench, X } from "lucide-react";
import { useEffect, useState } from "react";
import { isMac, isWindows } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { useSettings, useToast, useUpdate } from "../lib/store";
import type { ToolsSettings } from "../lib/types";
import { cx, IconButton, Menu, SwitchRow } from "./ui";

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

/** 工具菜单：按分组列出可以随时开关的功能，修改后立即保存。 */
function ToolsMenu() {
  const { t } = useI18n();
  const toast = useToast();
  const { settings, save } = useSettings();
  const tools = settings?.tools;
  const setEnhance = (patch: Partial<ToolsSettings["enhance"]>) =>
    tools && save({ tools: { ...tools, enhance: { ...tools.enhance, ...patch } } }).catch(toast.error);
  return (
    <Menu
      trigger={(open) => (
        <IconButton label={t("tools.title")} onClick={open}>
          <Wrench size={16} />
        </IconButton>
      )}
      footer={
        tools && (
          <div className="w-80 px-2.5 py-2">
            <div className="mb-2 text-xs font-semibold tracking-wide text-muted">{t("tools.enhance")}</div>
            <SwitchRow
              title={t("tools.clickMovesCursor")}
              desc={t("tools.clickMovesCursorDesc")}
              checked={tools.enhance.clickMovesCursor}
              onChange={(clickMovesCursor) => setEnhance({ clickMovesCursor })}
            />
          </div>
        )
      }
    />
  );
}

/** 窗口顶栏，同时作为拖动区域：右侧是工具、设置（再点一次回到终端）和 Windows 的窗口按钮。 */
export default function TopBar({ view, onChange }: { view: View; onChange: (v: View) => void }) {
  const { t } = useI18n();
  const update = useUpdate().info;
  return (
    <header data-tauri-drag-region className="flex h-12 shrink-0 items-center gap-5 border-b border-line bg-surface/60">
      {/* macOS 左侧留出红绿灯按钮的位置。 */}
      <div data-tauri-drag-region className={cx("flex items-center gap-2.5", isMac ? "pl-20" : "pl-4")}>
        <img src="/logo.svg" alt="" className="pointer-events-none size-6" />
        <span data-tauri-drag-region className="font-bold tracking-tight">
          Rain Term
        </span>
      </div>

      <div data-tauri-drag-region className="flex-1 self-stretch" />

      <div className="flex items-center gap-1">
        <ToolsMenu />
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
