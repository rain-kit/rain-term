import { FileUp, KeyRound, Lock, Pencil, Trash2, UserPlus, UserRound } from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { accountApi } from "../lib/api";
import { formatTime, KIND_LABEL, keyTypeLabel } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { useAccounts, useConns, useKeys, useToast } from "../lib/store";
import type { AccountAuth, AccountScope, AccountView, ConnKind, ConnView, SshKey } from "../lib/types";
import KeyImportDialog from "./KeyImportDialog";
import KindIcon from "./KindIcon";
import { Button, Card, Confirm, Dialog, Empty, Field, IconButton, Input, PasswordInput, Segmented, Select, Tag } from "./ui";

/** 登录用户能否用于这种连接。 */
export const accountAllows = (a: { scope: AccountScope }, kind: ConnKind) => a.scope === "all" || a.scope === kind;

/** 使用这个登录用户的连接。 */
export const accountUsers = (conns: ConnView[], id: string) => conns.filter((c) => c.accountId === id);

/** 下拉框里显示的名称：名称和用户名不同时两个都显示。 */
export const accountLabel = (a: AccountView) => (a.name === a.user ? a.user : `${a.name} · ${a.user}`);

interface Form {
  name: string;
  user: string;
  auth: AccountAuth;
  password: string;
  keyId: string;
  scope: AccountScope;
}

type Errors = Partial<Record<keyof Form, string>>;

const toForm = (a?: AccountView | null): Form => ({
  name: a && a.name !== a.user ? a.name : "",
  user: a?.user ?? "",
  auth: a?.auth ?? "password",
  password: "",
  keyId: a?.keyId ?? "",
  scope: a?.scope ?? "all",
});

/** 新建或编辑登录用户。onDone 在保存成功后调用。 */
function AccountEditor({
  open,
  account,
  onClose,
  onDone,
}: {
  open: boolean;
  /** 为 null 时新建。 */
  account: AccountView | null;
  onClose: () => void;
  onDone?: (a: AccountView) => void;
}) {
  const { t } = useI18n();
  const toast = useToast();
  const { keys } = useKeys();
  const [form, setForm] = useState<Form>(() => toForm(account));
  const [errors, setErrors] = useState<Errors>({});
  const [busy, setBusy] = useState(false);
  const [importing, setImporting] = useState(false);

  useEffect(() => {
    if (!open) return;
    setForm(toForm(account));
    setErrors({});
  }, [open, account]);

  const f = form;
  const set = <K extends keyof Form>(k: K, v: Form[K]) => {
    setForm((x) => ({ ...x, [k]: v }));
    if (errors[k]) setErrors((e) => ({ ...e, [k]: undefined }));
  };

  const keyOptions = useMemo(() => {
    const opts = keys.map((k) => ({ value: k.id, label: `${k.name} · ${keyTypeLabel(k)}` }));
    if (f.keyId && !keys.some((k) => k.id === f.keyId)) opts.push({ value: f.keyId, label: t("editor.keyMissing") });
    return [{ value: "", label: t(keys.length ? "editor.keyNone" : "editor.keyEmpty") }, ...opts];
  }, [keys, f.keyId, t]);

  const submit = async () => {
    const errs: Errors = {};
    if (!f.user.trim()) errs.user = t("editor.required");
    if (f.auth === "key" && !f.keyId) errs.keyId = t("editor.err.keyRequired");
    setErrors(errs);
    if (Object.keys(errs).length) return;
    setBusy(true);
    try {
      const key = f.auth === "key";
      const saved = await accountApi.save({
        id: account?.id ?? "",
        name: f.name.trim(),
        user: f.user.trim(),
        auth: f.auth,
        keyId: key ? f.keyId : "",
        scope: key ? "ssh" : f.scope,
        password: key ? "" : f.password,
      });
      toast.success(t(account ? "editor.saved" : "editor.created"));
      onDone?.(saved);
      onClose();
    } catch (e) {
      toast.error(e);
    } finally {
      setBusy(false);
    }
  };

  const saved = account?.auth === "password" && account.hasPassword;
  return (
    <Dialog
      open={open}
      onClose={busy ? undefined : onClose}
      title={t(account ? "accounts.editTitle" : "accounts.newTitle")}
      width="max-w-md"
      footer={
        <>
          <Button onClick={onClose} disabled={busy}>
            {t("common.cancel")}
          </Button>
          <Button variant="primary" onClick={submit} disabled={busy}>
            {t("common.save")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 pb-2">
        <Field label={t("accounts.name")}>
          <Input placeholder={t("accounts.namePlaceholder")} value={f.name} onChange={(e) => set("name", e.target.value)} />
        </Field>
        <Field label={t("editor.user")} error={errors.user} hint={t("accounts.userHint")}>
          <Input value={f.user} invalid={!!errors.user} onChange={(e) => set("user", e.target.value)} />
        </Field>
        <Field label={t("editor.authType")}>
          <Segmented<AccountAuth>
            value={f.auth}
            onChange={(v) => set("auth", v)}
            options={[
              { value: "password", label: t("editor.auth.password") },
              { value: "key", label: t("editor.auth.key") },
            ]}
          />
        </Field>
        {f.auth === "password" ? (
          <Field label={t("editor.password")} hint={saved ? t("editor.secretSaved") : undefined}>
            <PasswordInput value={f.password} placeholder={saved ? "••••••••" : undefined} onChange={(e) => set("password", e.target.value)} />
          </Field>
        ) : (
          <Field label={t("editor.key")} error={errors.keyId}>
            <div className="flex gap-2">
              <Select className="min-w-0 flex-1" value={f.keyId} onChange={(v) => set("keyId", v)} options={keyOptions} />
              <Button type="button" icon={<FileUp size={14} />} onClick={() => setImporting(true)}>
                {t("keys.import")}
              </Button>
            </div>
          </Field>
        )}
        <Field label={t("accounts.scope")} hint={t(f.auth === "key" ? "accounts.scopeKeyHint" : "accounts.scopeHint")}>
          {f.auth === "key" ? (
            <Segmented<AccountScope> value="ssh" onChange={() => {}} options={[{ value: "ssh", label: KIND_LABEL.ssh }]} />
          ) : (
            <Segmented<AccountScope>
              value={f.scope}
              onChange={(v) => set("scope", v)}
              options={[
                { value: "all", label: t("accounts.scope.all") },
                { value: "ssh", label: KIND_LABEL.ssh },
                { value: "rdp", label: KIND_LABEL.rdp },
              ]}
            />
          )}
        </Field>
      </div>
      <KeyImportDialog open={importing} onClose={() => setImporting(false)} onDone={(k: SshKey) => set("keyId", k.id)} />
    </Dialog>
  );
}

/** 密钥窗口的“用户”页：新建、编辑和删除登录用户。tabs 是页签切换，放在工具栏左侧。 */
export default function AccountsPanel({ tabs, onAdded }: { tabs: ReactNode; onAdded?: (a: AccountView) => void }) {
  const { t, tx } = useI18n();
  const toast = useToast();
  const { accounts } = useAccounts();
  const { keys } = useKeys();
  const { conns } = useConns();
  const [editing, setEditing] = useState<AccountView | null>(null);
  const [creating, setCreating] = useState(false);
  const [deleting, setDeleting] = useState<AccountView | null>(null);
  const [inUse, setInUse] = useState<{ account: AccountView; users: ConnView[] } | null>(null);

  const users = useMemo(() => new Map(accounts.map((a) => [a.id, accountUsers(conns, a.id)])), [accounts, conns]);
  const keyName = (id?: string) => keys.find((k) => k.id === id)?.name ?? t("editor.keyMissing");

  const askDelete = (a: AccountView) => {
    const list = users.get(a.id) ?? [];
    if (list.length) setInUse({ account: a, users: list });
    else setDeleting(a);
  };

  return (
    <>
      <div className="flex items-center gap-2">
        {tabs}
        <span className="flex-1" />
        <Button size="sm" icon={<UserPlus size={13} />} onClick={() => setCreating(true)}>
          {t("accounts.newTitle")}
        </Button>
      </div>
      <span className="text-xs text-muted">{t("accounts.desc")}</span>
      {accounts.length === 0 ? (
        <Empty icon={<UserRound size={22} />} title={t("accounts.empty")} desc={t("accounts.emptyDesc")} />
      ) : (
        <Card className="divide-y divide-line">
          {accounts.map((a) => {
            const count = users.get(a.id)?.length ?? 0;
            return (
              <div key={a.id} className="flex items-center gap-3 px-3.5 py-2.5">
                <div className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-subtle text-muted">
                  <UserRound size={16} />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <span className="truncate text-sm font-medium">{a.name}</span>
                    <Tag>{a.scope === "all" ? t("accounts.scope.all") : KIND_LABEL[a.scope]}</Tag>
                    {a.auth === "key" ? (
                      <Tag>
                        <KeyRound size={9} />
                        {keyName(a.keyId)}
                      </Tag>
                    ) : (
                      <Tag>
                        <Lock size={9} />
                        {t(a.hasPassword ? "editor.auth.password" : "accounts.noPassword")}
                      </Tag>
                    )}
                  </div>
                  <div className="mt-0.5 truncate font-mono text-[11px] text-muted">{a.user}</div>
                </div>
                <div className="shrink-0 text-right text-xs text-muted">
                  <div>{count ? t("keys.usedBy", { count }) : t("keys.unused")}</div>
                  {a.createdAt && <div className="mt-0.5 text-[11px] opacity-80">{formatTime(Date.parse(a.createdAt) / 1000)}</div>}
                </div>
                <IconButton label={t("accounts.edit")} size="sm" onClick={() => setEditing(a)}>
                  <Pencil size={14} />
                </IconButton>
                <IconButton label={t("common.delete")} size="sm" onClick={() => askDelete(a)}>
                  <Trash2 size={14} />
                </IconButton>
              </div>
            );
          })}
        </Card>
      )}

      <AccountEditor open={creating} account={null} onClose={() => setCreating(false)} onDone={onAdded} />
      <AccountEditor open={!!editing} account={editing} onClose={() => setEditing(null)} />

      <Confirm
        open={!!deleting}
        title={t("accounts.deleteTitle")}
        message={tx("accounts.deleteMessage", { name: <span className="font-medium text-ink">{deleting?.name}</span> })}
        confirmText={t("common.delete")}
        danger
        onConfirm={() =>
          deleting &&
          accountApi.remove(deleting.id).then(
            () => toast.success(t("accounts.deleted")),
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
        <div className="text-sm text-muted">{tx("accounts.inUseMessage", { name: <span className="font-medium text-ink">{inUse?.account.name}</span> })}</div>
        <ul className="mt-2 mb-1 flex flex-col gap-1">
          {inUse?.users.map((u) => (
            <li key={u.id} className="flex items-center gap-2 text-sm">
              <KindIcon kind={u.kind} />
              <span className="truncate">{u.name}</span>
            </li>
          ))}
        </ul>
      </Dialog>
    </>
  );
}
