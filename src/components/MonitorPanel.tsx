import { ArrowDown, ArrowUp, ChevronDown, LoaderCircle } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { errorMessage, monitorApi } from "../lib/api";
import { formatBytes, formatPercent, formatRate, formatUptime } from "../lib/format";
import { useI18n } from "../lib/i18n";
import type { ProcInfo, ProcSort, ServerStats } from "../lib/types";
import { cx } from "./ui";

/** 最多显示的进程数，与后端 service/monitor.rs 的 TOP 一致。 */
const MAX_PROCS = 30;
const DEFAULT_PROCS = 10;
const COUNT_KEY = "monitor.procCount";
const SORT_KEY = "monitor.sort";

const loadSort = (): ProcSort => (localStorage.getItem(SORT_KEY) === "mem" ? "mem" : "cpu");

/** 1 ~ MAX_PROCS 的整数，否则为 null。 */
const parseCount = (v: string | null) => {
  const n = Number(v);
  return Number.isInteger(n) && n >= 1 && n <= MAX_PROCS ? n : null;
};

const loadCount = () => parseCount(localStorage.getItem(COUNT_KEY)) ?? DEFAULT_PROCS;

const cpuOf = (p: ProcInfo) => p.cpu ?? -1;

function topProcs(procs: ProcInfo[], by: ProcSort, count: number) {
  const sorted = [...procs].sort((a, b) =>
    by === "cpu" ? cpuOf(b) - cpuOf(a) || b.mem - a.mem : b.mem - a.mem || cpuOf(b) - cpuOf(a),
  );
  return sorted.slice(0, count);
}

/** 显示的进程数：输入有效时立即生效并保存，离开输入框时把无效的输入恢复成当前值。 */
function CountInput({ value, onChange }: { value: number; onChange: (n: number) => void }) {
  const { t } = useI18n();
  const [draft, setDraft] = useState(String(value));
  useEffect(() => setDraft(String(value)), [value]);
  return (
    <input
      type="number"
      min={1}
      max={MAX_PROCS}
      value={draft}
      title={t("monitor.procCountHint", { max: MAX_PROCS })}
      aria-label={t("monitor.procCountHint", { max: MAX_PROCS })}
      onChange={(e) => {
        setDraft(e.target.value);
        const n = parseCount(e.target.value);
        if (n !== null) onChange(n);
      }}
      onBlur={() => setDraft(String(value))}
      onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
      className="h-5 w-12 shrink-0 rounded border border-line bg-surface px-1 text-center text-xs font-normal tabular-nums outline-none focus:border-accent"
    />
  );
}

const ratio = (used: number, total: number) => (total > 0 ? (used / total) * 100 : 0);

const barColor = (pct?: number) =>
  pct === undefined ? "bg-transparent" : pct >= 90 ? "bg-red-500" : pct >= 70 ? "bg-amber-500" : "bg-emerald-500";

function Meter({ label, detail, pct }: { label: string; detail?: string; pct?: number }) {
  return (
    <div>
      <div className="mb-1 flex items-baseline gap-2">
        <span className="shrink-0 text-muted">{label}</span>
        <span className="ml-auto min-w-0 truncate text-muted tabular-nums">{detail}</span>
        <span className="w-11 shrink-0 text-right font-medium tabular-nums">{pct === undefined ? "—" : formatPercent(pct)}</span>
      </div>
      <div className="h-1.5 overflow-hidden rounded-full bg-subtle">
        <div
          className={cx("h-full rounded-full transition-[width] duration-500", barColor(pct))}
          style={{ width: `${Math.min(100, pct ?? 0)}%` }}
        />
      </div>
    </div>
  );
}

function Row({ label, title, children }: { label: string; title?: string; children: ReactNode }) {
  return (
    <div className="flex items-center gap-2" title={title}>
      <span className="shrink-0 text-muted">{label}</span>
      <span className="ml-auto flex min-w-0 items-center gap-2 truncate tabular-nums">{children}</span>
    </div>
  );
}

function Note({ children, error }: { children: ReactNode; error?: boolean }) {
  return <div className={cx("px-1 py-6 text-center leading-relaxed", error ? "text-red-600 dark:text-red-400" : "text-muted")}>{children}</div>;
}

function SortHeader({ label, title, active, onClick }: { label: string; title: string; active: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      title={title}
      onClick={onClick}
      className={cx("flex items-center justify-end gap-0.5 rounded px-0.5 transition-colors hover:text-ink", active ? "font-medium text-ink" : "text-muted")}
    >
      {label}
      <ChevronDown size={12} className={cx(!active && "invisible")} />
    </button>
  );
}

function Procs({ stats }: { stats: ServerStats }) {
  const { t } = useI18n();
  const [sort, setSort] = useState(loadSort);
  const [count, setCount] = useState(loadCount);
  const pick = (v: ProcSort) => {
    setSort(v);
    localStorage.setItem(SORT_KEY, v);
  };
  const changeCount = (n: number) => {
    setCount(n);
    localStorage.setItem(COUNT_KEY, String(n));
  };
  const cols = "grid grid-cols-[minmax(0,1fr)_3.25rem_4.5rem] items-center gap-x-1.5";
  return (
    <div>
      <div className={cx(cols, "border-b border-line pb-1")}>
        <span className="flex min-w-0 items-center gap-1.5 font-medium">
          <span className="truncate">{t("monitor.procs")}</span>
          <CountInput value={count} onChange={changeCount} />
        </span>
        <SortHeader label={t("monitor.cpu")} title={t("monitor.sortCpu")} active={sort === "cpu"} onClick={() => pick("cpu")} />
        <SortHeader label={t("monitor.mem")} title={t("monitor.sortMem")} active={sort === "mem"} onClick={() => pick("mem")} />
      </div>
      {topProcs(stats.procs, sort, count).map((p) => (
        <div key={p.pid} className={cx(cols, "h-6 rounded hover:bg-subtle")} title={`${p.name} · ${t("monitor.pid", { pid: p.pid })}`}>
          <span className="truncate pl-0.5">{p.name}</span>
          <span className={cx("pr-0.5 text-right tabular-nums", sort !== "cpu" && "text-muted")}>
            {p.cpu === undefined ? "—" : formatPercent(p.cpu)}
          </span>
          <span className={cx("pr-0.5 text-right tabular-nums", sort !== "mem" && "text-muted")}>{formatBytes(p.mem)}</span>
        </div>
      ))}
    </div>
  );
}

function Overview({ stats: s }: { stats: ServerStats }) {
  const { t } = useI18n();
  return (
    <div className="flex flex-col gap-3">
      <Meter label={t("monitor.cpu")} detail={t("monitor.cores", { count: s.cores })} pct={s.cpu} />
      <Meter
        label={t("monitor.mem")}
        detail={`${formatBytes(s.memUsed)} / ${formatBytes(s.memTotal)}`}
        pct={ratio(s.memUsed, s.memTotal)}
      />
      {s.swapTotal > 0 && (
        <Meter
          label={t("monitor.swap")}
          detail={`${formatBytes(s.swapUsed)} / ${formatBytes(s.swapTotal)}`}
          pct={ratio(s.swapUsed, s.swapTotal)}
        />
      )}
      {s.disk && (
        <Meter
          label={t("monitor.disk")}
          detail={`${formatBytes(s.disk.used)} / ${formatBytes(s.disk.total)}`}
          pct={ratio(s.disk.used, s.disk.used + s.disk.avail)}
        />
      )}
      <div className="flex flex-col gap-1.5">
        <Row label={t("monitor.load")} title={t("monitor.loadHint")}>
          {s.load.map((v, i) => (
            <span key={i}>{v.toFixed(2)}</span>
          ))}
        </Row>
        {s.net && (
          <Row label={t("monitor.net")}>
            <span className="flex items-center">
              <ArrowDown size={12} className="text-emerald-500" />
              {formatRate(s.net.rx)}
            </span>
            <span className="flex items-center">
              <ArrowUp size={12} className="text-sky-500" />
              {formatRate(s.net.tx)}
            </span>
          </Row>
        )}
        <Row label={t("monitor.uptime")}>{formatUptime(s.uptime)}</Row>
      </div>
    </div>
  );
}

/** 侧栏里的服务器状态：负载、CPU、内存和占用最高的进程。只在连接已建立时采集。 */
export default function MonitorPanel({ connId, connected }: { connId: string; connected: boolean }) {
  const { t } = useI18n();
  const [stats, setStats] = useState<ServerStats | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [unsupported, setUnsupported] = useState(false);

  useEffect(() => {
    setStats(null);
    setError(null);
    setUnsupported(false);
    if (!connected) return;
    let closed = false;
    let id: string | null = null;
    monitorApi
      .open(connId, (e) => {
        if (closed) return;
        if (e.type === "stats") {
          setStats(e);
          setError(null);
        } else if (e.type === "unsupported") setUnsupported(true);
        else setError(e.error);
      })
      .then((x) => {
        if (closed) monitorApi.close(x).catch(() => {});
        else id = x;
      })
      .catch((err) => !closed && setError(err));
    return () => {
      closed = true;
      if (id) monitorApi.close(id).catch(() => {});
    };
  }, [connId, connected]);

  let body: ReactNode;
  if (!connected) body = <Note>{t("monitor.waiting")}</Note>;
  else if (unsupported) body = <Note>{t("monitor.unsupported")}</Note>;
  else if (!stats && error) body = <Note error>{errorMessage(error)}</Note>;
  else if (!stats)
    body = (
      <Note>
        <LoaderCircle size={16} className="mx-auto mb-2 animate-spin" />
        {t("monitor.loading")}
      </Note>
    );
  else
    body = (
      <div className="flex flex-col gap-5">
        {!!error && <div className="truncate text-red-600 dark:text-red-400" title={errorMessage(error)}>{errorMessage(error)}</div>}
        {/* 宽度够时（如放在下方）概况和进程左右并排，否则上下排列。 */}
        <div className="grid grid-cols-[repeat(auto-fit,minmax(min(16rem,100%),1fr))] items-start gap-x-8 gap-y-5">
          <Overview stats={stats} />
          <Procs stats={stats} />
        </div>
      </div>
    );

  return <div className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto p-3 text-xs select-none">{body}</div>;
}
