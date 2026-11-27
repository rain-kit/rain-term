import type { Terminal } from "@xterm/xterm";

/** 按下到松开超过这么久不算单击。 */
const CLICK_MS = 500;
/** 发出方向键后等远端回应的最长时间。 */
const REPLY_MS = 1000;
/** 这么久没有新输出，认为远端已经处理完这批方向键。 */
const SETTLE_MS = 40;
const MAX_ROUNDS = 4;

type Cell = { x: number; y: number };

/** 屏幕内容向上滚了几行（向下滚为负）：取非空行对得上最多的位移。 */
function scrolled(before: string[], after: string[]) {
  const n = Math.min(before.length, after.length);
  let best = 0;
  let bestScore = -1;
  for (let d = 1 - n; d < n; d++) {
    let score = 0;
    for (let i = Math.max(0, -d); i < Math.min(n, n - d); i++) {
      if (after[i] && after[i] === before[i + d]) score++;
    }
    if (score > bestScore || (score === bestScore && Math.abs(d) < Math.abs(best))) {
      best = d;
      bestScore = score;
    }
  }
  return best;
}

/**
 * 在 vim、nano 这类全屏程序里单击时，把点击位置换算成方向键发给远端，让光标移过去。
 *
 * 屏幕上的格子和文本并不一一对应：长行折行后上下键按整行移动、Tab 占多格、
 * 光标靠近边缘时屏幕会滚动。所以每发一批方向键都等远端画完，再按光标的实际位置修正。
 * 程序自己处理鼠标（vim 的 set mouse=a）时不介入。
 */
export function clickMovesCursor(xt: Terminal, el: HTMLElement, enabled: () => boolean, send: (d: string) => void) {
  let token = 0;
  let down: (Cell & { time: number }) | null = null;

  const buf = () => xt.buffer.active;
  const line = (y: number) => buf().getLine(buf().baseY + y);
  // 只有 vim、nano 这类程序会开应用光标键模式；光标在最后一行时多半是 vim 的命令行或 less 的提示符，
  // 这时上下键是翻历史、翻页。
  const usable = () =>
    enabled() &&
    buf().type === "alternate" &&
    xt.modes.mouseTrackingMode === "none" &&
    xt.modes.applicationCursorKeysMode &&
    buf().cursorY < xt.rows - 1;

  const arrows = (key: "A" | "B" | "C" | "D", n: number) =>
    `${xt.modes.applicationCursorKeysMode ? "\x1bO" : "\x1b["}${key}`.repeat(n);

  // 去掉行首的行号：开了相对行号时，每移动一次行号都会变。
  const snapshot = () =>
    Array.from({ length: xt.rows - 1 }, (_, y) => (line(y)?.translateToString(true) ?? "").replace(/^\s*\d+ /, ""));

  const blank = (y: number, x: number) => {
    const c = line(y)?.getCell(x);
    return !c || (c.getWidth() > 0 && !c.getChars().trim());
  };

  /** 点在行尾之后时算到行尾，点在宽字符右半边时算这个字符。 */
  const clampX = (y: number, x: number) => {
    let end = xt.cols;
    while (end > 0 && blank(y, end - 1)) end--;
    x = Math.min(x, end);
    if (x > 0 && x < xt.cols && line(y)?.getCell(x)?.getWidth() === 0) x--;
    return x;
  };

  /** a 到 b（a 在前）之间的字符数，宽字符占两格只算一个。 */
  const chars = (a: Cell, b: Cell) => {
    let n = 0;
    for (let y = a.y; y <= b.y; y++) {
      const l = line(y);
      for (let x = y === a.y ? a.x : 0; x < (y === b.y ? b.x : xt.cols); x++) {
        if ((l?.getCell(x)?.getWidth() ?? 1) > 0) n++;
      }
    }
    return n;
  };

  /** 跨行左右移动只在中间各行都写满（同一行折下来的）时才有意义。 */
  const sameLine = (y1: number, y2: number) => {
    for (let y = Math.min(y1, y2); y < Math.max(y1, y2); y++) if (blank(y, xt.cols - 1)) return false;
    return true;
  };

  /** 等远端画完；一直没有输出、被取消或程序已退出时返回 false。 */
  const settle = (my: number) =>
    new Promise<boolean>((resolve) => {
      let got = false;
      let quiet = 0;
      const finish = () => {
        sub.dispose();
        clearTimeout(quiet);
        clearTimeout(limit);
        resolve(got && my === token && usable());
      };
      const sub = xt.onWriteParsed(() => {
        got = true;
        clearTimeout(quiet);
        quiet = window.setTimeout(finish, SETTLE_MS);
      });
      const limit = window.setTimeout(finish, REPLY_MS);
    });

  /** 发一批方向键并等远端画完，目标行随屏幕滚动修正；返回 false 时停止。 */
  const step = async (my: number, keys: string, t: Cell) => {
    const before = snapshot();
    send(keys);
    if (!(await settle(my))) return false;
    t.y -= scrolled(before, snapshot());
    return t.y >= 0 && t.y < xt.rows - 1;
  };

  const move = async (t: Cell) => {
    const my = ++token;

    for (let round = 0, crossed = 0; round < MAX_ROUNDS; round++) {
      const y = buf().cursorY;
      if (y === t.y) break;
      if (!(await step(my, arrows(t.y < y ? "A" : "B", Math.abs(t.y - y)), t))) return;
      const ny = buf().cursorY;
      if (ny === y && t.y !== y) break;
      // 越过目标说明中间有折行，再反向修正一次；还越过就是目标在折行的后半段，交给左右键。
      if ((ny - t.y) * (y - t.y) < 0 && ++crossed > 1) break;
    }

    for (let round = 0; round < MAX_ROUNDS; round++) {
      const c = { x: buf().cursorX, y: buf().cursorY };
      const target = { x: clampX(t.y, t.x), y: t.y };
      if ((c.x === target.x && c.y === target.y) || !sameLine(c.y, target.y)) return;
      const forward = c.y < target.y || (c.y === target.y && c.x < target.x);
      const n = forward ? chars(c, target) : chars(target, c);
      if (!n || !(await step(my, arrows(forward ? "C" : "D", n), t))) return;
      if (buf().cursorX === c.x && buf().cursorY === c.y) return;
    }
  };

  const cellAt = (e: MouseEvent): Cell | null => {
    const r = el.querySelector(".xterm-screen")?.getBoundingClientRect();
    if (!r?.width || !r.height) return null;
    const x = Math.floor(((e.clientX - r.left) / r.width) * xt.cols);
    const y = Math.floor(((e.clientY - r.top) / r.height) * xt.rows);
    return x >= 0 && y >= 0 && x < xt.cols && y < xt.rows - 1 ? { x, y } : null;
  };

  const onDown = (e: MouseEvent) => {
    down = null;
    if (e.button !== 0 || e.detail > 1 || e.shiftKey || e.altKey || e.ctrlKey || e.metaKey || !usable()) return;
    const c = cellAt(e);
    if (c) down = { ...c, time: e.timeStamp };
  };
  const onUp = (e: MouseEvent) => {
    const d = down;
    down = null;
    if (!d || e.button !== 0 || e.timeStamp - d.time > CLICK_MS) return;
    const c = cellAt(e);
    if (c && c.x === d.x && c.y === d.y && usable()) void move(c);
  };
  el.addEventListener("mousedown", onDown, true);
  el.addEventListener("mouseup", onUp, true);

  return {
    /** 用户自己按键时停止，避免和用户的输入交错。 */
    cancel: () => void token++,
    dispose: () => {
      token++;
      el.removeEventListener("mousedown", onDown, true);
      el.removeEventListener("mouseup", onUp, true);
    },
  };
}
