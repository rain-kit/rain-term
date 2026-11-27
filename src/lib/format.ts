import type { ConnKind, Connection } from "./types";

export const isMac = navigator.userAgent.includes("Mac");
export const isWindows = navigator.userAgent.includes("Windows");
/** 快速连接的快捷键。终端里 Ctrl+K 是 shell 的删除到行尾，所以 Windows / Linux 上用 Ctrl+Shift+K。 */
export const QUICK_CONNECT_KEY = isMac ? "⌘K" : "Ctrl+Shift+K";
export const isQuickConnectKey = (e: KeyboardEvent) =>
  e.code === "KeyK" && !e.altKey && (isMac ? e.metaKey && !e.ctrlKey && !e.shiftKey : e.ctrlKey && e.shiftKey && !e.metaKey);

export const KIND_LABEL: Record<ConnKind, string> = { ssh: "SSH" };

export const DEFAULT_SSH_PORT = 22;

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

/** 列表中连接名下面的一行地址：用户@主机，端口不是 22 时带上端口。 */
export function connAddress(c: Connection): string {
  return `${c.user}@${c.port === DEFAULT_SSH_PORT ? c.host : hostPort(c.host, c.port)}`;
}

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
