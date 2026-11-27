import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { StatusBarHost } from "./StatusBar";
import { Empty } from "./ui";

/** 左侧连接列表，右侧打开的标签页和底部状态栏；没有标签页时显示说明。 */
export default function ModeLayout({
  list,
  icon: Icon,
  mainTitle,
  mainDesc,
  children,
}: {
  list: ReactNode;
  icon: LucideIcon;
  /** 没有标签页时的标题和说明。 */
  mainTitle: string;
  mainDesc: string;
  children?: ReactNode;
}) {
  return (
    <>
      {list}
      <section className="flex min-w-0 flex-1 flex-col">
        <StatusBarHost>
          {children || (
            <div className="flex min-h-0 flex-1 flex-col justify-center overflow-y-auto px-6">
              <Empty icon={<Icon size={22} />} title={mainTitle} desc={mainDesc} />
            </div>
          )}
        </StatusBarHost>
      </section>
    </>
  );
}
