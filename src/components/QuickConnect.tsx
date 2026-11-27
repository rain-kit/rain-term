import { History, Search } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { colorClass, connAddress } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { useConns } from "../lib/store";
import type { ConnView, TabKind } from "../lib/types";
import { cx, IncognitoBadge, ProdBadge } from "./ui";

const Kbd = ({ children }: { children: ReactNode }) => (
  <kbd className="rounded border border-line bg-subtle px-1 py-px font-sans text-[10px] text-ink">{children}</kbd>
);

/** 快速连接：搜索全部连接，最近打开的排在前面；Enter 打开终端，Shift+Enter 打开 SFTP。 */
export default function QuickConnect({
  open,
  recent,
  onClose,
  onPick,
}: {
  open: boolean;
  /** 最近打开的连接 ID，最近的在前。 */
  recent: string[];
  onClose: () => void;
  onPick: (c: ConnView, kind: TabKind) => void;
}) {
  const { t } = useI18n();
  const { conns } = useConns();
  const [q, setQ] = useState("");
  const [index, setIndex] = useState(0);
  const list = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    setQ("");
    setIndex(0);
  }, [open]);

  const rank = useMemo(() => new Map(recent.map((id, i) => [id, i])), [recent]);
  const items = useMemo(() => {
    const words = q.trim().toLowerCase().split(/\s+/).filter(Boolean);
    const hits = conns.filter((c) => {
      const text = [c.name, connAddress(c), c.group ?? "", c.note ?? ""].join("\n").toLowerCase();
      return words.every((w) => text.includes(w));
    });
    // 最近打开的按时间排在前面，其余保持连接列表的顺序。
    const r = (c: ConnView) => rank.get(c.id) ?? Number.MAX_SAFE_INTEGER;
    return hits.sort((a, b) => r(a) - r(b));
  }, [conns, q, rank]);

  useEffect(() => {
    list.current?.querySelector(`[data-index="${index}"]`)?.scrollIntoView({ block: "nearest" });
  }, [index, items]);

  if (!open) return null;

  const pick = (c: ConnView | undefined, sftp: boolean) => {
    if (!c) return;
    onClose();
    onPick(c, sftp ? "sftp" : "term");
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!items.length) return;
      const step = e.key === "ArrowDown" ? 1 : -1;
      setIndex((i) => (i + step + items.length) % items.length);
    } else if (e.key === "Enter") {
      e.preventDefault();
      pick(items[index], e.shiftKey);
    }
  };

  return createPortal(
    <div className="fixed inset-0 z-50 flex justify-center px-6 pt-[12vh]">
      <div className="absolute inset-0 animate-fade bg-black/40 dark:bg-black/60" onMouseDown={onClose} />
      <div className="relative flex max-h-[60vh] w-full max-w-lg animate-pop flex-col self-start overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl">
        <div className="flex items-center gap-2.5 border-b border-line px-4">
          <Search size={16} className="shrink-0 text-muted" />
          <input
            autoFocus
            spellCheck={false}
            autoComplete="off"
            value={q}
            placeholder={t("quick.placeholder")}
            onChange={(e) => {
              setQ(e.target.value);
              setIndex(0);
            }}
            onKeyDown={onKeyDown}
            className="h-12 min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted/60"
          />
        </div>
        <div ref={list} className="min-h-0 flex-1 overflow-y-auto p-1.5">
          {items.length === 0 ? (
            <div className="py-8 text-center text-xs text-muted">{t(conns.length ? "conn.noMatch" : "conn.empty")}</div>
          ) : (
            items.map((c, i) => {
              const color = colorClass(c.color);
              return (
                <div
                  key={c.id}
                  data-index={i}
                  onMouseMove={() => i !== index && setIndex(i)}
                  onClick={(e) => pick(c, e.shiftKey)}
                  className={cx("flex cursor-default items-center gap-2.5 rounded-lg px-2.5 py-2", i === index && "bg-accent/10")}
                >
                  {color ? <span className={cx("size-2 shrink-0 rounded-full", color)} /> : <span className="size-2 shrink-0" />}
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className={cx("truncate text-[13px] font-medium", c.production && "text-red-600 dark:text-red-400")}>{c.name}</span>
                      {c.production && <ProdBadge />}
                      {c.incognito && <IncognitoBadge />}
                      {c.group && <span className="shrink-0 rounded bg-subtle px-1.5 text-[11px] text-muted">{c.group}</span>}
                    </div>
                    <div className="truncate font-mono text-[11px] text-muted">{connAddress(c)}</div>
                  </div>
                  {rank.has(c.id) && (
                    <span className="shrink-0 text-muted" title={t("quick.recent")}>
                      <History size={13} />
                    </span>
                  )}
                </div>
              );
            })
          )}
        </div>
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-line px-4 py-2 text-[11px] text-muted">
          <span className="flex items-center gap-1">
            <Kbd>↑</Kbd>
            <Kbd>↓</Kbd>
            {t("quick.hintSelect")}
          </span>
          <span className="flex items-center gap-1">
            <Kbd>Enter</Kbd>
            {t("quick.hintTerm")}
          </span>
          <span className="flex items-center gap-1">
            <Kbd>Shift+Enter</Kbd>
            {t("quick.hintSftp")}
          </span>
          <span className="flex items-center gap-1">
            <Kbd>Esc</Kbd>
            {t("common.close")}
          </span>
        </div>
      </div>
    </div>,
    document.body,
  );
}
