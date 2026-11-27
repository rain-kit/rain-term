import { X } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useI18n } from "../lib/i18n";
import { ContextMenu, cx, type MenuItem } from "./ui";

/** idle：从上次的工作区恢复、还没有连接。 */
export type TabStatus = "idle" | "connecting" | "connected" | "closed";

export interface TabItem {
  key: string;
  title: string;
  icon?: ReactNode;
  status?: TabStatus;
  /** 生产环境的连接，标签显示红色。 */
  danger?: boolean;
}

const dot: Record<TabStatus, string> = {
  idle: "border border-zinc-400 dark:border-zinc-500",
  connecting: "bg-amber-500 animate-pulse",
  connected: "bg-emerald-500",
  closed: "bg-zinc-400 dark:bg-zinc-500",
};

/** 按下后横向移动超过这么多像素才开始拖动，避免点击时误触。 */
const DRAG_THRESHOLD = 4;

/** 右侧标签栏：点击切换，中键或关闭按钮关闭，拖动排序，右键菜单。 */
export default function TabBar({
  tabs,
  active,
  onSelect,
  onClose,
  onReorder,
  menu,
  actions,
}: {
  tabs: TabItem[];
  active: string | null;
  onSelect: (key: string) => void;
  onClose: (key: string) => void;
  onReorder: (keys: string[]) => void;
  /** 标签的右键菜单。 */
  menu: (key: string) => MenuItem[];
  /** 标签栏最右边的按钮。 */
  actions?: ReactNode;
}) {
  const { t } = useI18n();
  const bar = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<{ key: string; order: string[] } | null>(null);
  const [ctx, setCtx] = useState<{ at: { x: number; y: number }; key: string } | null>(null);

  // 用指针事件拖动排序：Windows 上原生的文件拖放处理会让 WebView 里的 HTML5 拖放失效。
  const pointer = useRef<{ key: string; x: number; order: string[] | null } | null>(null);
  const latest = useRef({ keys: [] as string[], onReorder });
  latest.current = { keys: tabs.map((x) => x.key), onReorder };
  useEffect(() => {
    const move = (e: PointerEvent) => {
      const p = pointer.current;
      if (!p) return;
      if (!p.order) {
        if (Math.abs(e.clientX - p.x) < DRAG_THRESHOLD) return;
        p.order = latest.current.keys;
        setDrag({ key: p.key, order: p.order });
      }
      const els = [...(bar.current?.querySelectorAll<HTMLElement>("[data-tab]") ?? [])];
      const el = els.find((x) => {
        const r = x.getBoundingClientRect();
        return e.clientX >= r.left && e.clientX < r.right;
      });
      const over = el?.dataset.tab;
      if (!el || !over || over === p.key) return;
      const { order } = p;
      const right = order.indexOf(over) > order.indexOf(p.key);
      // 越过目标标签的中线才交换，宽度不同的标签交换后不会来回跳。
      const r = el.getBoundingClientRect();
      if (right ? e.clientX < r.left + r.width / 2 : e.clientX > r.left + r.width / 2) return;
      const next = order.filter((k) => k !== p.key);
      next.splice(next.indexOf(over) + (right ? 1 : 0), 0, p.key);
      p.order = next;
      setDrag({ key: p.key, order: next });
    };
    const end = (commit: boolean) => () => {
      const p = pointer.current;
      pointer.current = null;
      if (!p?.order) return;
      if (commit) latest.current.onReorder(p.order);
      setDrag(null);
    };
    const [up, cancel] = [end(true), end(false)];
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", cancel);
    window.addEventListener("blur", cancel);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", cancel);
      window.removeEventListener("blur", cancel);
    };
  }, []);

  const byKey = new Map(tabs.map((x) => [x.key, x]));
  const shown = drag ? drag.order.map((k) => byKey.get(k)).filter((x) => x !== undefined) : tabs;

  return (
    <div className="flex h-9 shrink-0 border-b border-line bg-surface/60">
      <div ref={bar} className={cx("flex min-w-0 flex-1 items-end gap-0.5 overflow-x-auto px-1.5", drag && "cursor-grabbing")}>
        {shown.map((tab) => (
          <div
            key={tab.key}
            data-tab={tab.key}
            title={tab.status ? `${tab.title} · ${t(`terminal.status.${tab.status}` as const)}` : tab.title}
            onPointerDown={(e) => {
              if (e.button === 0) pointer.current = { key: tab.key, x: e.clientX, order: null };
            }}
            onMouseDown={(e) => {
              if (e.button === 1) {
                e.preventDefault();
                onClose(tab.key);
              } else if (e.button === 0) onSelect(tab.key);
            }}
            onContextMenu={(e) => {
              e.preventDefault();
              onSelect(tab.key);
              setCtx({ at: { x: e.clientX, y: e.clientY }, key: tab.key });
            }}
            className={cx(
              "group relative flex h-8 max-w-52 min-w-28 shrink-0 cursor-default items-center gap-1.5 rounded-t-lg border border-b-0 pr-1 pl-2.5 text-[13px] select-none",
              tab.key === active ? "border-line bg-canvas text-ink" : "border-transparent text-muted hover:bg-subtle hover:text-ink",
              tab.danger && "bg-red-500/8 !text-red-600 dark:bg-red-500/15 dark:!text-red-400",
            )}
          >
            {tab.danger && <span className="absolute inset-x-0 top-0 h-0.5 rounded-t-lg bg-red-500" />}
            {tab.status && <span className={cx("size-1.5 shrink-0 rounded-full", dot[tab.status])} />}
            {tab.icon}
            <span className="min-w-0 flex-1 truncate">{tab.title}</span>
            <button
              aria-label={t("terminal.closeTab")}
              title={t("terminal.closeTab")}
              onPointerDown={(e) => e.stopPropagation()}
              onMouseDown={(e) => e.stopPropagation()}
              onClick={() => onClose(tab.key)}
              className={cx(
                "flex size-5 shrink-0 items-center justify-center rounded text-muted hover:bg-subtle hover:text-ink",
                tab.key !== active && "opacity-0 group-hover:opacity-100",
              )}
            >
              <X size={13} />
            </button>
          </div>
        ))}
      </div>
      {actions && <div className="flex shrink-0 items-center gap-0.5 px-1.5">{actions}</div>}
      <ContextMenu at={ctx?.at ?? null} items={ctx ? menu(ctx.key) : []} onClose={() => setCtx(null)} />
    </div>
  );
}
