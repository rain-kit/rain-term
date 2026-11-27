import { useEffect, useState } from "react";
import HostKeyDialog from "./components/HostKeyDialog";
import Toaster from "./components/Toaster";
import UpdateDialog from "./components/UpdateDialog";
import TopBar, { type View } from "./components/TopBar";
import { cx } from "./components/ui";
import { isQuickConnectKey } from "./lib/format";
import Settings from "./pages/Settings";
import Terminal from "./pages/Terminal";

export default function App() {
  const [view, setView] = useState<View>("terminal");
  const [quick, setQuick] = useState(false);

  // 捕获阶段处理，焦点在终端里时按键也不会传给 shell。
  useEffect(() => {
    const fn = (e: KeyboardEvent) => {
      if (!isQuickConnectKey(e)) return;
      e.preventDefault();
      e.stopPropagation();
      setQuick((v) => !v);
    };
    window.addEventListener("keydown", fn, true);
    return () => window.removeEventListener("keydown", fn, true);
  }, []);

  // 设置页按 Esc 回到终端；对话框和快速连接自己处理 Esc。
  useEffect(() => {
    if (view !== "settings") return;
    const fn = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || quick) return;
      if (document.querySelector('[role="dialog"]')) return;
      setView("terminal");
    };
    window.addEventListener("keydown", fn);
    return () => window.removeEventListener("keydown", fn);
  }, [view, quick]);

  return (
    <div className="flex h-full flex-col" onContextMenu={(e) => import.meta.env.PROD && e.preventDefault()}>
      <TopBar view={view} onChange={setView} />
      {/* 打开设置时终端只隐藏不卸载，打开的标签页保持不变。 */}
      <div className={cx("flex min-h-0 flex-1", view !== "terminal" && "hidden")}>
        <Terminal quickConnect={quick} onQuickConnectClose={() => setQuick(false)} onShow={() => setView("terminal")} />
      </div>
      {view === "settings" && (
        <main className="flex min-h-0 flex-1 flex-col">
          <Settings onBack={() => setView("terminal")} />
        </main>
      )}
      <HostKeyDialog />
      <UpdateDialog />
      <Toaster />
    </div>
  );
}
