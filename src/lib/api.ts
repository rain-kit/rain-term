import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { tm } from "./i18n";
import type {
  ConnView,
  ExportResult,
  HostKeyPrompt,
  I18nMsg,
  LogEntry,
  SaveConnReq,
  Listing,
  SessionClosed,
  SessionEvent,
  Settings,
  SettingsInput,
  SftpEvent,
  SftpOpened,
  TestResult,
  Transfer,
  UpdateInfo,
  Workspace,
  WorkspaceTab,
} from "./types";

/** 后端命令失败时返回 { code, i18n, params }，见 src-tauri/src/error.rs。 */
export interface ApiError extends I18nMsg {
  code: number;
}

export function errorMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "i18n" in e) return tm(e as ApiError);
  if (e instanceof Error) return e.message;
  return String(e);
}

export const settingsApi = {
  get: () => invoke<Settings>("settings_get"),
  save: (input: SettingsInput) => invoke<Settings>("settings_save", { input }),
  openDataDir: () => invoke<void>("open_data_dir"),
  openLogDir: () => invoke<void>("open_log_dir"),
  /** 最近的 limit 条日志，按时间先后排列。 */
  logTail: (limit: number) => invoke<LogEntry[]>("log_tail", { limit }),
  /** 用系统浏览器打开 http / https 链接。 */
  openUrl: (url: string) => invoke<void>("open_url", { url }),
};

export const connApi = {
  list: () => invoke<ConnView[]>("conn_list"),
  get: (id: string) => invoke<ConnView>("conn_get", { id }),
  save: (req: SaveConnReq) => invoke<ConnView>("conn_save", { req }),
  duplicate: (id: string) => invoke<ConnView>("conn_duplicate", { id }),
  remove: (id: string) => invoke<void>("conn_delete", { id }),
  reorder: (ids: string[]) => invoke<void>("conn_reorder", { ids }),
  /** 用编辑器里尚未保存的内容测试连接。 */
  test: (req: SaveConnReq) => invoke<TestResult>("conn_test", { req }),
  /** 弹出保存对话框导出，ids 为空时导出全部；取消时返回 null。 */
  exportTo: (ids: string[] = []) => invoke<ExportResult | null>("conn_export", { ids }),
  /** 弹出文件选择框导入本应用导出的文件，返回导入的数量；取消时返回 null。 */
  importFrom: () => invoke<number | null>("conn_import"),
  /** 选择 FinalShell 数据目录并导入。取消时返回 null。 */
  importFinalShellDir: () => invoke<number | null>("conn_import_dir"),
  /** 选择 MobaXterm 会话文件并导入。取消时返回 null。 */
  importMobaXterm: () => invoke<number | null>("conn_import_moba"),
  /** 取消时返回空字符串。 */
  pickKeyFile: () => invoke<string>("pick_key_file"),
};

export const hostKeyApi = {
  pending: () => invoke<HostKeyPrompt[]>("hostkey_pending"),
  answer: (requestId: string, accept: boolean) => invoke<void>("hostkey_answer", { requestId, accept }),
};

/** 终端会话，见 src-tauri/src/service/session.rs。 */
export const sessionApi = {
  /**
   * 连接成功后返回会话 ID。输出和结束通知从 onEvent 推出，结束通知一定是最后一条。
   * 输出处理完后要调用 ack，否则后端发出约 1MB 未确认的输出后会暂停。
   */
  open: (connId: string, cols: number, rows: number, onEvent: (e: SessionEvent) => void) => {
    const ch = new Channel<ArrayBuffer | number[] | SessionClosed>();
    ch.onmessage = (m) => {
      if (m instanceof ArrayBuffer) onEvent({ type: "output", data: new Uint8Array(m) });
      else if (Array.isArray(m)) onEvent({ type: "output", data: Uint8Array.from(m) });
      else onEvent({ type: "closed", ...m });
    };
    return invoke<string>("session_open", { connId, cols, rows, onEvent: ch });
  },
  /** 键盘输入传字符串；xterm onBinary 的内容传字节。 */
  write: (id: string, data: string | Uint8Array) =>
    invoke<void>("session_write", { id, data: typeof data === "string" ? data : Array.from(data) }),
  resize: (id: string, cols: number, rows: number) => invoke<void>("session_resize", { id, cols, rows }),
  ack: (id: string, bytes: number) => invoke<void>("session_ack", { id, bytes }),
  close: (id: string) => invoke<void>("session_close", { id }),
  /** 页面加载时调用，关掉上一次页面留下的会话和 SFTP 面板。 */
  closeAll: () => invoke<void>("session_close_all"),
};

/** SFTP 面板，见 src-tauri/src/service/sftp.rs。和同一连接的终端标签共用 SSH 连接。 */
export const sftpApi = {
  open: (connId: string, onEvent: (e: SftpEvent) => void) => {
    const ch = new Channel<SftpEvent>();
    ch.onmessage = onEvent;
    return invoke<SftpOpened>("sftp_open", { connId, onEvent: ch });
  },
  list: (id: string, path: string) => invoke<Listing>("sftp_list", { id, path }),
  mkdir: (id: string, path: string) => invoke<void>("sftp_mkdir", { id, path }),
  rename: (id: string, from: string, to: string) => invoke<void>("sftp_rename", { id, from, to }),
  /** 目录连同其中的内容一起删除。 */
  remove: (id: string, paths: string[]) => invoke<void>("sftp_remove", { id, paths }),
  /** 加入传输队列，同名文件会被覆盖。 */
  upload: (id: string, localPaths: string[], remoteDir: string) => invoke<Transfer[]>("sftp_upload", { id, localPaths, remoteDir }),
  download: (id: string, remotePaths: string[], localDir: string) =>
    invoke<Transfer[]>("sftp_download", { id, remotePaths, localDir }),
  cancel: (id: string, taskId: string) => invoke<void>("sftp_cancel", { id, taskId }),
  close: (id: string) => invoke<void>("sftp_close", { id }),
  /** 取消时返回空数组。 */
  pickUploadFiles: () => invoke<string[]>("pick_upload_files"),
  /** 取消时返回空字符串。 */
  pickDownloadDir: (defaultDir?: string) => invoke<string>("pick_download_dir", { defaultDir }),
};

export const workspaceApi = {
  get: () => invoke<Workspace>("workspace_get"),
  saveTabs: (tabs: WorkspaceTab[], active: number | null) => invoke<void>("workspace_save_tabs", { tabs, active }),
  /** 记一次打开连接，用于快速连接里的最近排序。 */
  touch: (connId: string) => invoke<void>("workspace_touch", { connId }),
};

export const updateApi = {
  status: () => invoke<UpdateInfo>("update_status"),
  /** via 为跳板机的连接 ID 时经它访问 GitHub，为空时直接访问。 */
  check: (via?: string) => invoke<UpdateInfo>("update_check", { via: via ?? null }),
  /** 托盘是否请求打开更新窗口，取出后清除。 */
  takeOpenRequest: () => invoke<boolean>("update_take_open_request"),
  /** 下载并安装，成功后程序会重启，不会正常返回。via 同 check。 */
  install: (via?: string) => invoke<void>("update_install", { via: via ?? null }),
  openReleasePage: () => invoke<void>("open_release_page"),
};

/** 后端推送的事件，见 src-tauri/src/events.rs。 */
export const events = {
  onConnChanged: (fn: () => void) => sub<null>("conn:changed", () => fn()),
  /** 设置在设置页以外被修改（导入连接时带入了工具配置）。 */
  onSettingsChanged: (fn: () => void) => sub<null>("settings:changed", () => fn()),
  onHostKey: (fn: (p: HostKeyPrompt) => void) => sub<HostKeyPrompt>("hostkey:prompt", fn),
  /** 请求已结束（回答、超时或连接取消），载荷为 requestId。 */
  onHostKeyDone: (fn: (requestId: string) => void) => sub<string>("hostkey:done", fn),
  /** 点击了托盘菜单里的最近连接，载荷为连接 ID。 */
  onTrayOpen: (fn: (connId: string) => void) => sub<string>("tray:open", fn),
  onUpdate: (fn: (u: UpdateInfo) => void) => sub<UpdateInfo>("update:status", fn),
  onUpdateProgress: (fn: (p: { downloaded: number; total?: number }) => void) => sub("update:progress", fn),
  /** 点击了托盘菜单里的“发现新版本”。 */
  onUpdateOpen: (fn: () => void) => sub<null>("update:open", () => fn()),
};

/** 订阅事件，返回同步的取消函数，方便在 useEffect 中直接使用。 */
function sub<T>(name: string, fn: (payload: T) => void): () => void {
  if (!isTauri()) return () => {};
  let un: UnlistenFn | undefined;
  let cancelled = false;
  listen<T>(name, (e) => fn(e.payload)).then((u) => {
    if (cancelled) u();
    else un = u;
  });
  return () => {
    cancelled = true;
    un?.();
  };
}
