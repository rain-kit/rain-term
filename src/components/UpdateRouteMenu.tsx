import { Globe, ListFilter, Waypoints } from "lucide-react";
import { useState, type ReactNode } from "react";
import { useI18n } from "../lib/i18n";
import { useConns, useUpdate } from "../lib/store";
import JumpPicker from "./JumpPicker";
import { Menu, type MenuItem } from "./ui";

/** 检查更新、直接更新前选择访问 GitHub 的方式：直接访问、经最近选过的服务器，或另选一台服务器。 */
export default function UpdateRouteMenu({
  onPick,
  children,
}: {
  onPick: (via?: string) => void;
  children: (open: () => void) => ReactNode;
}) {
  const { t } = useI18n();
  const { conns } = useConns();
  const { lastJump } = useUpdate();
  const [picking, setPicking] = useState(false);
  const last = conns.find((c) => c.id === lastJump);
  const items: MenuItem[] = [
    { label: t("update.direct"), icon: <Globe size={14} />, onClick: () => onPick() },
    ...(last ? [{ label: t("update.viaJump", { name: last.name }), icon: <Waypoints size={14} />, onClick: () => onPick(last.id) }] : []),
    { label: t("update.pickJump"), icon: <ListFilter size={14} />, onClick: () => setPicking(true) },
  ];
  return (
    <>
      <Menu trigger={children} items={items} />
      <JumpPicker open={picking} last={lastJump} onClose={() => setPicking(false)} onPick={onPick} />
    </>
  );
}
