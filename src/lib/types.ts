// 与 src-tauri/src/model.rs、src-tauri/src/commands 对应。

import type { Language, Locale } from "./i18n";

/** 后端返回的国际化消息（src-tauri/src/i18n.rs 的 Msg），用 tm() 翻译。 */
export interface I18nMsg {
  i18n: string;
  params?: Record<string, string | I18nMsg>;
}

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
  sftp: {
    /** 连接没有单独设置时，SFTP 是否用 sudo 以 root 身份打开。 */
    sudo: boolean;
    /** SFTP 跟随终端目录；开启时打开终端会注入报告当前目录的命令。 */
    followTerminal: boolean;
  };
}

export interface Settings {
  theme: ThemeMode;
  language: Language;
  /** 后端识别的系统语言，系统语言不受支持时为 en。 */
  systemLanguage: Locale;
  terminal: TerminalSettings;
  tools: ToolsSettings;
  version: string;
}

/** terminal 不传时保持不变。 */
export type SettingsInput = Pick<Settings, "theme" | "language"> & { terminal?: TerminalSettings; tools?: ToolsSettings };

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
  /** 选了登录用户时是登录用户的用户名 */
  user: string;
  /** 登录用户（Account），设置后用户名、认证方式、密码和私钥都来自登录用户 */
  accountId?: string;
  auth: AuthType;
  /** 私钥认证使用的密钥（密钥管理里的 SshKey） */
  keyId?: string;
  /** 跳板机：另一个 SSH 连接的 ID */
  jumpId?: string;
  /** 保活间隔（秒），0 表示不发送 */
  keepaliveSeconds: number;
  initDir?: string;
  /** 无痕模式：本次会话的命令不写入历史文件 */
  incognito?: boolean;
  /** SFTP 是否用 sudo 以 root 身份打开，不传时按工具里的设置 */
  sftpSudo?: SftpSudo;
  password?: string;
}

/** default 表示按顶栏“工具”里的设置。 */
export type SftpSudo = "default" | "off" | "on";

/** 远程桌面的分辨率：跟随标签大小，或固定为 width × height。 */
export type RdpResolution = "fit" | "fixed";

export interface RdpParams {
  kind: "rdp";
  host: string;
  port: number;
  /** 选了登录用户时是登录用户的用户名 */
  user: string;
  /** 登录用户（Account），设置后用户名和密码都来自登录用户 */
  accountId?: string;
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
export type ConnView = Connection & { hasPassword: boolean };

/** 保存连接：密码留空表示不修改，clearPassword 为 true 时删除已保存的密码。 */
export type SaveConnReq = Connection & { clearPassword?: boolean };

export interface TestResult {
  elapsedMs: number;
  serverVersion?: string;
}

export interface ExportResult {
  path: string;
  count: number;
}

// ---------- 密钥管理，见 src-tauri/src/service/keys.rs ----------

/** 私钥保存在程序里，这里只有公开信息。 */
export interface SshKey {
  id: string;
  name: string;
  /** 如 ssh-ed25519、ssh-rsa、ecdsa-sha2-nistp256 */
  algorithm: string;
  /** RSA 密钥的位数 */
  bits?: number;
  /** SHA256:xxxx */
  fingerprint: string;
  /** authorized_keys 格式的一行公钥 */
  publicKey: string;
  /** 私钥设置了密码 */
  encrypted: boolean;
  createdAt?: string;
}

/** name 为空时用私钥的注释或类型。 */
export interface KeyImportReq {
  name: string;
  content: string;
  passphrase: string;
}

export type KeyType = "ed25519" | "ecdsa" | "rsa";

/** bits 只对 RSA 有效。 */
export interface KeyGenerateReq {
  name: string;
  type: KeyType;
  bits?: number;
  passphrase: string;
}

// ---------- 登录用户，见 src-tauri/src/service/accounts.rs ----------

/** 登录用户能用于哪些连接；私钥认证只能用于 SSH。 */
export type AccountScope = "all" | "ssh" | "rdp";
export type AccountAuth = Exclude<AuthType, "agent">;

export interface Account {
  /** 新建时为空字符串 */
  id: string;
  /** 为空时用用户名 */
  name: string;
  user: string;
  auth: AccountAuth;
  /** 私钥认证使用的密钥（密钥管理里的 SshKey） */
  keyId?: string;
  scope: AccountScope;
  createdAt?: string;
}

/** 后端返回的登录用户不含密码，只告诉前端是否已保存。 */
export type AccountView = Account & { hasPassword: boolean };

/** 保存登录用户：密码留空表示不修改，clearPassword 为 true 时删除已保存的密码。 */
export type SaveAccountReq = Account & { password?: string; clearPassword?: boolean };

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

/** 文本文件的预览内容。 */
export interface FilePreview {
  text: string;
  /** 文件大小。 */
  size: number;
  /** 文件太大，text 只是开头部分。 */
  truncated: boolean;
}

export interface SftpOpened {
  id: string;
  /** 初始目录。 */
  path: string;
  /** 是否用 sudo 以 root 身份打开。 */
  sudo: boolean;
}

export interface SftpOpenOptions {
  /** 是否用 sudo 以 root 身份打开；不传时按连接和工具里的设置。 */
  sudo?: boolean;
  /** 用户输入的 sudo 密码，只用于这一次打开。 */
  sudoPassword?: string;
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

// ---------- 服务器监控，见 src-tauri/src/service/monitor.rs ----------

export interface ProcInfo {
  pid: number;
  name: string;
  /** 100 表示占满一个核；刚打开、还没有上一次采集时没有。 */
  cpu?: number;
  /** 常驻内存，字节。 */
  mem: number;
}

/** 容量都是字节，网络是每秒的字节数。 */
export interface ServerStats {
  /** 总的 CPU 使用率 0 ~ 100；第一次采集时没有。 */
  cpu?: number;
  cores: number;
  /** 1、5、15 分钟的平均负载。 */
  load: [number, number, number];
  memTotal: number;
  memUsed: number;
  swapTotal: number;
  swapUsed: number;
  /** 根分区。 */
  disk?: { total: number; used: number; avail: number };
  net?: { rx: number; tx: number };
  /** 开机时长，秒。 */
  uptime: number;
  /** 按 CPU 和按内存各自最高的几个进程。 */
  procs: ProcInfo[];
}

/** unsupported（不是 Linux）一定是最后一条；failed 后下个周期会再试。 */
export type MonitorEvent = ({ type: "stats" } & ServerStats) | { type: "unsupported" } | { type: "failed"; error: I18nMsg };

export type ProcSort = "cpu" | "mem";

/** 终端侧栏在右侧还是下方。 */
export type PanelPosition = "right" | "bottom";

/** 终端侧栏显示服务器状态还是 SFTP。 */
export type PanelMode = "monitor" | "sftp";

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

export type TabKind = "term" | "rdp";

export interface WorkspaceTab {
  connId: string;
  /** sftp 是旧版本单独打开的 SFTP 标签，恢复时丢掉。 */
  kind: TabKind | "sftp";
}

/** 上次打开的标签和最近打开的连接，见 src-tauri/src/service/workspace.rs。 */
export interface Workspace {
  tabs: WorkspaceTab[];
  /** 当前标签在 tabs 中的位置。 */
  active: number | null;
  /** 最近打开的连接 ID，最近的在前。 */
  recent: string[];
}
