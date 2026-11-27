import { ChevronRight, CopyPlus, Download, FileInput, FolderInput, FolderPlus, Import, MoreHorizontal, Pencil, Play, Plus, Search, Trash2, Upload, Waypoints, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type HTMLAttributes, type ReactNode } from "react";
import { connApi } from "../lib/api";
import { loadGroups, saveGroups } from "../lib/groups";
import { colorClass, connAddress, jumpRef } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { useConns, useToast } from "../lib/store";
import type { MessageKey } from "../locales/zh-CN";
import type { ConnView } from "../lib/types";
import ConnEditor from "./ConnEditor";
import KindIcon from "./KindIcon";
import { Button, Confirm, ContextMenu, cx, Dialog, Field, IconButton, Input, Menu, ProdBadge, type MenuItem } from "./ui";

interface Props {
  title: string;
  emptyDesc: string;
  onOpen: (c: ConnView) => void;
  /** 右键菜单里紧跟“打开”的其他打开方式。 */
  openItems?: (c: ConnView) => MenuItem[];
}

/** 按下后移动超过这么多像素才开始拖动，避免点击时误触。 */
const DRAG_THRESHOLD = 4;

type OtherImport = "finalshell" | "moba";

/** 「从其他终端导入」里的来源。新增一种终端时在这里加一项，并接上对应的导入函数。 */
const OTHER_SOURCES: { kind: OtherImport; label: MessageKey; hint: MessageKey; icon: ReactNode }[] = [
  { kind: "finalshell", label: "conn.importFinalShell", hint: "conn.importFinalShellHint", icon: <FolderInput size={16} /> },
  { kind: "moba", label: "conn.importMobaXterm", hint: "conn.importMobaXtermHint", icon: <FileInput size={16} /> },
];

function useCollapsed(key: string) {
  const storageKey = `connList.collapsed.${key}`;
  const [collapsed, setCollapsed] = useState<Set<string>>(() => {
    try {
      return new Set(JSON.parse(localStorage.getItem(storageKey) ?? "[]") as string[]);
    } catch {
      return new Set();
    }
  });
  const toggle = (group: string) =>
    setCollapsed((cur) => {
      const next = new Set(cur);
      if (!next.delete(group)) next.add(group);
      localStorage.setItem(storageKey, JSON.stringify([...next]));
      return next;
    });
  return [collapsed, toggle] as const;
}

/** 左侧的连接列表：按分组显示，搜索，拖动排序，右键菜单。 */
export default function ConnList({ title, emptyDesc, onOpen, openItems }: Props) {
  const { t, tx } = useI18n();
  const toast = useToast();
  const { conns, loaded, reorder } = useConns();
  const [q, setQ] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [editor, setEditor] = useState<{ open: boolean; conn: ConnView | null; group?: string }>({ open: false, conn: null });
  const [menu, setMenu] = useState<{ at: { x: number; y: number }; conn: ConnView } | null>(null);
  const [deleting, setDeleting] = useState<ConnView | null>(null);
  const [inUse, setInUse] = useState<{ conn: ConnView; users: ConnView[] } | null>(null);
  const [collapsed, toggleGroup] = useCollapsed("ssh");
  const [drag, setDrag] = useState<{ id: string; order: string[] } | null>(null);
  const [storedGroups, setStoredGroups] = useState(loadGroups);
  const [naming, setNaming] = useState(false);
  const [groupName, setGroupName] = useState("");
  const [groupError, setGroupError] = useState("");
  const [otherOpen, setOtherOpen] = useState(false);

  const filtered = useMemo(() => {
    const s = q.trim().toLowerCase();
    if (!s) return conns;
    return conns.filter((c) => [c.name, connAddress(c), c.group ?? "", c.note ?? ""].some((v) => v.toLowerCase().includes(s)));
  }, [conns, q]);

  // 拖动过程中按临时顺序显示，松开后提交。
  const shown = useMemo(() => {
    if (!drag) return filtered;
    const rank = new Map(drag.order.map((id, i) => [id, i]));
    return [...filtered].sort((a, b) => rank.get(a.id)! - rank.get(b.id)!);
  }, [filtered, drag]);

  // 有名字的分组按首次出现的顺序排在前面，未分组的放最后。
  const groups = useMemo(() => {
    const map = new Map<string, ConnView[]>();
    for (const c of shown) {
      const g = c.group ?? "";
      if (!map.has(g)) map.set(g, []);
      map.get(g)!.push(c);
    }
    const ungrouped = map.get("");
    map.delete("");
    const list = [...map.entries()];
    if (ungrouped) list.push(["", ungrouped]);
    return list;
  }, [shown]);

  // 显式新建的分组排在前面，没有连接时也显示；其余分组仍按连接里的顺序。
  const rows = useMemo(() => {
    const qn = q.trim().toLowerCase();
    const byGroup = new Map(groups);
    const seen = new Set<string>();
    const list: [string, ConnView[]][] = [];
    for (const name of storedGroups) {
      const items = byGroup.get(name) ?? [];
      const nameHit = !qn || name.toLowerCase().includes(qn);
      if (items.length > 0 || nameHit) list.push([name, items]);
      seen.add(name);
    }
    for (const [g, items] of groups) {
      if (g && seen.has(g)) continue;
      list.push([g, items]);
    }
    return list;
  }, [groups, storedGroups, q]);

  const knownGroups = useMemo(() => {
    const names = new Set(storedGroups);
    for (const c of conns) if (c.group) names.add(c.group);
    return names;
  }, [conns, storedGroups]);

  const openNew = (group?: string) => setEditor({ open: true, conn: null, group });

  const addGroup = (name: string) => {
    const next = [name, ...storedGroups.filter((g) => g !== name)];
    setStoredGroups(next);
    saveGroups(next);
  };

  const removeGroup = (name: string) => {
    const next = storedGroups.filter((g) => g !== name);
    setStoredGroups(next);
    saveGroups(next);
  };

  const submitGroup = () => {
    const name = groupName.trim();
    if (!name) {
      setGroupError(t("conn.groupRequired"));
      return;
    }
    if (knownGroups.has(name)) {
      setGroupError(t("conn.groupExists"));
      return;
    }
    addGroup(name);
    setNaming(false);
  };

  const byId = useMemo(() => new Map(conns.map((c) => [c.id, c])), [conns]);
  const searching = !!q.trim();

  const run = async (fn: () => Promise<unknown>, ok?: string) => {
    try {
      await fn();
      if (ok) toast.success(ok);
    } catch (e) {
      toast.error(e);
    }
  };

  const askDelete = (c: ConnView) => {
    const users = conns.filter((x) => jumpRef(x) === c.id);
    if (users.length) setInUse({ conn: c, users });
    else setDeleting(c);
  };

  const importConns = (kind: "file" | OtherImport = "file") =>
    run(async () => {
      const n =
        kind === "finalshell" ? await connApi.importFinalShellDir() : kind === "moba" ? await connApi.importMobaXterm() : await connApi.importFrom();
      if (n !== null) toast.success(t("conn.imported", { count: n }));
    });

  const exportConns = (ids: string[] = []) =>
    run(async () => {
      const r = await connApi.exportTo(ids);
      if (r) toast.success(t("conn.exported", { count: r.count, path: r.path }));
    });

  const itemMenu = (c: ConnView): MenuItem[] => [
    { label: t("common.open"), icon: <Play size={14} />, onClick: () => onOpen(c) },
    ...(openItems?.(c) ?? []),
    { label: t("common.edit"), icon: <Pencil size={14} />, divider: !!openItems, onClick: () => setEditor({ open: true, conn: c }) },
    { label: t("conn.duplicate"), icon: <CopyPlus size={14} />, onClick: () => run(() => connApi.duplicate(c.id), t("conn.duplicated")) },
    { label: t("conn.export"), icon: <Upload size={14} />, onClick: () => exportConns([c.id]) },
    { label: t("common.delete"), icon: <Trash2 size={14} />, danger: true, divider: true, onClick: () => askDelete(c) },
  ];

  // 用指针事件拖动排序：Windows 上原生的文件拖放处理会让 WebView 里的 HTML5 拖放失效。
  const pointer = useRef<{ id: string; x: number; y: number; order: string[] | null } | null>(null);
  useEffect(() => {
    const move = (e: PointerEvent) => {
      const p = pointer.current;
      if (!p) return;
      if (!p.order) {
        if (Math.hypot(e.clientX - p.x, e.clientY - p.y) < DRAG_THRESHOLD) return;
        p.order = conns.map((x) => x.id);
        setDrag({ id: p.id, order: p.order });
      }
      const over = document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>("[data-conn]")?.dataset.conn;
      // 只能在同一分组内拖动。
      if (!over || over === p.id || (byId.get(over)?.group ?? "") !== (byId.get(p.id)?.group ?? "")) return;
      const { order } = p;
      const next = order.filter((id) => id !== p.id);
      next.splice(next.indexOf(over) + (order.indexOf(over) > order.indexOf(p.id) ? 1 : 0), 0, p.id);
      if (next.every((id, i) => id === order[i])) return;
      p.order = next;
      setDrag({ id: p.id, order: next });
    };
    const end = (commit: boolean) => () => {
      const p = pointer.current;
      pointer.current = null;
      if (!p?.order) return;
      if (commit) reorder(p.order);
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
  }, [conns, byId, reorder]);

  const dragProps = (c: ConnView): HTMLAttributes<HTMLDivElement> => {
    if (searching) return {};
    return {
      onPointerDown: (e) => {
        if (e.button === 0) pointer.current = { id: c.id, x: e.clientX, y: e.clientY, order: null };
      },
    };
  };

  const item = (c: ConnView) => {
    const ref = jumpRef(c);
    const refConn = ref ? byId.get(ref) : undefined;
    const color = colorClass(c.color);
    return (
      <div
        key={c.id}
        data-conn={c.id}
        {...dragProps(c)}
        onClick={() => setSelected(c.id)}
        onDoubleClick={() => onOpen(c)}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
          setSelected(c.id);
          setMenu({ at: { x: e.clientX, y: e.clientY }, conn: c });
        }}
        title={c.note || undefined}
        className={cx(
          "group flex items-center gap-2 rounded-lg px-2 py-1.5 select-none",
          selected === c.id ? "bg-accent/10" : "hover:bg-subtle",
          drag?.id === c.id && "opacity-40",
        )}
      >
        <KindIcon kind={c.kind} />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            {color && <span className={cx("size-2 shrink-0 rounded-full", color)} />}
            <span className={cx("truncate text-[13px] font-medium", c.production && "text-red-600 dark:text-red-400")}>{c.name}</span>
            {c.production && <ProdBadge />}
          </div>
          <div className="truncate font-mono text-[11px] text-muted">{connAddress(c)}</div>
        </div>
        {ref && (
          <span className="shrink-0 text-muted" title={t("conn.viaJump", { name: refConn?.name ?? t("conn.missing") })}>
            <Waypoints size={13} />
          </span>
        )}
      </div>
    );
  };

  return (
    <aside className={cx("flex w-1/4 shrink-0 flex-col border-r border-line bg-surface/60", drag && "cursor-grabbing")}>
      <div className="shrink-0 px-3 pt-2.5">
        <Input
          className="h-8 text-[13px]"
          placeholder={t("conn.search")}
          value={q}
          onChange={(e) => setQ(e.target.value)}
          suffix={<Search size={13} className="mr-1.5 text-muted" />}
        />
      </div>
      <div className="flex h-11 shrink-0 items-center gap-1 pr-2 pl-4">
        <span className="flex-1 truncate text-xs font-semibold tracking-wide text-muted">{title}</span>
        <Menu
          items={[
            { label: t("conn.new"), icon: <Plus size={14} />, onClick: () => openNew() },
            {
              label: t("conn.newGroup"),
              icon: <FolderPlus size={14} />,
              onClick: () => {
                setGroupName("");
                setGroupError("");
                setNaming(true);
              },
            },
          ]}
          trigger={(open) => (
            <IconButton label={t("conn.add")} size="sm" onClick={open}>
              <Plus size={16} />
            </IconButton>
          )}
        />
        <Menu
          items={[
            { label: t("conn.exportAll"), icon: <Upload size={14} />, disabled: conns.length === 0, onClick: () => exportConns() },
            { label: t("conn.import"), icon: <Download size={14} />, onClick: () => importConns() },
            { label: t("conn.importOthers"), icon: <Import size={14} />, onClick: () => setOtherOpen(true) },
          ]}
          trigger={(open) => (
            <IconButton label={t("common.more")} size="sm" onClick={open}>
              <MoreHorizontal size={16} />
            </IconButton>
          )}
        />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
        {loaded && conns.length === 0 && storedGroups.length === 0 ? (
          <div className="flex flex-col items-center px-3 pt-10 text-center">
            <div className="text-[13px] font-medium">{t("conn.empty")}</div>
            <div className="mt-1 text-xs text-muted">{emptyDesc}</div>
            <Button className="mt-4" size="sm" variant="primary" icon={<Plus size={14} />} onClick={() => openNew()}>
              {t("conn.new")}
            </Button>
          </div>
        ) : rows.length === 0 && searching ? (
          <div className="px-3 pt-10 text-center text-xs text-muted">{t("conn.noMatch")}</div>
        ) : (
          rows.map(([g, list]) => {
            // 只有未分组的连接时不显示分组标题。
            if (!g && rows.length === 1) return <div key="">{list.map(item)}</div>;
            const folded = collapsed.has(g) && !searching;
            return (
              <div key={g || "ungrouped"} className="group/g mb-1">
                <div className="flex items-center gap-0.5 rounded-md pr-0.5 text-[11px] font-semibold tracking-wide text-muted">
                  <button
                    className="flex min-w-0 flex-1 items-center gap-1 rounded-md px-1.5 py-1 text-left hover:text-ink"
                    onClick={() => toggleGroup(g)}
                  >
                    <ChevronRight size={13} className={cx("shrink-0 transition-transform", !folded && "rotate-90")} />
                    <span className="truncate">{g || t("conn.ungrouped")}</span>
                    <span className="ml-auto font-normal tabular-nums">{list.length}</span>
                  </button>
                  {g && (
                    <button
                      className="flex size-5 shrink-0 items-center justify-center rounded text-muted hover:bg-subtle hover:text-ink"
                      title={t("conn.newInGroup")}
                      onClick={() => openNew(g)}
                    >
                      <Plus size={12} />
                    </button>
                  )}
                  {g && list.length === 0 && (
                    <button
                      className="flex size-5 shrink-0 items-center justify-center rounded text-muted opacity-0 hover:bg-subtle hover:text-ink group-hover/g:opacity-100"
                      title={t("conn.deleteGroup")}
                      onClick={() => removeGroup(g)}
                    >
                      <X size={12} />
                    </button>
                  )}
                </div>
                {!folded && list.map(item)}
              </div>
            );
          })
        )}
      </div>

      <ContextMenu at={menu?.at ?? null} items={menu ? itemMenu(menu.conn) : []} onClose={() => setMenu(null)} />

      <ConnEditor open={editor.open} conn={editor.conn} group={editor.group} onClose={() => setEditor((s) => ({ ...s, open: false }))} />

      <Dialog open={otherOpen} onClose={() => setOtherOpen(false)} title={t("conn.importOthers")} width="max-w-sm">
        <p className="mb-2 text-sm text-muted">{t("conn.importOthersHint")}</p>
        <div className="mb-2 flex flex-col gap-1">
          {OTHER_SOURCES.map((src) => (
            <button
              key={src.kind}
              className="flex items-center gap-3 rounded-lg px-2 py-2 text-left hover:bg-subtle"
              onClick={() => {
                setOtherOpen(false);
                importConns(src.kind);
              }}
            >
              <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-subtle text-muted">{src.icon}</span>
              <span className="min-w-0 flex-1">
                <span className="block text-sm font-medium">{t(src.label)}</span>
                <span className="block text-xs text-muted">{t(src.hint)}</span>
              </span>
              <ChevronRight size={14} className="shrink-0 text-muted" />
            </button>
          ))}
        </div>
      </Dialog>

      <Dialog
        open={naming}
        onClose={() => setNaming(false)}
        title={t("conn.newGroup")}
        width="max-w-sm"
        footer={
          <>
            <Button onClick={() => setNaming(false)}>{t("common.cancel")}</Button>
            <Button variant="primary" onClick={submitGroup}>
              {t("common.confirm")}
            </Button>
          </>
        }
      >
        <Field label={t("conn.groupName")} error={groupError}>
          <Input
            autoFocus
            placeholder={t("conn.groupNamePlaceholder")}
            value={groupName}
            invalid={!!groupError}
            onChange={(e) => {
              setGroupName(e.target.value);
              if (groupError) setGroupError("");
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") submitGroup();
            }}
          />
        </Field>
      </Dialog>

      <Confirm
        open={!!deleting}
        title={t("conn.deleteTitle")}
        message={tx("conn.deleteMessage", { name: <span className="font-medium text-ink">{deleting?.name}</span> })}
        confirmText={t("common.delete")}
        danger
        onConfirm={() => deleting && run(() => connApi.remove(deleting.id), t("conn.deleted"))}
        onClose={() => setDeleting(null)}
      />

      <Dialog
        open={!!inUse}
        onClose={() => setInUse(null)}
        title={t("conn.inUseTitle")}
        width="max-w-sm"
        footer={<Button onClick={() => setInUse(null)}>{t("common.close")}</Button>}
      >
        <div className="text-sm text-muted">{tx("conn.inUseMessage", { name: <span className="font-medium text-ink">{inUse?.conn.name}</span> })}</div>
        <ul className="mt-2 mb-1 flex flex-col gap-1">
          {inUse?.users.map((u) => (
            <li key={u.id} className="flex items-center gap-2 text-sm">
              <KindIcon kind={u.kind} />
              <span className="truncate">{u.name}</span>
            </li>
          ))}
        </ul>
      </Dialog>
    </aside>
  );
}
