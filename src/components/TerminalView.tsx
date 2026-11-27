import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon, type ISearchOptions } from "@xterm/addon-search";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { ArrowDown, ArrowUp, CaseSensitive, ClipboardPaste, Copy, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { errorMessage, sessionApi, settingsApi } from "../lib/api";
import { clickMovesCursor } from "../lib/clickMove";
import { useI18n } from "../lib/i18n";
import { useSettings, useToast } from "../lib/store";
import { isLightScheme, terminalTheme } from "../lib/terminalThemes";
import type { SessionClosed, TerminalSettings } from "../lib/types";
import { ConnStatus } from "./StatusBar";
import { Confirm, ContextMenu, cx, IconButton } from "./ui";

export type TermStatus = "connecting" | "connected" | "closed";

/** 处理完这么多字节输出就告诉后端一次，见 src-tauri/src/service/session.rs 的 MAX_UNACKED。 */
const ACK_BYTES = 64 * 1024;
const RESIZE_DELAY = 80;
const FONT = '"Cascadia Mono", "JetBrains Mono", "SF Mono", Menlo, Consolas, "Microsoft YaHei UI", "PingFang SC", "Noto Sans Mono CJK SC", monospace';
const isMac = navigator.platform.toUpperCase().includes("MAC");
/** 重新连接前恢复上一个会话可能留下的状态：鼠标上报、括号粘贴、隐藏光标、颜色。 */
const RESET_MODES = "\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?2004l\x1b[?25h\x1b[0m";

/** 搜索高亮，浅色和深色配色各一套，保证文字看得清。 */
const searchDecorations = (light: boolean): ISearchOptions["decorations"] =>
  light
    ? {
        matchBackground: "#fde68a",
        matchBorder: "#f59e0b",
        matchOverviewRuler: "#f59e0b",
        activeMatchBackground: "#fbbf24",
        activeMatchBorder: "#d97706",
        activeMatchColorOverviewRuler: "#d97706",
      }
    : {
        matchBackground: "#5a4a1a",
        matchBorder: "#a08a3a",
        matchOverviewRuler: "#d4a72c",
        activeMatchBackground: "#8a6a10",
        activeMatchBorder: "#f0c040",
        activeMatchColorOverviewRuler: "#f0c040",
      };

function options(s: TerminalSettings, dark: boolean) {
  return {
    fontFamily: s.fontFamily.trim() || FONT,
    fontSize: s.fontSize,
    scrollback: s.scrollback,
    cursorStyle: s.cursorStyle,
    theme: terminalTheme(s.colorScheme, dark),
  };
}

/** 多于一行的粘贴内容（末尾单个换行不算），返回行数；单行返回 0。 */
function multiLine(text: string) {
  const lines = text.replace(/\r\n?/g, "\n").replace(/\n$/, "").split("\n");
  return lines.length > 1 ? lines.length : 0;
}

function pasteMessage(t: ReturnType<typeof useI18n>["t"], text: string, production: boolean, name: string) {
  const lines = multiLine(text);
  if (!production) return t("terminal.pasteMessage", { lines });
  return lines ? t("terminal.pasteProdLines", { name, lines }) : t("terminal.pasteProdMessage", { name });
}

let webglBroken = false;

/**
 * 只给正在显示的终端开 WebGL：每个 WebGL 上下文占几十 MB，
 * 而且浏览器同时最多保留约 16 个，超出时最早的会丢失。
 */
function setWebgl(xt: Terminal, gl: { current: WebglAddon | null }, on: boolean) {
  if (!on) {
    gl.current?.dispose();
    gl.current = null;
    return;
  }
  if (gl.current || webglBroken) return;
  try {
    const addon = new WebglAddon();
    addon.onContextLoss(() => {
      addon.dispose();
      if (gl.current === addon) gl.current = null;
    });
    xt.loadAddon(addon);
    gl.current = addon;
  } catch {
    // 不支持 WebGL 时使用默认的 DOM 渲染。
    webglBroken = true;
  }
}

/** 一个 SSH 终端标签：xterm 加上后端会话，断开后可以重新连接。 */
export default function TerminalView({
  connId,
  name,
  active,
  production,
  onStatus,
}: {
  connId: string;
  name: string;
  active: boolean;
  /** 生产环境：任何粘贴（包括单行）都要确认。 */
  production: boolean;
  onStatus: (s: TermStatus) => void;
}) {
  const { t, tm } = useI18n();
  const toast = useToast();
  const { settings, dark } = useSettings();
  const box = useRef<HTMLDivElement>(null);
  const term = useRef<Terminal | null>(null);
  const fit = useRef<FitAddon | null>(null);
  const search = useRef<SearchAddon | null>(null);
  const gl = useRef<WebglAddon | null>(null);
  const activeRef = useRef(active);
  activeRef.current = active;
  const session = useRef<string | null>(null);
  /** 每次连接加一，旧连接迟到的消息据此丢弃。 */
  const gen = useRef(0);
  const input = useRef<{ queue: (string | Uint8Array)[]; busy: boolean }>({ queue: [], busy: false });
  const clickMoves = useRef(false);
  clickMoves.current = settings?.tools.enhance.clickMovesCursor ?? false;
  const productionRef = useRef(production);
  productionRef.current = production;
  const onStatusRef = useRef(onStatus);
  onStatusRef.current = onStatus;

  const [status, setStatus] = useState<{ s: TermStatus; message?: string }>({ s: "connecting" });
  const [pasting, setPasting] = useState<string | null>(null);
  const [finding, setFinding] = useState(false);
  const [menu, setMenu] = useState<{ x: number; y: number; selected: boolean } | null>(null);

  useEffect(() => onStatusRef.current(status.s), [status.s]);

  // 输入按顺序逐个发送：并发的 invoke 不保证到达顺序。连续的文本合并成一次发送。
  const send = (d: string | Uint8Array) => {
    if (!session.current) return;
    const q = input.current;
    const last = q.queue[q.queue.length - 1];
    if (typeof d === "string" && typeof last === "string") q.queue[q.queue.length - 1] = last + d;
    else q.queue.push(d);
    if (q.busy) return;
    q.busy = true;
    (async () => {
      while (q.queue.length) {
        const item = q.queue.shift()!;
        const id = session.current;
        if (!id) break;
        await sessionApi.write(id, item).catch(() => {});
      }
      q.queue = [];
      q.busy = false;
    })();
  };

  const closedMessage = (c: SessionClosed) => {
    if (c.error) return tm(c.error);
    if (c.exitCode !== undefined) return t("terminal.exitCode", { code: c.exitCode });
    if (c.exitSignal) return t("terminal.exitSignal", { signal: c.exitSignal });
    return t("terminal.exited");
  };

  const connect = () => {
    const xt = term.current;
    if (!xt) return;
    const my = ++gen.current;
    let sid: string | null = null;
    let ended = false;
    let done = 0;
    setStatus({ s: "connecting" });
    sessionApi
      .open(connId, xt.cols, xt.rows, (e) => {
        if (gen.current !== my) return;
        if (e.type === "output") {
          const n = e.data.length;
          xt.write(e.data, () => {
            done += n;
            if (done >= ACK_BYTES && sid) {
              sessionApi.ack(sid, done).catch(() => {});
              done = 0;
            }
          });
          return;
        }
        ended = true;
        session.current = null;
        setStatus({ s: "closed", message: closedMessage(e) });
      })
      .then((id) => {
        if (gen.current !== my || ended) {
          if (gen.current !== my) sessionApi.close(id).catch(() => {});
          return;
        }
        sid = id;
        session.current = id;
        if (done > 0) {
          sessionApi.ack(id, done).catch(() => {});
          done = 0;
        }
        setStatus({ s: "connected" });
        // 连接期间窗口大小可能变了。
        if (xt.cols && xt.rows) sessionApi.resize(id, xt.cols, xt.rows).catch(() => {});
      })
      .catch((err) => {
        if (gen.current === my) setStatus({ s: "closed", message: errorMessage(err) });
      });
  };

  const reconnect = () => {
    const xt = term.current;
    if (!xt) return;
    // ?1049l 在普通屏幕下也会恢复保存的光标位置，只在备用屏幕（vim 等）中才发。
    const alt = xt.buffer.active.type === "alternate" ? "\x1b[?1049l" : "";
    const newline = xt.buffer.active.cursorX > 0 ? "\r\n" : "";
    xt.write(alt + RESET_MODES + newline);
    connect();
    xt.focus();
  };

  const copy = () => {
    const sel = term.current?.getSelection();
    if (sel) writeText(sel).catch(toast.error);
  };

  const paste = (text: string) => {
    if (!text || !session.current) return;
    if (productionRef.current || multiLine(text)) setPasting(text);
    else term.current?.paste(text);
  };

  const pasteClipboard = () => readText().then(paste, () => {});

  // 创建终端和会话，标签关闭时一起释放。
  useEffect(() => {
    const el = box.current!;
    const s = settings!.terminal;
    const xt = new Terminal({
      ...options(s, dark),
      allowProposedApi: true,
      cursorBlink: true,
      rightClickSelectsWord: false,
      macOptionIsMeta: true,
    });
    const fitAddon = new FitAddon();
    const searchAddon = new SearchAddon();
    xt.loadAddon(fitAddon);
    xt.loadAddon(searchAddon);
    xt.loadAddon(new Unicode11Addon());
    xt.unicode.activeVersion = "11";
    xt.loadAddon(new WebLinksAddon((_e, uri) => settingsApi.openUrl(uri).catch(toast.error)));
    xt.open(el);
    setWebgl(xt, gl, activeRef.current);
    term.current = xt;
    fit.current = fitAddon;
    search.current = searchAddon;

    const clickMove = clickMovesCursor(xt, el, () => clickMoves.current, send);
    xt.onData((d) => {
      clickMove.cancel();
      send(d);
    });
    xt.onBinary((d) => send(Uint8Array.from(d, (c) => c.charCodeAt(0))));
    xt.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown") return true;
      const ctrlShift = e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey;
      if (ctrlShift && e.code === "KeyC") {
        e.preventDefault();
        copy();
        return false;
      }
      if (ctrlShift && e.code === "KeyV") {
        e.preventDefault();
        pasteClipboard();
        return false;
      }
      if ((ctrlShift || (isMac && e.metaKey && !e.ctrlKey)) && e.code === "KeyF") {
        e.preventDefault();
        setFinding(true);
        return false;
      }
      return true;
    });

    let timer: number | undefined;
    xt.onResize(({ cols, rows }) => {
      clearTimeout(timer);
      timer = window.setTimeout(() => {
        if (session.current) sessionApi.resize(session.current, cols, rows).catch(() => {});
      }, RESIZE_DELAY);
    });

    // 所有粘贴（Ctrl+V 以外的快捷键、系统菜单、拖放文本）都先经过 paste 的确认。
    const onPaste = (e: ClipboardEvent) => {
      const text = e.clipboardData?.getData("text/plain");
      e.preventDefault();
      e.stopPropagation();
      if (text) paste(text);
    };
    el.addEventListener("paste", onPaste, true);

    let frame = 0;
    const ro = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        if (el.clientWidth > 0 && el.clientHeight > 0) fitAddon.fit();
      });
    });
    ro.observe(el);
    if (el.clientWidth > 0 && el.clientHeight > 0) fitAddon.fit();

    connect();
    return () => {
      gen.current++;
      ro.disconnect();
      cancelAnimationFrame(frame);
      clearTimeout(timer);
      el.removeEventListener("paste", onPaste, true);
      clickMove.dispose();
      if (session.current) sessionApi.close(session.current).catch(() => {});
      session.current = null;
      setWebgl(xt, gl, false);
      xt.dispose();
      term.current = null;
    };
  }, [connId]);

  // 设置或明暗主题变化时立即生效。
  useEffect(() => {
    const xt = term.current;
    if (!xt || !settings) return;
    Object.assign(xt.options, options(settings.terminal, dark));
    if (box.current && box.current.clientWidth > 0) fit.current?.fit();
  }, [settings?.terminal, dark]);

  // 切换到这个标签时开 WebGL、重新适配尺寸并聚焦；切走时释放 WebGL。
  useEffect(() => {
    if (term.current) setWebgl(term.current, gl, active);
    if (!active) return;
    const id = requestAnimationFrame(() => {
      if (box.current && box.current.clientWidth > 0) fit.current?.fit();
      term.current?.focus();
    });
    return () => cancelAnimationFrame(id);
  }, [active]);

  const bg = settings ? terminalTheme(settings.terminal.colorScheme, dark).background : undefined;

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      <div
        ref={box}
        className="min-h-0 flex-1 overflow-hidden py-1 pl-2"
        style={{ background: bg }}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
          const xt = term.current;
          // 程序自己处理鼠标时（vim 的 set mouse=a、tmux 等）右键已经发给了程序；按住 Shift 仍然弹出菜单。
          if (!xt || (xt.modes.mouseTrackingMode !== "none" && !e.shiftKey)) return;
          setMenu({ x: e.clientX, y: e.clientY, selected: xt.hasSelection() });
        }}
      />

      <ContextMenu
        at={menu}
        items={[
          {
            label: t("terminal.copy"),
            icon: <Copy size={14} />,
            disabled: !menu?.selected,
            onClick: () => {
              copy();
              term.current?.clearSelection();
            },
          },
          { label: t("terminal.paste"), icon: <ClipboardPaste size={14} />, onClick: pasteClipboard },
        ]}
        onClose={() => {
          setMenu(null);
          term.current?.focus();
        }}
      />

      {finding && (
        <SearchBar
          addon={search.current}
          light={settings ? isLightScheme(settings.terminal.colorScheme, dark) : !dark}
          onClose={() => {
            setFinding(false);
            search.current?.clearDecorations();
            term.current?.focus();
          }}
        />
      )}

      {active && (
        <ConnStatus
          status={status.s}
          name={name}
          connectingText={t("terminal.connecting", { name })}
          message={status.message}
          onReconnect={reconnect}
        />
      )}

      <Confirm
        open={pasting !== null}
        title={t(production ? "terminal.pasteProdTitle" : "terminal.pasteTitle")}
        danger={production}
        message={
          <>
            <div>{pasteMessage(t, pasting ?? "", production, name)}</div>
            <pre className="mt-2 max-h-40 overflow-auto rounded-lg bg-subtle p-2 font-mono text-xs whitespace-pre-wrap text-ink select-text">
              {pasting?.slice(0, 2000)}
            </pre>
          </>
        }
        confirmText={t("terminal.paste")}
        onConfirm={() => pasting && term.current?.paste(pasting)}
        onClose={() => {
          setPasting(null);
          term.current?.focus();
        }}
      />
    </div>
  );
}

function SearchBar({ addon, light, onClose }: { addon: SearchAddon | null; light: boolean; onClose: () => void }) {
  const { t } = useI18n();
  const [q, setQ] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [result, setResult] = useState<{ index: number; count: number } | null>(null);

  useEffect(() => {
    if (!addon) return;
    const sub = addon.onDidChangeResults((r) => setResult({ index: r.resultIndex, count: r.resultCount }));
    return () => sub.dispose();
  }, [addon]);

  const opts = (incremental: boolean): ISearchOptions => ({ caseSensitive, incremental, decorations: searchDecorations(light) });
  const next = () => q && addon?.findNext(q, opts(false));
  const prev = () => q && addon?.findPrevious(q, opts(false));

  useEffect(() => {
    if (!addon) return;
    if (q) addon.findNext(q, opts(true));
    else {
      addon.clearDecorations();
      setResult(null);
    }
  }, [q, caseSensitive]);

  return (
    <div className="absolute top-2 right-4 z-10 flex items-center gap-1 rounded-lg border border-line bg-surface p-1 shadow-lg">
      <input
        autoFocus
        spellCheck={false}
        value={q}
        placeholder={t("terminal.search")}
        onChange={(e) => setQ(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") onClose();
          else if (e.key === "Enter") {
            e.preventDefault();
            if (e.shiftKey) prev();
            else next();
          }
        }}
        className="h-7 w-48 rounded-md bg-transparent px-2 text-[13px] text-ink outline-none placeholder:text-muted/60"
      />
      <span className="min-w-12 text-center text-xs text-muted tabular-nums">
        {q && result ? (result.count ? `${result.index + 1}/${result.count}` : t("terminal.noResults")) : ""}
      </span>
      <IconButton
        label={t("terminal.caseSensitive")}
        size="sm"
        className={cx(caseSensitive && "bg-accent/12 text-accent")}
        onClick={() => setCaseSensitive((v) => !v)}
      >
        <CaseSensitive size={16} />
      </IconButton>
      <IconButton label={t("terminal.searchPrev")} size="sm" onClick={prev}>
        <ArrowUp size={15} />
      </IconButton>
      <IconButton label={t("terminal.searchNext")} size="sm" onClick={next}>
        <ArrowDown size={15} />
      </IconButton>
      <IconButton label={t("common.close")} size="sm" onClick={onClose}>
        <X size={15} />
      </IconButton>
    </div>
  );
}
