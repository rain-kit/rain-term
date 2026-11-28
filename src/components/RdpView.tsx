import { getCurrentWindow } from "@tauri-apps/api/window";
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { LoaderCircle, Maximize, Minimize, RotateCw } from "lucide-react";
import { useEffect, useRef, useState, type MouseEvent as ReactMouseEvent, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { errorMessage, rdpApi } from "../lib/api";
import { formatResolution, isMac } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { useToast } from "../lib/store";
import type { RdpInput, RdpNotice } from "../lib/types";
import { ConnStatus } from "./StatusBar";
import { Button, cx, IconButton } from "./ui";

export type RdpStatus = "connecting" | "connected" | "closed";

/** 与 src-tauri/src/core/rdp/session.rs 的 MSG_* 一致。 */
const MSG_POINTER = 2;
const MSG_POINTER_DEFAULT = 3;
const MSG_POINTER_HIDDEN = 4;

/** 标签大小停止变化这么久后再请求新的分辨率，拖动窗口时不会频繁切换。 */
const RESIZE_DELAY = 500;
/** 浏览器滚一格约 100 像素，RDP 一格是 120。 */
const WHEEL_RATIO = 1.2;
/** RDP 一次滚动的取值范围是 9 位有符号数。 */
const MAX_WHEEL = 255;
const FALLBACK_SIZE = { w: 1920, h: 1080 };

const dpr = () => window.devicePixelRatio || 1;
const clamp = (v: number, min: number, max: number) => Math.min(max, Math.max(min, v));

/** 元素的物理像素大小，标签隐藏时为 null。 */
function physicalSize(el: HTMLElement) {
  const w = Math.round(el.clientWidth * dpr());
  const h = Math.round(el.clientHeight * dpr());
  return w > 0 && h > 0 ? { w, h } : null;
}

/**
 * `[u16 画面宽][u16 画面高][u16 块数]`，每块 `[u16 x][u16 y][u16 宽][u16 高][RGBA]`，均为小端。
 * 画面大小变了先调整 canvas，返回是否调整过。
 */
function drawFrame(c: HTMLCanvasElement, ctx: CanvasRenderingContext2D, d: Uint8Array<ArrayBuffer>) {
  const dv = new DataView(d.buffer, d.byteOffset, d.byteLength);
  const [width, height, count] = [0, 2, 4].map((k) => dv.getUint16(k, true));
  const resized = c.width !== width || c.height !== height;
  if (resized) {
    c.width = width;
    c.height = height;
  }
  let off = 6;
  for (let i = 0; i < count; i++) {
    const [x, y, w, h] = [0, 2, 4, 6].map((k) => dv.getUint16(off + k, true));
    off += 8;
    const len = w * h * 4;
    if (w && h) ctx.putImageData(new ImageData(new Uint8ClampedArray(d.buffer, d.byteOffset + off, len), w, h), x, y);
    off += len;
  }
  return resized;
}

/** `[2][u16 宽][u16 高][u16 热点 x][u16 热点 y][RGBA]` 转成 CSS cursor。 */
function pointerCursor(d: Uint8Array<ArrayBuffer>) {
  const dv = new DataView(d.buffer, d.byteOffset, d.byteLength);
  const [w, h, hx, hy] = [1, 3, 5, 7].map((k) => dv.getUint16(k, true));
  if (!w || !h) return "none";
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  c.getContext("2d")!.putImageData(new ImageData(new Uint8ClampedArray(d.buffer, d.byteOffset + 9, w * h * 4), w, h), 0, 0);
  return `url(${c.toDataURL()}) ${hx} ${hy}, default`;
}

/** 一个远程桌面标签：canvas 显示画面，把键盘、鼠标和剪贴板转给后端，断开后可以重新连接。 */
export default function RdpView({
  connId,
  name,
  active,
  fit,
  clipboard,
  onStatus,
}: {
  connId: string;
  name: string;
  active: boolean;
  /** 分辨率跟随标签大小。 */
  fit: boolean;
  /** 和远程桌面共享文本剪贴板。 */
  clipboard: boolean;
  onStatus: (s: RdpStatus) => void;
}) {
  const { t, tm } = useI18n();
  const toast = useToast();
  const box = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const session = useRef<string | null>(null);
  /** 每次连接加一，旧连接迟到的消息据此丢弃。 */
  const gen = useRef(0);
  const input = useRef<{ queue: RdpInput[]; busy: boolean }>({ queue: [], busy: false });
  const wheel = useRef({ v: 0, h: 0 });
  /** 最近一次和远程同步过的剪贴板文本，避免来回同步同一段内容。 */
  const lastClip = useRef<string | null>(null);
  const fitRef = useRef(fit);
  fitRef.current = fit;
  const clipboardRef = useRef(clipboard);
  clipboardRef.current = clipboard;
  const onStatusRef = useRef(onStatus);
  onStatusRef.current = onStatus;

  const [status, setStatus] = useState<{ s: RdpStatus; message?: string }>({ s: "connecting" });
  const [size, setSize] = useState<{ width: number; height: number } | null>(null);
  const [fullscreen, setFullscreen] = useState(false);

  useEffect(() => onStatusRef.current(status.s), [status.s]);

  /** 按远程分辨率等比缩小到标签里，不放大。 */
  const layout = () => {
    const el = box.current;
    const c = canvas.current;
    if (!el || !c || !el.clientWidth || !el.clientHeight) return;
    const w = c.width / dpr();
    const h = c.height / dpr();
    const k = Math.min(1, el.clientWidth / w, el.clientHeight / h);
    c.style.width = `${w * k}px`;
    c.style.height = `${h * k}px`;
  };

  // 输入按顺序逐批发送：并发的 invoke 不保证到达顺序。排队时连续的鼠标移动只保留最后一个。
  const send = (...events: RdpInput[]) => {
    if (!session.current) return;
    const q = input.current;
    for (const e of events) {
      if (e.t === "move" && q.queue[q.queue.length - 1]?.t === "move") q.queue[q.queue.length - 1] = e;
      else q.queue.push(e);
    }
    if (q.busy) return;
    q.busy = true;
    (async () => {
      while (q.queue.length) {
        const batch = q.queue;
        q.queue = [];
        const id = session.current;
        if (!id) break;
        await rdpApi.input(id, batch).catch(() => {});
      }
      q.queue = [];
      q.busy = false;
    })();
  };

  const syncClipboard = () => {
    const id = session.current;
    if (!id || !clipboardRef.current) return;
    readText().then(
      (text) => {
        if (!text || text === lastClip.current || session.current !== id) return;
        lastClip.current = text;
        rdpApi.clipboard(id, text).catch(() => {});
      },
      () => {},
    );
  };

  const connect = () => {
    const el = box.current;
    const c = canvas.current;
    const ctx = c?.getContext("2d", { alpha: false });
    if (!el || !c || !ctx) return;
    const my = ++gen.current;
    let ended = false;
    // 画完一条再取下一条；会话结束或被关闭时取到 null。
    const pull = async (id: string) => {
      for (;;) {
        const d = await rdpApi.frame(id).catch(() => null);
        if (!d || gen.current !== my) return;
        if (drawFrame(c, ctx, d)) {
          layout();
          setSize({ width: c.width, height: c.height });
        }
      }
    };
    const px = physicalSize(el) ?? FALLBACK_SIZE;
    setStatus({ s: "connecting" });
    rdpApi
      .open(
        connId,
        px.w,
        px.h,
        Math.round(dpr() * 100),
        (d) => {
          if (gen.current !== my) return;
          if (d[0] === MSG_POINTER) c.style.cursor = pointerCursor(d);
          else if (d[0] === MSG_POINTER_DEFAULT) c.style.cursor = "default";
          else if (d[0] === MSG_POINTER_HIDDEN) c.style.cursor = "none";
        },
        (n: RdpNotice) => {
          if (gen.current !== my) return;
          if (n.type === "clipboard") {
            lastClip.current = n.text;
            writeText(n.text).catch(toast.error);
          } else {
            ended = true;
            session.current = null;
            setStatus({ s: "closed", message: n.error ? tm(n.error) : t("rdp.ended") });
          }
        },
      )
      .then((id) => {
        if (gen.current !== my || ended) {
          if (gen.current !== my) rdpApi.close(id).catch(() => {});
          return;
        }
        session.current = id;
        pull(id);
        setStatus({ s: "connected" });
        if (document.activeElement === el) syncClipboard();
      })
      .catch((err) => {
        if (gen.current === my) setStatus({ s: "closed", message: errorMessage(err) });
      });
  };

  const reconnect = () => {
    connect();
    box.current?.focus();
  };

  const toggleFullscreen = (on = !fullscreen) => {
    setFullscreen(on);
    getCurrentWindow()
      .setFullscreen(on)
      .catch(() => {});
    requestAnimationFrame(() => box.current?.focus());
  };

  // 创建会话，标签关闭时释放。
  useEffect(() => {
    const el = box.current!;
    let timer: number | undefined;
    const ro = new ResizeObserver(() => {
      layout();
      if (!fitRef.current) return;
      clearTimeout(timer);
      timer = window.setTimeout(() => {
        const id = session.current;
        const c = canvas.current;
        const px = physicalSize(el);
        if (!id || !c || !px || (px.w === c.width && px.h === c.height)) return;
        rdpApi.resize(id, px.w, px.h, Math.round(dpr() * 100)).catch(() => {});
      }, RESIZE_DELAY);
    });
    ro.observe(el);
    connect();
    return () => {
      gen.current++;
      ro.disconnect();
      clearTimeout(timer);
      if (session.current) rdpApi.close(session.current).catch(() => {});
      session.current = null;
    };
  }, [connId]);

  // 关闭标签时退出全屏。
  useEffect(() => {
    if (!fullscreen) return;
    return () => {
      getCurrentWindow()
        .setFullscreen(false)
        .catch(() => {});
    };
  }, [fullscreen]);

  // 切换到这个标签时聚焦；切走时松开所有按键，避免远程卡键。
  useEffect(() => {
    if (!active) {
      if (session.current) rdpApi.releaseAll(session.current).catch(() => {});
      return;
    }
    const id = requestAnimationFrame(() => {
      layout();
      box.current?.focus();
    });
    return () => cancelAnimationFrame(id);
  }, [active]);

  const pos = (e: { clientX: number; clientY: number }) => {
    const c = canvas.current!;
    const r = c.getBoundingClientRect();
    if (!r.width || !r.height) return null;
    return {
      x: clamp(Math.floor(((e.clientX - r.left) * c.width) / r.width), 0, c.width - 1),
      y: clamp(Math.floor(((e.clientY - r.top) * c.height) / r.height), 0, c.height - 1),
    };
  };

  const move = (e: { clientX: number; clientY: number }) => {
    const p = pos(e);
    if (p) send({ t: "move", ...p });
  };

  const onButton = (e: ReactMouseEvent, down: boolean) => {
    e.preventDefault();
    if (down) box.current?.focus();
    move(e);
    send({ t: "button", button: e.button, down });
  };

  const onKey = (e: ReactKeyboardEvent, down: boolean) => {
    e.preventDefault();
    e.stopPropagation();
    if (down && e.ctrlKey && e.altKey && e.code === "Enter") {
      toggleFullscreen();
      return;
    }
    if (isMac && (e.code === "MetaLeft" || e.code === "MetaRight")) {
      send({ t: "key", code: e.code === "MetaLeft" ? "ControlLeft" : "ControlRight", down });
      return;
    }
    // macOS 按住 ⌘ 时其他键不触发 keyup，按下时直接补上松开，否则远程会卡键。
    if (isMac && e.metaKey) {
      if (down && e.code) send({ t: "key", code: e.code, down: true }, { t: "key", code: e.code, down: false });
      return;
    }
    if (e.code) send({ t: "key", code: e.code, down });
    else if (down && e.key.length === 1) send({ t: "text", text: e.key });
  };

  const tapWin = () => {
    send({ t: "key", code: "MetaLeft", down: true }, { t: "key", code: "MetaLeft", down: false });
    box.current?.focus();
  };

  const ctrlAltDel = () => {
    if (session.current) rdpApi.ctrlAltDel(session.current).catch(toast.error);
    box.current?.focus();
  };

  const connected = status.s === "connected";

  return (
    <div className={cx("flex min-h-0 flex-1 flex-col bg-zinc-900", fullscreen && "fixed inset-0 z-40")}>
      <div className="flex h-8 shrink-0 items-center gap-1 border-b border-line bg-surface px-2">
        <Button size="sm" variant="ghost" disabled={!connected} onClick={ctrlAltDel}>
          Ctrl+Alt+Del
        </Button>
        <Button size="sm" variant="ghost" disabled={!connected} title={t("rdp.winHint")} onClick={tapWin}>
          Win
        </Button>
        <span className="flex-1" />
        <IconButton
          label={t(fullscreen ? "rdp.exitFullscreen" : "rdp.fullscreen")}
          size="sm"
          onClick={() => toggleFullscreen()}
        >
          {fullscreen ? <Minimize size={15} /> : <Maximize size={15} />}
        </IconButton>
      </div>
      <div
        ref={box}
        tabIndex={0}
        className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden outline-none"
        onKeyDown={(e) => onKey(e, true)}
        onKeyUp={(e) => onKey(e, false)}
        onFocus={syncClipboard}
        onBlur={() => session.current && rdpApi.releaseAll(session.current).catch(() => {})}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
        }}
      >
        <canvas
          ref={canvas}
          width={0}
          height={0}
          className={cx(!size && "invisible")}
          onPointerDown={(e) => e.currentTarget.setPointerCapture(e.pointerId)}
          onPointerMove={move}
          onMouseDown={(e) => onButton(e, true)}
          onMouseUp={(e) => onButton(e, false)}
          onWheel={(e) => {
            const k = (e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? 800 : 1) * WHEEL_RATIO;
            const w = wheel.current;
            w.v -= e.deltaY * k;
            w.h += e.deltaX * k;
            const out: RdpInput[] = [];
            for (const vertical of [true, false]) {
              const key = vertical ? "v" : "h";
              const units = Math.trunc(clamp(w[key], -MAX_WHEEL, MAX_WHEEL));
              if (!units) continue;
              w[key] -= units;
              out.push({ t: "wheel", vertical, units });
            }
            if (out.length) send(...out);
          }}
        />
        {status.s !== "connected" && (
          <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 bg-black/50 px-6 text-center text-sm text-zinc-200">
            {status.s === "connecting" ? (
              <>
                <LoaderCircle size={22} className="animate-spin" />
                {t("rdp.connecting", { name })}
              </>
            ) : (
              <>
                <div className="max-w-lg break-words">{status.message}</div>
                <Button variant="primary" icon={<RotateCw size={14} />} onClick={reconnect}>
                  {t("terminal.reconnect")}
                </Button>
              </>
            )}
          </div>
        )}
      </div>

      {active && (
        <ConnStatus
          status={status.s}
          name={name}
          connectingText={t("rdp.connecting", { name })}
          message={status.message}
          extra={size && formatResolution(size.width, size.height)}
          onReconnect={reconnect}
        />
      )}
    </div>
  );
}
