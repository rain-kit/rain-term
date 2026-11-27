import { Activity, CopyPlus, FolderOpen, Monitor, PlugZap, SquareTerminal, X, type LucideIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import ConnList from "../components/ConnList";
import ModeLayout from "../components/ModeLayout";
import MonitorPanel from "../components/MonitorPanel";
import QuickConnect from "../components/QuickConnect";
import RdpView from "../components/RdpView";
import SftpPanel from "../components/SftpPanel";
import TabBar, { type TabStatus } from "../components/TabBar";
import TerminalView from "../components/TerminalView";
import { Button, Confirm, cx, Empty, IconButton, type MenuItem } from "../components/ui";
import { events, workspaceApi } from "../lib/api";
import { defaultTab, isIncognito } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { useConns, useSettings } from "../lib/store";
import type { ConnView, MonitorPosition, TabKind } from "../lib/types";
import type { MessageKey } from "../locales/zh-CN";

/** 与后端 service/workspace.rs 的 MAX_RECENT 一致。 */
const MAX_RECENT = 20;
/** 是否显示服务器状态，默认显示。 */
const MONITOR_KEY = "monitor.visible";
/** 服务器状态在右侧还是下方，默认右侧。 */
const POSITION_KEY = "monitor.position";

interface Tab {
  key: string;
  kind: TabKind;
  connId: string;
  /** 打开时的连接名，连接被删除后仍用它显示。 */
  name: string;
  /** 同一连接同一类标签的第几个，从 1 开始。 */
  seq: number;
  status: TabStatus;
  /** SFTP 标签排队中和传输中的任务数。 */
  busy: number;
}

const needsConfirm = (x: Tab) => (x.kind === "sftp" ? x.busy > 0 : x.status === "connected");

const TAB_ICONS: Record<TabKind, LucideIcon> = { term: SquareTerminal, sftp: FolderOpen, rdp: Monitor };

const CLOSE_TITLE: Record<TabKind, MessageKey> = { term: "terminal.closeTitle", sftp: "sftp.closeTitle", rdp: "rdp.closeTitle" };

function TabIcon({ kind, size, className }: { kind: TabKind; size: number; className?: string }) {
  const Icon = TAB_ICONS[kind];
  return <Icon size={size} className={className} />;
}

export default function Terminal({
  quickConnect,
  onQuickConnectClose,
  onShow,
}: {
  quickConnect: boolean;
  onQuickConnectClose: () => void;
  /** 从快速连接打开标签后切回终端页。 */
  onShow: () => void;
}) {
  const { t, tx } = useI18n();
  const { settings } = useSettings();
  const { conns, loaded } = useConns();
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [closing, setClosing] = useState<Tab[]>([]);
  const [recent, setRecent] = useState<string[]>([]);
  const [restored, setRestored] = useState(false);
  const [monitor, setMonitor] = useState(() => localStorage.getItem(MONITOR_KEY) !== "0");
  const seq = useRef(0);

  const [position, setPosition] = useState<MonitorPosition>(() =>
    localStorage.getItem(POSITION_KEY) === "bottom" ? "bottom" : "right",
  );

  const showMonitor = (v: boolean) => {
    setMonitor(v);
    localStorage.setItem(MONITOR_KEY, v ? "1" : "0");
  };
  const moveMonitor = (p: MonitorPosition) => {
    setPosition(p);
    localStorage.setItem(POSITION_KEY, p);
  };

  const names = useMemo(() => new Map(conns.map((c) => [c.id, c.name])), [conns]);
  const production = useMemo(() => new Set(conns.filter((c) => c.production).map((c) => c.id)), [conns]);
  const incognito = useMemo(() => new Set(conns.filter(isIncognito).map((c) => c.id)), [conns]);
  const connById = useMemo(() => new Map<string, ConnView>(conns.map((c) => [c.id, c])), [conns]);
  const rdpOf = (id: string) => {
    const c = connById.get(id);
    return c?.kind === "rdp" ? c : undefined;
  };
  const title = (tab: Tab) => {
    const name = names.get(tab.connId) ?? tab.name;
    return tab.seq > 1 ? `${name} (${tab.seq})` : name;
  };
  const current = tabs.find((x) => x.key === active);

  const nextSeq = (list: Tab[], connId: string, kind: TabKind) =>
    Math.max(0, ...list.filter((x) => x.connId === connId && x.kind === kind).map((x) => x.seq)) + 1;

  const touch = (connId: string) => {
    setRecent((r) => [connId, ...r.filter((id) => id !== connId)].slice(0, MAX_RECENT));
    workspaceApi.touch(connId).catch(() => {});
  };

  // 恢复上次打开的标签，只显示不连接；连接已被删除的标签直接丢掉。
  useEffect(() => {
    if (!loaded || restored) return;
    workspaceApi
      .get()
      .then((ws) => {
        const byId = new Map(conns.map((c) => [c.id, c]));
        const list: Tab[] = [];
        let activeKey: string | undefined;
        ws.tabs.forEach((w, i) => {
          const c = byId.get(w.connId);
          if (!c) return;
          const key = `t${++seq.current}`;
          list.push({ key, kind: w.kind, connId: c.id, name: c.name, seq: nextSeq(list, c.id, w.kind), status: "idle", busy: 0 });
          if (i === ws.active) activeKey = key;
        });
        setRecent(ws.recent);
        if (!list.length) return;
        setTabs((cur) => [...list, ...cur]);
        setActive((cur) => cur ?? activeKey ?? list[0].key);
      })
      .catch(() => {})
      .finally(() => setRestored(true));
  }, [loaded]);

  // 标签的组成、顺序或当前标签变化时保存，下次启动时恢复。
  const layout = useMemo(() => tabs.map((x) => `${x.key}:${x.kind}:${x.connId}`).join(","), [tabs]);
  useEffect(() => {
    if (!restored) return;
    const timer = setTimeout(() => {
      const i = tabs.findIndex((x) => x.key === active);
      workspaceApi.saveTabs(tabs.map((x) => ({ connId: x.connId, kind: x.kind })), i >= 0 ? i : null).catch(() => {});
    }, 300);
    return () => clearTimeout(timer);
  }, [restored, layout, active]);

  const open = (c: { id: string; name: string }, kind: TabKind) => {
    const key = `t${++seq.current}`;
    setTabs((list) => [...list, { key, kind, connId: c.id, name: c.name, seq: nextSeq(list, c.id, kind), status: "connecting", busy: 0 }]);
    setActive(key);
    touch(c.id);
  };

  // 托盘菜单里点击最近连接：打开终端标签并切回终端页。
  const trayOpen = useRef((_id: string) => {});
  trayOpen.current = (id) => {
    const c = conns.find((x) => x.id === id);
    if (!c) return;
    open(c, defaultTab(c));
    onShow();
  };
  useEffect(() => events.onTrayOpen((id) => trayOpen.current(id)), []);

  const connect = (tab: Tab) => {
    update(tab.key, { status: "connecting" });
    touch(tab.connId);
  };

  const remove = (keys: string[]) => {
    const gone = new Set(keys);
    setTabs((list) => list.filter((x) => !gone.has(x.key)));
    if (active && gone.has(active)) {
      // 优先切到右边最近的剩余标签，没有时切到左边的。
      const i = tabs.findIndex((x) => x.key === active);
      const next = tabs.slice(i + 1).find((x) => !gone.has(x.key)) ?? tabs.slice(0, i).reverse().find((x) => !gone.has(x.key));
      setActive(next?.key ?? null);
    }
  };

  const askClose = (keys: string[]) => {
    const list = tabs.filter((x) => keys.includes(x.key));
    if (list.some(needsConfirm)) setClosing(list);
    else remove(keys);
  };

  const update = (key: string, patch: Partial<Pick<Tab, "status" | "busy">>) =>
    setTabs((list) =>
      list.map((x) => (x.key === key && Object.entries(patch).some(([k, v]) => x[k as keyof Tab] !== v) ? { ...x, ...patch } : x)),
    );

  const reorder = (keys: string[]) =>
    setTabs((list) => {
      const rank = new Map(keys.map((k, i) => [k, i]));
      return [...list].sort((a, b) => (rank.get(a.key) ?? Infinity) - (rank.get(b.key) ?? Infinity));
    });

  const tabMenu = (key: string): MenuItem[] => {
    const i = tabs.findIndex((x) => x.key === key);
    const tab = tabs[i];
    if (!tab) return [];
    return [
      {
        label: t("terminal.duplicateTab"),
        icon: <CopyPlus size={14} />,
        disabled: !names.has(tab.connId),
        onClick: () => open({ id: tab.connId, name: title(tab) }, tab.kind),
      },
      { label: t("terminal.closeTab"), icon: <X size={14} />, divider: true, onClick: () => askClose([key]) },
      {
        label: t("terminal.closeOthers"),
        disabled: tabs.length < 2,
        onClick: () => askClose(tabs.filter((x) => x.key !== key).map((x) => x.key)),
      },
      {
        label: t("terminal.closeRight"),
        disabled: i === tabs.length - 1,
        onClick: () => askClose(tabs.slice(i + 1).map((x) => x.key)),
      },
    ];
  };

  const closeOne = closing.length === 1 ? closing[0] : null;
  const closeName = <span className="font-medium text-ink">{closeOne && title(closeOne)}</span>;

  return (
    <>
      <ModeLayout
        list={
          <ConnList
            title={t("terminal.connections")}
            emptyDesc={t("conn.emptyDesc.terminal")}
            onOpen={(c) => open(c, defaultTab(c))}
            openItems={(c) => (c.kind === "ssh" ? [{ label: t("sftp.open"), icon: <FolderOpen size={14} />, onClick: () => open(c, "sftp") }] : [])}
          />
        }
        icon={SquareTerminal}
        mainTitle={t("terminal.emptyTitle")}
        mainDesc={t("terminal.emptyDesc")}
      >
        {tabs.length > 0 && settings && (
          <>
            <TabBar
              tabs={tabs.map((x) => ({
                key: x.key,
                title: title(x),
                status: x.status,
                danger: production.has(x.connId),
                incognito: incognito.has(x.connId),
                icon: x.kind === "term" ? undefined : <TabIcon kind={x.kind} size={13} className="shrink-0 text-muted" />,
              }))}
              active={active}
              onSelect={setActive}
              onClose={(key) => askClose([key])}
              onReorder={reorder}
              menu={tabMenu}
              actions={
                current?.kind === "term" && (
                  <>
                    <IconButton
                      label={t(monitor ? "monitor.hide" : "monitor.show")}
                      size="sm"
                      className={cx(monitor && "bg-accent/12 text-accent")}
                      onClick={() => showMonitor(!monitor)}
                    >
                      <Activity size={15} />
                    </IconButton>
                    <IconButton
                      label={t("sftp.openForTab")}
                      size="sm"
                      onClick={() => open({ id: current.connId, name: names.get(current.connId) ?? current.name }, "sftp")}
                    >
                      <FolderOpen size={15} />
                    </IconButton>
                  </>
                )
              }
            />
            <div className={cx("flex min-h-0 flex-1", position === "bottom" && "flex-col")}>
              <div className="flex min-h-0 min-w-0 flex-1 flex-col">
                {tabs.map((x) => (
                  <div key={x.key} className={cx("flex min-h-0 flex-1 flex-col", x.key !== active && "hidden")}>
                    {x.status === "idle" ? (
                      <div className="flex flex-1 flex-col justify-center overflow-y-auto px-6">
                        <Empty
                          icon={<TabIcon kind={x.kind} size={22} />}
                          title={title(x)}
                          desc={t("terminal.idleDesc")}
                          action={
                            <Button variant="primary" icon={<PlugZap size={14} />} onClick={() => connect(x)}>
                              {t("terminal.connect")}
                            </Button>
                          }
                        />
                      </div>
                    ) : x.kind === "term" ? (
                      <TerminalView
                        connId={x.connId}
                        name={title(x)}
                        active={x.key === active}
                        production={production.has(x.connId)}
                        incognito={incognito.has(x.connId)}
                        onStatus={(s) => update(x.key, { status: s })}
                      />
                    ) : x.kind === "rdp" ? (
                      <RdpView
                        connId={x.connId}
                        name={title(x)}
                        active={x.key === active}
                        fit={rdpOf(x.connId)?.resolution !== "fixed"}
                        clipboard={rdpOf(x.connId)?.clipboard ?? true}
                        onStatus={(s) => update(x.key, { status: s })}
                      />
                    ) : (
                      <SftpPanel
                        connId={x.connId}
                        name={title(x)}
                        active={x.key === active}
                        onStatus={(s) => update(x.key, { status: s })}
                        onBusy={(n) => update(x.key, { busy: n })}
                      />
                    )}
                  </div>
                ))}
              </div>
              {monitor && current?.kind === "term" && current.status !== "idle" && (
                <MonitorPanel
                  key={current.key}
                  connId={current.connId}
                  connected={current.status === "connected"}
                  position={position}
                  onPosition={moveMonitor}
                  onClose={() => showMonitor(false)}
                />
              )}
            </div>
            <Confirm
              open={closing.length > 0}
              title={t(closeOne ? CLOSE_TITLE[closeOne.kind] : "terminal.closeManyTitle")}
              message={
                !closeOne
                  ? t("terminal.closeManyMessage", { count: closing.length, busy: closing.filter(needsConfirm).length })
                  : closeOne.kind === "sftp"
                    ? tx("sftp.closeMessage", { name: closeName, count: String(closeOne.busy) })
                    : tx(closeOne.kind === "rdp" ? "rdp.closeMessage" : "terminal.closeMessage", { name: closeName })
              }
              confirmText={t("common.close")}
              danger
              onConfirm={() => remove(closing.map((x) => x.key))}
              onClose={() => setClosing([])}
            />
          </>
        )}
      </ModeLayout>
      <QuickConnect
        open={quickConnect}
        recent={recent}
        onClose={onQuickConnectClose}
        onPick={(c, kind) => {
          open(c, kind);
          onShow();
        }}
      />
    </>
  );
}
