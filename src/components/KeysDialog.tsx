import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { Copy, Download, FileUp, KeyRound, Lock, MoreHorizontal, Pencil, Sparkles, Trash2, UserRound } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { keyApi } from "../lib/api";
import { formatTime, keyTypeLabel } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { useAccounts, useConns, useKeys, useToast } from "../lib/store";
import type { AccountView, ConnView, KeyType, SshKey } from "../lib/types";
import AccountsPanel from "./AccountsPanel";
import KeyImportDialog from "./KeyImportDialog";
import KindIcon from "./KindIcon";
import { Button, Card, Confirm, Dialog, Empty, Field, IconButton, Input, Menu, PasswordInput, Segmented, Select, Tag } from "./ui";

/** 直接选了这个密钥登录的连接（不含通过登录用户使用的）。 */
export const keyUsers = (conns: ConnView[], id: string) =>
  conns.filter((c) => c.kind === "ssh" && !c.accountId && c.auth === "key" && c.keyId === id);

/** 使用这个密钥的登录用户。 */
const keyAccounts = (accounts: AccountView[], id: string) => accounts.filter((a) => a.auth === "key" && a.keyId === id);

export type KeysTab = "keys" | "accounts";

const RSA_BITS = ["2048", "3072", "4096"] as const;

/** 生成新的密钥对，私钥保存在程序里，公钥复制到服务器的 authorized_keys。 */
function KeyGenerateDialog({ open, onClose, onDone }: { open: boolean; onClose: () => void; onDone?: (k: SshKey) => void }) {
  const { t } = useI18n();
  const toast = useToast();
  const [name, setName] = useState("");
  const [type, setType] = useState<KeyType>("ed25519");
  const [bits, setBits] = useState<(typeof RSA_BITS)[number]>("4096");
  const [passphrase, setPassphrase] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setName("");
    setType("ed25519");
    setBits("4096");
    setPassphrase("");
  }, [open]);

  const submit = async () => {
    setBusy(true);
    try {
      const k = await keyApi.generate({
        name: name.trim() || `id_${type}`,
        type,
        bits: type === "rsa" ? Number(bits) : undefined,
        passphrase,
      });
      toast.success(t("keys.generated", { name: k.name }));
      onDone?.(k);
      onClose();
    } catch (e) {
      toast.error(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={open}
      onClose={busy ? undefined : onClose}
      title={t("keys.generateTitle")}
      width="max-w-md"
      footer={
        <>
          <Button onClick={onClose} disabled={busy}>
            {t("common.cancel")}
          </Button>
          <Button variant="primary" onClick={submit} disabled={busy}>
            {t(busy ? "keys.generating" : "keys.generate")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 pb-2">
        <Field label={t("keys.name")}>
          <Input placeholder={`id_${type}`} value={name} onChange={(e) => setName(e.target.value)} />
        </Field>
        <div className="flex gap-3">
          <Field label={t("keys.type")} hint={t("keys.typeHint")} className="flex-1">
            <Segmented<KeyType>
              value={type}
              onChange={setType}
              options={[
                { value: "ed25519", label: "ED25519" },
                { value: "ecdsa", label: "ECDSA" },
                { value: "rsa", label: "RSA" },
              ]}
            />
          </Field>
          {type === "rsa" && (
            <Field label={t("keys.bits")} className="w-24 shrink-0">
              <Select value={bits} onChange={setBits} options={RSA_BITS.map((b) => ({ value: b, label: b }))} />
            </Field>
          )}
        </div>
        <Field label={t("keys.passphrase")} hint={t("keys.generatePassphraseHint")}>
          <PasswordInput value={passphrase} onChange={(e) => setPassphrase(e.target.value)} />
        </Field>
      </div>
    </Dialog>
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
  const [importing, setImporting] = useState(false);
  const [generating, setGenerating] = useState(false);
  const [renaming, setRenaming] = useState<SshKey | null>(null);
  const [newName, setNewName] = useState("");
  const [deleting, setDeleting] = useState<SshKey | null>(null);
  const [inUse, setInUse] = useState<{ key: SshKey; users: ConnView[]; accounts: AccountView[] } | null>(null);

  useEffect(() => {
    if (open) setTab(initialTab);
  }, [open, initialTab]);

  const users = useMemo(() => new Map(keys.map((k) => [k.id, keyUsers(conns, k.id)])), [keys, conns]);
  const usedByAccounts = useMemo(() => new Map(keys.map((k) => [k.id, keyAccounts(accounts, k.id)])), [keys, accounts]);

  const copyPublic = (k: SshKey) => writeText(k.publicKey).then(() => toast.success(t("keys.publicCopied")), toast.error);

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
    if (list.length || accs.length) setInUse({ key: k, users: list, accounts: accs });
    else setDeleting(k);
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

  const tabs = (
    <Segmented<KeysTab>
      value={tab}
      onChange={setTab}
      options={[
        { value: "keys", label: t("keys.tab.keys") },
        { value: "accounts", label: t("keys.tab.accounts") },
      ]}
    />
  );

  return (
    <>
      <Dialog
        open={open}
        onClose={onClose}
        title={t("keys.title")}
        width="max-w-2xl"
        footer={
          <>
            <span className="flex-1 self-center text-xs text-muted">{t(tab === "keys" ? "keys.footerHint" : "accounts.footerHint")}</span>
            <Button onClick={onClose}>{t("common.close")}</Button>
          </>
        }
      >
        <div className="flex min-h-72 flex-col gap-3 pb-1">
          {tab === "accounts" ? (
            <AccountsPanel tabs={tabs} onAdded={onAccountAdded} />
          ) : (
            <>
              <div className="flex items-center gap-2">
                {tabs}
                <span className="flex-1" />
                <Button size="sm" icon={<FileUp size={13} />} onClick={() => setImporting(true)}>
                  {t("keys.importTitle")}
                </Button>
                <Button size="sm" icon={<Sparkles size={13} />} onClick={() => setGenerating(true)}>
                  {t("keys.generateTitle")}
                </Button>
              </div>
              <span className="text-xs text-muted">{t("keys.desc")}</span>
              {keys.length === 0 ? (
                <Empty icon={<KeyRound size={22} />} title={t("keys.empty")} desc={t("keys.emptyDesc")} />
              ) : (
                <Card className="divide-y divide-line">
                  {keys.map((k) => {
                    const count = users.get(k.id)?.length ?? 0;
                    const accountCount = usedByAccounts.get(k.id)?.length ?? 0;
                    return (
                      <div key={k.id} className="flex items-center gap-3 px-3.5 py-2.5">
                        <div className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-subtle text-muted">
                          <KeyRound size={16} />
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
                          <div className="mt-0.5 truncate font-mono text-[11px] text-muted" title={k.fingerprint}>
                            {k.fingerprint}
                          </div>
                        </div>
                        <div className="shrink-0 text-right text-xs text-muted">
                          {count > 0 && <div>{t("keys.usedBy", { count })}</div>}
                          {accountCount > 0 && <div>{t("keys.usedByAccounts", { count: accountCount })}</div>}
                          {!count && !accountCount && <div>{t("keys.unused")}</div>}
                          {k.createdAt && <div className="mt-0.5 text-[11px] opacity-80">{formatTime(Date.parse(k.createdAt) / 1000)}</div>}
                        </div>
                        <IconButton label={t("keys.copyPublic")} size="sm" onClick={() => copyPublic(k)}>
                          <Copy size={14} />
                        </IconButton>
                        <Menu
                          items={[
                            {
                              label: t("keys.rename"),
                              icon: <Pencil size={14} />,
                              onClick: () => {
                                setNewName(k.name);
                                setRenaming(k);
                              },
                            },
                            { label: t("keys.export"), icon: <Download size={14} />, onClick: () => exportKey(k) },
                            { label: t("common.delete"), icon: <Trash2 size={14} />, danger: true, divider: true, onClick: () => askDelete(k) },
                          ]}
                          trigger={(openMenu) => (
                            <IconButton label={t("common.more")} size="sm" onClick={openMenu}>
                              <MoreHorizontal size={15} />
                            </IconButton>
                          )}
                        />
                      </div>
                    );
                  })}
                </Card>
              )}
            </>
          )}
        </div>
      </Dialog>

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
        open={!!inUse}
        onClose={() => setInUse(null)}
        title={t("conn.inUseTitle")}
        width="max-w-sm"
        footer={<Button onClick={() => setInUse(null)}>{t("common.close")}</Button>}
      >
        <div className="text-sm text-muted">{tx("keys.inUseMessage", { name: <span className="font-medium text-ink">{inUse?.key.name}</span> })}</div>
        <ul className="mt-2 mb-1 flex flex-col gap-1">
          {inUse?.users.map((u) => (
            <li key={u.id} className="flex items-center gap-2 text-sm">
              <KindIcon kind={u.kind} />
              <span className="truncate">{u.name}</span>
            </li>
          ))}
          {inUse?.accounts.map((a) => (
            <li key={a.id} className="flex items-center gap-2 text-sm">
              <UserRound size={14} className="shrink-0 text-muted" />
              <span className="truncate">{a.name}</span>
            </li>
          ))}
        </ul>
      </Dialog>
    </>
  );
}
