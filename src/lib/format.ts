import { t } from "./i18n";
import type { ConnKind, Connection, TabKind } from "./types";

export const isMac = navigator.userAgent.includes("Mac");
export const isWindows = navigator.userAgent.includes("Windows");
/** 快速连接的快捷键。终端里 Ctrl+K 是 shell 的删除到行尾，所以 Windows / Linux 上用 Ctrl+Shift+K。 */
export const QUICK_CONNECT_KEY = isMac ? "⌘K" : "Ctrl+Shift+K";
export const isQuickConnectKey = (e: KeyboardEvent) =>
  e.code === "KeyK" && !e.altKey && (isMac ? e.metaKey && !e.ctrlKey && !e.shiftKey : e.ctrlKey && e.shiftKey && !e.metaKey);

export const KIND_LABEL: Record<ConnKind, string> = { ssh: "SSH", rdp: "RDP" };

export const DEFAULT_SSH_PORT = 22;
export const DEFAULT_PORT: Record<ConnKind, number> = { ssh: DEFAULT_SSH_PORT, rdp: 3389 };
export const DEFAULT_USER: Record<ConnKind, string> = { ssh: "", rdp: "Administrator" };

/** 与后端 model.rs 的 RDP_MIN_SIZE、RDP_MAX_SIZE 一致。 */
export const RDP_MIN_SIZE = 200;
export const RDP_MAX_SIZE = 8192;

/** 连接标记颜色，值保存在 connections.json 的 color 字段。 */
export const CONN_COLORS = {
  red: "bg-red-500",
  orange: "bg-orange-500",
  yellow: "bg-yellow-400",
  green: "bg-emerald-500",
  cyan: "bg-cyan-500",
  blue: "bg-blue-500",
  purple: "bg-violet-500",
  pink: "bg-pink-500",
} as const;

export type ConnColor = keyof typeof CONN_COLORS;

export const colorClass = (c?: string) => (c && c in CONN_COLORS ? CONN_COLORS[c as ConnColor] : undefined);

export function hostPort(host: string, port: number) {
  return host.includes(":") ? `[${host}]:${port}` : `${host}:${port}`;
}

/** 列表中连接名下面的一行地址：用户@主机，端口不是默认端口时带上端口；没有用户名时只显示主机。 */
export function connAddress(c: Connection): string {
  const host = c.port === DEFAULT_PORT[c.kind] ? c.host : hostPort(c.host, c.port);
  return c.user ? `${c.user}@${host}` : host;
}

/** 无痕模式只有 SSH 连接有。 */
export const isIncognito = (c: Connection) => c.kind === "ssh" && !!c.incognito;

/** 打开连接时默认的标签类型。 */
export const defaultTab = (c: Connection): TabKind => (c.kind === "rdp" ? "rdp" : "term");

/** 1920 × 1080 */
export const formatResolution = (width: number, height: number) => `${width} × ${height}`;

/** 1536 → 1.5 KB；字节数原样显示，其余保留一位小数。 */
export function formatBytes(n: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return i === 0 ? `${n} B` : `${n.toFixed(1)} ${units[i]}`;
}

/** 每秒的字节数：1536 → 1.5 KB/s。 */
export const formatRate = (n: number) => t("monitor.rate", { value: formatBytes(n) });

/** 12.345 → 12.3%；100 以上不保留小数。 */
export const formatPercent = (n: number) => `${n >= 100 ? Math.round(n) : n.toFixed(1)}%`;

/** 开机时长（秒）→ 3 天 4 小时、5 小时 6 分、7 分钟。 */
export function formatUptime(secs: number): string {
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return t("monitor.uptimeDays", { d, h });
  if (h > 0) return t("monitor.uptimeHours", { h, m });
  return t("monitor.uptimeMinutes", { m });
}

const pad = (n: number) => String(n).padStart(2, "0");

/** Unix 秒 → 本地时间 2026-11-17 09:05。 */
export function formatTime(secs: number): string {
  const d = new Date(secs * 1000);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** 日志时间（RFC 3339）→ 本地时间 11-27 09:05:01.123；解析不了时原样返回。 */
export function formatLogTime(time: string): string {
  // 后端写的是纳秒精度，WebKit 只认三位小数。
  const d = new Date(time.replace(/(\.\d{3})\d+/, "$1"));
  if (Number.isNaN(d.getTime())) return time;
  const ms = String(d.getMilliseconds()).padStart(3, "0");
  return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${ms}`;
}

/** 跳板机的连接 ID。 */
export const jumpRef = (c: Connection): string | undefined => c.jumpId || undefined;
