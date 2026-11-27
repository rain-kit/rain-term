// 与 src-tauri/src/model.rs、src-tauri/src/commands 对应。

import type { Language, Locale } from "./i18n";

/** 后端返回的国际化消息（src-tauri/src/i18n.rs 的 Msg），用 tm() 翻译。 */
export interface I18nMsg {
  i18n: string;
  params?: Record<string, string | I18nMsg>;
}

export type LogLevel = "debug" | "info" | "warn" | "error";
export type ThemeMode = "system" | "light" | "dark";

export type ColorScheme = "auto" | "oneDark" | "dracula" | "nord" | "solarizedDark" | "solarizedLight";
export type CursorStyle = "block" | "bar" | "underline";

/** config.yaml 的 terminal 一节（src-tauri/src/config.rs 的 TerminalConfig）。 */
export interface TerminalSettings {
  /** 为空时使用界面的等宽字体。 */
  fontFamily: string;
  fontSize: number;
  scrollback: number;
  colorScheme: ColorScheme;
  cursorStyle: CursorStyle;
}

/** 顶栏“工具”里的开关（src-tauri/src/config.rs 的 ToolsConfig），导出连接时一起导出。 */
export interface ToolsSettings {
  enhance: {
    /** 在 vim 等全屏程序里单击时把光标移到点击的位置。 */
    clickMovesCursor: boolean;
  };
}

export interface Settings {
  logLevel: LogLevel;
  theme: ThemeMode;
  language: Language;
  /** 后端识别的系统语言，系统语言不受支持时为 en。 */
  systemLanguage: Locale;
  terminal: TerminalSettings;
  tools: ToolsSettings;
  dataDir: string;
  logDir: string;
  version: string;
}

/** 日志文件里的一条记录，见 src-tauri/src/logging.rs。不是 JSON 的行只有 message。 */
export interface LogEntry {
  /** RFC 3339，带时区。 */
  time: string;
  /** DEBUG、INFO、WARN、ERROR。 */
  level: string;
  target: string;
  message: string;
  fields: [string, string][];
}

/** terminal 不传时保持不变。 */
export type SettingsInput = Pick<Settings, "logLevel" | "theme" | "language"> & { terminal?: TerminalSettings; tools?: ToolsSettings };

/** 见 src-tauri/src/update.rs。 */
export interface UpdateInfo {
  current: string;
  /** 尚未成功检查过时为空 */
  latest?: string;
  hasUpdate: boolean;
  checkedAt?: string;
  /** 更新内容（Markdown） */
  notes?: string;
  publishedAt?: string;
  /** 能否直接更新，否则只能前往下载页 */
  canInstall: boolean;
}

// ---------- 连接 ----------

export type ConnKind = "ssh" | "rdp";
export type AuthType = "password" | "key" | "agent";

export interface SshParams {
  kind: "ssh";
  host: string;
  port: number;
  user: string;
  auth: AuthType;
  keyPath?: string;
  /** 跳板机：另一个 SSH 连接的 ID */
  jumpId?: string;
  /** 保活间隔（秒），0 表示不发送 */
  keepaliveSeconds: number;
  initDir?: string;
  /** 无痕模式：定期清除服务器上的命令历史，本次会话不写入历史 */
  incognito?: boolean;
  password?: string;
  keyPassphrase?: string;
}

/** 远程桌面的分辨率：跟随标签大小，或固定为 width × height。 */
export type RdpResolution = "fit" | "fixed";

export interface RdpParams {
  kind: "rdp";
  host: string;
  port: number;
  user: string;
  domain?: string;
  /** 经这个 SSH 连接转发到远程桌面 */
  jumpId?: string;
  resolution: RdpResolution;
  width: number;
  height: number;
  /** 和远程桌面共享文本剪贴板 */
  clipboard: boolean;
  password?: string;
}

export type ConnParams = SshParams | RdpParams;

export type Connection = {
  /** 新建时为空字符串 */
  id: string;
  name: string;
  group?: string;
  color?: string;
  note?: string;
  /** 生产环境：列表和标签显示红色，终端粘贴前确认 */
  production?: boolean;
  createdAt?: string;
} & ConnParams;

/** 后端返回的连接不含密码，只告诉前端是否已保存。 */
export type ConnView = Connection & { hasPassword: boolean; hasKeyPassphrase: boolean };

/** 保存连接：密码类字段留空表示不修改，clear* 为 true 时删除已保存的值。 */
export type SaveConnReq = Connection & { clearPassword?: boolean; clearKeyPassphrase?: boolean };

export interface TestResult {
  elapsedMs: number;
  serverVersion?: string;
}

export interface ExportResult {
  path: string;
  count: number;
}

// ---------- SSH ----------

/** 首次连接某台服务器时请用户确认指纹。 */
export interface HostKeyPrompt {
  requestId: string;
  kind: ConnKind;
  /** 测试尚未保存的连接时为空。 */
  connId: string;
  connName: string;
  host: string;
  /** SSH 是密钥类型，远程桌面是证书的使用者。 */
  keyType: string;
  fingerprint: string;
  /** 远程桌面证书变化时，之前信任的指纹。 */
  previous?: string;
}

/** 终端会话结束：shell 退出或连接断开（用户主动关闭时不推送）。 */
export interface SessionClosed {
  exitCode?: number;
  exitSignal?: string;
  /** 连接异常断开的原因，shell 正常退出时没有。 */
  error?: I18nMsg;
}

export type SessionEvent = { type: "output"; data: Uint8Array } | ({ type: "closed" } & SessionClosed);

// ---------- SFTP，见 src-tauri/src/service/sftp.rs ----------

export type FileKind = "dir" | "file" | "symlink" | "other";

export interface FileEntry {
  name: string;
  path: string;
  kind: FileKind;
  /** 符号链接指向的是目录。 */
  linkDir?: boolean;
  size: number;
  /** 修改时间，Unix 秒。 */
  mtime?: number;
  /** 如 drwxr-xr-x；服务器没给权限时为空。 */
  perm: string;
}

export interface Listing {
  /** 规范化后的绝对路径。 */
  path: string;
  /** 目录在前，再按名称排序。 */
  entries: FileEntry[];
}

export interface SftpOpened {
  id: string;
  /** 初始目录。 */
  path: string;
}

export type TransferDirection = "upload" | "download";

export interface Transfer {
  id: string;
  direction: TransferDirection;
  name: string;
  src: string;
  dst: string;
}

/** 传输进度和面板断开通知；closed 一定是最后一条，用户主动关闭面板时不推送。 */
export type SftpEvent =
  | { type: "started"; id: string; total: number }
  | { type: "progress"; id: string; done: number; total: number }
  | { type: "done"; id: string }
  | { type: "failed"; id: string; error: I18nMsg }
  | { type: "cancelled"; id: string }
  | { type: "closed"; error: I18nMsg };

// ---------- 远程桌面，见 src-tauri/src/service/rdp.rs ----------

/** 会话的 JSON 通知；closed 一定是最后一条，用户主动关闭时不推送。 */
export type RdpNotice =
  | { type: "clipboard"; text: string }
  | { type: "closed"; error?: I18nMsg };

/** 输入事件，坐标是远程桌面的像素坐标，见 src-tauri/src/core/rdp/input.rs。 */
export type RdpInput =
  | { t: "key"; code: string; down: boolean }
  | { t: "text"; text: string }
  | { t: "move"; x: number; y: number }
  | { t: "button"; button: number; down: boolean }
  | { t: "wheel"; vertical: boolean; units: number };

// ---------- 工作区 ----------

export type TabKind = "term" | "sftp" | "rdp";

export interface WorkspaceTab {
  connId: string;
  kind: TabKind;
}

/** 上次打开的标签和最近打开的连接，见 src-tauri/src/service/workspace.rs。 */
export interface Workspace {
  tabs: WorkspaceTab[];
  /** 当前标签在 tabs 中的位置。 */
  active: number | null;
  /** 最近打开的连接 ID，最近的在前。 */
  recent: string[];
}
