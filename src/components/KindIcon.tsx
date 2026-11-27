import { Monitor, SquareTerminal, type LucideIcon } from "lucide-react";
import type { ConnKind } from "../lib/types";
import { cx } from "./ui";

const icons: Record<ConnKind, { icon: LucideIcon; cls: string }> = {
  ssh: { icon: SquareTerminal, cls: "bg-slate-500/12 text-slate-600 dark:text-slate-300" },
  rdp: { icon: Monitor, cls: "bg-sky-500/12 text-sky-600 dark:text-sky-400" },
};

/** 连接类型的小图标。 */
export default function KindIcon({ kind, className }: { kind: ConnKind; className?: string }) {
  const { icon: Icon, cls } = icons[kind];
  return (
    <span className={cx("flex size-6 shrink-0 items-center justify-center rounded-md", cls, className)}>
      <Icon size={14} />
    </span>
  );
}
