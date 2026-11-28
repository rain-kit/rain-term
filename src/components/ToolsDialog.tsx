import { FolderTree, SquareTerminal } from "lucide-react";
import { useState, type ReactNode } from "react";
import { useI18n } from "../lib/i18n";
import { useSettings, useToast } from "../lib/store";
import type { ToolsSettings } from "../lib/types";
import { Card, cx, Dialog, SwitchRow } from "./ui";

type Section = keyof ToolsSettings;

/** 工具窗口：左侧按分类切换，右侧是可以随时开关的功能，修改后立即保存。 */
export default function ToolsDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { t } = useI18n();
  const toast = useToast();
  const { settings, save } = useSettings();
  const [section, setSection] = useState<Section>("sftp");
  const tools = settings?.tools;
  const setGroup = <K extends Section>(group: K, patch: Partial<ToolsSettings[K]>) =>
    tools && save({ tools: { ...tools, [group]: { ...tools[group], ...patch } } }).catch(toast.error);

  const sections: { id: Section; icon: ReactNode; label: string; content: (tools: ToolsSettings) => ReactNode }[] = [
    {
      id: "sftp",
      icon: <FolderTree size={15} />,
      label: t("tools.sftp"),
      content: (tools) => (
        <>
          <SwitchRow
            title={t("tools.sftpSudo")}
            desc={t("tools.sftpSudoDesc")}
            checked={tools.sftp.sudo}
            onChange={(sudo) => setGroup("sftp", { sudo })}
          />
          <SwitchRow
            title={t("tools.sftpFollow")}
            desc={t("tools.sftpFollowDesc")}
            checked={tools.sftp.followTerminal}
            onChange={(followTerminal) => setGroup("sftp", { followTerminal })}
          />
        </>
      ),
    },
    {
      id: "enhance",
      icon: <SquareTerminal size={15} />,
      label: t("tools.terminal"),
      content: (tools) => (
        <SwitchRow
          title={t("tools.clickMovesCursor")}
          desc={t("tools.clickMovesCursorDesc")}
          checked={tools.enhance.clickMovesCursor}
          onChange={(clickMovesCursor) => setGroup("enhance", { clickMovesCursor })}
        />
      ),
    },
  ];
  const current = sections.find((s) => s.id === section) ?? sections[0];

  return (
    <Dialog open={open} onClose={onClose} title={t("tools.title")} width="max-w-2xl">
      <div className="flex min-h-80 gap-4 pb-3">
        <nav className="flex w-36 shrink-0 flex-col gap-0.5">
          {sections.map((s) => (
            <button
              key={s.id}
              onClick={() => setSection(s.id)}
              className={cx(
                "flex h-8 items-center gap-2 rounded-lg px-2.5 text-left text-sm transition-colors",
                s.id === current.id ? "bg-subtle font-medium text-ink" : "text-muted hover:bg-subtle hover:text-ink",
              )}
            >
              {s.icon}
              <span className="truncate">{s.label}</span>
            </button>
          ))}
        </nav>
        <div className="min-w-0 flex-1">
          <h3 className="mb-2 px-1 text-xs font-semibold tracking-wide text-muted">{current.label}</h3>
          {tools && <Card className="divide-y divide-line [&>*]:px-4 [&>*]:py-3.5">{current.content(tools)}</Card>}
        </div>
      </div>
    </Dialog>
  );
}
