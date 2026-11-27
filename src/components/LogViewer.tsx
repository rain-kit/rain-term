import { FolderOpen, RefreshCw, Search } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { settingsApi } from "../lib/api";
import { formatLogTime } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { useToast } from "../lib/store";
import type { LogEntry } from "../lib/types";
import { Button, cx, Dialog, Input, Segmented } from "./ui";

const LIMIT = 2000;

type MinLevel = "all" | "info" | "warn" | "error";

const RANK: Record<string, number> = { DEBUG: 0, INFO: 1, WARN: 2, ERROR: 3 };
const MIN_RANK: Record<MinLevel, number> = { all: -1, info: 1, warn: 2, error: 3 };

const LEVEL_CLASS: Record<string, string> = {
  ERROR: "text-red-600 dark:text-red-400",
  WARN: "text-amber-600 dark:text-amber-400",
  INFO: "text-accent",
  DEBUG: "text-muted",
};

const matches = (e: LogEntry, q: string) =>
  [e.message, e.target, ...e.fields.flat()].some((v) => v.toLowerCase().includes(q));

/** 查看最近的日志：按级别筛选、搜索，打开时滚到最新一条。 */
export default function LogViewer({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { t } = useI18n();
  const toast = useToast();
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [level, setLevel] = useState<MinLevel>("all");
  const [q, setQ] = useState("");
  const list = useRef<HTMLDivElement>(null);
  // toast 每弹出一条提示都会换成新对象，不能放进 load 的依赖，否则读取失败时会反复重试。
  const toastRef = useRef(toast);
  toastRef.current = toast;

  const load = useCallback(() => {
    setLoading(true);
    settingsApi
      .logTail(LIMIT)
      .then(setEntries)
      .catch((e) => toastRef.current.error(e))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    if (open) load();
  }, [open, load]);

  const shown = useMemo(() => {
    const min = MIN_RANK[level];
    const s = q.trim().toLowerCase();
    return entries.filter((e) => (RANK[e.level] ?? -1) >= min && (!s || matches(e, s)));
  }, [entries, level, q]);

  useLayoutEffect(() => {
    const el = list.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [shown]);

  return (
    <Dialog
      open={open}
      onClose={onClose}
      width="max-w-5xl"
      title={t("log.title")}
      footer={
        <>
          <Button icon={<FolderOpen size={14} />} onClick={() => settingsApi.openLogDir().catch(toast.error)}>
            {t("log.openDir")}
          </Button>
          <Button icon={<RefreshCw size={14} className={loading ? "animate-spin" : undefined} />} disabled={loading} onClick={load}>
            {t("log.refresh")}
          </Button>
        </>
      }
    >
      <div className="mb-2 flex items-center gap-3">
        <Segmented<MinLevel>
          value={level}
          onChange={setLevel}
          options={[
            { value: "all", label: t("log.level.all") },
            { value: "info", label: "Info" },
            { value: "warn", label: "Warn" },
            { value: "error", label: "Error" },
          ]}
        />
        <div className="w-64">
          <Input
            placeholder={t("log.search")}
            value={q}
            onChange={(e) => setQ(e.target.value)}
            suffix={<Search size={14} className="mr-1.5 text-muted" />}
          />
        </div>
        <span className="ml-auto text-xs text-muted">{t("log.count", { count: shown.length })}</span>
      </div>
      <div ref={list} className="h-[60vh] overflow-y-auto rounded-lg border border-line bg-subtle/40 py-1 font-mono text-xs">
        {shown.length === 0 ? (
          <div className="flex h-full items-center justify-center font-sans text-sm text-muted">
            {t(loading ? "log.loading" : "log.empty")}
          </div>
        ) : (
          shown.map((e, i) => (
            <div key={i} className="flex gap-3 px-3 py-0.5 hover:bg-subtle" title={e.target || undefined}>
              <span className="shrink-0 text-muted">{e.time ? formatLogTime(e.time) : ""}</span>
              <span className={cx("w-10 shrink-0 font-semibold", LEVEL_CLASS[e.level])}>{e.level}</span>
              <span className="min-w-0 flex-1 break-all whitespace-pre-wrap">
                {e.message}
                {e.fields.map(([k, v]) => (
                  <span key={k} className="ml-2 text-muted">
                    {k}={v}
                  </span>
                ))}
              </span>
            </div>
          ))
        )}
      </div>
    </Dialog>
  );
}
