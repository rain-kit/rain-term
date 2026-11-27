import { useRef, useState } from "react";
import { useI18n } from "../lib/i18n";
import { cx } from "./ui";

/** 宽度或高度用占容器的比例表示，保存在 localStorage。 */
export interface SizeSpec {
  key: string;
  initial: number;
  min: number;
  max: number;
}

const clamp = (spec: SizeSpec, v: number) => Math.min(spec.max, Math.max(spec.min, v));

/** 返回当前大小、拖动中的临时修改和松开后的保存。 */
export function useStoredSize(spec: SizeSpec) {
  const [size, setSize] = useState(() => {
    const v = Number(localStorage.getItem(spec.key));
    return v > 0 ? clamp(spec, v) : spec.initial;
  });
  const commit = (v: number) => {
    setSize(v);
    localStorage.setItem(spec.key, String(v));
  };
  return [size, setSize, commit] as const;
}

/**
 * 面板边缘的拖动条：按住拖动调整大小，双击恢复默认。
 * 放在面板里作为直接子元素，面板的父元素就是计算比例的容器；side 是拖动条所在的一边，
 * left / right 调整宽度，top 调整高度。
 */
export function ResizeHandle({
  spec,
  side,
  size,
  onChange,
  onCommit,
}: {
  spec: SizeSpec;
  side: "left" | "right" | "top";
  size: number;
  onChange: (v: number) => void;
  onCommit: (v: number) => void;
}) {
  const { t } = useI18n();
  const [active, setActive] = useState(false);
  const last = useRef(size);
  const vertical = side === "top";
  const ratio = (el: HTMLElement, e: { clientX: number; clientY: number }) => {
    const box = el.parentElement?.parentElement?.getBoundingClientRect();
    if (!box?.width || !box.height) return size;
    if (vertical) return clamp(spec, (box.bottom - e.clientY) / box.height);
    return clamp(spec, (side === "right" ? e.clientX - box.left : box.right - e.clientX) / box.width);
  };
  return (
    <div
      role="separator"
      aria-orientation={vertical ? "horizontal" : "vertical"}
      title={t(vertical ? "common.resizeHeightHint" : "common.resizeHint")}
      className={cx(
        "absolute z-10 transition-colors hover:bg-accent/40",
        vertical ? "inset-x-0 -top-0.75 h-1.5 cursor-row-resize" : "inset-y-0 w-1.5 cursor-col-resize",
        side === "right" && "-right-0.75",
        side === "left" && "-left-0.75",
        active && "bg-accent/60",
      )}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        e.currentTarget.setPointerCapture(e.pointerId);
        last.current = size;
        setActive(true);
      }}
      onPointerMove={(e) => {
        if (!active) return;
        last.current = ratio(e.currentTarget, e);
        onChange(last.current);
      }}
      onPointerUp={() => {
        if (!active) return;
        setActive(false);
        onCommit(last.current);
      }}
      onLostPointerCapture={() => setActive(false)}
      onDoubleClick={() => onCommit(spec.initial)}
    />
  );
}
