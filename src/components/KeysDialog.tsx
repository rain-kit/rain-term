import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import {
  Check,
  Copy,
  Download,
  FileUp,
  KeyRound,
  Lock,
  MoreHorizontal,
  Pencil,
  Search,
  Sparkles,
  Trash2,
  UserPlus,
  UserRound,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { keyApi } from "../lib/api";
import { formatTime, keyTypeLabel } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { useAccounts, useConns, useKeys, useToast } from "../lib/store";
import type { AccountView, ConnView, SshKey } from "../lib/types";
import AccountsPanel from "./AccountsPanel";
import KeyImportDialog, { KeyGenerateDialog } from "./KeyImportDialog";
import KindIcon from "./KindIcon";
import { Button, Card, Confirm, ContextMenu, cx, Dialog, Empty, Field, IconButton, Input, Menu, Segmented, Tag, type MenuItem } from "./ui";

/** 直接选了这个密钥登录的连接（不含通过登录用户使用的）。 */
export const keyUsers = (conns: ConnView[], id: string) =>
  conns.filter((c) => c.kind === "ssh" && !c.accountId && c.auth === "key" && c.keyId === id);

/** 使用这个密钥的登录用户。 */
const keyAccounts = (accounts: AccountView[], id: string) => accounts.filter((a) => a.auth === "key" && a.keyId === id);

export type KeysTab = "keys" | "accounts";

function CountPill({ count, active }: { count: number; active: boolean }) {
  if (!count) return null;
  return (
    <span
      className={cx(
        "rounded-full px-1.5 text-[11px] leading-4 tabular-nums transition-colors",
        active ? "bg-accent/12 text-accent font-semibold" : "bg-surface/80 text-muted",
      )}
    >
      {count}
    </span>
  );
}

/**
 * 密钥/用户窗口：“密钥”页导入、生成、改名、导出和删除密钥，复制公钥；“用户”页管理登录用户。
 * tab 是打开时显示的页；onAdded、onAccountAdded 在新增密钥、新建用户后调用。
 */
export default function KeysDialog({
  open,
  onClose,
  tab: initialTab = "keys",
  onAdded,
  onAccountAdded,
}: {
  open: boolean;
  onClose: () => void;
  tab?: KeysTab;
  onAdded?: (k: SshKey) => void;
  onAccountAdded?: (a: AccountView) => void;
}) {
  const { t, tx } = useI18n();
  const toast = useToast();
  const { keys } = useKeys();
  const { accounts } = useAccounts();
  const { conns } = useConns();
  const [tab, setTab] = useState<KeysTab>(initialTab);
  const [q, setQ] = useState("");
  const [importing, setImporting] = useState(false);
  const [generating, setGenerating] = useState(false);
  const [creatingAccount, setCreatingAccount] = useState(false);
  const [renaming, setRenaming] = useState<SshKey | null>(null);
  const [newName, setNewName] = useState("");
  const [deleting, setDeleting] = useState<SshKey | null>(null);
  const [refInfo, setRefInfo] = useState<{ mode: "inUse" | "usage"; key: SshKey; users: ConnView[]; accounts: AccountView[] } | null>(null);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [ctxMenu, setCtxMenu] = useState<{ at: { x: number; y: number }; key: SshKey } | null>(null);
  const copyTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!open) return;
    setTab(initialTab);
    setQ("");
  }, [open, initialTab]);

  useEffect(
    () => () => {
      if (copyTimer.current) clearTimeout(copyTimer.current);
    },
    [],
  );

  const switchTab = (next: KeysTab) => {
    setTab(next);
    setQ("");
  };

  const users = useMemo(() => new Map(keys.map((k) => [k.id, keyUsers(conns, k.id)])), [keys, conns]);
  const usedByAccounts = useMemo(() => new Map(keys.map((k) => [k.id, keyAccounts(accounts, k.id)])), [keys, accounts]);

  const filteredKeys = useMemo(() => {
    const s = q.trim().toLowerCase();
    if (!s) return keys;
    return keys.filter((k) => [k.name, keyTypeLabel(k), k.fingerprint].some((v) => v.toLowerCase().includes(s)));
  }, [keys, q]);

  const copyPublic = (k: SshKey) =>
    writeText(k.publicKey).then(() => {
      toast.success(t("keys.publicCopied"));
      setCopiedId(k.id);
      if (copyTimer.current) clearTimeout(copyTimer.current);
      copyTimer.current = setTimeout(() => setCopiedId((cur) => (cur === k.id ? null : cur)), 1500);
    }, toast.error);

  const exportKey = async (k: SshKey) => {
    try {
      const path = await keyApi.exportTo(k.id);
      if (path) toast.success(t("keys.exported", { path }));
    } catch (e) {
      toast.error(e);
    }
  };

  const askDelete = (k: SshKey) => {
    const list = users.get(k.id) ?? [];
    const accs = usedByAccounts.get(k.id) ?? [];
    if (list.length || accs.length) setRefInfo({ mode: "inUse", key: k, users: list, accounts: accs });
    else setDeleting(k);
  };

  const startRename = (k: SshKey) => {
    setNewName(k.name);
    setRenaming(k);
  };

  const rename = async () => {
    if (!renaming || !newName.trim()) return;
    try {
      await keyApi.rename(renaming.id, newName);
      setRenaming(null);
    } catch (e) {
      toast.error(e);
    }
  };

  const keyMenuItems = (k: SshKey, includeCopy = false): MenuItem[] => [
    ...(includeCopy ? [{ label: t("keys.copyPublic"), icon: <Copy size={14} />, onClick: () => void copyPublic(k) }] : []),
    { label: t("keys.rename"), icon: <Pencil size={14} />, onClick: () => startRename(k) },
    { label: t("keys.export"), icon: <Download size={14} />, onClick: () => exportKey(k) },
    { label: t("common.delete"), icon: <Trash2 size={14} />, danger: true, divider: true, onClick: () => askDelete(k) },
  ];

  const hasItems = tab === "keys" ? keys.length > 0 : accounts.length > 0;

  return (
    <>
      <Dialog
        open={open}
        onClose={onClose}
        divided
        bodyClassName="px-5 py-3.5"
        title={
          <span className="flex items-center gap-2">
            <span className="flex size-6 items-center justify-center rounded-md bg-accent/12 text-accent">
              <KeyRound size={14} />
            </span>
            {t("keys.title")}
          </span>
        }
        width="max-w-2xl"
        footer={
          <>
            <span className="flex-1 self-center text-xs text-muted">{tab === "accounts" && t("accounts.footerHint")}</span>
            <Button onClick={onClose}>{t("common.close")}</Button>
          </>
        }
      >
        <div className="flex flex-col gap-2.5">
          <div className="flex flex-wrap items-center gap-2">
            <Segmented<KeysTab>
              value={tab}
              onChange={switchTab}
              options={[
                {
                  value: "keys",
                  label: (
                    <>
                      <KeyRound size={13} />
                      <span>{t("keys.tab.keys")}</span>
                      <CountPill count={keys.length} active={tab === "keys"} />
                    </>
                  ),
                },
                {
                  value: "accounts",
                  label: (
                    <>
                      <UserRound size={13} />
                      <span>{t("keys.tab.accounts")}</span>
                      <CountPill count={accounts.length} active={tab === "accounts"} />
                    </>
                  ),
                },
              ]}
            />
            <span className="flex-1" />
            {hasItems && (
              <div className="w-40">
                <Input
                  className="h-8 text-xs"
                  placeholder={t(tab === "keys" ? "keys.search" : "accounts.search")}
                  value={q}
                  onChange={(e) => setQ(e.target.value)}
                  suffix={
                    q ? (
                      <IconButton label={t("keys.clear")} size="sm" type="button" onClick={() => setQ("")}>
                        <X size={13} />
                      </IconButton>
                    ) : (
                      <Search size={13} className="mr-1.5 text-muted" />
                    )
                  }
                />
              </div>
            )}
            {tab === "keys" ? (
              <>
                <Button size="sm" icon={<FileUp size={13} />} onClick={() => setImporting(true)}>
                  {t("keys.importTitle")}
                </Button>
                <Button size="sm" variant="primary" icon={<Sparkles size={13} />} onClick={() => setGenerating(true)}>
                  {t("keys.generateTitle")}
                </Button>
              </>
            ) : (
              <Button size="sm" variant="primary" icon={<UserPlus size={13} />} onClick={() => setCreatingAccount(true)}>
                {t("accounts.newTitle")}
              </Button>
            )}
          </div>

          <p className="text-xs text-muted">{t(tab === "keys" ? "keys.desc" : "accounts.desc")}</p>

          <div className="h-84 max-h-[52vh] min-h-56 overflow-y-auto">
            {tab === "accounts" ? (
              <AccountsPanel
                query={q}
                creating={creatingAccount}
                onCreateOpen={() => setCreatingAccount(true)}
                onCreateClose={() => setCreatingAccount(false)}
                onAdded={onAccountAdded}
              />
            ) : keys.length === 0 ? (
              <Empty
                compact
                icon={<KeyRound size={22} />}
                title={t("keys.empty")}
                desc={t("keys.emptyDesc")}
                action={
                  <div className="flex items-center gap-2">
                    <Button size="sm" icon={<FileUp size={14} />} onClick={() => setImporting(true)}>
                      {t("keys.importTitle")}
                    </Button>
                    <Button size="sm" variant="primary" icon={<Sparkles size={14} />} onClick={() => setGenerating(true)}>
                      {t("keys.generateTitle")}
                    </Button>
                  </div>
                }
              />
            ) : filteredKeys.length === 0 ? (
              <div className="py-12 text-center text-xs text-muted">{t("keys.noMatch")}</div>
            ) : (
              <Card className="divide-y divide-line overflow-hidden">
                {filteredKeys.map((k) => {
                  const connList = users.get(k.id) ?? [];
                  const accList = usedByAccounts.get(k.id) ?? [];
                  const count = connList.length;
                  const accountCount = accList.length;
                  const used = count > 0 || accountCount > 0;
                  const copied = copiedId === k.id;
                  const usageSummary = [
                    count > 0 ? t("keys.usedBy", { count }) : "",
                    accountCount > 0 ? t("keys.usedByAccounts", { count: accountCount }) : "",
                  ]
                    .filter(Boolean)
                    .join(" · ");
                  const usageNames = [...connList.map((u) => u.name), ...accList.map((a) => a.name)].join("、");

                  return (
                    <div
                      key={k.id}
                      onDoubleClick={() => startRename(k)}
                      onContextMenu={(e) => {
                        e.preventDefault();
                        setCtxMenu({ at: { x: e.clientX, y: e.clientY }, key: k });
                      }}
                      className="group flex items-center gap-3 px-3.5 py-2.5 transition-colors hover:bg-subtle/60"
                    >
                      <div className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-amber-500/10 text-amber-600 dark:bg-amber-400/15 dark:text-amber-400">
                        <KeyRound size={17} />
                      </div>
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-1.5">
                          <span className="truncate text-sm font-medium">{k.name}</span>
                          <Tag>{keyTypeLabel(k)}</Tag>
                          {k.encrypted && (
                            <Tag>
                              <Lock size={9} />
                              {t("keys.encrypted")}
                            </Tag>
                          )}
                        </div>
                        <div className="mt-0.5 flex items-center gap-1.5 truncate text-[11px] text-muted">
                          <span className="truncate font-mono" title={k.fingerprint}>
                            {k.fingerprint}
                          </span>
                          {k.createdAt && (
                            <>
                              <span className="opacity-50">·</span>
                              <span className="shrink-0 tabular-nums opacity-80">{formatTime(Date.parse(k.createdAt) / 1000)}</span>
                            </>
                          )}
                        </div>
                      </div>
                      <div className="shrink-0">
                        {used ? (
                          <button
                            type="button"
                            title={usageNames}
                            onClick={() => setRefInfo({ mode: "usage", key: k, users: connList, accounts: accList })}
                            className="rounded-md bg-accent/10 px-2 py-0.5 text-xs font-medium text-accent transition-colors hover:bg-accent/18"
                          >
                            {usageSummary}
                          </button>
                        ) : (
                          <span className="px-1.5 text-xs text-muted/70">{t("keys.unused")}</span>
                        )}
                      </div>
                      <div className="flex shrink-0 items-center gap-0.5">
                        <IconButton label={t("keys.copyPublic")} size="sm" onClick={() => copyPublic(k)}>
                          {copied ? <Check size={14} className="text-emerald-500" /> : <Copy size={14} />}
                        </IconButton>
                        <Menu
                          items={keyMenuItems(k)}
                          trigger={(openMenu) => (
                            <IconButton label={t("common.more")} size="sm" onClick={openMenu}>
                              <MoreHorizontal size={15} />
                            </IconButton>
                          )}
                        />
                      </div>
                    </div>
                  );
                })}
              </Card>
            )}
          </div>
        </div>
      </Dialog>

      <ContextMenu
        at={ctxMenu?.at ?? null}
        items={ctxMenu ? keyMenuItems(ctxMenu.key, true) : []}
        onClose={() => setCtxMenu(null)}
      />

      <KeyImportDialog open={importing} onClose={() => setImporting(false)} onDone={onAdded} />
      <KeyGenerateDialog open={generating} onClose={() => setGenerating(false)} onDone={onAdded} />

      <Dialog
        open={!!renaming}
        onClose={() => setRenaming(null)}
        title={t("keys.rename")}
        width="max-w-sm"
        footer={
          <>
            <Button onClick={() => setRenaming(null)}>{t("common.cancel")}</Button>
            <Button variant="primary" onClick={rename} disabled={!newName.trim()}>
              {t("common.save")}
            </Button>
          </>
        }
      >
        <Field label={t("keys.name")} className="pb-2">
          <Input autoFocus value={newName} onChange={(e) => setNewName(e.target.value)} onKeyDown={(e) => e.key === "Enter" && rename()} />
        </Field>
      </Dialog>

      <Confirm
        open={!!deleting}
        title={t("keys.deleteTitle")}
        message={tx("keys.deleteMessage", { name: <span className="font-medium text-ink">{deleting?.name}</span> })}
        confirmText={t("common.delete")}
        danger
        onConfirm={() =>
          deleting &&
          keyApi.remove(deleting.id).then(
            () => toast.success(t("keys.deleted")),
            (e) => toast.error(e),
          )
        }
        onClose={() => setDeleting(null)}
      />

      <Dialog
        open={!!refInfo}
        onClose={() => setRefInfo(null)}
        title={t(refInfo?.mode === "usage" ? "keys.usageTitle" : "conn.inUseTitle")}
        width="max-w-sm"
        footer={<Button onClick={() => setRefInfo(null)}>{t("common.close")}</Button>}
      >
        <div className="text-sm text-muted">
          {tx(refInfo?.mode === "usage" ? "keys.usageMessage" : "keys.inUseMessage", {
            name: <span className="font-medium text-ink">{refInfo?.key.name}</span>,
          })}
        </div>
        <ul className="mt-2.5 mb-1 flex max-h-48 flex-col gap-1.5 overflow-y-auto">
          {refInfo?.users.map((u) => (
            <li key={u.id} className="flex items-center gap-2 rounded-lg bg-subtle/60 px-2.5 py-1.5 text-sm">
              <KindIcon kind={u.kind} />
              <span className="truncate">{u.name}</span>
            </li>
          ))}
          {refInfo?.accounts.map((a) => (
            <li key={a.id} className="flex items-center gap-2 rounded-lg bg-subtle/60 px-2.5 py-1.5 text-sm">
              <span className="flex size-6 shrink-0 items-center justify-center rounded-md bg-accent/12 text-accent">
                <UserRound size={13} />
              </span>
              <span className="truncate">{a.name}</span>
            </li>
          ))}
        </ul>
      </Dialog>
    </>
  );
}
