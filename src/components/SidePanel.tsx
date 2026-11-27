import { PanelBottom, PanelRight, X } from "lucide-react";
import type { ReactNode } from "react";
import { useI18n } from "../lib/i18n";
import type { PanelMode, PanelPosition } from "../lib/types";
import { ResizeHandle, useStoredSize, type SizeSpec } from "./Resizer";
import { cx, IconButton } from "./ui";

/** 放在右侧时宽度、放在下方时高度占标签内容区域的比例，两者分别保存；服务器状态和 SFTP 共用，切换时大小不变。 */
const WIDTH: SizeSpec = { key: "panel.width", initial: 0.25, min: 0.05, max: 0.7 };
const HEIGHT: SizeSpec = { key: "panel.height", initial: 0.3, min: 0.05, max: 0.7 };

const TITLE = { monitor: "monitor.title", sftp: "sftp.title" } as const;
const HIDE = { monitor: "monitor.hide", sftp: "sftp.hide" } as const;

/**
 * 终端右侧或下方的侧栏，显示服务器状态或 SFTP。
 * 隐藏时仍然渲染子元素，SFTP 不会因此断开或取消传输。
 */
export default function SidePanel({
  mode,
  visible,
  position,
  onPosition,
  onClose,
  children,
}: {
  mode: PanelMode;
  visible: boolean;
  position: PanelPosition;
  onPosition: (p: PanelPosition) => void;
  onClose: () => void;
  children: ReactNode;
}) {
  const { t } = useI18n();
  const width = useStoredSize(WIDTH);
  const height = useStoredSize(HEIGHT);
  const bottom = position === "bottom";
  const spec = bottom ? HEIGHT : WIDTH;
  const [size, setSize, commitSize] = bottom ? height : width;

  return (
    <aside
      className={cx(
        "relative flex min-h-0 min-w-0 shrink-0 flex-col border-line bg-surface/60",
        bottom ? "border-t" : "border-l",
        !visible && "hidden",
      )}
      style={bottom ? { height: `${size * 100}%` } : { width: `${size * 100}%` }}
    >
      <ResizeHandle spec={spec} side={bottom ? "top" : "left"} size={size} onChange={setSize} onCommit={commitSize} />
      <div className={cx("flex shrink-0 items-center border-b border-line pr-1 pl-3 select-none", bottom ? "h-8" : "h-9")}>
        <span className="flex-1 truncate text-[13px] font-medium">{t(TITLE[mode])}</span>
        <IconButton
          label={t(bottom ? "monitor.moveRight" : "monitor.moveBottom")}
          size="sm"
          onClick={() => onPosition(bottom ? "right" : "bottom")}
        >
          {bottom ? <PanelRight size={15} /> : <PanelBottom size={15} />}
        </IconButton>
        <IconButton label={t(HIDE[mode])} size="sm" onClick={onClose}>
          <X size={15} />
        </IconButton>
      </div>
      {children}
    </aside>
  );
}
