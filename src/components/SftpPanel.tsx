import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import {
  ArrowUp,
  ChevronDown,
  Copy,
  Download,
  File,
  FileSymlink,
  Folder,
  FolderPlus,
  FolderSymlink,
  House,
  LoaderCircle,
  Pencil,
  RotateCw,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent, type MouseEvent } from "react";
import { errorMessage, sftpApi } from "../lib/api";
import { formatBytes, formatTime } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { useToast } from "../lib/store";
import type { FileEntry, I18nMsg, SftpEvent, Transfer } from "../lib/types";
import type { TabStatus } from "./TabBar";
import { Button, Confirm, ContextMenu, cx, Dialog, IconButton, Input, type MenuItem } from "./ui";

type TaskState = "queued" | "running" | "done" | "failed" | "cancelled";

interface Task extends Transfer {
  state: TaskState;
  done: number;
  total: number;
  /** 字节每秒，开始传输约一秒后才有。 */
  speed?: number;
  error?: string;
  /** 上次计算速度的时间和当时已传的字节数。 */
  mark: { at: number; done: number };
}

const SPEED_WINDOW = 1000;
const REFRESH_DELAY = 300;
const DOWNLOAD_DIR_KEY = "sftp.downloadDir";
const COLS = "grid grid-cols-[minmax(0,1fr)_6rem_9rem_6.5rem] gap-3";
/** 列表只渲染可见的行；行高、上边距要和行的 h-7、列表的 py-1 一致。 */
const ROW_H = 28;
const LIST_PAD = 4;
const OVERSCAN = 10;

const parentOf = (p: string) => {
  const i = p.replace(/\/+$/, "").lastIndexOf("/");
  return i <= 0 ? "/" : p.slice(0, i);
};
const join = (dir: string, name: string) => (dir.endsWith("/") ? dir + name : `${dir}/${name}`);
/** 本地路径的最后一段，和后端 upload 里 Path::file_name 的结果一致。 */
const localName = (p: string) => p.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || p;
const isDir = (e: FileEntry) => e.kind === "dir" || !!e.linkDir;
const validName = (s: string) => !!s && s !== "." && s !== ".." && !s.includes("/");
const finished = (s: TaskState) => s === "done" || s === "failed" || s === "cancelled";

type TaskEvent = Exclude<SftpEvent, { type: "closed" }>;

function apply(x: Task, e: TaskEvent, now: number, tm: (m: I18nMsg) => string): Task {
  switch (e.type) {
    case "started":
      return { ...x, state: "running", total: e.total, done: 0, mark: { at: now, done: 0 } };
    case "progress": {
      if (now - x.mark.at < SPEED_WINDOW) return { ...x, done: e.done, total: e.total };
      const speed = ((e.done - x.mark.done) * 1000) / (now - x.mark.at);
      return { ...x, done: e.done, total: e.total, speed, mark: { at: now, done: e.done } };
    }
    case "done":
      return { ...x, state: "done", done: x.total };
    case "failed":
      return { ...x, state: "failed", error: tm(e.error) };
    case "cancelled":
      return { ...x, state: "cancelled" };
  }
}

function EntryIcon({ e }: { e: FileEntry }) {
  if (e.kind === "dir") return <Folder size={15} className="shrink-0 text-accent" />;
  if (e.linkDir) return <FolderSymlink size={15} className="shrink-0 text-accent" />;
  if (e.kind === "symlink") return <FileSymlink size={15} className="shrink-0 text-muted" />;
  return <File size={15} className="shrink-0 text-muted" />;
}

/** 一个 SFTP 标签：浏览远程目录、管理文件、上传下载，断开后可以重新连接。 */
export default function SftpPanel({
  connId,
  name,
  active,
  onStatus,
  onBusy,
}: {
  connId: string;
  name: string;
  active: boolean;
  onStatus: (s: TabStatus) => void;
  /** 排队中和传输中的任务数。 */
  onBusy: (n: number) => void;
}) {
  const { t, tm, tx } = useI18n();
  const toast = useToast();
  const panel = useRef<string | null>(null);
  /** 每次连接加一，旧连接迟到的消息据此丢弃。 */
  const gen = useRef(0);
  const loadSeq = useRef(0);
  const [status, setStatus] = useState<{ s: TabStatus; message?: string }>({ s: "connecting" });
  const [home, setHome] = useState("");
  const [cwd, setCwd] = useState("");
  const cwdRef = useRef("");
  cwdRef.current = cwd;
  const [pathInput, setPathInput] = useState("");
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [sel, setSel] = useState<Set<string>>(new Set());
  const anchor = useRef(-1);
  const [menu, setMenu] = useState<{ at: { x: number; y: number }; items: MenuItem[] } | null>(null);
  const [naming, setNaming] = useState<{ entry?: FileEntry; value: string } | null>(null);
  const [deleting, setDeleting] = useState<FileEntry[] | null>(null);
  const [overwriting, setOverwriting] = useState<{ paths: string[]; dir: string; names: string[] } | null>(null);
  const [dropping, setDropping] = useState(false);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [queueOpen, setQueueOpen] = useState(true);
  /** 传输任务的信息；任务事件可能先于 upload / download 的返回值到达，先存起来。 */
  const known = useRef(new Map<string, Transfer>());
  const early = useRef(new Map<string, TaskEvent[]>());
  const refreshTimer = useRef<number | undefined>(undefined);
  const box = useRef<HTMLDivElement>(null);
  const listEl = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ top: 0, height: 0 });
  const nameInput = useRef<HTMLInputElement>(null);
  const onStatusRef = useRef(onStatus);
  onStatusRef.current = onStatus;
  const onBusyRef = useRef(onBusy);
  onBusyRef.current = onBusy;

  const connected = status.s === "connected";
  const selected = entries.filter((e) => sel.has(e.path));
  const busy = tasks.filter((x) => !finished(x.state)).length;

  useEffect(() => onStatusRef.current(status.s), [status.s]);
  useEffect(() => onBusyRef.current(busy), [busy]);

  useEffect(() => {
    const el = listEl.current!;
    const ro = new ResizeObserver(() => setView({ top: el.scrollTop, height: el.clientHeight }));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const run = async (fn: (id: string) => Promise<unknown>) => {
    const id = panel.current;
    if (!id) return;
    try {
      await fn(id);
    } catch (e) {
      toast.error(e);
    }
  };

  /** 列出目录，成功返回 true；keep 为 true 时保留仍存在的选中项。 */
  const load = async (path: string, keep = false) => {
    const id = panel.current;
    if (!id) return false;
    const my = ++loadSeq.current;
    setLoading(true);
    try {
      const l = await sftpApi.list(id, path);
      if (my !== loadSeq.current) return true;
      setCwd(l.path);
      setPathInput(l.path);
      setEntries(l.entries);
      const paths = new Set(l.entries.map((e) => e.path));
      setSel((s) => (keep ? new Set([...s].filter((p) => paths.has(p))) : new Set()));
      if (!keep) {
        anchor.current = -1;
        if (listEl.current) listEl.current.scrollTop = 0;
      }
      return true;
    } catch (e) {
      if (my === loadSeq.current) {
        toast.error(e);
        setPathInput(cwdRef.current);
      }
      return false;
    } finally {
      if (my === loadSeq.current) setLoading(false);
    }
  };

  const refresh = () => load(cwdRef.current, true);

  const scheduleRefresh = () => {
    clearTimeout(refreshTimer.current);
    refreshTimer.current = window.setTimeout(refresh, REFRESH_DELAY);
  };

  const onTaskEvent = (e: TaskEvent) => {
    const tr = known.current.get(e.id);
    if (!tr) {
      early.current.set(e.id, [...(early.current.get(e.id) ?? []), e]);
      return;
    }
    const now = Date.now();
    setTasks((list) => list.map((x) => (x.id === e.id ? apply(x, e, now, tm) : x)));
    // 上传结束后刷新目标目录；失败、取消时没传完的文件已被删除，也要刷新。
    if (e.type !== "started" && e.type !== "progress" && tr.direction === "upload" && parentOf(tr.dst) === cwdRef.current) scheduleRefresh();
  };

  const addTasks = (list: Transfer[]) => {
    const now = Date.now();
    const added = list.map((tr) => {
      known.current.set(tr.id, tr);
      let task: Task = { ...tr, state: "queued", done: 0, total: 0, mark: { at: now, done: 0 } };
      for (const e of early.current.get(tr.id) ?? []) task = apply(task, e, now, tm);
      early.current.delete(tr.id);
      return task;
    });
    setTasks((cur) => [...cur, ...added]);
    setQueueOpen(true);
  };

  const connect = (path?: string) => {
    const my = ++gen.current;
    let ended = false;
    setStatus({ s: "connecting" });
    sftpApi
      .open(connId, (e) => {
        if (gen.current !== my) return;
        if (e.type !== "closed") return onTaskEvent(e);
        ended = true;
        panel.current = null;
        const message = tm(e.error);
        setStatus({ s: "closed", message });
        setTasks((list) => list.map((x) => (finished(x.state) ? x : { ...x, state: "failed", error: message })));
      })
      .then(async (o) => {
        if (gen.current !== my || ended) {
          if (gen.current !== my) sftpApi.close(o.id).catch(() => {});
          return;
        }
        panel.current = o.id;
        setHome(o.path);
        setStatus({ s: "connected" });
        if (!(await load(path ?? o.path)) && path) await load(o.path);
      })
      .catch((err) => {
        if (gen.current === my) setStatus({ s: "closed", message: errorMessage(err) });
      });
  };

  useEffect(() => {
    connect();
    return () => {
      gen.current++;
      clearTimeout(refreshTimer.current);
      if (panel.current) sftpApi.close(panel.current).catch(() => {});
      panel.current = null;
    };
  }, [connId]);

  const open = (e: FileEntry) => (isDir(e) ? load(e.path) : download([e]));
  const up = () => cwd && cwd !== "/" && load(parentOf(cwd));

  /** 目标目录里已有同名的文件或文件夹时先确认；按服务器上的最新内容判断，不用可能过时的列表。 */
  const upload = (paths?: string[]) =>
    run(async (id) => {
      const list = paths ?? (await sftpApi.pickUploadFiles());
      if (!list.length) return;
      const dir = cwdRef.current;
      const existing = new Set((await sftpApi.list(id, dir)).entries.map((e) => e.name));
      const names = list.map(localName).filter((n) => existing.has(n));
      if (names.length) setOverwriting({ paths: list, dir, names });
      else addTasks(await sftpApi.upload(id, list, dir));
    });
  const uploadRef = useRef(upload);
  uploadRef.current = upload;

  const download = (list: FileEntry[]) =>
    run(async (id) => {
      const dir = await sftpApi.pickDownloadDir(localStorage.getItem(DOWNLOAD_DIR_KEY) ?? undefined);
      if (!dir) return;
      localStorage.setItem(DOWNLOAD_DIR_KEY, dir);
      addTasks(await sftpApi.download(id, list.map((e) => e.path), dir));
    });

  const copyPath = (list: FileEntry[]) =>
    writeText(list.map((e) => e.path).join("\n")).then(() => toast.success(t("sftp.pathCopied")), toast.error);

  const submitName = () => {
    if (!naming) return;
    const value = naming.value.trim();
    if (!validName(value)) return;
    const { entry } = naming;
    setNaming(null);
    run(async (id) => {
      const to = join(cwdRef.current, value);
      if (entry) {
        if (entry.name === value) return;
        await sftpApi.rename(id, entry.path, to);
      } else await sftpApi.mkdir(id, to);
      await load(cwdRef.current);
      setSel(new Set([to]));
    });
  };

  const remove = (list: FileEntry[]) =>
    run(async (id) => {
      try {
        await sftpApi.remove(id, list.map((e) => e.path));
        toast.success(t("sftp.deleted"));
      } finally {
        await refresh();
      }
    });

  // 重命名时默认选中扩展名之前的部分。
  useEffect(() => {
    const el = nameInput.current;
    if (!naming || !el) return;
    const dot = naming.entry && !isDir(naming.entry) ? naming.value.lastIndexOf(".") : -1;
    el.setSelectionRange(0, dot > 0 ? dot : naming.value.length);
  }, [naming !== null]);

  // 从系统拖入文件上传到当前目录；只有当前标签响应。
  useEffect(() => {
    if (!active || !isTauri()) return;
    const inside = ({ x, y }: { x: number; y: number }) => {
      const r = box.current?.getBoundingClientRect();
      const [px, py] = [x / devicePixelRatio, y / devicePixelRatio];
      return !!r && px >= r.left && px <= r.right && py >= r.top && py <= r.bottom;
    };
    let un: (() => void) | undefined;
    let cancelled = false;
    getCurrentWebview()
      .onDragDropEvent(({ payload: p }) => {
        if (p.type === "leave") setDropping(false);
        else if (p.type === "enter" || p.type === "over") setDropping(!!panel.current && inside(p.position));
        else if (p.type === "drop") {
          setDropping(false);
          if (panel.current && p.paths.length && inside(p.position)) uploadRef.current(p.paths);
        }
      })
      .then((u) => {
        if (cancelled) u();
        else un = u;
      });
    return () => {
      cancelled = true;
      un?.();
      setDropping(false);
    };
  }, [active]);

  const click = (e: MouseEvent, i: number) => {
    const p = entries[i].path;
    if (e.shiftKey && anchor.current >= 0) {
      const [a, b] = [anchor.current, i].sort((x, y) => x - y);
      setSel(new Set(entries.slice(a, b + 1).map((x) => x.path)));
      return;
    }
    anchor.current = i;
    if (e.ctrlKey || e.metaKey)
      setSel((s) => {
        const next = new Set(s);
        if (!next.delete(p)) next.add(p);
        return next;
      });
    else setSel(new Set([p]));
  };

  const select = (i: number) => {
    anchor.current = i;
    setSel(new Set([entries[i].path]));
    const el = listEl.current;
    if (!el) return;
    const top = LIST_PAD + i * ROW_H;
    if (top < el.scrollTop) el.scrollTop = top;
    else if (top + ROW_H > el.scrollTop + el.clientHeight) el.scrollTop = top + ROW_H - el.clientHeight;
  };

  const fileMenu = (list: FileEntry[]): MenuItem[] => [
    ...(list.length === 1 && isDir(list[0]) ? [{ label: t("common.open"), icon: <Folder size={14} />, onClick: () => load(list[0].path) }] : []),
    { label: t("sftp.download"), icon: <Download size={14} />, onClick: () => download(list) },
    {
      label: t("sftp.rename"),
      icon: <Pencil size={14} />,
      disabled: list.length !== 1,
      onClick: () => setNaming({ entry: list[0], value: list[0].name }),
    },
    { label: t("sftp.copyPath"), icon: <Copy size={14} />, onClick: () => copyPath(list) },
    { label: t("common.delete"), icon: <Trash2 size={14} />, danger: true, divider: true, onClick: () => setDeleting(list) },
  ];

  const dirMenu: MenuItem[] = [
    { label: t("sftp.newFolder"), icon: <FolderPlus size={14} />, onClick: () => setNaming({ value: "" }) },
    { label: t("sftp.upload"), icon: <Upload size={14} />, onClick: () => upload() },
    { label: t("sftp.refresh"), icon: <RotateCw size={14} />, onClick: refresh },
  ];

  const onKey = (e: KeyboardEvent) => {
    const mod = e.ctrlKey || e.metaKey;
    const i = anchor.current;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      if (!entries.length) return;
      const n = i < 0 ? 0 : Math.max(0, Math.min(entries.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)));
      select(n);
    } else if (e.key === "Enter" && selected.length === 1) open(selected[0]);
    else if (e.key === "Backspace") up();
    else if (e.key === "Delete" && selected.length) setDeleting(selected);
    else if (e.key === "F2" && selected.length === 1) setNaming({ entry: selected[0], value: selected[0].name });
    else if (e.key === "F5") refresh();
    else if (mod && e.key.toLowerCase() === "a") setSel(new Set(entries.map((x) => x.path)));
    else if (mod && e.key.toLowerCase() === "c" && selected.length) copyPath(selected);
    else return;
    e.preventDefault();
  };

  const nameValid = naming ? validName(naming.value.trim()) : true;
  const first = Math.max(0, Math.floor((view.top - LIST_PAD) / ROW_H) - OVERSCAN);
  const last = Math.min(entries.length, Math.ceil((view.top + view.height) / ROW_H) + OVERSCAN);

  return (
    <div ref={box} className="relative flex min-h-0 flex-1 flex-col bg-canvas">
      <div className="flex h-10 shrink-0 items-center gap-1 border-b border-line px-2">
        <IconButton label={t("sftp.up")} size="sm" disabled={!connected || cwd === "/"} onClick={up}>
          <ArrowUp size={16} />
        </IconButton>
        <IconButton label={t("sftp.home")} size="sm" disabled={!connected} onClick={() => load(home)}>
          <House size={15} />
        </IconButton>
        <IconButton label={t("sftp.refresh")} size="sm" disabled={!connected} onClick={refresh}>
          <RotateCw size={15} className={cx(loading && "animate-spin")} />
        </IconButton>
        <Input
          mono
          className="h-7.5 flex-1"
          aria-label={t("sftp.path")}
          disabled={!connected}
          value={pathInput}
          onChange={(e) => setPathInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && pathInput.trim()) load(pathInput.trim());
            else if (e.key === "Escape") setPathInput(cwd);
          }}
          onBlur={() => setPathInput(cwdRef.current)}
        />
        <IconButton label={t("sftp.newFolder")} size="sm" disabled={!connected} onClick={() => setNaming({ value: "" })}>
          <FolderPlus size={16} />
        </IconButton>
        <IconButton label={t("sftp.upload")} size="sm" disabled={!connected} onClick={() => upload()}>
          <Upload size={15} />
        </IconButton>
        <IconButton label={t("sftp.download")} size="sm" disabled={!connected || !selected.length} onClick={() => download(selected)}>
          <Download size={15} />
        </IconButton>
      </div>

      <div className={cx(COLS, "shrink-0 border-b border-line px-3 py-1.5 text-xs font-medium text-muted")}>
        <span>{t("sftp.col.name")}</span>
        <span className="text-right">{t("sftp.col.size")}</span>
        <span>{t("sftp.col.mtime")}</span>
        <span>{t("sftp.col.perm")}</span>
      </div>
      <div
        ref={listEl}
        tabIndex={0}
        onKeyDown={onKey}
        onMouseDown={(e) => {
          if (e.target === e.currentTarget && e.button === 0) setSel(new Set());
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          if (connected) setMenu({ at: { x: e.clientX, y: e.clientY }, items: dirMenu });
        }}
        onScroll={(e) => setView({ top: e.currentTarget.scrollTop, height: e.currentTarget.clientHeight })}
        className="min-h-0 flex-1 overflow-y-auto py-1 outline-none select-none"
      >
        <div style={{ height: entries.length * ROW_H, paddingTop: first * ROW_H }}>
          {entries.slice(first, last).map((e, k) => {
            const i = first + k;
            return (
              <div
                key={e.path}
                data-i={i}
                title={e.name}
                onMouseDown={(ev) => ev.button === 0 && click(ev, i)}
                onDoubleClick={() => open(e)}
                onContextMenu={(ev) => {
                  ev.preventDefault();
                  ev.stopPropagation();
                  if (!connected) return;
                  let list = selected;
                  if (!sel.has(e.path)) {
                    anchor.current = i;
                    setSel(new Set([e.path]));
                    list = [e];
                  }
                  setMenu({ at: { x: ev.clientX, y: ev.clientY }, items: fileMenu(list) });
                }}
                className={cx(COLS, "h-7 cursor-default items-center px-3 text-[13px]", sel.has(e.path) ? "bg-accent/12" : "hover:bg-subtle")}
              >
                <span className="flex min-w-0 items-center gap-2">
                  <EntryIcon e={e} />
                  <span className="truncate">{e.name}</span>
                </span>
                <span className="text-right text-xs text-muted tabular-nums">{isDir(e) ? "" : formatBytes(e.size)}</span>
                <span className="text-xs text-muted tabular-nums">{e.mtime !== undefined ? formatTime(e.mtime) : ""}</span>
                <span className="font-mono text-xs text-muted">{e.perm}</span>
              </div>
            );
          })}
        </div>
        {connected && !loading && entries.length === 0 && cwd && (
          <div className="pt-10 text-center text-xs text-muted">{t("sftp.emptyDir")}</div>
        )}
      </div>

      <div className="flex h-7 shrink-0 items-center gap-3 border-t border-line px-3 text-xs text-muted">
        <span>{t("sftp.items", { count: entries.length })}</span>
        {selected.length > 0 && <span>{t("sftp.selectedItems", { count: selected.length })}</span>}
      </div>

      {tasks.length > 0 && (
        <TransferQueue
          tasks={tasks}
          open={queueOpen}
          busy={busy}
          onToggle={() => setQueueOpen((v) => !v)}
          onCancel={(id) => panel.current && sftpApi.cancel(panel.current, id).catch(toast.error)}
          onClear={() => setTasks((list) => list.filter((x) => !finished(x.state)))}
        />
      )}

      {dropping && (
        <div className="pointer-events-none absolute inset-1 z-10 flex items-center justify-center rounded-xl border-2 border-dashed border-accent bg-accent/8">
          <div className="flex items-center gap-2 rounded-lg bg-surface px-4 py-2 text-sm shadow-lg">
            <Upload size={16} className="text-accent" />
            {t("sftp.dropHint", { path: cwd })}
          </div>
        </div>
      )}

      {!connected && (
        <div className="flex shrink-0 items-center gap-3 border-t border-line bg-surface px-4 py-2 text-[13px]">
          {status.s === "connecting" ? (
            <>
              <LoaderCircle size={15} className="animate-spin text-muted" />
              <span className="text-muted">{t("sftp.connecting", { name })}</span>
            </>
          ) : (
            <>
              <span className="min-w-0 flex-1 truncate" title={status.message}>
                {status.message}
              </span>
              <Button size="sm" variant="primary" icon={<RotateCw size={13} />} onClick={() => connect(cwdRef.current || undefined)}>
                {t("terminal.reconnect")}
              </Button>
            </>
          )}
        </div>
      )}

      <ContextMenu at={menu?.at ?? null} items={menu?.items ?? []} onClose={() => setMenu(null)} />

      <Dialog
        open={!!naming}
        onClose={() => setNaming(null)}
        title={t(naming?.entry ? "sftp.rename" : "sftp.newFolder")}
        width="max-w-sm"
        footer={
          <>
            <Button onClick={() => setNaming(null)}>{t("common.cancel")}</Button>
            <Button variant="primary" disabled={!nameValid} onClick={submitName}>
              {t("common.confirm")}
            </Button>
          </>
        }
      >
        <Input
          ref={nameInput}
          autoFocus
          invalid={!!naming?.value.trim() && !nameValid}
          placeholder={t("sftp.namePlaceholder")}
          value={naming?.value ?? ""}
          onChange={(e) => setNaming((n) => n && { ...n, value: e.target.value })}
          onKeyDown={(e) => e.key === "Enter" && submitName()}
        />
        <div className="mt-1.5 mb-1 text-xs text-muted">{t("sftp.nameHint")}</div>
      </Dialog>

      <Confirm
        open={!!deleting}
        title={t("sftp.deleteTitle")}
        message={
          deleting?.length === 1
            ? tx("sftp.deleteOne", { name: <span className="font-medium text-ink">{deleting[0].name}</span> })
            : t("sftp.deleteMany", { count: deleting?.length ?? 0 })
        }
        confirmText={t("common.delete")}
        danger
        onConfirm={() => deleting && remove(deleting)}
        onClose={() => {
          setDeleting(null);
          listEl.current?.focus();
        }}
      />

      <Confirm
        open={!!overwriting}
        title={t("sftp.overwriteTitle")}
        message={
          <>
            <div>
              {overwriting?.names.length === 1
                ? tx("sftp.overwriteOne", { name: <span className="font-medium text-ink">{overwriting.names[0]}</span> })
                : t("sftp.overwriteMany", { count: overwriting?.names.length ?? 0 })}
            </div>
            {(overwriting?.names.length ?? 0) > 1 && (
              <div className="mt-2 max-h-32 overflow-auto rounded-lg bg-subtle px-2.5 py-1.5 font-mono text-xs text-ink">
                {overwriting?.names.map((n) => (
                  <div key={n} className="truncate">
                    {n}
                  </div>
                ))}
              </div>
            )}
            <div className="mt-2 text-xs">{t("sftp.overwriteHint")}</div>
          </>
        }
        confirmText={t("sftp.overwrite")}
        danger
        onConfirm={() => {
          const o = overwriting;
          if (o) run(async (id) => addTasks(await sftpApi.upload(id, o.paths, o.dir)));
        }}
        onClose={() => setOverwriting(null)}
      />
    </div>
  );
}

function TransferQueue({
  tasks,
  open,
  busy,
  onToggle,
  onCancel,
  onClear,
}: {
  tasks: Task[];
  open: boolean;
  busy: number;
  onToggle: () => void;
  onCancel: (id: string) => void;
  onClear: () => void;
}) {
  const { t } = useI18n();
  return (
    <div className="shrink-0 border-t border-line bg-surface/60">
      <div className="flex h-8 items-center gap-2 px-2">
        <button onClick={onToggle} className="flex items-center gap-1 rounded-md px-1 text-xs font-medium text-muted hover:text-ink">
          <ChevronDown size={14} className={cx("transition-transform", !open && "-rotate-90")} />
          {busy ? t("sftp.transfersActive", { count: busy }) : t("sftp.transfers")}
        </button>
        <span className="flex-1" />
        {tasks.some((x) => finished(x.state)) && (
          <Button size="sm" variant="ghost" onClick={onClear}>
            {t("sftp.clearFinished")}
          </Button>
        )}
      </div>
      {open && (
        <div className="max-h-44 overflow-y-auto px-2 pb-2">
          {tasks.map((x) => (
            <TaskRow key={x.id} task={x} onCancel={() => onCancel(x.id)} />
          ))}
        </div>
      )}
    </div>
  );
}

function TaskRow({ task: x, onCancel }: { task: Task; onCancel: () => void }) {
  const { t } = useI18n();
  const pct = x.total > 0 ? Math.min(100, (x.done / x.total) * 100) : x.state === "done" ? 100 : 0;
  const Icon = x.direction === "upload" ? Upload : Download;
  let info: string;
  switch (x.state) {
    case "queued":
      info = t("sftp.state.queued");
      break;
    case "running":
      info = `${pct.toFixed(0)}% · ${formatBytes(x.done)} / ${formatBytes(x.total)}${x.speed !== undefined ? ` · ${formatBytes(Math.round(x.speed))}/s` : ""}`;
      break;
    case "done":
      info = `${t("sftp.state.done")} · ${formatBytes(x.total)}`;
      break;
    case "failed":
      info = x.error ?? t("sftp.state.failed");
      break;
    case "cancelled":
      info = t("sftp.state.cancelled");
      break;
  }
  return (
    <div className="flex items-center gap-2 rounded-lg px-1.5 py-1 hover:bg-subtle" title={`${x.src} → ${x.dst}`}>
      <Icon size={14} className="shrink-0 text-muted" />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2 text-[13px]">
          <span className="min-w-0 flex-1 truncate">{x.name}</span>
          <span
            className={cx(
              "max-w-1/2 shrink-0 truncate text-xs tabular-nums",
              x.state === "failed" ? "text-red-600 dark:text-red-400" : x.state === "done" ? "text-emerald-600 dark:text-emerald-400" : "text-muted",
            )}
          >
            {info}
          </span>
        </div>
        {(x.state === "running" || x.state === "queued") && (
          <div className="mt-1 h-1 overflow-hidden rounded-full bg-subtle">
            <div className="h-full rounded-full bg-accent transition-[width] duration-100" style={{ width: `${pct}%` }} />
          </div>
        )}
      </div>
      {!finished(x.state) && (
        <IconButton label={t("sftp.cancelTask")} size="sm" className="size-6" onClick={onCancel}>
          <X size={13} />
        </IconButton>
      )}
    </div>
  );
}
