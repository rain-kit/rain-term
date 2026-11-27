import { History, Search, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { colorClass, connAddress } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { useConns } from "../lib/store";
import { cx, IconButton, IncognitoBadge, ProdBadge } from "./ui";

/** 选择经哪台服务器访问 GitHub：列出全部 SSH 连接，可搜索，最近选过的排在最前。 */
export default function JumpPicker({
  open,
  last,
  onClose,
  onPick,
}: {
  open: boolean;
  /** 最近选过的连接 ID。 */
  last: string | null;
  onClose: () => void;
  onPick: (connId: string) => void;
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

  useEffect(() => {
    if (!open) return;
    // 可能叠在更新窗口上面，在捕获阶段拦下 Esc，只关闭这个框。
    const fn = (e: globalThis.KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      onClose();
    };
    window.addEventListener("keydown", fn, true);
    return () => window.removeEventListener("keydown", fn, true);
  }, [open, onClose]);

  const items = useMemo(() => {
    const words = q.trim().toLowerCase().split(/\s+/).filter(Boolean);
    const hits = conns.filter((c) => {
      const text = [c.name, connAddress(c), c.group ?? "", c.note ?? ""].join("\n").toLowerCase();
      return words.every((w) => text.includes(w));
    });
    return hits.sort((a, b) => Number(b.id === last) - Number(a.id === last));
  }, [conns, q, last]);

  useEffect(() => {
    list.current?.querySelector(`[data-index="${index}"]`)?.scrollIntoView({ block: "nearest" });
  }, [index, items]);

  if (!open) return null;

  const pick = (id: string | undefined) => {
    if (!id) return;
    onClose();
    onPick(id);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!items.length) return;
      const step = e.key === "ArrowDown" ? 1 : -1;
      setIndex((i) => (i + step + items.length) % items.length);
    } else if (e.key === "Enter") {
      e.preventDefault();
      pick(items[index]?.id);
    }
  };

  return createPortal(
    <div className="fixed inset-0 z-50 flex justify-center px-6 pt-[12vh]">
      <div className="absolute inset-0 animate-fade bg-black/40 dark:bg-black/60" onMouseDown={onClose} />
      <div
        role="dialog"
        aria-modal="true"
        className="relative flex max-h-[60vh] w-full max-w-lg animate-pop flex-col self-start overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl"
      >
        <div className="flex items-center gap-2 px-4 pt-3.5 pb-1">
          <div className="flex-1 text-base font-semibold">{t("update.pickJumpTitle")}</div>
          <IconButton label={t("common.close")} size="sm" onClick={onClose}>
            <X size={16} />
          </IconButton>
        </div>
        <div className="px-4 pb-2 text-xs text-muted">{t("update.pickJumpHint")}</div>
        <div className="flex items-center gap-2.5 border-y border-line px-4">
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
            className="h-11 min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted/60"
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
                  onClick={() => pick(c.id)}
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
                  {c.id === last && (
                    <span className="shrink-0 text-muted" title={t("update.lastJump")}>
                      <History size={13} />
                    </span>
                  )}
                </div>
              );
            })
          )}
        </div>
      </div>
    </div>,
    document.body,
  );
}
