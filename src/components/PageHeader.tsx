import type { ReactNode } from "react";

/** 日志、设置等整页内容的标题行。 */
export default function PageHeader({ title, actions }: { title: ReactNode; actions?: ReactNode }) {
  return (
    <div className="flex h-13 shrink-0 items-center gap-2 px-6">
      <h1 className="flex-1 truncate text-lg font-semibold">{title}</h1>
      {actions && <div className="flex items-center gap-2">{actions}</div>}
    </div>
  );
}
